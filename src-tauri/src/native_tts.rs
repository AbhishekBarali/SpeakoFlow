//! Native local speech: Kokoro on the processor, and the lighter Kitten voice.
//!
//! The default local voice is Kokoro running inside the assistant panel's
//! WebView (kokoro-js on WebGPU). On a working graphics card that is fast, but
//! the WebView is a poor host for everything else: without WebGPU (every Linux
//! WebKitGTK build, many VMs, older GPUs) it falls back to single-threaded
//! WebAssembly, which on a Ryzen 7 took over ten seconds per reply, and some GPU
//! and driver combinations produce garbled audio that throws no error at all
//! (issue #29). This module is the other way to run a local voice: the
//! [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) engine, natively, on
//! every core of the processor. Measured on a Ryzen 7 7700X, Kokoro renders
//! speech about five times faster than it plays (RTF 0.21 at 8 threads) and
//! Kitten nano about fifteen times faster (RTF 0.06).
//!
//! Four decisions shape it.
//!
//! **The engine is downloaded, not bundled.** sherpa-onnx's shared build is
//! ~22 MB on disk (onnxruntime plus the C API). Most users never need it, so it
//! arrives with the first native voice pack instead of making every installer
//! heavier. Both the runtime archive and the voice packs are pinned by SHA-256.
//!
//! **It is loaded with `libloading`, not linked.** The app already links ONNX
//! Runtime statically through `ort` (VAD, transcription, diarization); linking a
//! second copy would collide at link time. Loading sherpa's own onnxruntime by
//! absolute path keeps the two apart: on Windows a DLL's imports bind by module,
//! on Linux the preloaded SONAME satisfies the C API's `DT_NEEDED`, and macOS
//! uses two-level namespaces. The library reports its version on load and
//! anything but [`SHERPA_VERSION`] is refused, because the `#[repr(C)]` structs
//! below mirror that release's `c-api.h` field for field.
//!
//! **One engine is loaded at a time, and it goes away when idle.** Kokoro costs
//! ~400 MB of RAM while loaded and Kitten ~80 MB. Synthesis is serialized on one
//! lock (a CPU engine gains nothing from running two requests at once), and a
//! watcher releases the engine after [`IDLE_UNLOAD`] without speech.
//!
//! **Kokoro keeps its GPU path.** [`route`] only sends Kokoro here when the
//! user asks for the processor, or when Automatic finds the WebView cannot use
//! the graphics card. Kitten always runs here.

use log::{debug, info, warn};
use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::settings::AppSettings;

/// The sherpa-onnx release whose C API the structs below mirror. Bumping it
/// means re-checking every `#[repr(C)]` struct against that release's
/// `sherpa-onnx/c-api/c-api.h`, then updating the runtime hashes.
pub const SHERPA_VERSION: &str = "1.13.8";

/// How long a loaded engine may sit unused before its memory is given back.
const IDLE_UNLOAD: Duration = Duration::from_secs(180);

/// Longest piece of text sent to the engine in one call. The engine splits on
/// sentence ends itself, but an unpunctuated run past its token window is cut
/// off rather than split, so callers break text below this first.
pub const MAX_CHARS: usize = 400;

/// Kitten speaks at roughly 95 words a minute at its own speed 1.0 (upstream
/// KittenTTS 0.8 and sherpa-onnx agree: ~11 s for an 18-word sentence that
/// Kokoro says in ~5 s). The app's 1x is scaled by this so both voices sound
/// like a normal speaking pace.
const KITTEN_PACE: f64 = 1.6;

// ---------------------------------------------------------------------------
// Voice packs and the engine runtime
// ---------------------------------------------------------------------------

/// Which model family a pack contains; decides which config block is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceFamily {
    Kokoro,
    Kitten,
}

/// A downloadable native voice. The model catalog in `managers/model.rs`
/// builds its entries from these, so the URL and hash live in one place.
#[derive(Debug)]
pub struct VoicePack {
    /// Catalog id in the model manager.
    pub model_id: &'static str,
    /// Directory under `<app data>/models/` the archive extracts to.
    pub dir: &'static str,
    pub archive_url: &'static str,
    pub sha256: &'static str,
    /// Size of the download, for the UI.
    pub download_bytes: u64,
    pub family: VoiceFamily,
    model_file: &'static str,
}

/// Kokoro v1.0, full precision. The int8 build is smaller (132 MB) but its
/// `ConvInteger` operators have no fast x86 kernel: it measured slower than
/// real time at every thread count, while fp32 runs at RTF 0.2–0.3.
pub const KOKORO_PACK: VoicePack = VoicePack {
    model_id: "kokoro-82m-native",
    dir: "kokoro-multi-lang-v1_0",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2",
    sha256: "c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298",
    download_bytes: 349_906_910,
    family: VoiceFamily::Kokoro,
    model_file: "model.onnx",
};

/// Kitten TTS nano 0.8, full precision. Like Kokoro, the int8 build is the
/// slower one on a desktop CPU (RTF 0.27 against 0.06–0.11 for fp32).
pub const KITTEN_PACK: VoicePack = VoicePack {
    model_id: "kitten-nano-0.8",
    dir: "kitten-nano-en-v0_8-fp32",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kitten-nano-en-v0_8-fp32.tar.bz2",
    sha256: "16092117bfe591ddcd58d078e1454603b8e1caea46f85653b2c2efae76bd883e",
    download_bytes: 63_815_222,
    family: VoiceFamily::Kitten,
    model_file: "model.fp32.onnx",
};

pub const PACKS: [&VoicePack; 2] = [&KOKORO_PACK, &KITTEN_PACK];

/// The pack a catalog id refers to.
#[cfg(test)]
fn pack_for_model_id(model_id: &str) -> Option<&'static VoicePack> {
    PACKS.iter().copied().find(|pack| pack.model_id == model_id)
}

/// A prebuilt sherpa-onnx shared-library archive for one platform.
#[derive(Debug)]
pub struct RuntimeAsset {
    pub archive: &'static str,
    pub sha256: &'static str,
}

impl RuntimeAsset {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/v{SHERPA_VERSION}/{}",
            self.archive
        )
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-lib.tar.bz2",
    sha256: "b8eedf41bd6d3779218887b48367bb7a3ece5aaa7667f01f69ee823a12b0a9e7",
});
#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-win-arm64-shared-MT-Release-lib.tar.bz2",
    sha256: "22cd2b2b5e35c1132abc74a91ee77f683645437382617af6a4bd5818b9c509f4",
});
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-linux-x64-shared-lib.tar.bz2",
    sha256: "3892d184be41027e18165e67f549cd4e4cdd8dcd73ac5579e97afd55e14e30b6",
});
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-linux-aarch64-shared-cpu-lib.tar.bz2",
    sha256: "fb98b80628a383909bf83626daca2e0d0ffc93bf1f7d222eaaa4282b388f072f",
});
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-osx-arm64-shared-lib.tar.bz2",
    sha256: "ae77050cdae565496059d96f5ab33d77b397a0864282a4e4a26e3b3b3effb948",
});
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-osx-x64-shared-lib.tar.bz2",
    sha256: "0f88371565a06372889c76266e253abd1b415fcc6f98232cabd968933efb0711",
});
#[cfg(not(any(
    all(
        target_os = "windows",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "macos",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
)))]
const RUNTIME: Option<RuntimeAsset> = None;

/// This platform's runtime archive, or `None` where sherpa-onnx ships none.
pub fn runtime_asset() -> Option<&'static RuntimeAsset> {
    RUNTIME.as_ref()
}

/// Whether native voices can run on this platform at all.
pub fn native_supported() -> bool {
    runtime_asset().is_some()
}

/// Set when the downloaded engine was on disk but refused to load this session,
/// e.g. macOS library validation rejecting a library the app didn't ship with.
/// While set, nothing counts as ready, so Kokoro keeps speaking in the WebView
/// instead of failing every reply on a route that cannot work.
static LOAD_FAILED: AtomicBool = AtomicBool::new(false);

/// The engine is on disk but could not be loaded this session.
pub fn load_failed() -> bool {
    LOAD_FAILED.load(Ordering::Relaxed)
}

/// Called when something inside this module changes where voices speak (the
/// engine failing to load), so the windows can follow. Set once at startup.
static STATUS_LISTENER: OnceCell<Box<dyn Fn() + Send + Sync>> = OnceCell::new();

pub fn set_status_listener(listener: impl Fn() + Send + Sync + 'static) {
    let _ = STATUS_LISTENER.set(Box::new(listener));
}

fn notify_status() {
    if let Some(listener) = STATUS_LISTENER.get() {
        listener();
    }
}

#[cfg(target_os = "windows")]
const ORT_LIB: &str = "onnxruntime.dll";
#[cfg(target_os = "windows")]
const C_API_LIB: &str = "sherpa-onnx-c-api.dll";
#[cfg(target_os = "macos")]
const ORT_LIB: &str = "libonnxruntime.dylib";
#[cfg(target_os = "macos")]
const C_API_LIB: &str = "libsherpa-onnx-c-api.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const ORT_LIB: &str = "libonnxruntime.so";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const C_API_LIB: &str = "libsherpa-onnx-c-api.so";

/// Files copied out of the runtime archive. The Windows provider bridge is only
/// needed by non-CPU execution providers, so it is copied when present but not
/// required.
const RUNTIME_OPTIONAL: &[&str] = &["onnxruntime_providers_shared.dll"];

/// `<models>/tts-runtime/sherpa-onnx-<version>/`.
pub fn runtime_dir_in(models_dir: &Path) -> PathBuf {
    models_dir
        .join("tts-runtime")
        .join(format!("sherpa-onnx-{SHERPA_VERSION}"))
}

/// Whether this version's runtime libraries are on disk.
pub fn runtime_installed_in(models_dir: &Path) -> bool {
    let dir = runtime_dir_in(models_dir);
    dir.join(ORT_LIB).is_file() && dir.join(C_API_LIB).is_file()
}

/// Whether a voice pack's files are on disk (not whether it is loaded).
pub fn pack_installed_in(models_dir: &Path, pack: &VoicePack) -> bool {
    let dir = models_dir.join(pack.dir);
    dir.join(pack.model_file).is_file()
        && dir.join("voices.bin").is_file()
        && dir.join("tokens.txt").is_file()
        && dir.join("espeak-ng-data").is_dir()
}

/// Unpack a verified runtime archive into [`runtime_dir_in`]. Only the two
/// libraries (plus the optional provider bridge) are kept; headers and import
/// libraries are discarded, as is the archive itself.
pub fn install_runtime_archive(archive: &Path, models_dir: &Path) -> Result<(), String> {
    let base = models_dir.join("tts-runtime");
    let staging = base.join(format!(".extracting-{SHERPA_VERSION}"));
    let installing = base.join(format!(".installing-{SHERPA_VERSION}"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&installing);
    std::fs::create_dir_all(&staging)
        .and_then(|_| std::fs::create_dir_all(&installing))
        .map_err(|e| format!("Couldn't prepare the voice engine folder: {e}"))?;

    let result = (|| -> Result<(), String> {
        let file = std::fs::File::open(archive)
            .map_err(|e| format!("Couldn't open the voice engine download: {e}"))?;
        let decoder = bzip2::read::MultiBzDecoder::new(std::io::BufReader::new(file));
        tar::Archive::new(decoder)
            .unpack(&staging)
            .map_err(|e| format!("Couldn't unpack the voice engine: {e}"))?;

        let mut wanted: Vec<(&str, bool)> = vec![(ORT_LIB, true), (C_API_LIB, true)];
        wanted.extend(RUNTIME_OPTIONAL.iter().map(|name| (*name, false)));
        for (name, required) in wanted {
            match find_file(&staging, name) {
                Some(source) => {
                    std::fs::copy(&source, installing.join(name))
                        .map_err(|e| format!("Couldn't install {name}: {e}"))?;
                }
                None if required => {
                    return Err(format!("The voice engine download is missing {name}"));
                }
                None => {}
            }
        }

        let final_dir = runtime_dir_in(models_dir);
        if final_dir.exists() {
            std::fs::remove_dir_all(&final_dir)
                .map_err(|e| format!("Couldn't replace the old voice engine: {e}"))?;
        }
        std::fs::rename(&installing, &final_dir)
            .map_err(|e| format!("Couldn't install the voice engine: {e}"))
    })();

    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&installing);
    if result.is_ok() {
        let _ = std::fs::remove_file(archive);
        // New files: let the next voice try loading them again.
        LOAD_FAILED.store(false, Ordering::Relaxed);
        info!("Native voice engine installed (sherpa-onnx {SHERPA_VERSION})");
    }
    result
}

/// Depth-first search for a file by name under `dir` (the archives nest their
/// libraries one or two folders down).
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
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

/// Where the model manager keeps its files. Set once at startup so the speech
/// paths, which only have settings in hand, can find the packs.
static MODELS_DIR: OnceCell<PathBuf> = OnceCell::new();

/// Record the models directory, and tidy a runtime nobody needs any more: one
/// left behind by an older version, or one whose last voice pack was removed.
pub fn init(models_dir: PathBuf) {
    let tts_runtime = models_dir.join("tts-runtime");
    if let Ok(entries) = std::fs::read_dir(&tts_runtime) {
        let current = runtime_dir_in(&models_dir);
        for entry in entries.flatten() {
            let path = entry.path();
            if path != current {
                debug!("Removing stale native voice files at {}", path.display());
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
            }
        }
    }
    let any_pack = PACKS.iter().any(|pack| models_dir.join(pack.dir).is_dir());
    if !any_pack && tts_runtime.exists() {
        debug!("No native voice pack installed; removing the unused voice engine");
        let _ = std::fs::remove_dir_all(&tts_runtime);
    }
    let _ = MODELS_DIR.set(models_dir);
}

fn models_dir() -> Option<&'static Path> {
    MODELS_DIR.get().map(PathBuf::as_path)
}

/// Whether a pack and the runtime it needs are both installed.
pub fn pack_ready(pack: &VoicePack) -> bool {
    !load_failed()
        && models_dir().is_some_and(|dir| {
            native_supported() && runtime_installed_in(dir) && pack_installed_in(dir, pack)
        })
}

/// Remove the runtime once no voice pack is left to use it. Skipped while the
/// libraries are loaded: Windows cannot delete a mapped DLL, and the next
/// launch's [`init`] cleans it up instead.
pub fn remove_runtime_if_unused(models_dir: &Path) {
    let any_pack = PACKS.iter().any(|pack| models_dir.join(pack.dir).is_dir());
    if any_pack || RUNTIME_LIB.lock().map(|r| r.is_some()).unwrap_or(true) {
        return;
    }
    let dir = models_dir.join("tts-runtime");
    if dir.exists() {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => info!("Removed the native voice engine (no voice pack uses it)"),
            Err(e) => warn!("Couldn't remove the native voice engine: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Where speech is produced
// ---------------------------------------------------------------------------

/// Where the selected engine's speech is produced right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VoiceRoute {
    /// Kokoro inside the assistant panel (kokoro-js, WebGPU or WebAssembly).
    Webview,
    /// This module (sherpa-onnx on the processor).
    Native,
    /// A cloud or self-hosted engine, spoken by `tts.rs`.
    Remote,
}

/// What the WebView reported about WebGPU on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WebGpuState {
    /// Nothing reported yet this session.
    Unknown,
    /// An adapter exists and has not been caught producing broken audio.
    Usable,
    /// No adapter, it failed to start, or its audio was garbled.
    Unusable,
}

static WEBGPU: AtomicU8 = AtomicU8::new(0);

pub fn webgpu_state() -> WebGpuState {
    match WEBGPU.load(Ordering::SeqCst) {
        1 => WebGpuState::Usable,
        2 => WebGpuState::Unusable,
        _ => WebGpuState::Unknown,
    }
}

/// Record the WebView's report. Returns whether anything changed.
pub fn report_webgpu(usable: bool) -> bool {
    let value = if usable { 1 } else { 2 };
    WEBGPU.swap(value, Ordering::SeqCst) != value
}

/// Where Kokoro should run, as the user asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KokoroDevice {
    Auto,
    Gpu,
    Cpu,
}

impl KokoroDevice {
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "gpu" => Self::Gpu,
            "cpu" => Self::Cpu,
            _ => Self::Auto,
        }
    }
}

/// The routing rule for Kokoro, separated from global state so every branch is
/// testable.
///
/// Automatic prefers the graphics card whenever the WebView can use it — on a
/// working GPU that is the fastest path and the one the owner's machines run
/// instantly — and only moves to the processor when the WebView reported no
/// usable GPU. In every case where the processor voice is wanted but not yet
/// downloaded, the answer is the WebView, which still speaks (on its slower
/// WebAssembly fallback) rather than going silent.
pub fn resolve_kokoro_route(
    device: KokoroDevice,
    webgpu: WebGpuState,
    native_ready: bool,
) -> VoiceRoute {
    let want_native = match device {
        KokoroDevice::Gpu => false,
        KokoroDevice::Cpu => true,
        KokoroDevice::Auto => webgpu == WebGpuState::Unusable,
    };
    if want_native && native_ready {
        VoiceRoute::Native
    } else {
        VoiceRoute::Webview
    }
}

/// Where the configured engine speaks.
pub fn route(settings: &AppSettings) -> VoiceRoute {
    match settings.assistant_tts_engine.trim() {
        "kitten" => VoiceRoute::Native,
        "" | "kokoro" => resolve_kokoro_route(
            KokoroDevice::parse(&settings.assistant_tts_kokoro_device),
            webgpu_state(),
            pack_ready(&KOKORO_PACK),
        ),
        _ => VoiceRoute::Remote,
    }
}

/// Whether speech is produced by the panel's WebView.
pub fn uses_webview(settings: &AppSettings) -> bool {
    route(settings) == VoiceRoute::Webview
}

/// The pack that speaks for these settings, when the route is native.
pub fn active_pack(settings: &AppSettings) -> Option<&'static VoicePack> {
    if route(settings) != VoiceRoute::Native {
        return None;
    }
    match settings.assistant_tts_engine.trim() {
        "kitten" => Some(&KITTEN_PACK),
        _ => Some(&KOKORO_PACK),
    }
}

/// Why the configured native voice cannot speak, or `None` when it can. Only
/// Kitten can be blocked: Kokoro falls back to the WebView when its processor
/// pack is missing.
pub fn blocker(settings: &AppSettings) -> Option<String> {
    if settings.assistant_tts_engine.trim() != "kitten" {
        return None;
    }
    if !native_supported() {
        return Some(
            "Kitten isn't available on this computer yet. Switch the voice to Kokoro in Models → Voice."
                .to_string(),
        );
    }
    if load_failed() {
        return Some(
            "Kitten's voice engine couldn't start on this computer. Switch the voice to Kokoro in Models → Voice."
                .to_string(),
        );
    }
    if !pack_ready(&KITTEN_PACK) {
        return Some(
            "The assistant's voice is set to Kitten, which isn't downloaded yet. Download it in Models → Voice, or switch the voice to Kokoro."
                .to_string(),
        );
    }
    None
}

/// Everything the Voice settings need to explain the current state.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LocalVoiceStatus {
    pub route: VoiceRoute,
    pub webgpu: WebGpuState,
    /// This platform has a native engine build.
    pub native_supported: bool,
    /// The downloaded engine refused to load this session.
    pub native_load_failed: bool,
    pub kokoro_native_ready: bool,
    pub kitten_ready: bool,
}

pub fn status(settings: &AppSettings) -> LocalVoiceStatus {
    LocalVoiceStatus {
        route: route(settings),
        webgpu: webgpu_state(),
        native_supported: native_supported(),
        native_load_failed: load_failed(),
        kokoro_native_ready: pack_ready(&KOKORO_PACK),
        kitten_ready: pack_ready(&KITTEN_PACK),
    }
}

// ---------------------------------------------------------------------------
// Voices, speed, threads
// ---------------------------------------------------------------------------

/// Kokoro v1.0's English speakers, in the order of the model's `voices.bin`
/// (the model's own `id2speaker` metadata; ids 28+ are other languages).
const KOKORO_VOICES: [&str; 28] = [
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_heart",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
];

pub const DEFAULT_KOKORO_VOICE: &str = "af_heart";

/// Kitten 0.8's voices by the names its authors give them, in `voices.bin`
/// order (`expr-voice-2-m`, `-2-f`, `-3-m`, … as sherpa-onnx packs them).
pub const KITTEN_VOICES: [(&str, &str); 8] = [
    ("Jasper", "expr-voice-2-m"),
    ("Bella", "expr-voice-2-f"),
    ("Bruno", "expr-voice-3-m"),
    ("Luna", "expr-voice-3-f"),
    ("Hugo", "expr-voice-4-m"),
    ("Rosie", "expr-voice-4-f"),
    ("Leo", "expr-voice-5-m"),
    ("Kiki", "expr-voice-5-f"),
];

pub const DEFAULT_KITTEN_VOICE: &str = "Bella";

/// Speaker id for a Kokoro voice name, falling back to Heart for anything this
/// model does not know (a hand-edited setting, a voice from a newer build).
fn kokoro_sid(voice: &str) -> i32 {
    let voice = voice.trim();
    KOKORO_VOICES
        .iter()
        .position(|name| *name == voice)
        .or_else(|| {
            KOKORO_VOICES
                .iter()
                .position(|n| *n == DEFAULT_KOKORO_VOICE)
        })
        .unwrap_or(0) as i32
}

/// British voices are phonemized with eSpeak's British English (`en`); every
/// other English voice uses the model's default `en-us`.
fn kokoro_is_british(voice: &str) -> bool {
    let voice = voice.trim();
    voice.starts_with("bf_") || voice.starts_with("bm_")
}

/// Speaker id for a Kitten voice, by friendly name or `expr-voice-*` id,
/// case-insensitively. Unknown names get Bella.
fn kitten_sid(voice: &str) -> i32 {
    let voice = voice.trim();
    let find = |v: &str| {
        KITTEN_VOICES
            .iter()
            .position(|(name, id)| name.eq_ignore_ascii_case(v) || id.eq_ignore_ascii_case(v))
    };
    find(voice)
        .or_else(|| find(DEFAULT_KITTEN_VOICE))
        .unwrap_or(0) as i32
}

/// The speed the model is asked for, from the app's 0.25–4x setting.
fn model_speed(family: VoiceFamily, user_speed: f64) -> f32 {
    let speed = if user_speed.is_finite() {
        user_speed
    } else {
        1.0
    }
    .clamp(0.25, 4.0);
    match family {
        VoiceFamily::Kokoro => speed as f32,
        VoiceFamily::Kitten => (speed * KITTEN_PACE).clamp(0.25, 4.0) as f32,
    }
}

/// Intra-op threads for a family on a machine with `logical` CPUs.
///
/// Half the logical CPUs, i.e. the physical cores on an SMT machine: on a
/// 16-thread Ryzen, Kokoro measured RTF 0.27 at 4 threads, 0.20 at 8, and got
/// slower beyond. Kitten is small enough that 4 threads is already its ceiling.
fn threads_for(family: VoiceFamily, logical: usize) -> i32 {
    let physical = (logical / 2).max(1);
    let cap = match family {
        VoiceFamily::Kokoro => 8,
        VoiceFamily::Kitten => 4,
    };
    physical.min(cap) as i32
}

fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Text as the engine should see it: control characters and runs of whitespace
/// collapsed to single spaces. The speech pipeline has already removed Markdown
/// and emoji; this only guards the C boundary (an interior NUL would truncate).
fn engine_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() {
            if !out.is_empty() {
                space = true;
            }
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// FFI: mirrors sherpa-onnx v1.13.8 `c-api.h`, field for field
// ---------------------------------------------------------------------------

#[repr(C)]
struct VitsModelConfig {
    model: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: f32,
    noise_scale_w: f32,
    length_scale: f32,
    dict_dir: *const c_char,
}

#[repr(C)]
struct MatchaModelConfig {
    acoustic_model: *const c_char,
    vocoder: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: f32,
    length_scale: f32,
    dict_dir: *const c_char,
}

#[repr(C)]
struct KokoroModelConfig {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: f32,
    dict_dir: *const c_char,
    lexicon: *const c_char,
    lang: *const c_char,
}

#[repr(C)]
struct KittenModelConfig {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: f32,
}

#[repr(C)]
struct ZipvoiceModelConfig {
    tokens: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    vocoder: *const c_char,
    data_dir: *const c_char,
    lexicon: *const c_char,
    feat_scale: f32,
    t_shift: f32,
    target_rms: f32,
    guidance_scale: f32,
}

#[repr(C)]
struct PocketModelConfig {
    lm_flow: *const c_char,
    lm_main: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    text_conditioner: *const c_char,
    vocab_json: *const c_char,
    token_scores_json: *const c_char,
    voice_embedding_cache_capacity: i32,
}

#[repr(C)]
struct SupertonicModelConfig {
    duration_predictor: *const c_char,
    text_encoder: *const c_char,
    vector_estimator: *const c_char,
    vocoder: *const c_char,
    tts_json: *const c_char,
    unicode_indexer: *const c_char,
    voice_style: *const c_char,
}

#[repr(C)]
struct TtsModelConfig {
    vits: VitsModelConfig,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    matcha: MatchaModelConfig,
    kokoro: KokoroModelConfig,
    kitten: KittenModelConfig,
    zipvoice: ZipvoiceModelConfig,
    pocket: PocketModelConfig,
    supertonic: SupertonicModelConfig,
}

#[repr(C)]
struct TtsConfig {
    model: TtsModelConfig,
    rule_fsts: *const c_char,
    max_num_sentences: i32,
    rule_fars: *const c_char,
    silence_scale: f32,
}

#[repr(C)]
struct GenerationConfig {
    silence_scale: f32,
    speed: f32,
    sid: i32,
    reference_audio: *const f32,
    reference_audio_len: i32,
    reference_sample_rate: i32,
    reference_text: *const c_char,
    num_steps: i32,
    extra: *const c_char,
}

#[repr(C)]
struct GeneratedAudio {
    samples: *const f32,
    n: i32,
    sample_rate: i32,
}

type ProgressCallback = unsafe extern "C" fn(*const f32, i32, f32, *mut c_void) -> i32;

struct Api {
    version: unsafe extern "C" fn() -> *const c_char,
    create: unsafe extern "C" fn(*const TtsConfig) -> *const c_void,
    destroy: unsafe extern "C" fn(*const c_void),
    sample_rate: unsafe extern "C" fn(*const c_void) -> i32,
    generate: unsafe extern "C" fn(
        *const c_void,
        *const c_char,
        *const GenerationConfig,
        Option<ProgressCallback>,
        *mut c_void,
    ) -> *const GeneratedAudio,
    destroy_audio: unsafe extern "C" fn(*const GeneratedAudio),
}

/// The loaded libraries. Kept for the life of the process once loaded: ONNX
/// Runtime owns thread pools and thread-local state, and unloading it under a
/// live process is not something it supports.
struct Runtime {
    api: Api,
    _ort: libloading::Library,
    _c_api: libloading::Library,
}

static RUNTIME_LIB: Lazy<Mutex<Option<Arc<Runtime>>>> = Lazy::new(|| Mutex::new(None));

#[cfg(target_os = "windows")]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::windows::{Library, LOAD_WITH_ALTERED_SEARCH_PATH};
    // Resolve the library's own imports from its folder first, so
    // sherpa-onnx-c-api.dll binds to the onnxruntime.dll beside it and never to
    // the unrelated copy Windows ships in System32.
    Library::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH).map(Into::into)
}

#[cfg(unix)]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
    Library::open(Some(path), RTLD_NOW | RTLD_LOCAL).map(Into::into)
}

/// Load the runtime once; later calls return the same handle.
fn runtime() -> Result<Arc<Runtime>, String> {
    let mut guard = RUNTIME_LIB
        .lock()
        .map_err(|_| "The voice engine lock was poisoned".to_string())?;
    if let Some(runtime) = guard.as_ref() {
        return Ok(runtime.clone());
    }
    let dir = models_dir()
        .map(runtime_dir_in)
        .ok_or_else(|| "The voice engine isn't set up yet".to_string())?;
    if !dir.join(ORT_LIB).is_file() || !dir.join(C_API_LIB).is_file() {
        return Err(
            "The voice engine isn't downloaded. Download the voice again in Models → Voice."
                .to_string(),
        );
    }
    // SAFETY: both libraries come from the pinned, hash-verified sherpa-onnx
    // release; their initializers only set up ONNX Runtime's own state. The
    // symbol types below are the declarations from that release's c-api.h.
    let loaded = unsafe { load_runtime_from(&dir) };
    let runtime = match loaded {
        Ok(runtime) => runtime,
        Err(e) => {
            // Files present but unusable: stop routing voices here until the
            // next launch (or a fresh download), so replies fall back instead
            // of failing one after another.
            if !LOAD_FAILED.swap(true, Ordering::Relaxed) {
                warn!("Native voice engine failed to load; using the in-app voice instead: {e}");
                drop(guard);
                notify_status();
            }
            return Err(e);
        }
    };
    info!("Native voice engine loaded (sherpa-onnx {SHERPA_VERSION})");
    let runtime = Arc::new(runtime);
    *guard = Some(runtime.clone());
    Ok(runtime)
}

/// Open both libraries in `dir`, resolve the C API, and check its version.
///
/// # Safety
/// `dir` must hold the pinned sherpa-onnx release (see [`runtime`]).
unsafe fn load_runtime_from(dir: &Path) -> Result<Runtime, String> {
    let ort = open_library(&dir.join(ORT_LIB))
        .map_err(|e| format!("Couldn't load the voice engine ({ORT_LIB}): {e}"))?;
    let c_api = open_library(&dir.join(C_API_LIB))
        .map_err(|e| format!("Couldn't load the voice engine ({C_API_LIB}): {e}"))?;
    macro_rules! symbol {
        ($name:literal) => {
            *c_api
                .get($name)
                .map_err(|e| format!("The voice engine is missing a function: {e}"))?
        };
    }
    let api = Api {
        version: symbol!(b"SherpaOnnxGetVersionStr\0"),
        create: symbol!(b"SherpaOnnxCreateOfflineTts\0"),
        destroy: symbol!(b"SherpaOnnxDestroyOfflineTts\0"),
        sample_rate: symbol!(b"SherpaOnnxOfflineTtsSampleRate\0"),
        generate: symbol!(b"SherpaOnnxOfflineTtsGenerateWithConfig\0"),
        destroy_audio: symbol!(b"SherpaOnnxDestroyOfflineTtsGeneratedAudio\0"),
    };
    let version_ptr = (api.version)();
    let version = if version_ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(version_ptr).to_string_lossy().into_owned()
    };
    if version != SHERPA_VERSION {
        return Err(format!(
            "The voice engine on disk is version {version:?}, but this app needs {SHERPA_VERSION}. Download the voice again in Models → Voice."
        ));
    }
    Ok(Runtime {
        api,
        _ort: ort,
        _c_api: c_api,
    })
}

/// A path the engine can open. The C API takes narrow strings, which the
/// Windows build opens in the ANSI code page, so a profile folder with a
/// non-ASCII name would fail to load. The 8.3 short name is ASCII and opens the
/// same file.
fn engine_path(path: &Path) -> Result<CString, String> {
    let text = path.to_string_lossy().into_owned();
    #[cfg(target_os = "windows")]
    if !text.is_ascii() {
        if let Some(short) = short_path(path) {
            return CString::new(short).map_err(|_| "Invalid path".to_string());
        }
        warn!("Native voice path is not ASCII and has no short name: {text}");
    }
    CString::new(text).map_err(|_| "Invalid path".to_string())
}

#[cfg(target_os = "windows")]
fn short_path(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long_path: *const u16, short_path: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and `buffer` is writable for its length.
    let len = unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
    if len == 0 || len as usize >= buffer.len() {
        return None;
    }
    let short = String::from_utf16_lossy(&buffer[..len as usize]);
    short.is_ascii().then_some(short)
}

// ---------------------------------------------------------------------------
// The loaded engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EngineKey {
    pack: &'static str,
    threads: i32,
}

struct Engine {
    handle: *const c_void,
    key: EngineKey,
    runtime: Arc<Runtime>,
}

// SAFETY: a sherpa-onnx OfflineTts handle may be used from any thread; this
// module only ever uses it under the `ENGINE` mutex, so never concurrently.
unsafe impl Send for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: `handle` came from this runtime's create and is destroyed once.
        unsafe { (self.runtime.api.destroy)(self.handle) };
        debug!("Native voice released ({})", self.key.pack);
    }
}

struct Loaded {
    engine: Engine,
    last_used: Instant,
}

static ENGINE: Lazy<Mutex<Option<Loaded>>> = Lazy::new(|| Mutex::new(None));
static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);

/// Release the engine after [`IDLE_UNLOAD`] without use. `try_lock` so the
/// watcher never waits behind a synthesis in progress, which is use anyway.
fn ensure_idle_watcher() {
    if WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("native-voice-idle".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_secs(15));
            if let Ok(mut guard) = ENGINE.try_lock() {
                if guard
                    .as_ref()
                    .is_some_and(|loaded| loaded.last_used.elapsed() >= IDLE_UNLOAD)
                {
                    info!("Native voice idle; releasing its memory");
                    *guard = None;
                }
            }
        });
    if spawned.is_err() {
        WATCHER_STARTED.store(false, Ordering::SeqCst);
    }
}

fn create_engine(
    runtime: &Arc<Runtime>,
    pack: &'static VoicePack,
    threads: i32,
) -> Result<Engine, String> {
    let dir = models_dir()
        .map(|models| models.join(pack.dir))
        .ok_or_else(|| "The voice isn't set up yet".to_string())?;
    let model = dir.join(pack.model_file);
    let voices = dir.join("voices.bin");
    let tokens = dir.join("tokens.txt");
    let data_dir = dir.join("espeak-ng-data");
    // Checked here rather than left to the engine: some of its missing-file
    // paths end the whole process instead of returning an error.
    if !model.is_file() || !voices.is_file() || !tokens.is_file() || !data_dir.is_dir() {
        return Err(
            "This voice's files are missing. Download it again in Models → Voice.".to_string(),
        );
    }
    let model = engine_path(&model)?;
    let voices = engine_path(&voices)?;
    let tokens = engine_path(&tokens)?;
    let data_dir = engine_path(&data_dir)?;
    let provider = CString::new("cpu").expect("static string");
    // Kokoro v1.0 is multilingual and needs a lexicon or a language. English
    // goes through eSpeak whenever a language is set, so the 6 MB lexicon adds
    // nothing but load time (measured: 1.5 s → 1.1 s, 42 MB less memory).
    let lang = CString::new("en-us").expect("static string");

    // SAFETY: a zeroed config is the documented starting point (every pointer
    // null, every number zero); only the chosen family's block is filled in.
    let mut config: TtsConfig = unsafe { std::mem::zeroed() };
    config.model.num_threads = threads;
    config.model.provider = provider.as_ptr();
    config.max_num_sentences = 1;
    config.silence_scale = 0.2;
    match pack.family {
        VoiceFamily::Kokoro => {
            config.model.kokoro.model = model.as_ptr();
            config.model.kokoro.voices = voices.as_ptr();
            config.model.kokoro.tokens = tokens.as_ptr();
            config.model.kokoro.data_dir = data_dir.as_ptr();
            config.model.kokoro.length_scale = 1.0;
            config.model.kokoro.lang = lang.as_ptr();
        }
        VoiceFamily::Kitten => {
            config.model.kitten.model = model.as_ptr();
            config.model.kitten.voices = voices.as_ptr();
            config.model.kitten.tokens = tokens.as_ptr();
            config.model.kitten.data_dir = data_dir.as_ptr();
            config.model.kitten.length_scale = 1.0;
        }
    }

    let started = Instant::now();
    // SAFETY: every pointer in `config` is either null or a CString alive until
    // after this call; the engine copies what it keeps.
    let handle = unsafe { (runtime.api.create)(&config) };
    if handle.is_null() {
        return Err(
            "The local voice couldn't start. Try downloading it again in Models → Voice."
                .to_string(),
        );
    }
    info!(
        "Native voice loaded: {} with {} threads in {:?}",
        pack.model_id,
        threads,
        started.elapsed()
    );
    Ok(Engine {
        handle,
        key: EngineKey {
            pack: pack.model_id,
            threads,
        },
        runtime: runtime.clone(),
    })
}

/// Lets a Stop end synthesis at the next sentence instead of after the whole
/// piece: the engine calls this after every sentence and stops on 0.
unsafe extern "C" fn keep_going(
    _samples: *const f32,
    _n: i32,
    _progress: f32,
    arg: *mut c_void,
) -> i32 {
    // SAFETY: `arg` points at the `u64` epoch owned by `synthesize_pcm`'s frame,
    // which outlives the generate call that invokes this.
    let epoch = unsafe { *(arg as *const u64) };
    i32::from(crate::tts::current_epoch() == epoch)
}

/// One request for the native engine.
#[derive(Debug, Clone)]
pub struct NativeRequest {
    pub pack: &'static VoicePack,
    pub voice: String,
    pub speed: f64,
}

impl NativeRequest {
    /// The request these settings describe, when their route is native.
    pub fn from_settings(settings: &AppSettings) -> Option<Self> {
        let pack = active_pack(settings)?;
        let voice = match pack.family {
            VoiceFamily::Kokoro => settings.assistant_tts_voice.clone(),
            VoiceFamily::Kitten => settings.assistant_tts_remote_voice.clone(),
        };
        Some(Self {
            pack,
            voice,
            speed: settings.assistant_tts_speed,
        })
    }
}

/// Synthesize `text` into mono samples. Blocking; call off the async runtime.
/// Returns `Ok(None)` when a Stop superseded the request mid-way.
fn synthesize_pcm(request: &NativeRequest, text: &str) -> Result<Option<(Vec<f32>, u32)>, String> {
    let text = engine_text(text);
    if text.is_empty() {
        return Err("There was nothing to say".to_string());
    }
    let epoch = crate::tts::current_epoch();
    let runtime = runtime()?;
    let family = request.pack.family;
    let key = EngineKey {
        pack: request.pack.model_id,
        threads: threads_for(family, logical_cpus()),
    };

    let mut guard = ENGINE
        .lock()
        .map_err(|_| "The voice engine lock was poisoned".to_string())?;
    if guard.as_ref().is_none_or(|loaded| loaded.engine.key != key) {
        // Drop the old engine before creating the next, so two are never
        // resident at once.
        *guard = None;
        let engine = create_engine(&runtime, request.pack, key.threads)?;
        *guard = Some(Loaded {
            engine,
            last_used: Instant::now(),
        });
        ensure_idle_watcher();
    }
    let loaded = guard.as_mut().expect("engine loaded above");

    let (sid, extra) = match family {
        VoiceFamily::Kokoro => (
            kokoro_sid(&request.voice),
            kokoro_is_british(&request.voice)
                .then(|| CString::new(r#"{"lang":"en"}"#).expect("static")),
        ),
        VoiceFamily::Kitten => (kitten_sid(&request.voice), None),
    };
    let c_text = CString::new(text).map_err(|_| "Invalid text".to_string())?;
    // SAFETY: zeroed is the documented default; the fields set are plain values
    // and pointers that outlive the call.
    let mut generation: GenerationConfig = unsafe { std::mem::zeroed() };
    generation.silence_scale = 0.2;
    generation.speed = model_speed(family, request.speed);
    generation.sid = sid;
    generation.extra = extra.as_ref().map_or(std::ptr::null(), |e| e.as_ptr());
    let mut epoch_arg: u64 = epoch;

    let started = Instant::now();
    // SAFETY: the handle is live under the lock; `c_text`, `generation` and
    // `epoch_arg` outlive the call; `keep_going` matches the callback type.
    let audio = unsafe {
        (loaded.engine.runtime.api.generate)(
            loaded.engine.handle,
            c_text.as_ptr(),
            &generation,
            Some(keep_going),
            &mut epoch_arg as *mut u64 as *mut c_void,
        )
    };
    loaded.last_used = Instant::now();
    if crate::tts::current_epoch() != epoch {
        if !audio.is_null() {
            // SAFETY: returned by generate, freed once.
            unsafe { (loaded.engine.runtime.api.destroy_audio)(audio) };
        }
        return Ok(None);
    }
    if audio.is_null() {
        return Err("The local voice couldn't read that text aloud".to_string());
    }
    // SAFETY: a non-null result owns `n` samples until destroy_audio.
    let (samples, rate) = unsafe {
        let result = &*audio;
        let samples = if result.samples.is_null() || result.n <= 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(result.samples, result.n as usize).to_vec()
        };
        let rate = result.sample_rate;
        (loaded.engine.runtime.api.destroy_audio)(audio);
        (samples, rate)
    };
    let rate = if rate > 0 {
        rate as u32
    } else {
        // SAFETY: live handle under the lock.
        unsafe { (loaded.engine.runtime.api.sample_rate)(loaded.engine.handle) }.max(1) as u32
    };
    debug!(
        "Native voice: {:.2}s of audio in {:?}",
        samples.len() as f64 / rate as f64,
        started.elapsed()
    );
    if samples.is_empty() {
        return Err("The local voice produced no audio".to_string());
    }
    Ok(Some((samples, rate)))
}

/// Encode mono float samples as a 16-bit PCM WAV file.
fn samples_to_wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    crate::tts::pcm_to_wav(&pcm, rate, 1, 16)
}

/// Synthesize `text` to WAV bytes. Blocking. `Ok(empty)` means a Stop won.
pub fn synthesize_wav(request: &NativeRequest, text: &str) -> Result<Vec<u8>, String> {
    Ok(synthesize_pcm(request, text)?
        .map(|(samples, rate)| samples_to_wav(&samples, rate))
        .unwrap_or_default())
}

/// Load the engine for these settings in the background, so the first reply of
/// a call does not wait for it. A two-word synthesis also pays ONNX Runtime's
/// first-run cost (~1 s on Kokoro) before anyone is listening.
pub fn prewarm(settings: &AppSettings) {
    let Some(request) = NativeRequest::from_settings(settings) else {
        return;
    };
    if !pack_ready(request.pack) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("native-voice-prewarm".into())
        .spawn(move || {
            let started = Instant::now();
            match synthesize_pcm(&request, "Hi.") {
                Ok(_) => debug!("Native voice warmed in {:?}", started.elapsed()),
                Err(e) => warn!("Native voice warm-up failed: {e}"),
            }
        });
}

/// Release the engine now if it belongs to `model_id` (before its files are
/// deleted), or unconditionally when `model_id` is `None`.
pub fn release(model_id: Option<&str>) {
    if let Ok(mut guard) = ENGINE.lock() {
        if guard
            .as_ref()
            .is_some_and(|loaded| model_id.is_none_or(|id| loaded.engine.key.pack == id))
        {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library that cannot be opened is an error that names it, never a
    /// crash. This is the path macOS library validation takes, and it is what
    /// arms the fallback in [`runtime`].
    #[test]
    fn a_runtime_that_cannot_be_opened_is_a_named_error() {
        let dir = std::env::temp_dir().join(format!(
            "speakoflow-native-tts-empty-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: nothing in the folder can be loaded, so no foreign code runs.
        let error = match unsafe { load_runtime_from(&dir) } {
            Ok(_) => panic!("an empty folder must not load"),
            Err(error) => error,
        };
        assert!(error.contains(ORT_LIB), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole point of Automatic: the graphics card is used whenever the
    /// WebView can use it, the processor only when it cannot, and a missing
    /// processor pack never leaves the voice silent.
    #[test]
    fn automatic_prefers_the_gpu_and_falls_back_to_the_processor() {
        use KokoroDevice::*;
        use WebGpuState::*;
        assert_eq!(
            resolve_kokoro_route(Auto, Usable, true),
            VoiceRoute::Webview
        );
        assert_eq!(
            resolve_kokoro_route(Auto, Unknown, true),
            VoiceRoute::Webview
        );
        assert_eq!(
            resolve_kokoro_route(Auto, Unusable, true),
            VoiceRoute::Native
        );
        assert_eq!(
            resolve_kokoro_route(Auto, Unusable, false),
            VoiceRoute::Webview,
            "without the processor pack the WebView still speaks"
        );
    }

    #[test]
    fn an_explicit_choice_wins_over_detection() {
        use KokoroDevice::*;
        use WebGpuState::*;
        assert_eq!(
            resolve_kokoro_route(Gpu, Unusable, true),
            VoiceRoute::Webview
        );
        assert_eq!(resolve_kokoro_route(Cpu, Usable, true), VoiceRoute::Native);
        assert_eq!(
            resolve_kokoro_route(Cpu, Usable, false),
            VoiceRoute::Webview
        );
    }

    #[test]
    fn device_setting_parses_leniently() {
        assert_eq!(KokoroDevice::parse("gpu"), KokoroDevice::Gpu);
        assert_eq!(KokoroDevice::parse(" cpu "), KokoroDevice::Cpu);
        assert_eq!(KokoroDevice::parse("auto"), KokoroDevice::Auto);
        assert_eq!(KokoroDevice::parse("something-else"), KokoroDevice::Auto);
    }

    #[test]
    fn engine_ids_route_to_the_right_place() {
        let mut settings = crate::settings::get_default_settings();
        settings.assistant_tts_engine = "kitten".into();
        assert_eq!(route(&settings), VoiceRoute::Native);
        settings.assistant_tts_engine = "elevenlabs".into();
        assert_eq!(route(&settings), VoiceRoute::Remote);
        settings.assistant_tts_engine = "kokoro".into();
        settings.assistant_tts_kokoro_device = "gpu".into();
        assert_eq!(route(&settings), VoiceRoute::Webview);
    }

    /// Speaker ids are the model's own order; getting one wrong silently speaks
    /// with someone else's voice, so the voices the app offers are pinned.
    #[test]
    fn kokoro_voices_map_to_the_models_speaker_ids() {
        assert_eq!(kokoro_sid("af_heart"), 3);
        assert_eq!(kokoro_sid("af_bella"), 2);
        assert_eq!(kokoro_sid("af_nicole"), 6);
        assert_eq!(kokoro_sid("af_sky"), 10);
        assert_eq!(kokoro_sid("am_adam"), 11);
        assert_eq!(kokoro_sid("am_michael"), 16);
        assert_eq!(kokoro_sid("bf_emma"), 21);
        assert_eq!(kokoro_sid("bm_george"), 26);
        assert_eq!(kokoro_sid("not-a-voice"), 3, "unknown voices get Heart");
    }

    #[test]
    fn british_voices_use_british_phonemes() {
        assert!(kokoro_is_british("bf_emma"));
        assert!(kokoro_is_british("bm_george"));
        assert!(!kokoro_is_british("af_heart"));
        assert!(!kokoro_is_british("am_adam"));
    }

    #[test]
    fn kitten_voices_accept_names_and_ids() {
        assert_eq!(kitten_sid("Jasper"), 0);
        assert_eq!(kitten_sid("Bella"), 1);
        assert_eq!(kitten_sid("bella"), 1);
        assert_eq!(kitten_sid("expr-voice-5-f"), 7);
        assert_eq!(kitten_sid("Kiki"), 7);
        assert_eq!(kitten_sid(""), 1, "empty means the default, Bella");
        assert_eq!(
            kitten_sid("af_heart"),
            1,
            "a Kokoro voice is not a Kitten voice"
        );
    }

    #[test]
    fn speed_is_clamped_and_kitten_is_paced() {
        assert_eq!(model_speed(VoiceFamily::Kokoro, 1.0), 1.0);
        assert_eq!(model_speed(VoiceFamily::Kokoro, 10.0), 4.0);
        assert_eq!(model_speed(VoiceFamily::Kokoro, f64::NAN), 1.0);
        assert!((model_speed(VoiceFamily::Kitten, 1.0) - 1.6).abs() < 1e-6);
        assert_eq!(model_speed(VoiceFamily::Kitten, 4.0), 4.0);
    }

    #[test]
    fn threads_follow_physical_cores_within_a_cap() {
        assert_eq!(threads_for(VoiceFamily::Kokoro, 16), 8);
        assert_eq!(threads_for(VoiceFamily::Kokoro, 32), 8);
        assert_eq!(threads_for(VoiceFamily::Kokoro, 8), 4);
        assert_eq!(threads_for(VoiceFamily::Kokoro, 2), 1);
        assert_eq!(threads_for(VoiceFamily::Kokoro, 1), 1);
        assert_eq!(threads_for(VoiceFamily::Kitten, 16), 4);
    }

    #[test]
    fn engine_text_collapses_whitespace_and_controls() {
        assert_eq!(engine_text("  Hello,\n\nworld\t!  "), "Hello, world !");
        assert_eq!(engine_text("a\u{0}b"), "a b");
        assert_eq!(engine_text("   "), "");
    }

    #[test]
    fn wav_encoding_is_16_bit_mono_and_clamped() {
        let wav = samples_to_wav(&[0.0, 1.0, -1.0, 2.0], 24_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 8);
        let sample = |i: usize| i16::from_le_bytes([wav[44 + i * 2], wav[45 + i * 2]]);
        assert_eq!(sample(0), 0);
        assert_eq!(sample(1), i16::MAX);
        assert_eq!(sample(2), -i16::MAX);
        assert_eq!(sample(3), i16::MAX, "out-of-range input is clamped");
    }

    #[test]
    fn packs_are_found_by_catalog_id() {
        assert_eq!(
            pack_for_model_id("kokoro-82m-native").map(|p| p.dir),
            Some("kokoro-multi-lang-v1_0")
        );
        assert_eq!(
            pack_for_model_id("kitten-nano-0.8").map(|p| p.dir),
            Some("kitten-nano-en-v0_8-fp32")
        );
        assert!(pack_for_model_id("kokoro-82m").is_none());
    }

    #[test]
    fn pack_readiness_needs_every_file() {
        let dir = std::env::temp_dir().join(format!("native-voice-pack-{}", std::process::id()));
        let pack_dir = dir.join(KITTEN_PACK.dir);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(pack_dir.join("espeak-ng-data")).unwrap();
        assert!(!pack_installed_in(&dir, &KITTEN_PACK));
        for file in [KITTEN_PACK.model_file, "voices.bin", "tokens.txt"] {
            std::fs::write(pack_dir.join(file), b"x").unwrap();
        }
        assert!(pack_installed_in(&dir, &KITTEN_PACK));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn webgpu_reports_are_tracked() {
        report_webgpu(true);
        assert_eq!(webgpu_state(), WebGpuState::Usable);
        assert!(report_webgpu(false), "a change is reported");
        assert!(
            !report_webgpu(false),
            "the same report again is not a change"
        );
        assert_eq!(webgpu_state(), WebGpuState::Unusable);
        WEBGPU.store(0, Ordering::SeqCst);
    }

    /// Real synthesis through the installed runtime and packs. Needs the app's
    /// models folder with both packs downloaded (or `SPEAKOFLOW_NATIVE_TTS_DIR`
    /// pointing at a folder laid out the same way):
    /// `cargo test --lib native_tts::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_native_synthesis() {
        let dir = std::env::var_os("SPEAKOFLOW_NATIVE_TTS_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("APPDATA").map(|appdata| {
                    PathBuf::from(appdata)
                        .join("com.abhishekbarali.speakoflow")
                        .join("models")
                })
            })
            .expect("a models directory");
        init(dir.clone());
        for (pack, voice) in [
            (&KOKORO_PACK, "af_heart"),
            (&KOKORO_PACK, "bf_emma"),
            (&KITTEN_PACK, "Bella"),
        ] {
            if !pack_ready(pack) {
                eprintln!(
                    "skipping {}: not installed in {}",
                    pack.model_id,
                    dir.display()
                );
                continue;
            }
            let request = NativeRequest {
                pack,
                voice: voice.to_string(),
                speed: 1.0,
            };
            let text = "The capital of France is Paris. It has been the country's political and cultural center for centuries.";
            let started = Instant::now();
            let (samples, rate) = synthesize_pcm(&request, text).unwrap().unwrap();
            let seconds = samples.len() as f64 / rate as f64;
            let elapsed = started.elapsed().as_secs_f64();
            eprintln!(
                "{} {voice}: {seconds:.2}s of audio in {elapsed:.2}s (RTF {:.2}, includes load)",
                pack.model_id,
                elapsed / seconds
            );
            assert_eq!(rate, 24_000);
            assert!(
                seconds > 2.0 && seconds < 12.0,
                "implausible length {seconds}"
            );
            let started = Instant::now();
            let (samples, rate) = synthesize_pcm(&request, text).unwrap().unwrap();
            let seconds = samples.len() as f64 / rate as f64;
            eprintln!(
                "  warm: RTF {:.2}",
                started.elapsed().as_secs_f64() / seconds
            );
        }
        release(None);
    }
}
