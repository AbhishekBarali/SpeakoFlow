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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

/// Loopback port the bundled engine listens on. Deliberately different from
/// Ollama's default (11434) so a user's existing Ollama install is untouched.
const ENGINE_PORT: u16 = 11435;

/// Pinned llama.cpp release used as a fallback when the GitHub "latest" API is
/// unreachable or rate-limited. Its assets are fetched directly from the
/// release-download host, which — unlike `api.github.com` — is NOT subject to
/// the 60-requests/hour unauthenticated rate limit that shared campus/office
/// NAT IPs routinely exhaust (the root cause of "No assets in latest llama.cpp
/// release"). Overridable at runtime via the `HANDY_LLAMA_RELEASE_TAG` env var
/// so a newer/older build can be pinned without a rebuild.
const PINNED_ENGINE_TAG: &str = "b10075";

/// Default context window if the user hasn't chosen one. 8192 leaves room for
/// the system prompt, chat history, a screenshot (vision models can spend
/// ~1k+ tokens on one image), and the reply — a 4096 window overflows on a
/// screenshot + web-search turn. Users can raise it (16384 is a good target on
/// machines with RAM to spare) or lower it to save memory.
pub const DEFAULT_CONTEXT_SIZE: u32 = 8192;

/// Lower bound for the user-configurable context window (tokens). Below this a
/// model can't hold a useful prompt.
pub const MIN_CONTEXT_SIZE: u32 = 512;

/// Upper bound for the user-configurable context window (tokens). Caps memory
/// use on low-end / CPU-only machines while leaving headroom for capable GPUs.
pub const MAX_CONTEXT_SIZE: u32 = 32_768;

/// Max time to wait for the engine to load a model and start serving. Large
/// models, slow disks, and first-run GPU shader compilation can take a while.
const READY_TIMEOUT: Duration = Duration::from_secs(180);

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
    /// Last time the engine was used (ms since the Unix epoch); drives the
    /// idle-unload watcher.
    last_activity: Arc<AtomicU64>,
    /// Number of in-flight LLM requests. The idle watcher never unloads while
    /// this is greater than zero, so a long generation can't be cut off.
    in_flight: Arc<AtomicUsize>,
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
            last_activity: Arc::new(AtomicU64::new(Self::now_ms())),
            in_flight: Arc::new(AtomicUsize::new(0)),
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

    /// Path to the engine's captured stdout/stderr log.
    fn engine_log_path(&self) -> PathBuf {
        self.models_dir.join("engine").join("llama-server.log")
    }

    /// Read the last `max_lines` lines of the engine log, for surfacing the
    /// real failure reason (bad arg, missing DLL, model load error, ...).
    fn engine_log_tail(&self, max_lines: usize) -> Option<String> {
        let content = std::fs::read_to_string(self.engine_log_path()).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(max_lines);
        let tail = lines[start..].join("\n");
        if tail.trim().is_empty() {
            None
        } else {
            Some(tail)
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

    /// Ensure an engine binary is available, downloading a llama.cpp release
    /// for this platform into `<models>/engine/` if none is found anywhere.
    /// Returns the path to the binary.
    ///
    /// Robustness: tries the current GitHub "latest" release first, then falls
    /// back to a PINNED release whose download URLs are constructed directly.
    /// The pinned URLs hit the release-download host rather than the
    /// rate-limited `api.github.com`, so a throttled or offline GitHub API — the
    /// usual cause of "No assets in latest llama.cpp release" on shared
    /// school/office networks — no longer blocks setup. On total failure it
    /// returns a clear, actionable message instead of a cryptic API error.
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

        // Ordered candidate archive URLs: the live "latest" asset first (best
        // match for this platform, when the API is reachable), then the pinned
        // fallback URLs. Trying the pinned build means a rate-limited/offline
        // GitHub API can't stop the built-in engine from installing.
        let mut candidates: Vec<String> = Vec::new();
        match self.resolve_latest_asset_url().await {
            Ok(url) => candidates.push(url),
            Err(e) => warn!(
                "Could not resolve the latest llama.cpp release ({}); falling back \
                 to the pinned build {}",
                e, PINNED_ENGINE_TAG
            ),
        }
        for url in self.pinned_asset_urls() {
            if !candidates.contains(&url) {
                candidates.push(url);
            }
        }

        if candidates.is_empty() {
            let _ = self.app_handle.emit("local-llm-engine-status", "error");
            return Err(
                "No compatible llama.cpp prebuilt binary is available for this platform."
                    .to_string(),
            );
        }

        let mut last_error = String::from("unknown error");
        for url in candidates {
            match self.download_and_extract_engine(&url, &engine_dir).await {
                Ok(()) => {
                    if let Some(resolved) = self.resolve_engine_binary() {
                        let _ = self.app_handle.emit("local-llm-engine-status", "ready");
                        info!("Built-in LLM engine installed at {}", resolved.display());
                        return Ok(resolved);
                    }
                    last_error = "engine archive downloaded but no llama-server binary was inside"
                        .to_string();
                    warn!("{} (source: {})", last_error, url);
                }
                Err(e) => {
                    warn!("Engine download attempt failed ({}): {}", url, e);
                    last_error = e;
                }
            }
        }

        let _ = self.app_handle.emit("local-llm-engine-status", "error");
        Err(format!(
            "Couldn't set up the built-in engine (llama.cpp). This is usually a network \
             problem or GitHub rate-limiting (common on shared school/office Wi-Fi). Try \
             again in a few minutes, switch networks, or pick a cloud or local \
             (Ollama / LM Studio) provider in Settings → Assistant. Last error: {}",
            last_error
        ))
    }

    /// Download one engine archive `url` into `engine_dir` and extract it.
    /// The extractor is chosen from the URL's extension: Windows assets are
    /// `.zip`, macOS/Linux assets are `.tar.gz`.
    async fn download_and_extract_engine(
        &self,
        url: &str,
        engine_dir: &Path,
    ) -> Result<(), String> {
        let lower = url.to_ascii_lowercase();
        let is_tar_gz = lower.ends_with(".tar.gz") || lower.ends_with(".tgz");
        let archive_name = if is_tar_gz {
            "llama-engine.tar.gz"
        } else {
            "llama-engine.zip"
        };
        let archive_path = engine_dir.join(archive_name);

        let _ = self
            .app_handle
            .emit("local-llm-engine-status", "downloading");
        self.download_to_file(url, &archive_path).await?;

        let _ = self
            .app_handle
            .emit("local-llm-engine-status", "extracting");
        let result = if is_tar_gz {
            Self::extract_tar_gz(&archive_path, engine_dir)
        } else {
            Self::extract_zip(&archive_path, engine_dir)
        };
        // Always clean up the archive, success or failure, so a partial/corrupt
        // download can't wedge the next attempt.
        let _ = std::fs::remove_file(&archive_path);
        result
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

    /// Query the GitHub "latest" release for the best-matching prebuilt binary
    /// URL for this platform.
    ///
    /// Crucially, this checks the HTTP status before parsing: an unauthenticated
    /// request that is rate-limited returns HTTP 403/429 with a JSON body that
    /// has no `assets` field, which the previous code misreported as "No assets
    /// in latest llama.cpp release". Returning a real error here lets the caller
    /// fall back to the pinned build instead of dead-ending.
    async fn resolve_latest_asset_url(&self) -> Result<String, String> {
        let client = reqwest::Client::new();
        let response = client
            .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
            .header("User-Agent", "speakoflow")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| format!("Failed to query llama.cpp releases: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let hint = if status.as_u16() == 403 || status.as_u16() == 429 {
                " (GitHub API rate limit — common on shared networks)"
            } else {
                ""
            };
            return Err(format!("GitHub API returned HTTP {}{}", status, hint));
        }

        let release: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse llama.cpp release info: {}", e))?;

        let assets = release
            .get("assets")
            .and_then(|a| a.as_array())
            .ok_or_else(|| "latest release listing contained no assets".to_string())?;

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
        Err("no compatible prebuilt asset in the latest release".to_string())
    }

    /// Directly-constructed download URLs for the pinned llama.cpp release, in
    /// priority order for this OS/arch. These target the release-download host
    /// (not the rate-limited `api.github.com`), so they succeed even when the
    /// "latest" lookup is throttled or the machine is behind a busy shared IP.
    /// The tag is overridable via `HANDY_LLAMA_RELEASE_TAG`.
    fn pinned_asset_urls(&self) -> Vec<String> {
        let tag = std::env::var("HANDY_LLAMA_RELEASE_TAG")
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| PINNED_ENGINE_TAG.to_string());
        Self::pinned_asset_urls_for(&tag)
    }

    /// Build the pinned release-download URLs for `tag`. Split out from
    /// `pinned_asset_urls` so it is unit-testable without a Tauri `AppHandle`.
    fn pinned_asset_urls_for(tag: &str) -> Vec<String> {
        Self::pinned_asset_names(tag)
            .into_iter()
            .map(|name| {
                format!(
                    "https://github.com/ggml-org/llama.cpp/releases/download/{}/{}",
                    tag, name
                )
            })
            .collect()
    }

    /// Prebuilt archive filenames for the pinned release, in priority order for
    /// this OS/arch, following llama.cpp's `llama-<tag>-bin-<platform>.<ext>`
    /// convention. Windows ships `.zip`; macOS/Linux ship `.tar.gz`. Vulkan
    /// builds are preferred (matching the app's Whisper backend) with a CPU
    /// build as the always-works fallback.
    fn pinned_asset_names(tag: &str) -> Vec<String> {
        if cfg!(target_os = "windows") {
            if cfg!(target_arch = "aarch64") {
                vec![format!("llama-{tag}-bin-win-cpu-arm64.zip")]
            } else {
                vec![
                    format!("llama-{tag}-bin-win-vulkan-x64.zip"),
                    format!("llama-{tag}-bin-win-cpu-x64.zip"),
                ]
            }
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                vec![format!("llama-{tag}-bin-macos-arm64.tar.gz")]
            } else {
                vec![format!("llama-{tag}-bin-macos-x64.tar.gz")]
            }
        } else if cfg!(target_arch = "aarch64") {
            // Linux arm64
            vec![
                format!("llama-{tag}-bin-ubuntu-vulkan-arm64.tar.gz"),
                format!("llama-{tag}-bin-ubuntu-arm64.tar.gz"),
            ]
        } else {
            // Linux x64
            vec![
                format!("llama-{tag}-bin-ubuntu-vulkan-x64.tar.gz"),
                format!("llama-{tag}-bin-ubuntu-x64.tar.gz"),
            ]
        }
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

    /// Extract a `.tar.gz` archive into `dest`. The `tar` crate restores the
    /// executable bit stored in the archive, so `llama-server` stays runnable.
    /// Used for the macOS/Linux release assets (which ship as `.tar.gz`);
    /// Windows assets are `.zip` and go through `extract_zip`.
    fn extract_tar_gz(archive_path: &Path, dest: &Path) -> Result<(), String> {
        let file = std::fs::File::open(archive_path)
            .map_err(|e| format!("Failed to open archive: {}", e))?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(dest)
            .map_err(|e| format!("Failed to extract archive: {}", e))?;
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

        // Count this as activity so the idle watcher measures from now (covers
        // prewarm, which calls ensure_running before any request is made).
        self.touch_activity();

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

        // The engine listens on a single fixed port. A just-killed engine's
        // listening socket lingers for a short window (and a crashed previous
        // run can leave an orphan still holding it); spawning the replacement
        // before the port is free makes the new llama-server fail to bind and
        // exit — the root cause of rapid model switches "not switching". Wait
        // for the port to clear first (returns immediately when it's already
        // free) so the new engine binds reliably, and so a later /health 200
        // can only come from the newly started model.
        self.wait_for_port_release().await;

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
                // Surface the engine's real output so the failure is actionable
                // (e.g. unsupported flag, missing GPU runtime, model load error).
                let detail = match self.engine_log_tail(20) {
                    Some(tail) => format!(
                        "{}\n\nEngine log ({}):\n{}",
                        e,
                        self.engine_log_path().display(),
                        tail
                    ),
                    None => e.clone(),
                };
                error!("Built-in LLM engine failed to become ready: {}", detail);
                self.stop();
                self.set_error(Some(detail.clone()));
                self.emit_status();
                Err(detail)
            }
        }
    }

    /// On-disk path to the model's vision projector, if it's a multimodal
    /// model and the projector has been downloaded.
    fn mmproj_path_for(&self, model_id: &str) -> Option<PathBuf> {
        let model_manager = self.app_handle.state::<Arc<ModelManager>>();
        let (name, _) = model_manager.resolve_mmproj(model_id)?;
        let path = self.models_dir.join(name);
        if path.is_file() {
            Some(path)
        } else {
            None
        }
    }

    /// The context window to launch the engine with: the user's configured
    /// value clamped to a safe range, or the default if settings can't be read.
    fn configured_context_size(&self) -> u32 {
        crate::settings::get_settings(&self.app_handle)
            .local_llm_context_size
            .clamp(MIN_CONTEXT_SIZE, MAX_CONTEXT_SIZE)
    }

    fn spawn_server(
        &self,
        engine: &Path,
        gguf: &Path,
        mmproj: Option<&Path>,
    ) -> std::io::Result<Child> {
        let context_size = self.configured_context_size();

        let mut cmd = Command::new(engine);
        cmd.arg("-m")
            .arg(gguf)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(self.port.to_string())
            .arg("-c")
            .arg(context_size.to_string())
            // ONE generation slot. Without this, llama-server defaults to
            // several parallel slots (observed: n_slots = 4) and SPLITS the
            // context window across them — so each turn gets only a fraction of
            // the tokens (e.g. n_ctx_slot = 1792 out of ~7168). A single desktop
            // conversation then overflows its slot almost immediately, and a
            // vision turn overflows instantly (one screenshot alone costs
            // ~1000-2000 tokens). That surfaces in the engine log as
            // `decode: failed to find free space in the KV cache` plus
            // `truncated = 1`, and to the user as replies that "work for a while
            // then break." This app is single-user, so one slot owning the FULL
            // context window is both correct and far more reliable; overlapping
            // requests (assistant + post-processing) simply queue instead of
            // fighting over a fragmented KV cache.
            .arg("--parallel")
            .arg("1")
            // Offload as many layers to the GPU as fit; CPU-only builds ignore this.
            .arg("-ngl")
            .arg("999")
            // Use the model's embedded Jinja chat template — needed for correct
            // prompting, tool calls, and reasoning separation on modern models.
            .arg("--jinja")
            // Discourage the short repetition loops small models fall into
            // (e.g. printing "Repeat: ..." and re-emitting the same sentence).
            .arg("--repeat-penalty")
            .arg("1.1");

        // Keep local chat on the same dedicated-first GPU policy used by the
        // rest of the app. This matters on hybrid Windows/Linux systems where
        // an integrated adapter can report more shared memory than a discrete
        // card. Metal exposes a single device, so index 0 remains a no-op there.
        if let Some(device) = crate::managers::transcription::preferred_gpu_device() {
            info!(
                "Selecting llama.cpp main GPU {}: {} ({}, {} MiB)",
                device.id, device.name, device.kind, device.total_vram_mb
            );
            cmd.arg("--main-gpu").arg(device.id.to_string());
        }

        // Flash Attention in "auto" mode: the engine enables it only on backends
        // that support it and silently falls back to standard attention
        // otherwise — safe on CPU-only machines and every GPU type, while cutting
        // KV-cache memory and speeding up attention where available. Passed via
        // the env var rather than the `-fa` CLI flag so that older cached engine
        // builds (which predate the `-fa on|off|auto` syntax) ignore it instead
        // of failing to start. `auto` is already llama.cpp's default; this just
        // pins the intent against any future default change.
        cmd.env("LLAMA_ARG_FLASH_ATTN", "auto");

        // Multimodal models need their vision projector to "see" images
        // (the assistant's screenshot feature).
        if let Some(mmproj) = mmproj {
            cmd.arg("--mmproj").arg(mmproj);
        }

        // Capture the engine's output to a log file so failures (bad argument,
        // missing DLL, model load error) are diagnosable instead of silent.
        let log_path = self.engine_log_path();
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let (stdout, stderr) = match std::fs::File::create(&log_path) {
            Ok(f) => match f.try_clone() {
                Ok(f2) => (Stdio::from(f), Stdio::from(f2)),
                Err(_) => (Stdio::from(f), Stdio::null()),
            },
            Err(_) => (Stdio::null(), Stdio::null()),
        };
        cmd.stdout(stdout).stderr(stderr);

        // Don't pop up a console window on Windows.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn()?;

        // Tie the engine's lifetime to ours. On Windows we place it in a
        // kill-on-close Job Object so it is terminated even if we crash, panic,
        // or are force-killed by the OS during a memory-pressure freeze — the
        // paths where `Drop` / exit handlers never run and an orphaned multi-GB
        // `llama-server.exe` would otherwise survive and keep exhausting RAM.
        // The graceful quit path additionally calls `stop()` via `RunEvent::Exit`
        // in `lib.rs`.
        #[cfg(windows)]
        assign_child_to_kill_on_close_job(&child);

        Ok(child)
    }

    /// True if something is currently accepting TCP connections on the engine
    /// port (our engine, or a just-killed one whose socket the OS hasn't freed
    /// yet).
    fn port_in_use(&self) -> bool {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
    }

    /// Wait (bounded) for the engine port to be released after stopping a
    /// model, before starting the replacement.
    ///
    /// The engine listens on a single fixed port. When switching models we kill
    /// the running `llama-server` and immediately spawn a new one — but the OS
    /// keeps the killed process's listening socket for a short window, so the
    /// replacement can lose the race to `bind()` that same port, fail to start,
    /// and exit. That is the root cause of "switching models quickly doesn't
    /// work": slow, deliberate switches leave enough time for the socket to
    /// free up, while rapid ones don't. Polling until the port is free both
    /// lets the new engine bind reliably AND guarantees the next `/health` 200
    /// can only come from the newly started model, never a half-dead
    /// predecessor still holding the port.
    async fn wait_for_port_release(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.port_in_use() {
            if Instant::now() >= deadline {
                warn!(
                    "Engine port {} still busy after stopping the previous model; \
                     starting the new engine anyway (it may report a bind error)",
                    self.port
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Probe the engine's `/health` endpoint with a minimal raw HTTP request
    /// (no async/blocking-client deps needed). Returns true only on HTTP 200,
    /// which llama-server reports once the model is fully loaded. While loading
    /// it returns 503, so we keep waiting.
    fn check_health(addr: &SocketAddr) -> bool {
        use std::io::{Read, Write};

        let mut stream = match TcpStream::connect_timeout(addr, Duration::from_millis(800)) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(1500)));

        let request = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        if stream.write_all(request.as_bytes()).is_err() {
            return false;
        }

        let mut buf = [0u8; 256];
        match stream.read(&mut buf) {
            Ok(n) if n > 0 => {
                let resp = String::from_utf8_lossy(&buf[..n]);
                // Status line looks like "HTTP/1.1 200 OK" when ready.
                resp.lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .map(|code| code == "200")
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Wait until the engine has fully loaded the model and `/health` returns
    /// 200, or time out. Runs on a blocking thread so the async runtime is
    /// never stalled.
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

                // The engine accepts TCP connections before the model is
                // loaded, so a bare connect check fires too early (causes a
                // 503 "Loading model" on the first request). Poll /health and
                // only proceed once it reports the model is ready (HTTP 200).
                if Self::check_health(&addr) {
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
        // Called from `Drop` on app quit, so recover from a poisoned mutex
        // instead of panicking — a panic inside Drop calls abort() (extends the
        // intent of Handy #1354 to this SpeakoFlow-added manager). Recovering
        // the guard still lets us kill the child process during shutdown.
        let mut st = match self.state.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
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

    /// Current time in milliseconds since the Unix epoch.
    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Reset the idle timer to "now". Called on every engine use (prewarm,
    /// `ensure_running`, and at the start/end of each request) so the watcher
    /// only unloads after a genuine idle period.
    pub fn touch_activity(&self) {
        self.last_activity.store(Self::now_ms(), Ordering::Relaxed);
    }

    /// Mark the start of an LLM request. Increments the in-flight counter (so
    /// the idle watcher won't unload mid-request) and refreshes the idle timer.
    /// The returned guard decrements the counter and refreshes the timer again
    /// on drop, so the idle countdown restarts from the end of the request.
    pub fn begin_request(&self) -> LlmActivityGuard {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        self.touch_activity();
        LlmActivityGuard {
            in_flight: self.in_flight.clone(),
            last_activity: self.last_activity.clone(),
        }
    }

    /// Spawn the background watcher that unloads the engine once it has been
    /// idle for longer than `local_llm_unload_timeout`. Holds only a weak ref,
    /// so it exits automatically when the manager is dropped (app shutdown).
    /// Mirrors the transcription manager's idle watcher.
    pub fn spawn_idle_watcher(manager: &Arc<Self>) {
        let weak = Arc::downgrade(manager);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(10));
            match weak.upgrade() {
                Some(manager) => manager.idle_check_and_maybe_stop(),
                None => break,
            }
        });
    }

    /// Stop the engine if it has been idle longer than the configured timeout.
    /// Never unloads while a request is in flight, or when the timeout is set to
    /// `Never`.
    fn idle_check_and_maybe_stop(&self) {
        let timeout = crate::settings::get_settings(&self.app_handle).local_llm_unload_timeout;
        let Some(limit_seconds) = timeout.to_seconds() else {
            return; // `Never` — keep the engine resident.
        };

        // Keep the engine alive while a request is in flight, and keep the idle
        // timer fresh so a long generation can't be unloaded out from under us.
        if self.in_flight.load(Ordering::SeqCst) > 0 {
            self.touch_activity();
            return;
        }

        // Only act if the engine is actually running.
        {
            let mut st = self.state.lock().unwrap();
            let running = match st.child.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => false,
            };
            if !running {
                return;
            }
        }

        let idle_ms = Self::now_ms().saturating_sub(self.last_activity.load(Ordering::Relaxed));
        if idle_ms >= limit_seconds.saturating_mul(1000) {
            info!(
                "Built-in LLM idle for {}s (limit {}s); unloading to free memory",
                idle_ms / 1000,
                limit_seconds
            );
            self.stop();
            self.emit_status();
        }
    }
}

/// RAII guard returned by [`LocalLlmManager::begin_request`]. Decrements the
/// in-flight counter and refreshes the idle timer when dropped, so the idle
/// countdown starts from the moment the request finishes.
pub struct LlmActivityGuard {
    in_flight: Arc<AtomicUsize>,
    last_activity: Arc<AtomicU64>,
}

impl Drop for LlmActivityGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.last_activity
            .store(LocalLlmManager::now_ms(), Ordering::Relaxed);
    }
}

impl Drop for LocalLlmManager {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Assign a spawned engine child to a process-wide Job Object configured with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The job handle is created once and kept
/// open for the entire process lifetime (stored as a raw `isize` in a
/// `OnceLock`), so the only event that closes the job's last handle is THIS
/// process terminating — at which point Windows kills every process in the job,
/// including `llama-server.exe`.
///
/// This is the crash / force-kill backstop for orphaned engine processes (the
/// root cause of the "memory keeps climbing, disk hits 100%, PC freezes"
/// reports). The normal quit path also calls `stop()` via `RunEvent::Exit`, but
/// that — like `Drop` — never runs when the process is killed hard (panic,
/// crash, or the OS reaping us during a low-memory freeze).
#[cfg(windows)]
fn assign_child_to_kill_on_close_job(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // Raw job handle value (stored as isize so it is Send + Sync for the
    // OnceLock). 0 means creation failed; we then skip assignment so a failure
    // here only disables cleanup rather than breaking engine startup.
    static JOB: OnceLock<isize> = OnceLock::new();

    let job_raw = *JOB.get_or_init(|| unsafe {
        match CreateJobObjectW(None, PCWSTR::null()) {
            Ok(job) => {
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if let Err(e) = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) {
                    warn!("Failed to configure engine Job Object (orphan cleanup disabled): {e}");
                }
                job.0 as isize
            }
            Err(e) => {
                warn!("Failed to create engine Job Object (orphan cleanup disabled): {e}");
                0
            }
        }
    });

    if job_raw == 0 {
        return;
    }

    let job = HANDLE(job_raw as *mut core::ffi::c_void);
    let child_handle = HANDLE(child.as_raw_handle() as *mut core::ffi::c_void);
    // SAFETY: `job` is a valid job handle kept open for the process lifetime and
    // `child_handle` is the live child's process handle owned by `child`.
    unsafe {
        if let Err(e) = AssignProcessToJobObject(job, child_handle) {
            warn!("Failed to assign engine process to Job Object: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Asset naming was verified live against the llama.cpp b10075 release:
    // Windows ships `llama-<tag>-bin-win-*-x64.zip`, macOS/Linux ship
    // `llama-<tag>-bin-*.tar.gz`. These guard against silent drift in the
    // pinned-fallback filenames (which are constructed, not discovered).
    #[test]
    fn pinned_asset_names_match_platform_convention() {
        let names = LocalLlmManager::pinned_asset_names("b10075");
        assert!(
            !names.is_empty(),
            "the current platform must have at least one pinned asset"
        );
        for name in &names {
            assert!(
                name.starts_with("llama-b10075-bin-"),
                "unexpected asset name: {name}"
            );
            if cfg!(target_os = "windows") {
                assert!(name.ends_with(".zip"), "Windows assets are .zip: {name}");
            } else {
                assert!(
                    name.ends_with(".tar.gz"),
                    "macOS/Linux assets are .tar.gz: {name}"
                );
            }
        }
    }

    #[test]
    fn pinned_asset_urls_target_release_download_host_not_api() {
        let urls = LocalLlmManager::pinned_asset_urls_for("b10075");
        assert!(!urls.is_empty());
        for url in &urls {
            assert!(
                url.starts_with(
                    "https://github.com/ggml-org/llama.cpp/releases/download/b10075/llama-b10075-bin-"
                ),
                "unexpected pinned url: {url}"
            );
            // The whole point of the fallback is to bypass the rate-limited API
            // (api.github.com), so it must never appear in a fallback URL.
            assert!(
                !url.contains("api.github.com"),
                "pinned fallback must not hit the rate-limited API host: {url}"
            );
        }
    }
}
