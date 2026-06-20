//! Built-in local LLM engine.
//!
//! Manages a bundled `llama.cpp` server (`llama-server`) as a child process so
//! the "Built-in (Local)" assistant/post-processing provider works with zero
//! setup: no separate Ollama/LM Studio install and no API key. The user simply
//! downloads a GGUF model from the Models tab; this manager starts the engine
//! against that file on a loopback port and the existing OpenAI-compatible
//! `llm_client` talks to it like any other provider.
//!
//! The engine binary is resolved (in order) from:
//!   1. the `HANDY_LLAMA_SERVER` environment variable,
//!   2. a bundled resource at `resources/bin/llama-server[.exe]`,
//!   3. `<app-data>/models/engine/` (auto-downloaded on first use),
//!   4. the system `PATH`.
//!
//! If none is present, the manager downloads the official llama.cpp build for
//! this platform into the app-data engine directory on first use, so the
//! feature works with zero manual setup.

use crate::managers::model::{EngineType, ModelManager};
use anyhow::Result;
use log::{debug, error, info, warn};
use serde::Serialize;
use specta::Type;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Loopback port the bundled engine listens on. Deliberately different from
/// Ollama's default (11434) so a user's existing Ollama install is untouched.
const ENGINE_PORT: u16 = 11435;

/// Context window passed to the engine. Kept modest so memory stays reasonable
/// on the small models this feature targets.
const CONTEXT_SIZE: u32 = 4096;

/// Max time to wait for the engine to load a model and start accepting
/// connections. Large models on slow disks can take a while on first load.
const READY_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Serialize, Type)]
pub struct LocalLlmStatus {
    /// Whether the engine process is currently running.
    pub running: bool,
    /// The model id the engine is currently serving (if any).
    pub model_id: Option<String>,
    /// Whether an engine binary could be located on this machine.
    pub engine_present: bool,
    /// Loopback port the engine serves on.
    pub port: u16,
    /// The last start error, if the most recent start attempt failed.
    pub error: Option<String>,
}

struct ServerState {
    child: Option<Child>,
    model_id: Option<String>,
    last_error: Option<String>,
}

pub struct LocalLlmManager {
    app_handle: AppHandle,
    models_dir: PathBuf,
    port: u16,
    state: Arc<Mutex<ServerState>>,
    /// Serializes concurrent `ensure_running` calls (e.g. two assistant turns
    /// fired in quick succession) so only one start happens at a time.
    start_lock: tokio::sync::Mutex<()>,
}

impl LocalLlmManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let models_dir = crate::portable::app_data_dir(app_handle)
            .map_err(|e| anyhow::anyhow!("Failed to get app data dir: {}", e))?
            .join("models");

        Ok(Self {
            app_handle: app_handle.clone(),
            models_dir,
            port: ENGINE_PORT,
            state: Arc::new(Mutex::new(ServerState {
                child: None,
                model_id: None,
                last_error: None,
            })),
            start_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// Platform-specific engine binary filename.
    fn engine_filename() -> &'static str {
        if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        }
    }

    /// Locate the engine binary, or `None` if it isn't installed/bundled.
    pub fn resolve_engine_binary(&self) -> Option<PathBuf> {
        // 1. Explicit override via environment variable.
        if let Some(path) = std::env::var_os("HANDY_LLAMA_SERVER") {
            let pb = PathBuf::from(path);
            if pb.is_file() {
                return Some(pb);
            }
        }

        // 2. Bundled resource (shipped with the app installer).
        if let Ok(pb) = self.app_handle.path().resolve(
            format!("resources/bin/{}", Self::engine_filename()),
            tauri::path::BaseDirectory::Resource,
        ) {
            if pb.is_file() {
                return Some(pb);
            }
        }

        // 3. In the app-data engine directory (auto-downloaded on first use).
        //    Search recursively because release archives may nest the binary
        //    inside a subfolder; its DLLs sit alongside it either way.
        if let Some(found) =
            Self::find_binary_in(&self.models_dir.join("engine"), Self::engine_filename())
        {
            return Some(found);
        }

        // 4. On the system PATH.
        Self::find_in_path(Self::engine_filename())
    }

    fn find_in_path(name: &str) -> Option<PathBuf> {
        let paths = std::env::var_os("PATH")?;
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }

    /// Recursively search `dir` (bounded, breadth-first) for a file named
    /// `name`, returning its full path.
    fn find_binary_in(dir: &Path, name: &str) -> Option<PathBuf> {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let entries = match std::fs::read_dir(&d) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
                    return Some(path);
                }
            }
        }
        None
    }

    /// Ensure an engine binary is available, downloading the official
    /// llama.cpp release for this platform into `<models>/engine/` if none is
    /// found anywhere. Returns the path to the binary.
    async fn ensure_engine_installed(&self) -> Result<PathBuf, String> {
        if let Some(path) = self.resolve_engine_binary() {
            return Ok(path);
        }

        let engine_dir = self.models_dir.join("engine");
        std::fs::create_dir_all(&engine_dir)
            .map_err(|e| format!("Failed to create engine directory: {}", e))?;

        info!("Built-in LLM engine not found; downloading llama.cpp build...");
        let _ = self
            .app_handle
            .emit("local-llm-engine-status", "downloading");

        let url = self.resolve_engine_asset_url().await?;
        let zip_path = engine_dir.join("llama-engine.zip");
        self.download_to_file(&url, &zip_path).await?;

        let _ = self
            .app_handle
            .emit("local-llm-engine-status", "extracting");
        Self::extract_zip(&zip_path, &engine_dir)?;
        let _ = std::fs::remove_file(&zip_path);

        let resolved = self.resolve_engine_binary().ok_or_else(|| {
            "Engine downloaded but the llama-server binary was not found in the archive".to_string()
        })?;
        let _ = self.app_handle.emit("local-llm-engine-status", "ready");
        info!("Built-in LLM engine installed at {}", resolved.display());
        Ok(resolved)
    }

    /// Token sets (in priority order) used to pick the right release asset for
    /// this OS/arch. The first asset whose name contains all tokens of a set
    /// wins.
    fn engine_asset_preferences() -> Vec<Vec<&'static str>> {
        if cfg!(target_os = "windows") {
            // Prefer Vulkan (the app already ships Vulkan for Whisper), fall
            // back to the CPU build which runs anywhere.
            vec![vec!["win", "vulkan", "x64"], vec!["win", "cpu", "x64"]]
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                vec![vec!["macos", "arm64"]]
            } else {
                vec![vec!["macos", "x64"]]
            }
        } else {
            // Linux
            vec![vec!["ubuntu", "vulkan", "x64"], vec!["ubuntu", "x64"]]
        }
    }

    /// Query the latest llama.cpp GitHub release and return the download URL of
    /// the best-matching prebuilt binary for this platform.
    async fn resolve_engine_asset_url(&self) -> Result<String, String> {
        let client = reqwest::Client::new();
        let release: serde_json::Value = client
            .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
            .header("User-Agent", "speakoflow")
            .send()
            .await
            .map_err(|e| format!("Failed to query llama.cpp releases: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse llama.cpp release info: {}", e))?;

        let assets = release
            .get("assets")
            .and_then(|a| a.as_array())
            .ok_or_else(|| "No assets in latest llama.cpp release".to_string())?;

        for tokens in Self::engine_asset_preferences() {
            for asset in assets {
                let name = asset.get("name").and_then(|n| n.as_str()).unwrap_or("");
                if tokens.iter().all(|t| name.contains(t)) {
                    if let Some(url) = asset.get("browser_download_url").and_then(|u| u.as_str()) {
                        return Ok(url.to_string());
                    }
                }
            }
        }
        Err("No compatible llama.cpp prebuilt binary found for this platform".to_string())
    }

    /// Stream a URL to a file, emitting coarse progress events.
    async fn download_to_file(&self, url: &str, dest: &Path) -> Result<(), String> {
        use futures_util::StreamExt;
        use std::io::Write;

        let client = reqwest::Client::new();
        let resp = client
            .get(url)
            .header("User-Agent", "speakoflow")
            .send()
            .await
            .map_err(|e| format!("Engine download request failed: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("Engine download failed: HTTP {}", resp.status()));
        }

        let total = resp.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;
        let mut file =
            std::fs::File::create(dest).map_err(|e| format!("Failed to create file: {}", e))?;
        let mut stream = resp.bytes_stream();
        let mut last_emit = Instant::now();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Engine download error: {}", e))?;
            file.write_all(&chunk)
                .map_err(|e| format!("Failed to write engine file: {}", e))?;
            downloaded += chunk.len() as u64;
            if last_emit.elapsed() >= Duration::from_millis(250) {
                let _ = self.app_handle.emit(
                    "local-llm-engine-progress",
                    serde_json::json!({ "downloaded": downloaded, "total": total }),
                );
                last_emit = Instant::now();
            }
        }
        file.flush()
            .map_err(|e| format!("Failed to flush engine file: {}", e))?;
        Ok(())
    }

    /// Extract a `.zip` archive into `dest`, preserving the executable bit on
    /// Unix so `llama-server` stays runnable.
    fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
        let file =
            std::fs::File::open(zip_path).map_err(|e| format!("Failed to open archive: {}", e))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Failed to read archive: {}", e))?;

        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("Failed to read archive entry: {}", e))?;
            let outpath = match entry.enclosed_name() {
                Some(p) => dest.join(p),
                None => continue,
            };

            if entry.is_dir() {
                std::fs::create_dir_all(&outpath)
                    .map_err(|e| format!("Failed to create dir: {}", e))?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create dir: {}", e))?;
                }
                let mut out = std::fs::File::create(&outpath)
                    .map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut entry, &mut out)
                    .map_err(|e| format!("Failed to extract file: {}", e))?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Some(mode) = entry.unix_mode() {
                        let _ = std::fs::set_permissions(
                            &outpath,
                            std::fs::Permissions::from_mode(mode),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve the on-disk GGUF path for a downloaded local-LLM model id.
    fn gguf_path_for(&self, model_id: &str) -> Result<PathBuf, String> {
        let model_manager = self.app_handle.state::<Arc<ModelManager>>();
        let info = model_manager
            .get_model_info(model_id)
            .ok_or_else(|| format!("Unknown model: {}", model_id))?;

        if info.engine_type != EngineType::LlamaCpp {
            return Err(format!("Model '{}' is not a local LLM", model_id));
        }

        let path = self.models_dir.join(&info.filename);
        if !path.is_file() {
            return Err(format!(
                "Model '{}' is not downloaded yet. Download it from the Models tab.",
                model_id
            ));
        }
        Ok(path)
    }

    /// Ensure the engine is running and serving `model_id`, starting (or
    /// restarting with a different model) as needed. Returns once the server is
    /// accepting connections, or an error describing why it could not start.
    pub async fn ensure_running(&self, model_id: &str) -> Result<(), String> {
        // Only one start sequence at a time.
        let _start_guard = self.start_lock.lock().await;

        // Fast path: already serving this model and the process is alive.
        {
            let mut st = self.state.lock().unwrap();
            let already = st.model_id.as_deref() == Some(model_id)
                && match st.child.as_mut() {
                    Some(child) => matches!(child.try_wait(), Ok(None)),
                    None => false,
                };
            if already {
                return Ok(());
            }
            // Stop any process serving a different (or dead) model.
            if let Some(mut child) = st.child.take() {
                debug!("Stopping local LLM engine before switching models");
                let _ = child.kill();
                let _ = child.wait();
            }
            st.model_id = None;
        }

        // Ensure the engine binary exists, downloading the official llama.cpp
        // build for this platform on first use (zero manual setup).
        let engine = self.ensure_engine_installed().await.map_err(|e| {
            self.set_error(Some(e.clone()));
            self.emit_status();
            e
        })?;
        let gguf = self.gguf_path_for(model_id)?;
        let mmproj = self.mmproj_path_for(model_id);

        info!(
            "Starting built-in LLM engine '{}' with model '{}' on port {}",
            engine.display(),
            model_id,
            self.port
        );

        let child = self
            .spawn_server(&engine, &gguf, mmproj.as_deref())
            .map_err(|e| {
                let msg = format!("Failed to start local LLM engine: {}", e);
                self.set_error(Some(msg.clone()));
                msg
            })?;

        {
            let mut st = self.state.lock().unwrap();
            st.child = Some(child);
            st.model_id = Some(model_id.to_string());
            st.last_error = None;
        }
        self.emit_status();

        match self.wait_until_ready().await {
            Ok(()) => {
                info!("Built-in LLM engine ready (model '{}')", model_id);
                self.emit_status();
                Ok(())
            }
            Err(e) => {
                error!("Built-in LLM engine failed to become ready: {}", e);
                self.stop();
                self.set_error(Some(e.clone()));
                self.emit_status();
                Err(e)
            }
        }
    }

    /// On-disk path to the model's vision projector, if it's a multimodal
    /// model and the projector has been downloaded.
    fn mmproj_path_for(&self, model_id: &str) -> Option<PathBuf> {
        crate::managers::model::mmproj_for(model_id).and_then(|(name, _)| {
            let path = self.models_dir.join(name);
            if path.is_file() {
                Some(path)
            } else {
                None
            }
        })
    }

    fn spawn_server(
        &self,
        engine: &Path,
        gguf: &Path,
        mmproj: Option<&Path>,
    ) -> std::io::Result<Child> {
        let mut cmd = Command::new(engine);
        cmd.arg("-m")
            .arg(gguf)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(self.port.to_string())
            .arg("-c")
            .arg(CONTEXT_SIZE.to_string())
            // Offload as many layers to the GPU as fit; CPU-only builds ignore this.
            .arg("-ngl")
            .arg("999")
            // Use the model's embedded Jinja chat template — needed for correct
            // prompting, tool calls, and reasoning separation on modern models.
            .arg("--jinja");

        // Multimodal models need their vision projector to "see" images
        // (the assistant's screenshot feature).
        if let Some(mmproj) = mmproj {
            cmd.arg("--mmproj").arg(mmproj);
        }

        cmd.stdout(Stdio::null()).stderr(Stdio::null());

        // Don't pop up a console window on Windows.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.spawn()
    }

    /// Poll the loopback port until the engine accepts connections (it only
    /// binds after the model has finished loading), or time out. Runs on a
    /// blocking thread so the async runtime is never stalled.
    async fn wait_until_ready(&self) -> Result<(), String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let state = self.state.clone();

        let join = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            let start = Instant::now();
            loop {
                // Detect an engine that exited immediately (e.g. bad model).
                {
                    let mut st = state.lock().unwrap();
                    match st.child.as_mut() {
                        Some(child) => {
                            if let Ok(Some(status)) = child.try_wait() {
                                return Err(format!(
                                    "Local LLM engine exited unexpectedly (status: {})",
                                    status
                                ));
                            }
                        }
                        None => return Err("Local LLM engine was stopped".to_string()),
                    }
                }

                if TcpStream::connect_timeout(&addr, Duration::from_millis(800)).is_ok() {
                    return Ok(());
                }
                if start.elapsed() > READY_TIMEOUT {
                    return Err("Local LLM engine did not become ready in time".to_string());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        })
        .await;

        match join {
            Ok(result) => result,
            Err(e) => Err(format!("Local LLM readiness task failed: {}", e)),
        }
    }

    /// Stop the engine if running. Safe to call when already stopped.
    pub fn stop(&self) {
        let mut st = self.state.lock().unwrap();
        if let Some(mut child) = st.child.take() {
            debug!("Stopping built-in LLM engine");
            let _ = child.kill();
            let _ = child.wait();
        }
        st.model_id = None;
    }

    fn set_error(&self, error: Option<String>) {
        if let Ok(mut st) = self.state.lock() {
            st.last_error = error;
        }
    }

    /// Current engine status for the UI.
    pub fn status(&self) -> LocalLlmStatus {
        let engine_present = self.resolve_engine_binary().is_some();
        let mut st = self.state.lock().unwrap();
        // Reap a process that died since the last check so `running` is accurate.
        let running = match st.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => {
                    st.child = None;
                    st.model_id = None;
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            },
            None => false,
        };
        LocalLlmStatus {
            running,
            model_id: st.model_id.clone(),
            engine_present,
            port: self.port,
            error: st.last_error.clone(),
        }
    }

    fn emit_status(&self) {
        let status = self.status();
        if let Err(e) = self.app_handle.emit("local-llm-status", &status) {
            warn!("Failed to emit local-llm-status event: {}", e);
        }
    }
}

impl Drop for LocalLlmManager {
    fn drop(&mut self) {
        self.stop();
    }
}
