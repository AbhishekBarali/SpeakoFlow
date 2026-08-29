use crate::managers::local_models::{self, DiscoveredModel};
use crate::settings::{get_settings, write_settings};
use anyhow::Result;
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tar::Archive;
use tauri::{AppHandle, Emitter, Manager};

/// Outcome of a single download attempt for one URL.
enum AttemptOutcome {
    /// The full body was written to the partial file (caller then finalizes).
    Completed,
    /// The user cancelled mid-stream (partial is kept for a later resume).
    Cancelled,
}

/// Error carrying the HTTP status of a failed download response, so the retry
/// loop can distinguish a transient/server error (worth retrying the same URL)
/// from a permanent client error like 404 (skip straight to the next mirror).
#[derive(Debug)]
struct HttpStatusError {
    status: reqwest::StatusCode,
}

impl std::fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "server returned HTTP {}", self.status)
    }
}

impl std::error::Error for HttpStatusError {}

/// Byte span one worker claims per range request.
///
/// 8 MiB is large enough that per-request overhead (a TLS handshake plus the
/// Hugging Face → CDN redirect) is amortised, and small enough that a dropped
/// connection costs little and progress stays responsive.
const DOWNLOAD_CHUNK_SIZE: u64 = 8 * 1024 * 1024;

/// Concurrent range requests per download.
///
/// A single TCP stream to a distant origin is limited by bandwidth-delay
/// product, not by available bandwidth: one stream measured 10.1 MB/s from
/// Kathmandu to `us-east-1` while eight measured 19.1 MB/s on the same link at
/// the same moment. Eight is where the measured gain flattened; more connections
/// mostly add load without adding throughput.
const DOWNLOAD_CONCURRENCY: usize = 8;

/// Below this, a download finishes before parallelism can pay back its setup
/// (a probe request, preallocation, and a completion record).
const PARALLEL_DOWNLOAD_MIN_BYTES: u64 = 16 * 1024 * 1024;

/// Buffer for the sequential fallback path, so a chunk arriving off the socket
/// is not one unbuffered `write` syscall.
const DOWNLOAD_WRITE_BUFFER: usize = 4 * 1024 * 1024;

/// HTTP client for model downloads.
///
/// **HTTP/1.1 only, deliberately.** hyper's HTTP/2 flow-control window defaults
/// to 64 KiB per stream, and a window that small throttles a long transfer to
/// roughly `window / round-trip time` regardless of link capacity — on a
/// high-latency path to `us-east-1` that is a fraction of a megabyte per second,
/// which is what users far from the origin were seeing. HTTP/2 multiplexing buys
/// nothing for bulk file transfer, and on 1.1 each parallel range request gets
/// its own connection with no shared window to throttle it.
fn download_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("SpeakoFlow/", env!("CARGO_PKG_VERSION")))
        .http1_only()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(DOWNLOAD_CONCURRENCY)
        .build()
}

/// Sidecar recording which chunks of a `.partial` are complete.
///
/// Parallel workers write out of order, so file length no longer implies
/// progress the way it does for a sequential append. Without this record a
/// resumed download cannot tell a written chunk from a preallocated hole, and
/// would produce a file that only fails checksum verification at the very end.
/// One byte per chunk: simple to write incrementally and impossible to
/// half-parse.
fn parts_path_for(partial_path: &Path) -> PathBuf {
    let mut name = partial_path.as_os_str().to_os_string();
    name.push(".parts");
    PathBuf::from(name)
}

/// Write `buf` at an absolute offset without disturbing any other worker's
/// file position. `File::write_all` seeks, which is why it cannot be shared.
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    let mut written = 0usize;
    while written < buf.len() {
        let n = {
            #[cfg(windows)]
            {
                use std::os::windows::fs::FileExt;
                file.seek_write(&buf[written..], offset + written as u64)?
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt;
                file.write_at(&buf[written..], offset + written as u64)?
            }
        };
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "positioned write made no progress",
            ));
        }
        written += n;
    }
    Ok(())
}

/// Total transfer size, but only when the host honours byte ranges.
///
/// Asks for a single byte and reads the total out of `Content-Range`. A plain
/// `HEAD` is not enough: it reports `Content-Length` without proving that a
/// ranged request will be answered with `206`, and a host that silently ignores
/// `Range` would hand every worker the whole body.
async fn probe_ranged_total(client: &reqwest::Client, url: &str) -> Option<u64> {
    let response = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .ok()?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return None;
    }
    let header = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    // "bytes 0-0/833591776"
    let total = header.rsplit('/').next()?.trim().parse::<u64>().ok()?;
    (total > 0).then_some(total)
}

/// How many bytes of `partial_path` are actually downloaded.
///
/// File length answers this for a sequential append, but not for the parallel
/// path: that preallocates the file to its full size up front, so length would
/// report a download as complete the instant it started. When the chunk record
/// exists it is the only honest source, so count the chunks marked done.
fn real_partial_size(partial_path: &Path) -> u64 {
    let Ok(meta) = partial_path.metadata() else {
        return 0;
    };
    let total = meta.len();
    let Ok(record) = fs::read(parts_path_for(partial_path)) else {
        return total;
    };
    let chunk_count = record.len() as u64;
    if chunk_count == 0 || total.div_ceil(DOWNLOAD_CHUNK_SIZE) != chunk_count {
        // The record does not describe this file, so it cannot be trusted to
        // describe its progress either.
        return total;
    }
    record
        .iter()
        .enumerate()
        .filter(|(_, done)| **done == 1)
        .map(|(index, _)| {
            let start = index as u64 * DOWNLOAD_CHUNK_SIZE;
            DOWNLOAD_CHUNK_SIZE.min(total - start)
        })
        .sum()
}

/// Delete a `.partial` together with its chunk record, so a later download
/// cannot find a record describing a file that is no longer there.
fn remove_partial(partial_path: &Path) {
    let _ = fs::remove_file(parts_path_for(partial_path));
    let _ = fs::remove_file(partial_path);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum EngineType {
    Whisper,
    Parakeet,
    Moonshine,
    MoonshineStreaming,
    SenseVoice,
    GigaAM,
    Canary,
    Cohere,
    /// Native transcribe.cpp (ggml/GGUF) engine, added side-by-side with
    /// transcribe-rs for the new single-file GGUF models (batch in Session 2,
    /// real streaming in Session 4). This IS a transcription engine.
    TranscribeCpp,
    /// Local large-language-model engine (GGUF served via the bundled
    /// llama.cpp sidecar). Not a transcription engine.
    LlamaCpp,
    /// Local text-to-speech engine (Kokoro, runs in the assistant webview).
    /// Not a transcription engine.
    Kokoro,
}

impl EngineType {
    /// Whether this engine transcribes speech to text. Only transcription
    /// engines are eligible to be the "active" model used by the recording
    /// pipeline; LLM and TTS engines are managed independently.
    pub fn is_transcription(&self) -> bool {
        !matches!(self, EngineType::LlamaCpp | EngineType::Kokoro)
    }
}

/// The recommended default speech-to-text model for new users: Handy's native
/// transcribe.cpp streaming English model (PLAN.md §4, rank 1). This is what a
/// fresh onboarding features first and what `default_model()` seeds a brand-new
/// install with. Crucially, [`ModelManager::auto_select_model_if_needed`] falls
/// back to any *other* downloaded transcription model when this isn't on disk
/// yet, so the app is never left without a working model (PLAN.md Session 6 / N1).
pub const RECOMMENDED_MODEL_ID: &str = "parakeet-unified-en-0.6b-gguf";

/// The recommended multilingual streaming model (28 languages), offered
/// alongside the English default for multilingual users (PLAN.md §4, rank 2).
/// Surfaced by the same catalog `recommended`/`recommended_rank` metadata that
/// drives ordering, so onboarding lists it right after the English default.
/// Referenced by tests and kept here as the canonical id for future sessions
/// (e.g. S7 FOLLOW_HANDY); the live wiring is the catalog rank, not this const.
#[allow(dead_code)]
pub const RECOMMENDED_MULTILINGUAL_MODEL_ID: &str = "nemotron-3.5-asr-streaming-0.6b-gguf";

/// Internal sentinel returned when the user intentionally cancels a download.
/// The command layer maps it to a failed result for awaiting callers but does
/// not emit the normal download-failed toast.
pub const DOWNLOAD_CANCELLED_ERROR: &str = "Download cancelled";

/// SpeakoFlow Mini — the bundled dictation-cleanup fine-tune.
///
/// It is a `LlamaCpp` model like the chat models, but it is not a chat model:
/// 0.8B parameters trained on exactly one transform (raw English dictation →
/// cleaned English text) with one short system prompt. That makes it the
/// recommended AI-cleanup engine and a poor assistant, which is why the two
/// catalogs feature different models even though they share one download list.
pub const SPEAKOFLOW_MINI_MODEL_ID: &str = "speakoflow-mini";

// Download coordinates for SpeakoFlow Mini, kept together so re-pointing at a
// new build is a four-line change.
//
// Q8_0 is the reference build: it is the file every published evaluation number
// describes, and the repo's own quantisation table puts Q6_K/Q5_K_M/Q4_K_M
// within one case of it on every axis — a difference this evaluation cannot
// resolve — while 7.3% of Q4_K_M outputs differ from Q8_0. So the smaller files
// are not measurably worse, they are just unmeasured, and the default should be
// the build the numbers belong to.
const SPEAKOFLOW_MINI_REPO_ID: &str = "SpeakoFlow/speakoflow-mini-0.8b-GGUF";
const SPEAKOFLOW_MINI_FILENAME: &str = "SpeakoFlow-Mini-0.8B-Q8_0.gguf";
/// 833,591,776 bytes, reported in MiB like every other catalog entry.
const SPEAKOFLOW_MINI_SIZE_MB: u64 = 795;
/// SHA-256 of the Q8_0 reference build, so a truncated or substituted download
/// is caught instead of silently becoming the cleanup engine.
const SPEAKOFLOW_MINI_SHA256: &str =
    "696769bb6911f51bc231b112926e934cf7bfc760e6cdfa24212907bc5ad41fc9";

/// Every model known to be fine-tuned specifically for dictation cleanup.
///
/// Being on this list changes how the app prompts the model: see
/// [`ResolvedPostProcessConfig::trained_for_cleanup`](crate::settings) — the
/// app's own steering (final-output contract, JSON schema) is dropped, because
/// each of those exists to keep a general-purpose chat model on task and each
/// one fights a model already trained for it.
const CLEANUP_SPECIALIST_MODEL_IDS: &[&str] = &[SPEAKOFLOW_MINI_MODEL_ID];

/// Whether `model` is a dictation-cleanup fine-tune.
///
/// Matched on a normalized *name* rather than only the catalog id, so the same
/// weights are recognized however the user got hold of them: the bundled
/// download (`speakoflow-mini`), a `.gguf` they imported from disk
/// (`SpeakoFlow-Mini-Q8_0.gguf`), or a model served by their own Ollama / LM
/// Studio endpoint (`speakoflow_mini:latest`). Prompting is a property of the
/// weights, so recognizing it must not depend on the delivery route.
pub fn is_cleanup_specialist(model: &str) -> bool {
    let normalized: String = model
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    CLEANUP_SPECIALIST_MODEL_IDS
        .iter()
        .any(|id| normalized.contains(id))
}

/// Fill in the derived fields of a `ModelInfo` on its way out to a caller.
///
/// Applied at the two read accessors rather than at the ~30 construction sites
/// so that a model reaching the catalog by *any* route — bundled literal,
/// `catalog.json`, a Hugging Face import, a folder scan — is classified by the
/// same rule. The stored copies keep the default; nothing internal reads the
/// field, because the request path asks [`is_cleanup_specialist`] about the
/// resolved model string directly.
fn stamped(mut info: ModelInfo) -> ModelInfo {
    info.is_cleanup_specialist =
        is_cleanup_specialist(&info.id) || is_cleanup_specialist(&info.name);
    info
}

/// For vision (multimodal) LLM models, the companion multimodal projector that
/// llama.cpp's server needs (passed via `--mmproj`). Returns the local filename
/// to save it as and the download URL, or `None` for text-only models.
pub fn mmproj_for(model_id: &str) -> Option<(&'static str, &'static str)> {
    match model_id {
        "qwen3.5-2b" => Some((
            "mmproj-Qwen_Qwen3.5-2B-f16.gguf",
            "https://huggingface.co/bartowski/Qwen_Qwen3.5-2B-GGUF/resolve/main/mmproj-Qwen_Qwen3.5-2B-f16.gguf",
        )),
        "qwen3.5-4b" => Some((
            "mmproj-Qwen_Qwen3.5-4B-f16.gguf",
            "https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF/resolve/main/mmproj-Qwen_Qwen3.5-4B-f16.gguf",
        )),
        "qwen3.5-9b" => Some((
            "mmproj-Qwen3.5-9B-F16.gguf",
            "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/mmproj-F16.gguf",
        )),
        "qwen3.5-27b" => Some((
            "mmproj-Qwen3.5-27B-F16.gguf",
            "https://huggingface.co/unsloth/Qwen3.5-27B-GGUF/resolve/main/mmproj-F16.gguf",
        )),
        "gemma-4-e2b" => Some((
            "gemma-4-E2B-it-mmproj.gguf",
            "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/main/gemma-4-E2B-it-mmproj.gguf",
        )),
        "gemma-4-e4b" => Some((
            "gemma-4-E4B-it-mmproj.gguf",
            "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/main/gemma-4-E4B-it-mmproj.gguf",
        )),
        "gemma-4-12b" => Some((
            "mmproj-gemma-4-12b-it-qat-q4_0.gguf",
            "https://huggingface.co/google/gemma-4-12B-it-qat-q4_0-gguf/resolve/main/mmproj-gemma-4-12b-it-qat-q4_0.gguf",
        )),
        "gemma-3-4b" => Some((
            "mmproj-gemma-3-4b-it-f16.gguf",
            "https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/main/mmproj-model-f16.gguf",
        )),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub filename: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub size_mb: u64,
    pub is_downloaded: bool,
    pub is_downloading: bool,
    pub partial_size: u64,
    pub is_directory: bool,
    pub engine_type: EngineType,
    pub accuracy_score: f32,        // 0.0 to 1.0, higher is more accurate
    pub speed_score: f32,           // 0.0 to 1.0, higher is faster
    pub supports_translation: bool, // Whether the model supports translating to English
    pub supports_streaming: bool, // Whether the model supports native live-streaming transcription
    pub is_recommended: bool,     // Whether this is the recommended model for new users
    /// Overall recommendation rank (1 = top); `None` when unranked. Mirrors the
    /// GGUF catalog `recommended_rank` and drives the model-list ordering.
    pub recommended_rank: Option<u32>,
    pub supported_languages: Vec<String>, // Languages this model can transcribe
    pub supports_language_selection: bool, // Whether the user can explicitly pick a language
    pub is_custom: bool,                  // Whether this is a user-provided custom model
    /// True for a model fine-tuned specifically for dictation cleanup (see
    /// [`is_cleanup_specialist`]). The cleanup catalog features these and the
    /// settings UI recommends leaving the prompt layers alone for them; the
    /// assistant catalog hides them, because a cleanup fine-tune cannot chat.
    #[serde(default)]
    pub is_cleanup_specialist: bool,
    /// Absolute path to a model the user already had on disk, outside the app's
    /// own models directory — either a file they picked or one found in a linked
    /// folder. `None` for every managed model, which resolves to
    /// `<models_dir>/<filename>` as before.
    ///
    /// Set means "this file is not ours": it is never downloaded, never moved,
    /// and never deleted. [`ModelManager::delete_model`] only forgets the path.
    pub local_path: Option<String>,
    /// The linked folder this model was discovered in, when it came from a
    /// folder scan rather than an individually picked file. This is what
    /// separates "unlink the whole folder" from "remove just this entry", and it
    /// lets the UI show where an entry came from.
    pub local_folder: Option<String>,
}

/// Persisted metadata for a user-added custom GGUF language model.
///
/// Custom LLM models aren't part of the hardcoded catalog, so their definition
/// (download URL, on-disk filename, optional vision projector) is saved to
/// `<models_dir>/custom_models.json` and reloaded on startup. This is the
/// counterpart to disk-scanning custom Whisper discovery, but kept explicit so
/// we retain the source repo, download URL, and projector info that a bare
/// `.gguf` file on disk wouldn't tell us.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomModelRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub filename: String,
    pub url: String,
    pub size_mb: u64,
    pub repo_id: String,
    #[serde(default)]
    pub mmproj_filename: Option<String>,
    #[serde(default)]
    pub mmproj_url: Option<String>,
    #[serde(default)]
    pub is_vision: bool,
}

/// Persisted metadata for a model the user already had on disk.
///
/// This is the counterpart to [`CustomModelRecord`]: that one describes
/// something to *download*, this one describes something that is already there.
/// The defining property is that the file is **not ours** — it lives wherever
/// the user keeps it, and registering it copies nothing.
///
/// Only records with `folder: None` (a file the user picked individually) are
/// written to `local_models.json`. Folder-derived records are deliberately not
/// persisted: they are re-derived from `settings.model_folders` on every scan, so
/// a model added to or removed from a linked folder shows up or disappears on its
/// own instead of leaving a dead catalog entry behind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModelRecord {
    pub id: String,
    pub name: String,
    /// Absolute path to the model file.
    pub path: String,
    pub engine_type: EngineType,
    pub size_mb: u64,
    /// Companion vision projector found beside the model, enabling screen vision.
    #[serde(default)]
    pub mmproj_path: Option<String>,
    /// `general.architecture` from the GGUF header, shown to the user so an
    /// otherwise anonymous fine-tune is still identifiable.
    #[serde(default)]
    pub architecture: Option<String>,
    /// The linked folder this was discovered in; `None` for an individually
    /// picked file. See the note above on why that distinction matters.
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded: u64,
    pub total: u64,
    pub percentage: f64,
}

/// RAII guard that cleans up download state (`is_downloading` flag and cancel flag)
/// when dropped, unless explicitly disarmed. This ensures consistent cleanup on
/// every error path without requiring manual cleanup at each `?` or `return Err`.
///
/// Cleanup is **ownership-checked**, not keyed on the model id alone. A cancelled
/// download can take seconds to notice its flag (the byte stream only yields at
/// chunk boundaries, and on a slow connection that is a long time), so the user
/// can press Download again and start a replacement before the old task has
/// finished unwinding. When the old task then dropped its guard it cleared
/// `is_downloading` and removed the cancel flag that by then belonged to the
/// *replacement*: the UI offered a Download button for a model that was actively
/// downloading, pressing it started a second writer appending to the same
/// `.partial`, and Cancel had no flag left to set. Comparing the flag by pointer
/// identity is what makes a superseded guard a no-op.
struct DownloadCleanup<'a> {
    available_models: &'a Mutex<HashMap<String, ModelInfo>>,
    cancel_flags: &'a Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    model_id: String,
    /// This download's own cancel flag, used to prove the registry entry is
    /// still ours before any shared state is cleared.
    cancel_flag: Arc<AtomicBool>,
    disarmed: bool,
}

/// Remove `model_id`'s cancel flag, but only while `cancel_flag` is still the
/// registered one. Returns whether the caller still owned the slot.
///
/// Lock order here is cancel_flags → available_models. Every other site releases
/// `available_models` before touching `cancel_flags`, so no thread ever holds
/// them in the opposite order and this cannot deadlock.
fn release_download_ownership(
    cancel_flags: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    model_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> bool {
    let mut flags = cancel_flags.lock().unwrap();
    let is_ours = flags
        .get(model_id)
        .is_some_and(|registered| Arc::ptr_eq(registered, cancel_flag));
    if is_ours {
        flags.remove(model_id);
    }
    is_ours
}

impl Drop for DownloadCleanup<'_> {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        // A newer download owns this model id now; clearing its state would
        // strand it as an invisible, uncancellable background transfer.
        if !release_download_ownership(self.cancel_flags, &self.model_id, &self.cancel_flag) {
            return;
        }
        let mut models = self.available_models.lock().unwrap();
        if let Some(model) = models.get_mut(self.model_id.as_str()) {
            model.is_downloading = false;
        }
    }
}

pub struct ModelManager {
    app_handle: AppHandle,
    models_dir: PathBuf,
    available_models: Mutex<HashMap<String, ModelInfo>>,
    cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    extracting_models: Arc<Mutex<HashSet<String>>>,
    /// User-added custom GGUF LLM definitions, keyed by model id. Mirrors the
    /// custom entries in `available_models` but retains the download URL and
    /// projector metadata needed to (re)download and serve them.
    custom_models: Mutex<HashMap<String, CustomModelRecord>>,
    /// Models the user already had on disk, keyed by model id — both files they
    /// picked individually and everything found in their linked folders. Holds
    /// the absolute path that every path-resolution site reads instead of
    /// `<models_dir>/<filename>`.
    local_models: Mutex<HashMap<String, LocalModelRecord>>,
}

impl ModelManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Create models directory in app data
        let models_dir = crate::portable::app_data_dir(app_handle)
            .map_err(|e| anyhow::anyhow!("Failed to get app data dir: {}", e))?
            .join("models");

        if !models_dir.exists() {
            fs::create_dir_all(&models_dir)?;
        }

        let mut available_models = HashMap::new();

        // Whisper supported languages (99 languages from tokenizer)
        // Including zh-Hans and zh-Hant variants to match frontend language codes
        let whisper_languages: Vec<String> = vec![
            "en", "zh", "zh-Hans", "zh-Hant", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl",
            "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs",
            "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy",
            "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is",
            "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo",
            "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht",
            "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
            "ha", "ba", "jw", "su", "yue",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        // TODO this should be read from a JSON file or something..
        available_models.insert(
            "small".to_string(),
            ModelInfo {
                id: "small".to_string(),
                name: "Whisper Small".to_string(),
                description: "Fast and fairly accurate.".to_string(),
                filename: "ggml-small.bin".to_string(),
                url: Some("https://blob.handy.computer/ggml-small.bin".to_string()),
                sha256: Some(
                    "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b".to_string(),
                ),
                size_mb: 465,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::Whisper,
                accuracy_score: 0.60,
                speed_score: 0.85,
                supports_translation: true,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // Add downloadable models
        available_models.insert(
            "medium".to_string(),
            ModelInfo {
                id: "medium".to_string(),
                name: "Whisper Medium".to_string(),
                description: "Good accuracy, medium speed".to_string(),
                filename: "whisper-medium-q4_1.bin".to_string(),
                url: Some("https://blob.handy.computer/whisper-medium-q4_1.bin".to_string()),
                sha256: Some(
                    "79283fc1f9fe12ca3248543fbd54b73292164d8df5a16e095e2bceeaaabddf57".to_string(),
                ),
                size_mb: 469,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::Whisper,
                accuracy_score: 0.75,
                speed_score: 0.60,
                supports_translation: true,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        available_models.insert(
            "turbo".to_string(),
            ModelInfo {
                id: "turbo".to_string(),
                name: "Whisper Turbo".to_string(),
                description: "Balanced accuracy and speed.".to_string(),
                filename: "ggml-large-v3-turbo.bin".to_string(),
                url: Some("https://blob.handy.computer/ggml-large-v3-turbo.bin".to_string()),
                sha256: Some(
                    "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69".to_string(),
                ),
                size_mb: 1549,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::Whisper,
                accuracy_score: 0.80,
                speed_score: 0.40,
                supports_translation: false, // Turbo doesn't support translation
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        available_models.insert(
            "large".to_string(),
            ModelInfo {
                id: "large".to_string(),
                name: "Whisper Large".to_string(),
                description: "Good accuracy, but slow.".to_string(),
                filename: "ggml-large-v3-q5_0.bin".to_string(),
                url: Some("https://blob.handy.computer/ggml-large-v3-q5_0.bin".to_string()),
                sha256: Some(
                    "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1".to_string(),
                ),
                size_mb: 1031,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::Whisper,
                accuracy_score: 0.85,
                speed_score: 0.30,
                supports_translation: true,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: whisper_languages.clone(),
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        available_models.insert(
            "breeze-asr".to_string(),
            ModelInfo {
                id: "breeze-asr".to_string(),
                name: "Breeze ASR".to_string(),
                description: "Optimized for Taiwanese Mandarin. Code-switching support."
                    .to_string(),
                filename: "breeze-asr-q5_k.bin".to_string(),
                url: Some("https://blob.handy.computer/breeze-asr-q5_k.bin".to_string()),
                sha256: Some(
                    "8efbf0ce8a3f50fe332b7617da787fb81354b358c288b008d3bdef8359df64c6".to_string(),
                ),
                size_mb: 1030,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::Whisper,
                accuracy_score: 0.85,
                speed_score: 0.35,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: whisper_languages,
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // Add NVIDIA Parakeet models (directory-based)
        available_models.insert(
            "parakeet-tdt-0.6b-v2".to_string(),
            ModelInfo {
                id: "parakeet-tdt-0.6b-v2".to_string(),
                name: "Parakeet V2".to_string(),
                description: "English only. The best model for English speakers.".to_string(),
                filename: "parakeet-tdt-0.6b-v2-int8".to_string(), // Directory name
                url: Some("https://blob.handy.computer/parakeet-v2-int8.tar.gz".to_string()),
                sha256: Some(
                    "ac9b9429984dd565b25097337a887bb7f0f8ac393573661c651f0e7d31563991".to_string(),
                ),
                size_mb: 451,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Parakeet,
                accuracy_score: 0.85,
                speed_score: 0.85,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // Parakeet V3 supported languages (25 EU languages + Russian/Ukrainian):
        // bg, hr, cs, da, nl, en, et, fi, fr, de, el, hu, it, lv, lt, mt, pl, pt, ro, sk, sl, es, sv, ru, uk
        let parakeet_v3_languages: Vec<String> = vec![
            "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv",
            "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        available_models.insert(
            "parakeet-tdt-0.6b-v3".to_string(),
            ModelInfo {
                id: "parakeet-tdt-0.6b-v3".to_string(),
                name: "Parakeet V3".to_string(),
                description: "Fast and accurate. Supports 25 European languages.".to_string(),
                filename: "parakeet-tdt-0.6b-v3-int8".to_string(), // Directory name
                url: Some("https://blob.handy.computer/parakeet-v3-int8.tar.gz".to_string()),
                sha256: Some(
                    "43d37191602727524a7d8c6da0eef11c4ba24320f5b4730f1a2497befc2efa77".to_string(),
                ),
                size_mb: 456,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Parakeet,
                accuracy_score: 0.80,
                speed_score: 0.85,
                supports_translation: false,
                supports_streaming: false,
                // Superseded as the recommended default by the native
                // transcribe.cpp streaming set (parakeet-unified-en-0.6b-gguf,
                // #1). Kept listed and fully usable via transcribe-rs — legacy
                // models are never removed or downgraded (N2) — just no longer
                // the default suggestion for new users (PLAN.md Session 6).
                is_recommended: false,
                recommended_rank: None,
                supported_languages: parakeet_v3_languages,
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        available_models.insert(
            "moonshine-base".to_string(),
            ModelInfo {
                id: "moonshine-base".to_string(),
                name: "Moonshine Base".to_string(),
                description: "Very fast, English only. Handles accents well.".to_string(),
                filename: "moonshine-base".to_string(),
                url: Some("https://blob.handy.computer/moonshine-base.tar.gz".to_string()),
                sha256: Some(
                    "04bf6ab012cfceebd4ac7cf88c1b31d027bbdd3cd704649b692e2e935236b7e8".to_string(),
                ),
                size_mb: 55,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Moonshine,
                accuracy_score: 0.70,
                speed_score: 0.90,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        available_models.insert(
            "moonshine-tiny-streaming-en".to_string(),
            ModelInfo {
                id: "moonshine-tiny-streaming-en".to_string(),
                name: "Moonshine V2 Tiny".to_string(),
                description: "Ultra-fast, English only".to_string(),
                filename: "moonshine-tiny-streaming-en".to_string(),
                url: Some(
                    "https://blob.handy.computer/moonshine-tiny-streaming-en.tar.gz".to_string(),
                ),
                sha256: Some(
                    "465addcfca9e86117415677dfdc98b21edc53537210333a3ecdb58509a80abaf".to_string(),
                ),
                size_mb: 31,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::MoonshineStreaming,
                accuracy_score: 0.55,
                speed_score: 0.95,
                supports_translation: false,
                supports_streaming: true,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        available_models.insert(
            "moonshine-small-streaming-en".to_string(),
            ModelInfo {
                id: "moonshine-small-streaming-en".to_string(),
                name: "Moonshine V2 Small".to_string(),
                description: "Fast, English only. Good balance of speed and accuracy.".to_string(),
                filename: "moonshine-small-streaming-en".to_string(),
                url: Some(
                    "https://blob.handy.computer/moonshine-small-streaming-en.tar.gz".to_string(),
                ),
                sha256: Some(
                    "dbb3e1c1832bd88a4ac712f7449a136cc2c9a18c5fe33a12ed1b7cb1cfe9cdd5".to_string(),
                ),
                size_mb: 99,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::MoonshineStreaming,
                accuracy_score: 0.65,
                speed_score: 0.90,
                supports_translation: false,
                supports_streaming: true,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        available_models.insert(
            "moonshine-medium-streaming-en".to_string(),
            ModelInfo {
                id: "moonshine-medium-streaming-en".to_string(),
                name: "Moonshine V2 Medium".to_string(),
                description: "English only. High quality.".to_string(),
                filename: "moonshine-medium-streaming-en".to_string(),
                url: Some(
                    "https://blob.handy.computer/moonshine-medium-streaming-en.tar.gz".to_string(),
                ),
                sha256: Some(
                    "07a66f3bff1c77e75a2f637e5a263928a08baae3c29c4c053fc968a9a9373d13".to_string(),
                ),
                size_mb: 192,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::MoonshineStreaming,
                accuracy_score: 0.75,
                speed_score: 0.80,
                supports_translation: false,
                supports_streaming: true,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // SenseVoice supported languages
        let sense_voice_languages: Vec<String> =
            vec!["zh", "zh-Hans", "zh-Hant", "en", "yue", "ja", "ko"]
                .into_iter()
                .map(String::from)
                .collect();

        available_models.insert(
            "sense-voice-int8".to_string(),
            ModelInfo {
                id: "sense-voice-int8".to_string(),
                name: "SenseVoice".to_string(),
                description: "Very fast. Chinese, English, Japanese, Korean, Cantonese."
                    .to_string(),
                filename: "sense-voice-int8".to_string(),
                url: Some("https://blob.handy.computer/sense-voice-int8.tar.gz".to_string()),
                sha256: Some(
                    "171d611fe5d353a50bbb741b6f3ef42559b1565685684e9aa888ef563ba3e8a4".to_string(),
                ),
                size_mb: 152,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::SenseVoice,
                accuracy_score: 0.65,
                speed_score: 0.95,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: sense_voice_languages,
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // GigaAM v3 supported languages
        let gigaam_languages: Vec<String> = vec!["ru"].into_iter().map(String::from).collect();

        available_models.insert(
            "gigaam-v3-e2e-ctc".to_string(),
            ModelInfo {
                id: "gigaam-v3-e2e-ctc".to_string(),
                name: "GigaAM v3".to_string(),
                description: "Russian speech recognition. Fast and accurate.".to_string(),
                filename: "giga-am-v3-int8".to_string(),
                url: Some("https://blob.handy.computer/giga-am-v3-int8.tar.gz".to_string()),
                sha256: Some(
                    "d872462268430db140b69b72e0fc4b787b194c1dbe51b58de39444d55b6da45b".to_string(),
                ),
                size_mb: 151,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::GigaAM,
                accuracy_score: 0.85,
                speed_score: 0.75,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: gigaam_languages,
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // Canary 180m Flash supported languages (4 languages)
        let canary_flash_languages: Vec<String> = vec!["en", "de", "es", "fr"]
            .into_iter()
            .map(String::from)
            .collect();

        available_models.insert(
            "canary-180m-flash".to_string(),
            ModelInfo {
                id: "canary-180m-flash".to_string(),
                name: "Canary 180M Flash".to_string(),
                description: "Very fast. English, German, Spanish, French. Supports translation."
                    .to_string(),
                filename: "canary-180m-flash".to_string(),
                url: Some("https://blob.handy.computer/canary-180m-flash.tar.gz".to_string()),
                sha256: Some(
                    "6d9cfca6118b296e196eaedc1c8fa9788305a7b0f1feafdb6dc91932ab6e53f7".to_string(),
                ),
                size_mb: 146,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Canary,
                accuracy_score: 0.75,
                speed_score: 0.85,
                supports_translation: true,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: canary_flash_languages,
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // Canary 1B v2 supported languages (25 EU languages)
        let canary_1b_languages: Vec<String> = vec![
            "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv",
            "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        available_models.insert(
            "canary-1b-v2".to_string(),
            ModelInfo {
                id: "canary-1b-v2".to_string(),
                name: "Canary 1B v2".to_string(),
                description: "Accurate multilingual. 25 European languages. Supports translation."
                    .to_string(),
                filename: "canary-1b-v2".to_string(),
                url: Some("https://blob.handy.computer/canary-1b-v2.tar.gz".to_string()),
                sha256: Some(
                    "02305b2a25f9cf3e7deaffa7f94df00efa44f442cd55c101c2cb9c000f904666".to_string(),
                ),
                size_mb: 691,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Canary,
                accuracy_score: 0.85,
                speed_score: 0.70,
                supports_translation: true,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: canary_1b_languages,
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        let cohere_languages: Vec<String> = vec![
            "en", "fr", "de", "it", "es", "pt", "el", "nl", "pl", "zh", "zh-Hans", "zh-Hant", "ja",
            "ko", "vi", "ar",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        available_models.insert(
            "cohere-int8".to_string(),
            ModelInfo {
                id: "cohere-int8".to_string(),
                name: "Cohere".to_string(),
                description: "A large, slower, but very accurate multilingual model.".to_string(),
                filename: "cohere-int8".to_string(),
                url: Some("https://blob.handy.computer/cohere-int8.tar.gz".to_string()),
                sha256: Some(
                    "ea2257d52434f3644574f187dcdcf666e302cd11b92866116ab8e14cd9c887f0".to_string(),
                ),
                size_mb: 1708,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Cohere,
                accuracy_score: 0.90,
                speed_score: 0.60,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: cohere_languages,
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // ---------------------------------------------------------------
        // transcribe.cpp (ggml/GGUF) engine models — the new recommended set,
        // loaded from the bundled catalog (`src/catalog/catalog.json`, embedded
        // via include_str!). Bundling the whole catalog plus a loader (rather
        // than hardcoding each model here) makes pulling a future Handy model
        // release a one-file copy — see `crate::catalog` and PLAN.md §4 /
        // Session 3 & 7. Single GGUF files reuse the existing resume-capable
        // download pipeline (no `.tar.gz`, `is_directory = false`).
        // ---------------------------------------------------------------
        Self::insert_catalog_models(&mut available_models);

        // ---------------------------------------------------------------
        // Local Large Language Models (GGUF), served by the bundled
        // llama.cpp engine. Single-file downloads reusing the Whisper
        // pipeline; vision models additionally fetch a companion mmproj
        // projector (see `mmproj_for`). Used by the "Built-in" provider.
        // ---------------------------------------------------------------

        // Broad multilingual tag so LLM entries stay visible under the
        // language filter; these models are all multilingual.
        let llm_languages: Vec<String> = vec![
            "en", "zh", "zh-Hans", "zh-Hant", "de", "es", "fr", "it", "pt", "ru", "ja", "ko", "ar",
            "hi", "vi", "id", "tr", "pl", "nl",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        // SpeakoFlow Mini — our own dictation-cleanup fine-tune.
        //
        // Listed first because it is the recommended AI-cleanup engine, and it
        // is deliberately NOT recommended for the assistant: at 0.8B, trained on
        // a single English text-to-text transform, it cannot hold a
        // conversation. `is_recommended` stays false for exactly that reason —
        // that flag drives the shared model-catalog ordering, and the cleanup
        // catalog features Mini through `is_cleanup_specialist` instead.
        //
        // English only: the fine-tune saw no other language, so cleanup on a
        // non-English dictation should stay on a general multilingual model.
        available_models.insert(
            SPEAKOFLOW_MINI_MODEL_ID.to_string(),
            ModelInfo {
                id: SPEAKOFLOW_MINI_MODEL_ID.to_string(),
                name: "SpeakoFlow Mini".to_string(),
                description:
                    "Our own dictation-cleanup model. Tiny, fast, and trained for one job: turning spoken English into clean written English. English only."
                        .to_string(),
                filename: SPEAKOFLOW_MINI_FILENAME.to_string(),
                url: Some(format!(
                    "https://huggingface.co/{}/resolve/main/{}",
                    SPEAKOFLOW_MINI_REPO_ID, SPEAKOFLOW_MINI_FILENAME
                )),
                sha256: Some(SPEAKOFLOW_MINI_SHA256.to_string()),
                size_mb: SPEAKOFLOW_MINI_SIZE_MB,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::LlamaCpp,
                // Scored for its actual job. It beats a general 4B model on
                // dictation cleanup and loses badly at everything else, so these
                // numbers describe cleanup quality, not chat ability.
                accuracy_score: 0.72,
                speed_score: 0.99,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: true,
            },
        );

        // Gemma 3 1B - text only, tiny, and suitable for low-memory systems.
        available_models.insert(
            "gemma-3-1b".to_string(),
            ModelInfo {
id: "gemma-3-1b".to_string(),
            name: "Gemma 3 1B".to_string(),
            description: "The lightest option for simple chat and writing help. Text only."
                .to_string(),
            filename: "gemma-3-1b-it-Q4_K_M.gguf".to_string(),
            url: Some(
                "https://huggingface.co/ggml-org/gemma-3-1b-it-GGUF/resolve/main/gemma-3-1b-it-Q4_K_M.gguf"
                    .to_string(),
            ),
            sha256: None, // GGUF hashes not pinned; verification skipped
            size_mb: 806,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.45,
            speed_score: 0.97,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Qwen3.5 2B — newest small multimodal model (text + vision).
        available_models.insert(
            "qwen3.5-2b".to_string(),
            ModelInfo {
id: "qwen3.5-2b".to_string(),
            name: "Qwen3.5 2B (Vision)".to_string(),
            description: "Small, fast, and sees images. Good on most laptops.".to_string(),
            filename: "Qwen_Qwen3.5-2B-Q4_K_M.gguf".to_string(),
            url: Some(
                "https://huggingface.co/bartowski/Qwen_Qwen3.5-2B-GGUF/resolve/main/Qwen_Qwen3.5-2B-Q4_K_M.gguf"
                    .to_string(),
            ),
            sha256: None,
            size_mb: 2350,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.58,
            speed_score: 0.82,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Qwen3.5 4B - the everyday multimodal recommendation.
        available_models.insert(
            "qwen3.5-4b".to_string(),
            ModelInfo {
id: "qwen3.5-4b".to_string(),
            name: "Qwen3.5 4B (Vision)".to_string(),
            description: "A quick everyday assistant with screen vision.".to_string(),
            filename: "Qwen_Qwen3.5-4B-Q4_K_M.gguf".to_string(),
            url: Some(
                "https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF/resolve/main/Qwen_Qwen3.5-4B-Q4_K_M.gguf"
                    .to_string(),
            ),
            sha256: None,
            // Main Q4_K_M weights plus the automatically downloaded F16 projector.
            size_mb: 3515,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.74,
            speed_score: 0.62,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Qwen3.5 9B - stronger answers for higher-memory desktops.
        available_models.insert(
            "qwen3.5-9b".to_string(),
            ModelInfo {
id: "qwen3.5-9b".to_string(),
            name: "Qwen3.5 9B (Vision)".to_string(),
            description: "Stronger answers and screen vision for powerful computers."
                .to_string(),
            filename: "Qwen3.5-9B-Q4_K_M.gguf".to_string(),
            url: Some(
                "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf"
                    .to_string(),
            ),
            sha256: None,
            // Verified Q4_K_M weights plus the automatically downloaded F16 projector.
            size_mb: 6293,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.84,
            speed_score: 0.44,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Qwen3.5 27B - highest-quality curated option for workstations.
        available_models.insert(
            "qwen3.5-27b".to_string(),
            ModelInfo {
id: "qwen3.5-27b".to_string(),
            name: "Qwen3.5 27B (Vision)".to_string(),
            description: "The best local quality for high-memory desktops and workstations."
                .to_string(),
            filename: "Qwen3.5-27B-Q4_K_M.gguf".to_string(),
            url: Some(
                "https://huggingface.co/unsloth/Qwen3.5-27B-GGUF/resolve/main/Qwen3.5-27B-Q4_K_M.gguf"
                    .to_string(),
            ),
            sha256: None,
            // Verified Q4_K_M weights plus the automatically downloaded F16 projector.
            size_mb: 16850,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.93,
            speed_score: 0.20,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Gemma 4 E2B — current on-device model, optimized for responsiveness.
        available_models.insert(
            "gemma-4-e2b".to_string(),
            ModelInfo {
id: "gemma-4-e2b".to_string(),
            name: "Gemma 4 E2B (Vision)".to_string(),
            description: "The quickest current Gemma for everyday conversation. Less capable on complex requests."
                .to_string(),
            filename: "gemma-4-E2B_q4_0-it.gguf".to_string(),
            url: Some(
                "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/main/gemma-4-E2B_q4_0-it.gguf"
                    .to_string(),
            ),
            sha256: None,
            // Official QAT Q4_0 weights plus the automatically downloaded projector.
            size_mb: 4135,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.66,
            speed_score: 0.86,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: true,
            recommended_rank: Some(2),
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Gemma 4 E4B — default conversational balance, with thinking opt-in.
        available_models.insert(
            "gemma-4-e4b".to_string(),
            ModelInfo {
id: "gemma-4-e4b".to_string(),
            name: "Gemma 4 E4B (Vision)".to_string(),
            description: "Recommended for conversation: a stronger quality-and-speed balance without default thinking."
                .to_string(),
            filename: "gemma-4-E4B_q4_0-it.gguf".to_string(),
            url: Some(
                "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/main/gemma-4-E4B_q4_0-it.gguf"
                    .to_string(),
            ),
            sha256: None,
            // Official QAT Q4_0 weights plus the automatically downloaded projector.
            size_mb: 5862,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.80,
            speed_score: 0.68,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: true,
            recommended_rank: Some(1),
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Gemma 4 12B — stronger answers, with a clear latency tradeoff.
        available_models.insert(
            "gemma-4-12b".to_string(),
            ModelInfo {
id: "gemma-4-12b".to_string(),
            name: "Gemma 4 12B (Vision)".to_string(),
            description: "More capable for nuanced questions, but noticeably slower and best with a strong GPU."
                .to_string(),
            filename: "gemma-4-12b-it-qat-q4_0.gguf".to_string(),
            url: Some(
                "https://huggingface.co/google/gemma-4-12B-it-qat-q4_0-gguf/resolve/main/gemma-4-12b-it-qat-q4_0.gguf"
                    .to_string(),
            ),
            sha256: None,
            // Official QAT Q4_0 weights plus the automatically downloaded projector.
            size_mb: 6821,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.91,
            speed_score: 0.38,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: true,
            recommended_rank: Some(3),
            supported_languages: llm_languages.clone(),
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // Gemma 3 4B — Google multimodal (text + vision), clean output.
        available_models.insert(
            "gemma-3-4b".to_string(),
            ModelInfo {
id: "gemma-3-4b".to_string(),
            name: "Gemma 3 4B (Vision)".to_string(),
            description: "Google's multimodal model. Clean, reliable answers and fast responses."
                .to_string(),
            filename: "gemma-3-4b-it-Q4_K_M.gguf".to_string(),
            url: Some(
                "https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf"
                    .to_string(),
            ),
            sha256: None,
            size_mb: 3350,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.70,
            speed_score: 0.60,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: llm_languages,
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None, is_cleanup_specialist: false },
        );

        // ---------------------------------------------------------------
        // Text-to-Speech. Kokoro runs locally inside the assistant panel
        // webview (kokoro-js / WebGPU) and manages its own weights, so it is
        // surfaced here as a built-in, always-available model rather than a
        // pipeline download. `update_download_status` keeps it marked as
        // downloaded.
        // ---------------------------------------------------------------
        available_models.insert(
            "kokoro-82m".to_string(),
            ModelInfo {
                id: "kokoro-82m".to_string(),
                name: "Kokoro".to_string(),
                description: "Built-in local voice for the assistant. No download required."
                    .to_string(),
                filename: "kokoro-82m".to_string(),
                url: None,
                sha256: None,
                size_mb: 0,
                is_downloaded: true, // managed by the webview; always available
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::Kokoro,
                accuracy_score: 0.0,
                speed_score: 0.0,
                supports_translation: false,
                supports_streaming: false,
                is_recommended: true,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: false,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // Auto-discover custom Whisper models (.bin files) in the models directory
        if let Err(e) = Self::discover_custom_whisper_models(&models_dir, &mut available_models) {
            warn!("Failed to discover custom models: {}", e);
        }

        // Load user-added custom GGUF LLM models from custom_models.json and
        // insert them into the catalog alongside the built-in models.
        let custom_models = Self::load_custom_llm_models(&models_dir, &mut available_models)
            .unwrap_or_else(|e| {
                warn!("Failed to load custom LLM models: {}", e);
                HashMap::new()
            });

        let manager = Self {
            app_handle: app_handle.clone(),
            models_dir,
            available_models: Mutex::new(available_models),
            cancel_flags: Arc::new(Mutex::new(HashMap::new())),
            extracting_models: Arc::new(Mutex::new(HashSet::new())),
            custom_models: Mutex::new(custom_models),
            local_models: Mutex::new(HashMap::new()),
        };

        // Migrate any bundled models to user directory
        manager.migrate_bundled_models()?;

        // Migrate GigaAM from single-file to directory format
        manager.migrate_gigaam_to_directory()?;

        // Register everything the user already has on disk: individually picked
        // files plus every model in their linked folders. Done before the status
        // and header passes below so local models go through exactly the same
        // availability check and capability probe as downloaded ones — and so
        // `auto_select_model_if_needed` can pick one, which matters on a fresh
        // install whose only model is a linked local file.
        manager.rebuild_local_entries();

        // Check which models are already downloaded
        manager.update_download_status()?;

        // Session 3: apply GGUF-header capability hints to any already-downloaded
        // transcribe.cpp models so the UI reflects their real capabilities before
        // the first load. Never guesses (Some-only); safe no-op otherwise.
        manager.reconcile_downloaded_cpp_headers();

        // Auto-select a model if none is currently selected
        manager.auto_select_model_if_needed()?;

        Ok(manager)
    }

    pub fn get_available_models(&self) -> Vec<ModelInfo> {
        let models = self.available_models.lock().unwrap();
        models.values().cloned().map(stamped).collect()
    }

    pub fn get_model_info(&self, model_id: &str) -> Option<ModelInfo> {
        let models = self.available_models.lock().unwrap();
        models.get(model_id).cloned().map(stamped)
    }

    /// Insert the transcribe.cpp GGUF models from the bundled catalog
    /// (`crate::catalog`). Only `recommended` entries are surfaced today — the
    /// five ranked models from PLAN.md §4 (flipping to show the whole catalog
    /// is a one-line change: drop the `.filter(|m| m.recommended)`). Each maps
    /// to a single-file `.gguf` `ModelInfo` on the `TranscribeCpp` engine that
    /// reuses the existing resume-capable download pipeline.
    ///
    /// Internal ids are `"<slug>-gguf"` (matching the Hugging Face repo suffix),
    /// which keeps them distinct from the legacy transcribe-rs ids that share a
    /// slug (e.g. `canary-180m-flash`) so neither shadows the other (N2).
    fn insert_catalog_models(available_models: &mut HashMap<String, ModelInfo>) {
        for model in crate::catalog::catalog()
            .models
            .iter()
            .filter(|m| m.recommended)
        {
            let Some(file) = model.default_file() else {
                warn!(
                    "Catalog model {} has no downloadable file; skipping",
                    model.slug
                );
                continue;
            };
            let id = format!("{}-gguf", model.slug);
            // Never shadow an existing entry (legacy transcribe-rs or custom).
            if available_models.contains_key(&id) {
                warn!("Catalog model id '{}' already present; skipping", id);
                continue;
            }
            let url = model.download_url(file);
            let size_mb = file.size_bytes / (1024 * 1024);
            // Catalog scores are 0–100; the UI meters use 0.0–1.0.
            let accuracy_score = (model.accuracy_score as f32 / 100.0).clamp(0.0, 1.0);
            let speed_score = (model.speed_score as f32 / 100.0).clamp(0.0, 1.0);
            available_models.insert(
                id.clone(),
                ModelInfo {
                    id,
                    name: model.name.clone(),
                    description: model.description.clone(),
                    filename: file.filename.clone(),
                    url: Some(url),
                    sha256: None, // catalog carries no per-file hash; verification skipped
                    size_mb,
                    is_downloaded: false,
                    is_downloading: false,
                    partial_size: 0,
                    is_directory: false,
                    engine_type: EngineType::TranscribeCpp,
                    accuracy_score,
                    speed_score,
                    supports_translation: model.capabilities.translate,
                    supports_streaming: model.capabilities.streaming,
                    is_recommended: model.recommended,
                    recommended_rank: model.recommended_rank,
                    supported_languages: model.languages.clone(),
                    // A language can be explicitly chosen only on multilingual models.
                    supports_language_selection: model.language_count > 1,
                    is_custom: false,
                    local_path: None,
                    local_folder: None,
                    is_cleanup_specialist: false,
                },
            );
        }
    }

    /// Reconcile a model's registry entry against the *loaded* model's real
    /// capabilities. transcribe.cpp reads these from the GGUF at load time
    /// (ground truth), so the transcription manager calls this post-load with
    /// `session.model().capabilities()`. Unlike the pre-load header probe (which
    /// leaves parakeet streaming unknown rather than guess), the loaded value is
    /// authoritative. No-op when nothing changed; the load path's existing
    /// `model-state-changed` completion event refreshes the UI list.
    pub fn set_runtime_capabilities(
        &self,
        model_id: &str,
        supports_streaming: bool,
        supports_translation: bool,
        languages: &[String],
    ) {
        let mut models = self.available_models.lock().unwrap();
        let Some(model) = models.get_mut(model_id) else {
            return;
        };
        let mut changed = false;
        if model.supports_streaming != supports_streaming {
            model.supports_streaming = supports_streaming;
            changed = true;
        }
        if model.supports_translation != supports_translation {
            model.supports_translation = supports_translation;
            changed = true;
        }
        // An empty language list from the engine means "unknown", not "none".
        if !languages.is_empty() && model.supported_languages != languages {
            model.supported_languages = languages.to_vec();
            changed = true;
        }
        if changed {
            info!(
                "Reconciled runtime capabilities for model {} (streaming={}, translate={}, langs={})",
                model_id,
                supports_streaming,
                supports_translation,
                model.supported_languages.len()
            );
        }
    }

    /// Apply capability hints read from a downloaded GGUF's header to its
    /// registry entry (TranscribeCpp models only). Uses the dependency-free
    /// [`crate::managers::model_capabilities::GgufHeaderProber`]; only fields the
    /// header explicitly declares are applied — parakeet streaming, which the
    /// header does not carry, is left at the catalog value and settled by a real
    /// load's [`Self::set_runtime_capabilities`] (never guesses). Safe no-op if
    /// the file is missing, not a GGUF, or the model isn't transcribe.cpp.
    fn apply_gguf_header_hints(&self, model_id: &str) {
        use crate::managers::model_capabilities::{CapabilityProber, GgufHeaderProber};

        let path = {
            let models = self.available_models.lock().unwrap();
            match models.get(model_id) {
                Some(m)
                    if matches!(m.engine_type, EngineType::TranscribeCpp) && m.is_downloaded =>
                {
                    self.resolve_model_file(m)
                }
                _ => return,
            }
        };
        if !path.exists() {
            return;
        }
        let probe = GgufHeaderProber.probe_file(&path);
        let mut models = self.available_models.lock().unwrap();
        if let Some(model) = models.get_mut(model_id) {
            if let Some(streaming) = probe.supports_streaming {
                model.supports_streaming = streaming;
            }
            if let Some(translate) = probe.supports_translation {
                model.supports_translation = translate;
            }
            if let Some(langs) = probe.languages {
                if !langs.is_empty() {
                    model.supported_languages = langs;
                }
            }
        }
    }

    /// Apply GGUF-header capability hints to every already-downloaded
    /// transcribe.cpp model. Called once at startup so the UI shows their real
    /// capabilities before the first load.
    fn reconcile_downloaded_cpp_headers(&self) {
        let ids: Vec<String> = {
            let models = self.available_models.lock().unwrap();
            models
                .values()
                .filter(|m| matches!(m.engine_type, EngineType::TranscribeCpp) && m.is_downloaded)
                .map(|m| m.id.clone())
                .collect()
        };
        for id in ids {
            self.apply_gguf_header_hints(&id);
        }
    }

    fn migrate_bundled_models(&self) -> Result<()> {
        // Check for bundled models and copy them to user directory
        let bundled_models = ["ggml-small.bin"]; // Add other bundled models here if any

        for filename in &bundled_models {
            let bundled_path = self.app_handle.path().resolve(
                &format!("resources/models/{}", filename),
                tauri::path::BaseDirectory::Resource,
            );

            if let Ok(bundled_path) = bundled_path {
                if bundled_path.exists() {
                    let user_path = self.models_dir.join(filename);

                    // Only copy if user doesn't already have the model
                    if !user_path.exists() {
                        info!("Migrating bundled model {} to user directory", filename);
                        fs::copy(&bundled_path, &user_path)?;
                        info!("Successfully migrated {}", filename);
                    }
                }
            }
        }

        Ok(())
    }

    /// Migrate GigaAM from the old single-file format (giga-am-v3.int8.onnx)
    /// to the new directory format (giga-am-v3-int8/model.int8.onnx + vocab.txt).
    /// This was required by the transcribe-rs 0.3.x upgrade.
    fn migrate_gigaam_to_directory(&self) -> Result<()> {
        let old_file = self.models_dir.join("giga-am-v3.int8.onnx");
        let new_dir = self.models_dir.join("giga-am-v3-int8");

        if !old_file.exists() || new_dir.exists() {
            return Ok(());
        }

        info!("Migrating GigaAM from single-file to directory format");

        let vocab_path = self
            .app_handle
            .path()
            .resolve(
                "resources/models/gigaam_vocab.txt",
                tauri::path::BaseDirectory::Resource,
            )
            .map_err(|e| anyhow::anyhow!("Failed to resolve GigaAM vocab path: {}", e))?;

        info!(
            "Resolved vocab path: {:?} (exists: {})",
            vocab_path,
            vocab_path.exists()
        );
        info!("Old file: {:?} (exists: {})", old_file, old_file.exists());
        info!("New dir: {:?} (exists: {})", new_dir, new_dir.exists());

        fs::create_dir_all(&new_dir)?;
        fs::rename(&old_file, new_dir.join("model.int8.onnx"))?;
        fs::copy(&vocab_path, new_dir.join("vocab.txt"))?;

        // Clean up old partial file if it exists
        let old_partial = self.models_dir.join("giga-am-v3.int8.onnx.partial");
        if old_partial.exists() {
            let _ = fs::remove_file(&old_partial);
        }

        info!("GigaAM migration complete");
        Ok(())
    }

    fn update_download_status(&self) -> Result<()> {
        // Custom-model projector metadata lives outside `available_models`.
        // Snapshot it before taking the model lock so vision completeness can
        // be checked without nested locks.
        let custom_projectors: HashMap<String, String> = {
            let customs = self.custom_models.lock().unwrap();
            customs
                .iter()
                .filter_map(|(id, record)| {
                    record
                        .mmproj_filename
                        .as_ref()
                        .map(|filename| (id.clone(), filename.clone()))
                })
                .collect()
        };
        // A local model's projector is an absolute path rather than a managed
        // filename, so it is snapshotted separately and checked as-is.
        let local_projectors: HashMap<String, String> = {
            let locals = self.local_models.lock().unwrap();
            locals
                .iter()
                .filter_map(|(id, record)| {
                    record
                        .mmproj_path
                        .as_ref()
                        .map(|path| (id.clone(), path.clone()))
                })
                .collect()
        };
        let projector_ready = |model_id: &str| {
            if let Some(path) = local_projectors.get(model_id) {
                return Path::new(path).exists();
            }
            let filename = mmproj_for(model_id)
                .map(|(filename, _)| filename.to_string())
                .or_else(|| custom_projectors.get(model_id).cloned());
            filename
                .map(|filename| self.models_dir.join(filename).exists())
                .unwrap_or(true)
        };

        let mut models = self.available_models.lock().unwrap();

        for model in models.values_mut() {
            // Built-in TTS (Kokoro) is managed by the assistant webview and is
            // always considered available; there is no file on disk to check.
            if model.engine_type == EngineType::Kokoro {
                model.is_downloaded = true;
                model.is_downloading = false;
                model.partial_size = 0;
                continue;
            }
            // A model the user already had on disk: availability is simply
            // whether their file is still reachable. There is no download to be
            // in progress and no `.partial` to account for, and an unplugged
            // external drive correctly reads as unavailable rather than as a
            // model that fails at load time.
            if let Some(local_path) = model.local_path.clone() {
                model.is_downloaded =
                    Path::new(&local_path).is_file() && projector_ready(&model.id);
                model.is_downloading = false;
                model.partial_size = 0;
                continue;
            }
            if model.is_directory {
                // For directory-based models, check if the directory exists
                let model_path = self.models_dir.join(&model.filename);
                let partial_path = self.models_dir.join(format!("{}.partial", &model.filename));
                let extracting_path = self
                    .models_dir
                    .join(format!("{}.extracting", &model.filename));

                // Clean up any leftover .extracting directories from interrupted extractions
                // But only if this model is NOT currently being extracted
                let is_currently_extracting = {
                    let extracting = self.extracting_models.lock().unwrap();
                    extracting.contains(&model.id)
                };
                if extracting_path.exists() && !is_currently_extracting {
                    warn!("Cleaning up interrupted extraction for model: {}", model.id);
                    let _ = fs::remove_dir_all(&extracting_path);
                }

                model.is_downloaded =
                    model_path.exists() && model_path.is_dir() && projector_ready(&model.id);
                model.is_downloading = false;

                // Get partial file size if it exists (for the .tar.gz being downloaded)
                if partial_path.exists() {
                    model.partial_size = real_partial_size(&partial_path);
                } else {
                    model.partial_size = 0;
                }
            } else {
                // For file-based models (existing logic)
                let model_path = self.models_dir.join(&model.filename);
                let partial_path = self.models_dir.join(format!("{}.partial", &model.filename));

                model.is_downloaded = model_path.exists() && projector_ready(&model.id);
                model.is_downloading = false;

                // Get partial file size if it exists
                if partial_path.exists() {
                    model.partial_size = real_partial_size(&partial_path);
                } else {
                    model.partial_size = 0;
                }
            }
        }

        Ok(())
    }

    fn auto_select_model_if_needed(&self) -> Result<()> {
        let mut settings = get_settings(&self.app_handle);

        // Clear a stale selection: set but no longer present in the catalog
        // (e.g. a deleted custom model whose file is gone). This lets the
        // picker below choose a fresh default instead of leaving a dangling id.
        if !settings.selected_model.is_empty() {
            let exists = {
                let models = self.available_models.lock().unwrap();
                models.contains_key(&settings.selected_model)
            };

            if !exists {
                info!(
                    "Selected model '{}' not found in available models, clearing selection",
                    settings.selected_model
                );
                settings.selected_model = String::new();
                write_settings(&self.app_handle, settings.clone());
            }
        }

        // Whether the current selection can actually transcribe right now — a
        // downloaded transcription model. This is false for an empty selection,
        // for an LLM/TTS id, and (importantly) for the recommended default GGUF
        // model before it has been downloaded.
        let selection_usable = {
            let models = self.available_models.lock().unwrap();
            models
                .get(&settings.selected_model)
                .map(|m| m.is_downloaded && m.engine_type.is_transcription())
                .unwrap_or(false)
        };

        // If the current selection can't transcribe, fall back to the best
        // *downloaded* transcription model. This is what keeps the existing
        // default working when the new recommended streaming model isn't
        // downloaded yet (PLAN.md Session 6 / N1): an upgrading user who already
        // has a legacy model keeps using it, while a fresh user is simply left
        // on the recommended id for onboarding to fetch (nothing downloaded →
        // `None`, so we leave the selection untouched). A valid, downloaded
        // selection is never overridden, so a user's explicit choice is kept.
        if !selection_usable {
            let best = {
                let models = self.available_models.lock().unwrap();
                Self::pick_default_transcription_model(&models)
            };

            if let Some(model_id) = best {
                info!(
                    "Auto-selecting transcription model: {} (previous selection '{}' unavailable)",
                    model_id, settings.selected_model
                );
                settings.selected_model = model_id;
                write_settings(&self.app_handle, settings);
            }
        }

        Ok(())
    }

    /// Pick the best *downloaded* transcription model to activate as the
    /// default, or `None` when none is downloaded yet (a fresh install, before
    /// onboarding). Preference order: recommended rank (1 = top), then the
    /// recommended flag, then higher accuracy, with the id as a stable
    /// tie-breaker (so the choice is deterministic despite the backing
    /// `HashMap`'s arbitrary iteration order). This makes the recommended
    /// streaming model the active default once it's on disk, but any other
    /// downloaded transcription model (legacy ONNX/whisper included) is a valid
    /// fallback — never LLM or TTS models.
    fn pick_default_transcription_model(models: &HashMap<String, ModelInfo>) -> Option<String> {
        models
            .values()
            .filter(|m| m.is_downloaded && m.engine_type.is_transcription())
            .min_by(|a, b| {
                let rank = |m: &ModelInfo| m.recommended_rank.unwrap_or(u32::MAX);
                rank(a)
                    .cmp(&rank(b))
                    // recommended before not-recommended (true sorts first)
                    .then_with(|| b.is_recommended.cmp(&a.is_recommended))
                    // higher accuracy first
                    .then_with(|| {
                        b.accuracy_score
                            .partial_cmp(&a.accuracy_score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    // stable, deterministic final tie-break
                    .then_with(|| a.id.cmp(&b.id))
            })
            .map(|m| m.id.clone())
    }

    /// Discover custom Whisper models (.bin files) in the models directory.
    /// Skips files that match predefined model filenames.
    fn discover_custom_whisper_models(
        models_dir: &Path,
        available_models: &mut HashMap<String, ModelInfo>,
    ) -> Result<()> {
        if !models_dir.exists() {
            return Ok(());
        }

        // Collect filenames of predefined Whisper file-based models to skip
        let predefined_filenames: HashSet<String> = available_models
            .values()
            .filter(|m| matches!(m.engine_type, EngineType::Whisper) && !m.is_directory)
            .map(|m| m.filename.clone())
            .collect();

        // Scan models directory for .bin files
        for entry in fs::read_dir(models_dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();

            // Only process .bin files (not directories)
            if !path.is_file() {
                continue;
            }

            let filename = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            // Skip hidden files
            if filename.starts_with('.') {
                continue;
            }

            // Only process .bin files (Whisper GGML format).
            // This also excludes .partial downloads (e.g., "model.bin.partial").
            // If we add discovery for other formats, add a .partial check before this filter.
            if !filename.ends_with(".bin") {
                continue;
            }

            // Skip predefined model files
            if predefined_filenames.contains(&filename) {
                continue;
            }

            // Generate model ID from filename (remove .bin extension)
            let model_id = filename.trim_end_matches(".bin").to_string();

            // Skip if model ID already exists (shouldn't happen, but be safe)
            if available_models.contains_key(&model_id) {
                continue;
            }

            // Generate display name: replace - and _ with space, capitalize words
            let display_name = model_id
                .replace(['-', '_'], " ")
                .split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            // Get file size in MB
            let size_mb = match path.metadata() {
                Ok(meta) => meta.len() / (1024 * 1024),
                Err(e) => {
                    warn!("Failed to get metadata for {}: {}", filename, e);
                    0
                }
            };

            info!(
                "Discovered custom Whisper model: {} ({}, {} MB)",
                model_id, filename, size_mb
            );

            available_models.insert(
                model_id.clone(),
                ModelInfo {
                    id: model_id,
                    name: display_name,
                    description: "Not officially supported".to_string(),
                    filename,
                    url: None,    // Custom models have no download URL
                    sha256: None, // Custom models skip verification
                    size_mb,
                    is_downloaded: true, // Already present on disk
                    is_downloading: false,
                    partial_size: 0,
                    is_directory: false,
                    engine_type: EngineType::Whisper,
                    accuracy_score: 0.0, // Sentinel: UI hides score bars when both are 0
                    speed_score: 0.0,
                    supports_translation: false,
                    supports_streaming: false,
                    is_recommended: false,
                    recommended_rank: None,
                    supported_languages: vec![],
                    supports_language_selection: true,
                    is_custom: true,
                    local_path: None,
                    local_folder: None,
                    is_cleanup_specialist: false,
                },
            );
        }

        Ok(())
    }

    /// Path to the persisted custom-LLM definitions file.
    fn custom_models_path(models_dir: &Path) -> PathBuf {
        models_dir.join("custom_models.json")
    }

    /// Broad multilingual tag list so custom LLM entries display a
    /// "multi-language" capability like the built-in LLMs. These models are
    /// generally multilingual; this only affects the capability chip, not
    /// transcription routing (LLMs are never the active transcription model).
    fn default_llm_languages() -> Vec<String> {
        vec![
            "en", "zh", "zh-Hans", "zh-Hant", "de", "es", "fr", "it", "pt", "ru", "ja", "ko", "ar",
            "hi", "vi", "id", "tr", "pl", "nl",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    /// Human-readable card description for a custom model. Derived from the
    /// source repo (we don't fabricate a marketing blurb), and intentionally
    /// free of the word "custom" — the Models tab already groups these under
    /// the user's downloaded models.
    fn custom_description(repo_id: &str, is_vision: bool) -> String {
        let mut description = format!("From {} on Hugging Face.", repo_id);
        if is_vision {
            description.push_str(" Supports vision.");
        }
        description
    }

    /// Build a `ModelInfo` (catalog entry) from a persisted custom record.
    /// `is_downloaded` is left false here; `update_download_status` sets it
    /// based on whether the file is actually present on disk.
    fn record_to_model_info(record: &CustomModelRecord) -> ModelInfo {
        ModelInfo {
            id: record.id.clone(),
            name: record.name.clone(),
            // Derive the description so older saved entries pick up the current
            // wording without needing to be re-added.
            description: Self::custom_description(&record.repo_id, record.is_vision),
            filename: record.filename.clone(),
            url: Some(record.url.clone()),
            sha256: None, // user-supplied; verification skipped
            size_mb: record.size_mb,
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: EngineType::LlamaCpp,
            accuracy_score: 0.0, // Sentinel: UI hides score bars when both are 0
            speed_score: 0.0,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: Self::default_llm_languages(),
            supports_language_selection: false,
            is_custom: true,
            local_path: None,
            local_folder: None,
            is_cleanup_specialist: false,
        }
    }

    /// Load persisted custom GGUF LLM models from `custom_models.json` and
    /// insert them into `available_models`. Returns the records keyed by id so
    /// the manager can resolve download URLs and vision projectors later.
    fn load_custom_llm_models(
        models_dir: &Path,
        available_models: &mut HashMap<String, ModelInfo>,
    ) -> Result<HashMap<String, CustomModelRecord>> {
        let path = Self::custom_models_path(models_dir);
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let contents = fs::read_to_string(&path)?;
        let records: Vec<CustomModelRecord> = serde_json::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Invalid custom_models.json: {}", e))?;

        let mut map = HashMap::new();
        for record in records {
            // Don't let a stale custom entry shadow a built-in model id.
            if available_models.contains_key(&record.id) && !map.contains_key(&record.id) {
                warn!(
                    "Custom model id '{}' collides with an existing model; skipping",
                    record.id
                );
                continue;
            }
            info!(
                "Loaded custom LLM model: {} ({})",
                record.id, record.filename
            );
            available_models.insert(record.id.clone(), Self::record_to_model_info(&record));
            map.insert(record.id.clone(), record);
        }

        Ok(map)
    }

    /// Persist the current set of custom-LLM records to `custom_models.json`.
    fn save_custom_models(&self) -> Result<()> {
        let records: Vec<CustomModelRecord> = {
            let customs = self.custom_models.lock().unwrap();
            customs.values().cloned().collect()
        };
        let path = Self::custom_models_path(&self.models_dir);
        let json = serde_json::to_string_pretty(&records)?;
        fs::write(&path, json)?;
        Ok(())
    }

    /// Turn an arbitrary string into a filesystem/id-safe slug.
    fn slugify(input: &str) -> String {
        let mut slug = String::with_capacity(input.len());
        let mut prev_dash = false;
        for ch in input.chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                prev_dash = false;
            } else if !prev_dash {
                slug.push('-');
                prev_dash = true;
            }
        }
        slug.trim_matches('-').to_string()
    }

    /// Generate a friendly display name from the repo id and filename, e.g.
    /// `bartowski/Qwen_Qwen3.5-4B-GGUF` + `...-Q4_K_M.gguf` -> "Qwen Qwen3.5 4B (Q4_K_M)".
    fn generate_custom_name(repo_id: &str, filename: &str) -> String {
        let model_part = repo_id.rsplit('/').next().unwrap_or(repo_id);
        let base = model_part
            .trim_end_matches("-GGUF")
            .trim_end_matches("-gguf")
            .replace(['_', '-'], " ");
        let base = base.split_whitespace().collect::<Vec<_>>().join(" ");
        let quant = crate::huggingface::extract_quant(filename);
        if quant.is_empty() {
            base
        } else {
            format!("{} ({})", base, quant)
        }
    }

    /// Add a user-chosen GGUF model from the Hugging Face Hub as a custom local
    /// LLM. Registers it in the in-memory catalog and persists it so it
    /// survives restarts. The caller then downloads it via `download_model`.
    ///
    /// `mmproj_filename`, when provided, is the repo's vision projector; it will
    /// be fetched alongside the weights so multimodal models can see images.
    pub fn add_custom_llm_model(
        &self,
        repo_id: &str,
        filename: &str,
        size_mb: u64,
        mmproj_filename: Option<String>,
    ) -> Result<ModelInfo> {
        let repo_id = repo_id.trim();
        let filename = filename.trim();
        if repo_id.is_empty() || filename.is_empty() {
            return Err(anyhow::anyhow!("Repository and file are required"));
        }
        if !filename.to_lowercase().ends_with(".gguf") {
            return Err(anyhow::anyhow!("Selected file must be a .gguf model"));
        }

        // Generate a unique id, avoiding collisions with built-in or other
        // custom models (different repos can share a filename).
        let base_id = format!(
            "custom-{}",
            Self::slugify(filename.trim_end_matches(".gguf"))
        );
        let id = {
            let models = self.available_models.lock().unwrap();
            let mut candidate = base_id.clone();
            let mut n = 2;
            while models.contains_key(&candidate) {
                candidate = format!("{}-{}", base_id, n);
                n += 1;
            }
            candidate
        };

        let is_vision = mmproj_filename.is_some();
        let mmproj_url = mmproj_filename
            .as_ref()
            .map(|f| crate::huggingface::resolve_url(repo_id, f));

        let record = CustomModelRecord {
            id: id.clone(),
            name: Self::generate_custom_name(repo_id, filename),
            description: Self::custom_description(repo_id, is_vision),
            filename: filename.to_string(),
            url: crate::huggingface::resolve_url(repo_id, filename),
            size_mb,
            repo_id: repo_id.to_string(),
            mmproj_filename,
            mmproj_url,
            is_vision,
        };

        let model_info = Self::record_to_model_info(&record);

        {
            let mut models = self.available_models.lock().unwrap();
            models.insert(id.clone(), model_info.clone());
        }
        {
            let mut customs = self.custom_models.lock().unwrap();
            customs.insert(id.clone(), record);
        }
        self.save_custom_models()?;

        info!("Added custom LLM model '{}' from {}", id, repo_id);
        Ok(model_info)
    }

    /// Resolve the vision projector (filename, download URL) for a model, if
    /// any. Checks the built-in mapping first, then user-added custom models.
    pub fn resolve_mmproj(&self, model_id: &str) -> Option<(String, String)> {
        if let Some((name, url)) = mmproj_for(model_id) {
            return Some((name.to_string(), url.to_string()));
        }
        let customs = self.custom_models.lock().unwrap();
        customs.get(model_id).and_then(|record| {
            match (&record.mmproj_filename, &record.mmproj_url) {
                (Some(filename), Some(url)) => Some((filename.clone(), url.clone())),
                _ => None,
            }
        })
    }

    // -----------------------------------------------------------------
    // Models the user already has on disk
    //
    // Two ways in — pick a file, or link a folder that gets scanned — both
    // ending as catalog entries whose `local_path` points at the user's own
    // file. The invariant across all of it: we read those files and never
    // write, move, or delete them.
    // -----------------------------------------------------------------

    /// Path to the persisted set of individually picked local models. Linked
    /// folders live in settings instead, since their contents are re-derived.
    fn local_models_path(models_dir: &Path) -> PathBuf {
        models_dir.join("local_models.json")
    }

    /// A stable key for a path, used only for identity — never for I/O.
    /// Separators are unified and, on Windows, case is folded, because
    /// `C:\Models\a.gguf` and `c:/models/a.gguf` are the same file there and
    /// must not produce two catalog entries.
    fn normalized_path_key(path: &Path) -> String {
        let raw = path.to_string_lossy().replace('\\', "/");
        let trimmed = raw.trim_end_matches('/').to_string();
        if cfg!(windows) {
            trimmed.to_lowercase()
        } else {
            trimmed
        }
    }

    /// Deterministic catalog id for a local model, derived purely from its path.
    ///
    /// Determinism is the whole requirement: the selected transcription model and
    /// the assistant's model are stored *by id*, so an id that shifted between
    /// runs would silently unselect the user's model on restart. The path hash is
    /// always included rather than only on collision, so the id of one model
    /// never depends on which other models happen to be present — two files with
    /// the same name in different folders coexist, and adding a third changes
    /// neither.
    fn local_model_id(path: &Path) -> String {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
        let mut hasher = Sha256::new();
        hasher.update(Self::normalized_path_key(path).as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        format!("local-{}-{}", Self::slugify(stem), &digest[..8])
    }

    /// Turn a filename stem into a display name: `my_fine-tune v2` -> `My Fine Tune V2`.
    fn prettify_stem(stem: &str) -> String {
        stem.replace(['-', '_'], " ")
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Card description for a local model. The full path is the most useful
    /// thing we can say — with several fine-tunes of the same base model, the
    /// path is often the only thing telling them apart — followed by the GGUF
    /// architecture and whether a projector was paired with it.
    fn local_description(record: &LocalModelRecord) -> String {
        let mut description = record.path.clone();
        if let Some(arch) = &record.architecture {
            description.push_str(&format!(" · {}", arch));
        }
        if record.mmproj_path.is_some() {
            description.push_str(" · Supports vision.");
        }
        description
    }

    /// Build a catalog entry from a local record.
    ///
    /// Capabilities start deliberately blank for transcription models:
    /// [`Self::apply_gguf_header_hints`] fills in the real language list,
    /// streaming, and translation support by reading the model's own GGUF header,
    /// and a load settles anything the header omits. Claiming capabilities we
    /// haven't verified would be worse than showing none.
    fn local_record_to_model_info(record: &LocalModelRecord) -> ModelInfo {
        let is_llm = matches!(record.engine_type, EngineType::LlamaCpp);
        ModelInfo {
            id: record.id.clone(),
            name: record.name.clone(),
            // Derived, not stored, so wording changes reach existing entries.
            description: Self::local_description(record),
            filename: Path::new(&record.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| record.path.clone()),
            url: None,    // nothing to download; it's already here
            sha256: None, // the user's own file; there is no expected digest
            size_mb: record.size_mb,
            // Set by `update_download_status` from whether the file is still
            // there, so an unplugged drive shows as unavailable rather than
            // failing at load time.
            is_downloaded: false,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type: record.engine_type.clone(),
            accuracy_score: 0.0, // Sentinel: UI hides score bars when both are 0
            speed_score: 0.0,
            supports_translation: false,
            supports_streaming: false,
            is_recommended: false,
            recommended_rank: None,
            supported_languages: if is_llm {
                Self::default_llm_languages()
            } else {
                vec![]
            },
            supports_language_selection: !is_llm,
            is_custom: true,
            local_path: Some(record.path.clone()),
            local_folder: record.folder.clone(),
            is_cleanup_specialist: false,
        }
    }

    /// Turn a classified file into a persistable record. `folder` is the linked
    /// folder it was found in, or `None` when the user picked the file directly.
    fn record_from_discovered(
        discovered: &DiscoveredModel,
        folder: Option<&Path>,
    ) -> LocalModelRecord {
        let stem = discovered
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Local model");
        LocalModelRecord {
            id: Self::local_model_id(&discovered.path),
            name: Self::prettify_stem(stem),
            path: discovered.path.to_string_lossy().to_string(),
            engine_type: discovered.engine_type(),
            size_mb: discovered.size_bytes / (1024 * 1024),
            mmproj_path: discovered
                .mmproj_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            architecture: discovered.architecture().map(|a| a.to_string()),
            folder: folder.map(|f| f.to_string_lossy().to_string()),
        }
    }

    /// Load individually picked local models from `local_models.json`.
    ///
    /// A corrupt file is reported and treated as empty rather than failing
    /// startup: losing the list of registered paths is recoverable by re-adding
    /// them, but refusing to start is not.
    fn load_local_model_records(models_dir: &Path) -> Vec<LocalModelRecord> {
        let path = Self::local_models_path(models_dir);
        if !path.exists() {
            return Vec::new();
        }
        match fs::read_to_string(&path).map(|c| serde_json::from_str::<Vec<LocalModelRecord>>(&c)) {
            Ok(Ok(records)) => records,
            Ok(Err(e)) => {
                warn!("Invalid local_models.json ({}); ignoring it", e);
                Vec::new()
            }
            Err(e) => {
                warn!("Failed to read local_models.json: {}", e);
                Vec::new()
            }
        }
    }

    /// Persist the individually picked local models. Folder-derived entries are
    /// intentionally excluded — writing them would resurrect models the user has
    /// since deleted from a linked folder.
    fn save_local_models(&self) -> Result<()> {
        let records: Vec<LocalModelRecord> = {
            let locals = self.local_models.lock().unwrap();
            locals
                .values()
                .filter(|record| record.folder.is_none())
                .cloned()
                .collect()
        };
        let path = Self::local_models_path(&self.models_dir);
        fs::write(&path, serde_json::to_string_pretty(&records)?)?;
        Ok(())
    }

    /// The linked folders from settings, as paths.
    pub fn model_folders(&self) -> Vec<String> {
        get_settings(&self.app_handle).model_folders
    }

    /// Rebuild every local catalog entry from scratch: the persisted picked
    /// files, then a fresh scan of each linked folder.
    ///
    /// A full rebuild rather than an incremental update, because that is what
    /// makes a linked folder behave like a view of the filesystem instead of a
    /// one-time import — a model dropped into the folder appears, one deleted
    /// disappears, and running it twice changes nothing.
    ///
    /// Returns the number of local models registered.
    fn rebuild_local_entries(&self) -> usize {
        use crate::managers::local_models::scan_folder;

        // Every local entry is identified by `local_path`, so a single sweep
        // clears both picked files and folder finds with no bookkeeping.
        {
            let mut models = self.available_models.lock().unwrap();
            models.retain(|_, model| model.local_path.is_none());
        }

        let mut records: HashMap<String, LocalModelRecord> = HashMap::new();

        // Individually picked files first, so they win over the same file also
        // appearing inside a linked folder (both produce the same id anyway,
        // since the id is derived from the path).
        for record in Self::load_local_model_records(&self.models_dir) {
            records.insert(record.id.clone(), record);
        }

        // Never rescan our own models directory: everything in it is already a
        // catalog entry, and a second entry for the same file would let the user
        // "remove" a managed model through the local-model path.
        let mut skip_dirs = HashSet::new();
        skip_dirs.insert(local_models::absolute_path(&self.models_dir));

        for folder in self.model_folders() {
            let root = PathBuf::from(&folder);
            for discovered in scan_folder(&root, &skip_dirs) {
                let record = Self::record_from_discovered(&discovered, Some(&root));
                // A file reachable from two linked folders (nested links) is one
                // model, registered once.
                records.entry(record.id.clone()).or_insert(record);
            }
        }

        let mut count = 0;
        {
            let mut models = self.available_models.lock().unwrap();
            for record in records.values() {
                // A local file must never shadow a built-in catalog id. Ids are
                // path-derived and prefixed, so this is belt-and-braces.
                if models.contains_key(&record.id) {
                    warn!(
                        "Local model id '{}' collides with an existing model; skipping {}",
                        record.id, record.path
                    );
                    continue;
                }
                models.insert(record.id.clone(), Self::local_record_to_model_info(record));
                count += 1;
            }
        }

        *self.local_models.lock().unwrap() = records;

        if count > 0 {
            info!("Registered {} local model(s) from disk", count);
        }
        count
    }

    /// Rebuild local entries and bring their availability and capabilities up to
    /// date. This is the entry point for anything that changes what's on disk or
    /// which folders are linked.
    pub fn refresh_local_models(&self) -> Result<usize> {
        let count = self.rebuild_local_entries();
        self.update_download_status()?;
        // Read real capabilities (languages, streaming, translation) out of each
        // local GGUF's own header, so a user's fine-tuned ASR model arrives with
        // an accurate language list instead of a blank one.
        self.reconcile_downloaded_cpp_headers();
        let _ = self.app_handle.emit("model-state-changed", ());
        Ok(count)
    }

    /// Register a single model file the user picked, wherever it lives.
    ///
    /// Rejects a path that isn't a usable model, and rejects a bare vision
    /// projector with an explanation, because both produce a catalog entry that
    /// could never load. Adding the same path twice is not an error — it
    /// resolves to the same id, so the existing entry is returned.
    pub fn add_local_model_file(&self, path: &str) -> Result<ModelInfo> {
        use crate::managers::local_models::describe_model_file;

        let path = Path::new(path.trim());
        if path.as_os_str().is_empty() {
            return Err(anyhow::anyhow!("No file was selected"));
        }
        if !path.is_file() {
            return Err(anyhow::anyhow!(
                "That file no longer exists: {}",
                path.display()
            ));
        }

        // Absolute, so the entry keeps working regardless of the process's
        // working directory on a later launch.
        let path = local_models::absolute_path(path);

        let discovered = describe_model_file(&path).map_err(|e| anyhow::anyhow!("{}", e))?;
        let record = Self::record_from_discovered(&discovered, None);

        // Already registered (possibly via a linked folder): promote it to a
        // picked file so it survives that folder being unlinked, and hand back
        // the entry rather than reporting a spurious error.
        let already_present = {
            let models = self.available_models.lock().unwrap();
            models.contains_key(&record.id)
        };

        let model_info = Self::local_record_to_model_info(&record);
        {
            let mut models = self.available_models.lock().unwrap();
            models.insert(record.id.clone(), model_info);
        }
        {
            let mut locals = self.local_models.lock().unwrap();
            locals.insert(record.id.clone(), record.clone());
        }
        self.save_local_models()?;

        // Resolve availability and read the header's real capabilities.
        self.update_download_status()?;
        self.apply_gguf_header_hints(&record.id);
        let _ = self.app_handle.emit("model-state-changed", ());

        if already_present {
            debug!("Local model '{}' was already registered", record.id);
        } else {
            info!(
                "Registered local model '{}' at {}",
                record.id,
                path.display()
            );
        }

        self.get_model_info(&record.id)
            .ok_or_else(|| anyhow::anyhow!("Failed to register {}", path.display()))
    }

    /// Link a folder and scan it. Returns how many models were found in it.
    ///
    /// Finding nothing is an error rather than a silent success: a folder with no
    /// models in it is almost always the wrong folder, and saying so immediately
    /// is more useful than adding an empty entry to the list.
    pub fn add_model_folder(&self, folder: &str) -> Result<usize> {
        use crate::managers::local_models::scan_folder;

        let folder = folder.trim();
        if folder.is_empty() {
            return Err(anyhow::anyhow!("No folder was selected"));
        }
        let root = PathBuf::from(folder);
        if !root.is_dir() {
            return Err(anyhow::anyhow!(
                "That folder doesn't exist: {}",
                root.display()
            ));
        }
        let root = local_models::absolute_path(&root);

        // Refuse the app's own models directory: its contents are already in the
        // catalog, and linking it would create a second, removable entry for
        // every managed model.
        let managed = local_models::absolute_path(&self.models_dir);
        if Self::normalized_path_key(&root) == Self::normalized_path_key(&managed) {
            return Err(anyhow::anyhow!(
                "That's the app's own models folder — everything in it is already listed."
            ));
        }

        let folder_string = root.to_string_lossy().to_string();
        let key = Self::normalized_path_key(&root);

        let mut settings = get_settings(&self.app_handle);
        if settings
            .model_folders
            .iter()
            .any(|existing| Self::normalized_path_key(Path::new(existing)) == key)
        {
            return Err(anyhow::anyhow!("That folder is already linked."));
        }

        let mut skip_dirs = HashSet::new();
        skip_dirs.insert(managed);
        let found = scan_folder(&root, &skip_dirs).len();
        if found == 0 {
            return Err(anyhow::anyhow!(
                "No models found in that folder. SpeakoFlow looks for .gguf and Whisper .bin files."
            ));
        }

        settings.model_folders.push(folder_string);
        write_settings(&self.app_handle, settings);

        let total = self.refresh_local_models()?;
        info!(
            "Linked model folder {} ({} model(s) found, {} local total)",
            root.display(),
            found,
            total
        );
        Ok(found)
    }

    /// Unlink a folder. Its models leave the catalog; the files are untouched.
    pub fn remove_model_folder(&self, folder: &str) -> Result<()> {
        let key = Self::normalized_path_key(Path::new(folder.trim()));
        let mut settings = get_settings(&self.app_handle);
        let before = settings.model_folders.len();
        settings
            .model_folders
            .retain(|existing| Self::normalized_path_key(Path::new(existing)) != key);
        if settings.model_folders.len() == before {
            return Err(anyhow::anyhow!("That folder isn't linked."));
        }
        write_settings(&self.app_handle, settings);
        self.refresh_local_models()?;
        info!("Unlinked model folder {}", folder);
        Ok(())
    }

    /// Where a model's vision projector actually lives, if it has one.
    ///
    /// The one place that resolves a projector for *both* kinds of model: a
    /// downloaded one sits in the models directory under a known filename, a
    /// local one is wherever the user's file is. Callers get a path and don't
    /// need to know which case they're in.
    pub fn resolve_mmproj_path(&self, model_id: &str) -> Option<PathBuf> {
        {
            let locals = self.local_models.lock().unwrap();
            if let Some(record) = locals.get(model_id) {
                // Local models resolve only here; they have no managed filename.
                return record.mmproj_path.as_ref().map(PathBuf::from);
            }
        }
        let (filename, _) = self.resolve_mmproj(model_id)?;
        Some(self.models_dir.join(filename))
    }

    /// Where a model's weights live: the user's own path for a local model,
    /// otherwise the managed `<models_dir>/<filename>`.
    ///
    /// Every path-resolution site funnels through this so a local model can
    /// never accidentally be looked for in the models directory (or, worse, be
    /// written to there).
    fn resolve_model_file(&self, model: &ModelInfo) -> PathBuf {
        match &model.local_path {
            Some(path) => PathBuf::from(path),
            None => self.models_dir.join(&model.filename),
        }
    }

    /// Verifies the SHA256 of `path` against `expected_sha256` (if provided).
    /// On mismatch or read error the partial file is deleted and an error is returned,
    /// so the next download attempt always starts from a clean state.
    /// When `expected_sha256` is `None` (custom user models) verification is skipped.
    fn verify_sha256(path: &Path, expected_sha256: Option<&str>, model_id: &str) -> Result<()> {
        let Some(expected) = expected_sha256 else {
            return Ok(());
        };
        match Self::compute_sha256(path) {
            Ok(actual) if actual == expected => {
                info!("SHA256 verified for model {}", model_id);
                Ok(())
            }
            Ok(actual) => {
                warn!(
                    "SHA256 mismatch for model {}: expected {}, got {}",
                    model_id, expected, actual
                );
                let _ = fs::remove_file(path);
                Err(anyhow::anyhow!(
                    "Download verification failed for model {}: file is corrupt. Please retry.",
                    model_id
                ))
            }
            Err(e) => {
                let _ = fs::remove_file(path);
                Err(anyhow::anyhow!(
                    "Failed to verify download for model {}: {}. Please retry.",
                    model_id,
                    e
                ))
            }
        }
    }

    /// Computes the SHA256 hex digest of a file, reading in 64KB chunks to handle large models.
    fn compute_sha256(path: &Path) -> Result<String> {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Download a companion file (e.g. a vision projector) to `dest`,
    /// streaming with progress events under `model_id`. No resume; on cancel
    /// the partial is removed. Skips if `dest` already exists.
    async fn download_companion(
        &self,
        model_id: &str,
        url: &str,
        dest: &std::path::Path,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<()> {
        if dest.exists() {
            return Ok(());
        }
        let file_name = dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mmproj.gguf");
        let tmp = self.models_dir.join(format!("{}.partial", file_name));

        let client = reqwest::Client::new();
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to download projector: HTTP {}",
                response.status()
            ));
        }
        let total = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;
        let mut stream = response.bytes_stream();
        let mut file = std::fs::File::create(&tmp)?;
        let mut last_emit = Instant::now();
        while let Some(chunk) = stream.next().await {
            if cancel_flag.load(Ordering::Relaxed) {
                drop(file);
                let _ = fs::remove_file(&tmp);
                return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
            }
            let chunk = chunk?;
            file.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            if last_emit.elapsed() >= Duration::from_millis(100) {
                let _ = self.app_handle.emit(
                    "model-download-progress",
                    &DownloadProgress {
                        model_id: model_id.to_string(),
                        downloaded,
                        total,
                        percentage: if total > 0 {
                            (downloaded as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        },
                    },
                );
                last_emit = Instant::now();
            }
        }
        file.flush()?;
        drop(file);
        fs::rename(&tmp, dest)?;
        Ok(())
    }

    /// Ordered list of URLs to try when downloading a model: any reliable
    /// mirror(s) first, then the canonical (original) source as a fallback.
    /// The downloader tries each in turn (retrying transient failures), so a
    /// flaky primary self-heals or falls back automatically.
    fn download_candidates(model_info: &ModelInfo) -> Vec<String> {
        let mut urls = Vec::new();
        if let Some(mirror) = Self::mirror_url_for(&model_info.id) {
            urls.push(mirror);
        }
        if let Some(url) = &model_info.url {
            // Avoid trying the same URL twice if a mirror equals the canonical.
            if !urls.iter().any(|u| u == url) {
                urls.push(url.clone());
            }
        }
        urls
    }

    /// A reliable self-hosted mirror for a bundled model, if one has been
    /// published. GitHub release assets are a great fit (global CDN, free) for
    /// files under GitHub's 2 GB per-asset limit — e.g. the small Gemma 3 1B.
    /// Larger models fall back to their canonical Hugging Face URL, which the
    /// downloader retries and resumes automatically.
    ///
    /// To activate a mirror: upload the exact model file as a GitHub release
    /// asset on the SpeakoFlow repo, then return its `browser_download_url`
    /// here. Until then this returns `None` and the canonical URL is used.
    // Intentional template: the match is a placeholder for per-model mirror
    // arms that maintainers uncomment once assets are uploaded, so keep it even
    // though it currently has only the wildcard arm.
    #[allow(clippy::match_single_binding)]
    fn mirror_url_for(model_id: &str) -> Option<String> {
        match model_id {
            // Example — uncomment and set the real release URL once the asset
            // is uploaded (Gemma 3 1B is 806 MB, well under the 2 GB limit):
            // "gemma-3-1b" => Some(
            //     "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/models-v1/gemma-3-1b-it-Q4_K_M.gguf".to_string(),
            // ),
            _ => None,
        }
    }

    /// Download `url` into the model's `.partial` file, resuming from whatever
    /// is already on disk. Returns `Completed` once the whole body is written,
    /// `Cancelled` if the user aborted mid-stream, or an `Err` for a transport,
    /// stream, or HTTP error (which the caller may retry). The partial file is
    /// preserved on error so the next attempt resumes instead of restarting.
    /// Download `url` into `partial_path` using concurrent range requests.
    ///
    /// Returns `Ok(None)` when the host cannot support this (no `206`, unknown
    /// total, or a file too small to be worth it), leaving the caller to use the
    /// sequential path. Every other failure is an `Err` the retry loop can act
    /// on, and the `.partial` plus its completion record survive so the next
    /// attempt resumes only the chunks that are actually missing.
    async fn attempt_parallel_download(
        &self,
        model_id: &str,
        url: &str,
        partial_path: &Path,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<Option<AttemptOutcome>> {
        let client = download_client()?;
        let Some(total_size) = probe_ranged_total(&client, url).await else {
            debug!("{} does not support ranged requests; using one stream", url);
            return Ok(None);
        };
        if total_size < PARALLEL_DOWNLOAD_MIN_BYTES {
            return Ok(None);
        }

        let chunk_count = total_size.div_ceil(DOWNLOAD_CHUNK_SIZE) as usize;
        let parts_path = parts_path_for(partial_path);

        // Resume only when the preallocated file and the completion record both
        // agree with the size just probed. Any mismatch means the pair cannot be
        // trusted, and trusting it is how out-of-order writes become a corrupt
        // file that passes as complete.
        let mut done = vec![false; chunk_count];
        let sizes_agree = partial_path
            .metadata()
            .map(|meta| meta.len() == total_size)
            .unwrap_or(false);
        let record = fs::read(&parts_path)
            .ok()
            .filter(|b| b.len() == chunk_count);
        match record.filter(|_| sizes_agree) {
            Some(bytes) => {
                for (slot, byte) in done.iter_mut().zip(bytes) {
                    *slot = byte == 1;
                }
                let resumed = done.iter().filter(|d| **d).count();
                if resumed > 0 {
                    info!(
                        "Resuming {} with {}/{} chunks already on disk",
                        model_id, resumed, chunk_count
                    );
                }
            }
            None => {
                let file = File::create(partial_path)?;
                file.set_len(total_size)?;
                fs::write(&parts_path, vec![0u8; chunk_count])?;
            }
        }

        let chunk_len = |index: usize| -> u64 {
            let start = index as u64 * DOWNLOAD_CHUNK_SIZE;
            DOWNLOAD_CHUNK_SIZE.min(total_size - start)
        };
        let already: u64 = done
            .iter()
            .enumerate()
            .filter(|(_, d)| **d)
            .map(|(i, _)| chunk_len(i))
            .sum();

        let file = Arc::new(std::fs::OpenOptions::new().write(true).open(partial_path)?);
        let done = Arc::new(Mutex::new(done));
        let downloaded = Arc::new(std::sync::atomic::AtomicU64::new(already));
        let next_index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let failure: Arc<Mutex<Option<anyhow::Error>>> = Arc::new(Mutex::new(None));

        let mut workers = Vec::with_capacity(DOWNLOAD_CONCURRENCY);
        for _ in 0..DOWNLOAD_CONCURRENCY.min(chunk_count) {
            let client = client.clone();
            let url = url.to_string();
            let file = file.clone();
            let done = done.clone();
            let parts_path = parts_path.clone();
            let downloaded = downloaded.clone();
            let next_index = next_index.clone();
            let failure = failure.clone();
            let cancel_flag = cancel_flag.clone();

            workers.push(tauri::async_runtime::spawn(async move {
                loop {
                    if cancel_flag.load(Ordering::Relaxed) || failure.lock().unwrap().is_some() {
                        return;
                    }
                    let index = next_index.fetch_add(1, Ordering::Relaxed);
                    if index >= done.lock().unwrap().len() {
                        return;
                    }
                    if done.lock().unwrap()[index] {
                        continue;
                    }

                    let start = index as u64 * DOWNLOAD_CHUNK_SIZE;
                    let end = (start + DOWNLOAD_CHUNK_SIZE).min(total_size) - 1;
                    let expected = (end - start + 1) as usize;

                    let outcome: Result<()> = async {
                        let response = client
                            .get(&url)
                            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                            .send()
                            .await?;
                        let status = response.status();
                        if status != reqwest::StatusCode::PARTIAL_CONTENT {
                            return Err(HttpStatusError { status }.into());
                        }
                        let mut buffer = Vec::with_capacity(expected);
                        let mut stream = response.bytes_stream();
                        while let Some(part) = stream.next().await {
                            if cancel_flag.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                            buffer.extend_from_slice(&part?);
                        }
                        if buffer.len() != expected {
                            return Err(anyhow::anyhow!(
                                "short chunk {}: got {} of {} bytes",
                                index,
                                buffer.len(),
                                expected
                            ));
                        }
                        // Record the chunk only after its bytes are durably
                        // placed, so a crash between the two can only ever
                        // under-report progress, never over-report it.
                        let file = file.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            write_all_at(&file, &buffer, start)
                        })
                        .await??;
                        Ok(())
                    }
                    .await;

                    match outcome {
                        Ok(()) if cancel_flag.load(Ordering::Relaxed) => return,
                        Ok(()) => {
                            let snapshot = {
                                let mut done = done.lock().unwrap();
                                done[index] = true;
                                done.iter().map(|d| u8::from(*d)).collect::<Vec<u8>>()
                            };
                            let _ = fs::write(&parts_path, &snapshot);
                            downloaded.fetch_add(expected as u64, Ordering::Relaxed);
                        }
                        Err(error) => {
                            *failure.lock().unwrap() = Some(error);
                            return;
                        }
                    }
                }
            }));
        }

        // Progress is reported from here rather than from the workers: eight
        // workers finishing chunks would otherwise emit in bursts, and the
        // frontend only needs a steady tick.
        let progress = {
            let app = self.app_handle.clone();
            let model_id = model_id.to_string();
            let downloaded = downloaded.clone();
            let cancel_flag = cancel_flag.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let bytes = downloaded.load(Ordering::Relaxed);
                    let _ = app.emit(
                        "model-download-progress",
                        &DownloadProgress {
                            model_id: model_id.clone(),
                            downloaded: bytes,
                            total: total_size,
                            percentage: (bytes as f64 / total_size as f64) * 100.0,
                        },
                    );
                    if bytes >= total_size || cancel_flag.load(Ordering::Relaxed) {
                        return;
                    }
                }
            })
        };

        for worker in workers {
            let _ = worker.await;
        }
        progress.abort();

        if let Some(error) = failure.lock().unwrap().take() {
            return Err(error);
        }
        if cancel_flag.load(Ordering::Relaxed) {
            info!("Download cancelled for: {}", model_id);
            return Ok(Some(AttemptOutcome::Cancelled));
        }
        if !done.lock().unwrap().iter().all(|d| *d) {
            return Err(anyhow::anyhow!(
                "parallel download for {} finished with chunks missing",
                model_id
            ));
        }

        file.sync_all()?;
        let _ = fs::remove_file(&parts_path);
        let _ = self.app_handle.emit(
            "model-download-progress",
            &DownloadProgress {
                model_id: model_id.to_string(),
                downloaded: total_size,
                total: total_size,
                percentage: 100.0,
            },
        );
        Ok(Some(AttemptOutcome::Completed))
    }

    async fn attempt_download(
        &self,
        model_id: &str,
        url: &str,
        partial_path: &Path,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<AttemptOutcome> {
        // Resume from the current partial size, if present.
        //
        // A `.parts` record means this `.partial` was preallocated to full size
        // by the parallel path, so its length says nothing about how much is
        // real. Appending to it would splice fresh bytes onto a file full of
        // holes, producing something that looks complete and cannot be. Start
        // clean instead.
        let parts_path = parts_path_for(partial_path);
        if parts_path.exists() {
            remove_partial(partial_path);
        }
        let mut resume_from = if partial_path.exists() {
            partial_path.metadata()?.len()
        } else {
            0
        };

        // A tuned client: a connect timeout stops a dead endpoint from hanging
        // forever, and a User-Agent keeps hosts like Hugging Face from rejecting
        // the request. Redirects (HF → CDN) are followed by default.
        let client = download_client()?;

        let mut request = client.get(url);
        if resume_from > 0 {
            request = request.header("Range", format!("bytes={}-", resume_from));
        }
        let mut response = request.send().await?;

        // Asked to resume but got 200 (not 206): the server ignored the Range,
        // so restart fresh to avoid appending a full body onto the partial.
        if resume_from > 0 && response.status() == reqwest::StatusCode::OK {
            warn!(
                "Server ignored range request for model {}, restarting download",
                model_id
            );
            drop(response);
            let _ = fs::remove_file(partial_path);
            resume_from = 0;
            response = client.get(url).send().await?;
        }

        let status = response.status();
        if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(HttpStatusError { status }.into());
        }

        let total_size = if resume_from > 0 {
            resume_from + response.content_length().unwrap_or(0)
        } else {
            response.content_length().unwrap_or(0)
        };

        let mut downloaded = resume_from;
        let mut stream = response.bytes_stream();
        // Buffered: reqwest hands back chunks in the tens of kilobytes, and
        // writing each one straight through was a syscall per chunk for the
        // whole file.
        let mut file = std::io::BufWriter::with_capacity(
            DOWNLOAD_WRITE_BUFFER,
            if resume_from > 0 {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(partial_path)?
            } else {
                std::fs::File::create(partial_path)?
            },
        );

        let emit_progress = |downloaded: u64| {
            let _ = self.app_handle.emit(
                "model-download-progress",
                &DownloadProgress {
                    model_id: model_id.to_string(),
                    downloaded,
                    total: total_size,
                    percentage: if total_size > 0 {
                        (downloaded as f64 / total_size as f64) * 100.0
                    } else {
                        0.0
                    },
                },
            );
        };

        emit_progress(downloaded);

        // Throttle progress events to max 10/sec (100ms intervals).
        let mut last_emit = Instant::now();
        let throttle_duration = Duration::from_millis(100);

        while let Some(chunk) = stream.next().await {
            if cancel_flag.load(Ordering::Relaxed) {
                // Flush before giving up so the bytes already accepted are on
                // disk and a later resume picks up from the real offset rather
                // than re-fetching a buffer's worth.
                let _ = file.flush();
                drop(file);
                info!("Download cancelled for: {}", model_id);
                return Ok(AttemptOutcome::Cancelled);
            }
            let chunk = chunk?;
            file.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            if last_emit.elapsed() >= throttle_duration {
                emit_progress(downloaded);
                last_emit = Instant::now();
            }
        }

        // Ensure 100% is shown, then flush and close before the caller moves it.
        emit_progress(downloaded);
        file.flush()?;
        drop(file);

        // A short read means the connection dropped before the body finished.
        // Keep the partial and report a (retryable) error so the caller resumes.
        if total_size > 0 && downloaded < total_size {
            return Err(anyhow::anyhow!(
                "incomplete download: got {} of {} bytes",
                downloaded,
                total_size
            ));
        }

        Ok(AttemptOutcome::Completed)
    }

    pub async fn download_model(&self, model_id: &str) -> Result<()> {
        let model_info = {
            let models = self.available_models.lock().unwrap();
            models.get(model_id).cloned()
        };

        let model_info =
            model_info.ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        // A local model is already on disk by definition; there is nothing to
        // fetch and no URL to fetch it from. Reachable if the UI ever routes a
        // "download" at one, so answer plainly instead of falling through to a
        // confusing "no download URL".
        if model_info.local_path.is_some() {
            return Err(anyhow::anyhow!(
                "This model is already on your device — there's nothing to download."
            ));
        }

        // Build the ordered list of sources to try (reliable mirror first, then
        // the canonical URL). Empty only if the model has no URL at all.
        let candidates = Self::download_candidates(&model_info);
        if candidates.is_empty() {
            return Err(anyhow::anyhow!("No download URL for model"));
        }
        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", &model_info.filename));

        // If the main weights already exist, repair any missing vision
        // projector through the normal registered download lifecycle. This
        // keeps progress, cancellation, and completion events consistent.
        if model_path.exists() {
            if partial_path.exists() {
                remove_partial(&partial_path);
            }

            let cancel_flag = Arc::new(AtomicBool::new(false));
            {
                let mut models = self.available_models.lock().unwrap();
                if let Some(model) = models.get_mut(model_id) {
                    model.is_downloading = true;
                }
            }
            {
                let mut flags = self.cancel_flags.lock().unwrap();
                flags.insert(model_id.to_string(), cancel_flag.clone());
            }
            let mut cleanup = DownloadCleanup {
                available_models: &self.available_models,
                cancel_flags: &self.cancel_flags,
                model_id: model_id.to_string(),
                cancel_flag: cancel_flag.clone(),
                disarmed: false,
            };

            if let Some((mmproj_name, mmproj_url)) = self.resolve_mmproj(model_id) {
                let mmproj_path = self.models_dir.join(&mmproj_name);
                if !mmproj_path.exists() {
                    self.download_companion(model_id, &mmproj_url, &mmproj_path, &cancel_flag)
                        .await?;
                }
            }

            {
                let mut flags = self.cancel_flags.lock().unwrap();
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
                }
                if flags
                    .get(model_id)
                    .is_some_and(|registered| Arc::ptr_eq(registered, &cancel_flag))
                {
                    flags.remove(model_id);
                }
            }
            cleanup.disarmed = true;
            self.update_download_status()?;
            let _ = self.app_handle.emit("model-download-complete", model_id);
            return Ok(());
        }

        // Claim this model id for exactly one in-flight download, atomically.
        //
        // Without this, a second transfer could start for a model that already
        // has one running, and both would append to the same `.partial` —
        // interleaving their writes into a file that can only fail checksum
        // verification after the user has waited out the whole download.
        // `cancel_flags` is the authoritative "a task is live" registry, so the
        // claim is a vacant-entry insert under a single lock: checking and
        // inserting separately would leave a window for two callers to both pass
        // the check.
        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut flags = self.cancel_flags.lock().unwrap();
            match flags.entry(model_id.to_string()) {
                std::collections::hash_map::Entry::Occupied(_) => {
                    debug!(
                        "Ignoring duplicate download request for {}: one is already in flight",
                        model_id
                    );
                    return Ok(());
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(cancel_flag.clone());
                }
            }
        }

        // Mark as downloading
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = true;
            }
        }

        // Guard ensures is_downloading and cancel_flags are cleaned up on every
        // error path. Disarmed only on success (which sets is_downloaded = true).
        let mut cleanup = DownloadCleanup {
            available_models: &self.available_models,
            cancel_flags: &self.cancel_flags,
            model_id: model_id.to_string(),
            cancel_flag: cancel_flag.clone(),
            disarmed: false,
        };

        // Try each source in turn; within a source, retry transient failures a
        // few times with exponential backoff. The partial file is preserved
        // across attempts, so every retry resumes rather than restarting — this
        // is what turns a flaky Hugging Face download into a reliable one.
        const MAX_ATTEMPTS_PER_URL: u32 = 4;
        let mut downloaded_ok = false;
        let mut last_error: Option<anyhow::Error> = None;

        'sources: for (source_idx, url) in candidates.iter().enumerate() {
            if source_idx > 0 {
                info!(
                    "Falling back to alternate source for model {}: {}",
                    model_id, url
                );
            } else {
                info!("Downloading model {} from {}", model_id, url);
            }

            for attempt in 1..=MAX_ATTEMPTS_PER_URL {
                if cancel_flag.load(Ordering::Relaxed) {
                    // Guard handles is_downloading + cancel_flags cleanup on drop.
                    return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
                }

                // Prefer concurrent range requests; fall back to one stream when
                // the host will not serve ranges or the file is small.
                let attempt_result = match self
                    .attempt_parallel_download(model_id, url, &partial_path, &cancel_flag)
                    .await
                {
                    Ok(Some(outcome)) => Ok(outcome),
                    Ok(None) => {
                        self.attempt_download(model_id, url, &partial_path, &cancel_flag)
                            .await
                    }
                    Err(error) => Err(error),
                };

                match attempt_result {
                    Ok(AttemptOutcome::Completed) => {
                        downloaded_ok = true;
                        break 'sources;
                    }
                    Ok(AttemptOutcome::Cancelled) => {
                        // Partial kept for resume; guard cleans up state on drop.
                        return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
                    }
                    Err(e) => {
                        // A 4xx (except 408/429) is permanent for this URL, so
                        // stop retrying it and fall through to the next source.
                        let retryable = match e.downcast_ref::<HttpStatusError>() {
                            Some(HttpStatusError { status }) => {
                                status.is_server_error()
                                    || *status == reqwest::StatusCode::REQUEST_TIMEOUT
                                    || *status == reqwest::StatusCode::TOO_MANY_REQUESTS
                            }
                            None => true, // transport / stream / IO error
                        };

                        warn!(
                            "Download attempt {}/{} for model {} from {} failed: {}",
                            attempt, MAX_ATTEMPTS_PER_URL, model_id, url, e
                        );
                        last_error = Some(e);

                        if !retryable {
                            break; // try the next source, if any
                        }
                        if attempt < MAX_ATTEMPTS_PER_URL {
                            // Interruptible exponential backoff: 1s, 2s, 4s.
                            let backoff = Duration::from_secs(1u64 << (attempt - 1));
                            let deadline = Instant::now() + backoff;
                            while Instant::now() < deadline {
                                if cancel_flag.load(Ordering::Relaxed) {
                                    return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
                                }
                                tokio::time::sleep(Duration::from_millis(200)).await;
                            }
                        }
                    }
                }
            }
        }

        if !downloaded_ok {
            return Err(last_error
                .unwrap_or_else(|| anyhow::anyhow!("Failed to download model {}", model_id)));
        }
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
        }

        // Verify SHA256 checksum. Runs in a blocking thread so the async executor is not
        // stalled while hashing large model files (up to 1.6 GB). On failure the partial
        // is deleted inside verify_sha256 so the next attempt always starts fresh.
        let _ = self.app_handle.emit("model-verification-started", model_id);
        info!("Verifying SHA256 for model {}...", model_id);
        let verify_path = partial_path.clone();
        let verify_expected = model_info.sha256.clone();
        let verify_model_id = model_id.to_string();
        let verify_result = tokio::task::spawn_blocking(move || {
            Self::verify_sha256(&verify_path, verify_expected.as_deref(), &verify_model_id)
        })
        .await
        .map_err(|e| anyhow::anyhow!("SHA256 task panicked: {}", e))?;
        verify_result?;
        let _ = self
            .app_handle
            .emit("model-verification-completed", model_id);
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
        }

        // Handle directory-based models (extract tar.gz) vs file-based models
        if model_info.is_directory {
            // Track that this model is being extracted
            {
                let mut extracting = self.extracting_models.lock().unwrap();
                extracting.insert(model_id.to_string());
            }

            // Emit extraction started event
            let _ = self.app_handle.emit("model-extraction-started", model_id);
            info!("Extracting archive for directory-based model: {}", model_id);

            // Use a temporary extraction directory to ensure atomic operations
            let temp_extract_dir = self
                .models_dir
                .join(format!("{}.extracting", &model_info.filename));
            let final_model_dir = self.models_dir.join(&model_info.filename);

            // Clean up any previous incomplete extraction
            if temp_extract_dir.exists() {
                let _ = fs::remove_dir_all(&temp_extract_dir);
            }

            // Create temporary extraction directory
            fs::create_dir_all(&temp_extract_dir)?;

            // Open the downloaded tar.gz file
            let tar_gz = File::open(&partial_path)?;
            let tar = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(tar);

            // Extract to the temporary directory first
            archive.unpack(&temp_extract_dir).map_err(|e| {
                let error_msg = format!("Failed to extract archive: {}", e);
                // Clean up failed extraction
                let _ = fs::remove_dir_all(&temp_extract_dir);
                // Delete the corrupt partial file so the next download attempt starts fresh
                // instead of resuming from a broken archive (issue #858).
                remove_partial(&partial_path);
                // Remove from extracting set
                {
                    let mut extracting = self.extracting_models.lock().unwrap();
                    extracting.remove(model_id);
                }
                let _ = self.app_handle.emit(
                    "model-extraction-failed",
                    &serde_json::json!({
                        "model_id": model_id,
                        "error": error_msg
                    }),
                );
                anyhow::anyhow!(error_msg)
            })?;

            // Find the actual extracted directory (archive might have a nested structure)
            let extracted_dirs: Vec<_> = fs::read_dir(&temp_extract_dir)?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                .collect();

            if extracted_dirs.len() == 1 {
                // Single directory extracted, move it to the final location
                let source_dir = extracted_dirs[0].path();
                if final_model_dir.exists() {
                    fs::remove_dir_all(&final_model_dir)?;
                }
                fs::rename(&source_dir, &final_model_dir)?;
                // Clean up temp directory
                let _ = fs::remove_dir_all(&temp_extract_dir);
            } else {
                // Multiple items or no directories, rename the temp directory itself
                if final_model_dir.exists() {
                    fs::remove_dir_all(&final_model_dir)?;
                }
                fs::rename(&temp_extract_dir, &final_model_dir)?;
            }

            info!("Successfully extracted archive for model: {}", model_id);
            // Remove from extracting set
            {
                let mut extracting = self.extracting_models.lock().unwrap();
                extracting.remove(model_id);
            }
            // Emit extraction completed event
            let _ = self.app_handle.emit("model-extraction-completed", model_id);

            // Remove the downloaded tar.gz file
            remove_partial(&partial_path);
        } else {
            // Move partial file to final location for file-based models
            fs::rename(&partial_path, &model_path)?;
        }
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
        }

        // For vision LLMs, fetch the companion multimodal projector now that
        // the main weights are in place. Reuses the same cancel flag so the
        // Cancel button aborts it too.
        if let Some((mmproj_name, mmproj_url)) = self.resolve_mmproj(model_id) {
            let mmproj_path = self.models_dir.join(&mmproj_name);
            if !mmproj_path.exists() {
                info!("Downloading vision projector for {}", model_id);
                self.download_companion(model_id, &mmproj_url, &mmproj_path, &cancel_flag)
                    .await?;
            }
        }

        // Atomically close the cancellation window. If Cancel wins the lock,
        // its flag is observed and success is withheld. If completion wins,
        // the flag is removed first so a late Cancel reports that the download
        // has already finished instead of pretending it was cancelled.
        //
        // The removal is ownership-checked for the same reason the guard is: this
        // download must never evict a replacement's flag.
        {
            let mut flags = self.cancel_flags.lock().unwrap();
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
            }
            if flags
                .get(model_id)
                .is_some_and(|registered| Arc::ptr_eq(registered, &cancel_flag))
            {
                flags.remove(model_id);
            }
        }

        // Disarm the guard - success path does its own state cleanup and marks
        // the model as downloaded.
        cleanup.disarmed = true;
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = false;
                model.is_downloaded = true;
                model.partial_size = 0;
            }
        }

        // Session 3: for transcribe.cpp GGUF models, read the freshly-downloaded
        // file's header and apply its declared capability hints (no-op for other
        // engines / non-GGUF). The authoritative reconcile still happens on load.
        self.apply_gguf_header_hints(model_id);

        // Emit completion event
        let _ = self.app_handle.emit("model-download-complete", model_id);

        info!(
            "Successfully downloaded model {} to {:?}",
            model_id, model_path
        );

        Ok(())
    }

    pub fn delete_model(&self, model_id: &str) -> Result<()> {
        debug!("ModelManager: delete_model called for: {}", model_id);

        let model_info = {
            let models = self.available_models.lock().unwrap();
            models.get(model_id).cloned()
        };

        let model_info =
            model_info.ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        debug!("ModelManager: Found model info: {:?}", model_info);

        // A model the user already had on disk is not ours to delete. "Removing"
        // it unregisters the path and nothing else — deleting someone's own
        // model file because they tidied up a list would be unforgivable, so the
        // file-touching code below is never reached for these.
        if let Some(local_path) = &model_info.local_path {
            if let Some(folder) = &model_info.local_folder {
                // Re-derived by the next scan, so removing it individually would
                // silently undo itself. Say what actually works instead.
                return Err(anyhow::anyhow!(
                    "This model comes from the linked folder {}. Unlink that folder to remove it, \
                     or delete the file yourself if you no longer want it.",
                    folder
                ));
            }

            let removed = self.local_models.lock().unwrap().remove(model_id);
            if removed.is_none() {
                return Err(anyhow::anyhow!("No saved entry found to remove"));
            }
            if let Err(error) = self.save_local_models() {
                // Roll back so the in-memory state matches what a restart will
                // load, and what we're about to tell the caller.
                if let Some(record) = removed {
                    self.local_models
                        .lock()
                        .unwrap()
                        .insert(model_id.to_string(), record);
                }
                return Err(error);
            }

            self.available_models.lock().unwrap().remove(model_id);
            info!(
                "Unregistered local model '{}' (file left in place at {})",
                model_id, local_path
            );
            let _ = self.app_handle.emit("model-deleted", model_id);
            return Ok(());
        }

        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", &model_info.filename));
        debug!("ModelManager: Model path: {:?}", model_path);
        debug!("ModelManager: Partial path: {:?}", partial_path);

        let mut deleted_something = false;

        if model_info.is_directory {
            // Delete complete model directory if it exists
            if model_path.exists() && model_path.is_dir() {
                info!("Deleting model directory at: {:?}", model_path);
                fs::remove_dir_all(&model_path)?;
                info!("Model directory deleted successfully");
                deleted_something = true;
            }
        } else {
            // Delete complete model file if it exists
            if model_path.exists() {
                info!("Deleting model file at: {:?}", model_path);
                fs::remove_file(&model_path)?;
                info!("Model file deleted successfully");
                deleted_something = true;
            }
        }

        // Delete partial file if it exists (same for both types)
        if partial_path.exists() {
            info!("Deleting partial file at: {:?}", partial_path);
            let _ = fs::remove_file(parts_path_for(&partial_path));
            fs::remove_file(&partial_path)?;
            info!("Partial file deleted successfully");
            deleted_something = true;
        }

        // Remove the companion vision projector (and its partial) for
        // multimodal models so deletion frees all associated files.
        if let Some((mmproj_name, _)) = self.resolve_mmproj(model_id) {
            let mmproj_path = self.models_dir.join(&mmproj_name);
            if mmproj_path.exists() {
                fs::remove_file(&mmproj_path)?;
                deleted_something = true;
            }
            let mmproj_partial = self.models_dir.join(format!("{}.partial", mmproj_name));
            if mmproj_partial.exists() {
                fs::remove_file(&mmproj_partial)?;
                deleted_something = true;
            }
        }

        if model_info.is_custom {
            // A saved Hugging Face entry must be removable even when the user
            // never downloaded its weights. Remove and persist its metadata;
            // roll the in-memory record back if the write fails so restart
            // behavior stays consistent with the result returned to the UI.
            let removed_record = self.custom_models.lock().unwrap().remove(model_id);
            if let Some(record) = removed_record {
                if let Err(error) = self.save_custom_models() {
                    self.custom_models
                        .lock()
                        .unwrap()
                        .insert(model_id.to_string(), record);
                    return Err(error);
                }
                deleted_something = true;
            }

            if !deleted_something {
                return Err(anyhow::anyhow!(
                    "No model files or saved entry found to delete"
                ));
            }

            let mut models = self.available_models.lock().unwrap();
            models.remove(model_id);
            debug!("ModelManager: removed custom model from available models");
        } else {
            if !deleted_something {
                return Err(anyhow::anyhow!("No model files found to delete"));
            }

            // Update download status (marks predefined models as not downloaded)
            self.update_download_status()?;
            debug!("ModelManager: download status updated");
        }

        // Emit event to notify UI
        let _ = self.app_handle.emit("model-deleted", model_id);

        Ok(())
    }

    pub fn get_model_path(&self, model_id: &str) -> Result<PathBuf> {
        let model_info = self
            .get_model_info(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        if !model_info.is_downloaded {
            return Err(anyhow::anyhow!("Model not available: {}", model_id));
        }

        // Ensure we don't return partial files/directories
        if model_info.is_downloading {
            return Err(anyhow::anyhow!(
                "Model is currently downloading: {}",
                model_id
            ));
        }

        // A model the user already had on disk resolves to their own path. No
        // `.partial` companion exists for it, so the only question is whether the
        // file is still there.
        if let Some(local_path) = &model_info.local_path {
            let path = PathBuf::from(local_path);
            if path.is_file() {
                return Ok(path);
            }
            return Err(anyhow::anyhow!(
                "This model's file is no longer at {}. It may have been moved, renamed, or be on a drive that isn't connected.",
                local_path
            ));
        }

        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", &model_info.filename));

        if model_info.is_directory {
            // For directory-based models, ensure the directory exists and is complete
            if model_path.exists() && model_path.is_dir() && !partial_path.exists() {
                Ok(model_path)
            } else {
                Err(anyhow::anyhow!(
                    "Complete model directory not found: {}",
                    model_id
                ))
            }
        } else {
            // For file-based models (existing logic)
            if model_path.exists() && !partial_path.exists() {
                Ok(model_path)
            } else {
                Err(anyhow::anyhow!(
                    "Complete model file not found: {}",
                    model_id
                ))
            }
        }
    }

    pub fn cancel_download(&self, model_id: &str) -> Result<()> {
        debug!("ModelManager: cancel_download called for: {}", model_id);

        // Claim the active cancellation flag. If completion already removed it,
        // the download is finished and the caller must not pretend cancellation
        // succeeded or clear a subsequently valid selection.
        {
            let flags = self.cancel_flags.lock().unwrap();
            let flag = flags
                .get(model_id)
                .ok_or_else(|| anyhow::anyhow!("No active download found for: {}", model_id))?;
            flag.store(true, Ordering::Relaxed);
            info!("Cancellation flag set for: {}", model_id);
        }

        // Update state immediately for UI responsiveness.
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = false;
            }
        }

        self.update_download_status()?;
        let _ = self.app_handle.emit("model-download-cancelled", model_id);

        info!("Download cancellation initiated for: {}", model_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// The primitive the whole parallel path rests on: eight workers writing at
    /// their own offsets into one preallocated file, in whatever order they
    /// finish, must assemble exactly the original bytes. `File::write_all` seeks,
    /// so it cannot be shared this way; this is what replaces it.
    #[test]
    fn positioned_writes_assemble_a_file_out_of_order() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("assembled.bin");

        const CHUNK: usize = 1024;
        const CHUNKS: usize = 8;
        let expected: Vec<u8> = (0..CHUNK * CHUNKS).map(|i| (i % 251) as u8).collect();

        let file = File::create(&path).expect("create");
        file.set_len((CHUNK * CHUNKS) as u64).expect("preallocate");

        // Deliberately not sequential, and the last chunk first.
        for index in [7usize, 0, 4, 2, 6, 1, 5, 3] {
            let start = index * CHUNK;
            write_all_at(&file, &expected[start..start + CHUNK], start as u64)
                .expect("positioned write");
        }
        file.sync_all().expect("sync");
        drop(file);

        assert_eq!(fs::read(&path).expect("read back"), expected);
    }

    /// Progress must come from the chunk record, not from file length.
    ///
    /// The parallel path preallocates the `.partial` to its full size on the
    /// first byte, so reporting length would show every download as finished the
    /// moment it began.
    #[test]
    fn progress_for_a_preallocated_partial_counts_chunks_not_length() {
        let dir = TempDir::new().expect("temp dir");
        let partial = dir.path().join("model.gguf.partial");

        let total = DOWNLOAD_CHUNK_SIZE * 4 + 500;
        let chunk_count = total.div_ceil(DOWNLOAD_CHUNK_SIZE) as usize;
        assert_eq!(chunk_count, 5);
        File::create(&partial)
            .and_then(|f| f.set_len(total))
            .expect("preallocate");

        // Nothing recorded yet: fully preallocated, genuinely zero downloaded.
        fs::write(parts_path_for(&partial), vec![0u8; chunk_count]).expect("record");
        assert_eq!(real_partial_size(&partial), 0);

        // Two whole chunks plus the short trailing chunk.
        let mut record = vec![0u8; chunk_count];
        record[0] = 1;
        record[2] = 1;
        record[4] = 1;
        fs::write(parts_path_for(&partial), &record).expect("record");
        assert_eq!(real_partial_size(&partial), DOWNLOAD_CHUNK_SIZE * 2 + 500);

        // Without a record, length is the honest answer (sequential append).
        fs::remove_file(parts_path_for(&partial)).expect("drop record");
        assert_eq!(real_partial_size(&partial), total);
    }

    /// A record that does not describe this file must never be believed.
    #[test]
    fn a_mismatched_resume_record_is_ignored() {
        let dir = TempDir::new().expect("temp dir");
        let partial = dir.path().join("model.gguf.partial");
        let total = DOWNLOAD_CHUNK_SIZE * 2;
        File::create(&partial)
            .and_then(|f| f.set_len(total))
            .expect("preallocate");

        // Claims nine chunks for a two-chunk file: a leftover from a different
        // download. Falling back to length is the safe reading.
        fs::write(parts_path_for(&partial), vec![1u8; 9]).expect("record");
        assert_eq!(real_partial_size(&partial), total);
    }

    #[test]
    fn removing_a_partial_also_removes_its_record() {
        let dir = TempDir::new().expect("temp dir");
        let partial = dir.path().join("model.gguf.partial");
        fs::write(&partial, b"partial bytes").expect("write partial");
        fs::write(parts_path_for(&partial), vec![0u8; 3]).expect("write record");

        remove_partial(&partial);

        assert!(!partial.exists(), "partial removed");
        assert!(
            !parts_path_for(&partial).exists(),
            "a record must never outlive the file it describes"
        );
    }

    #[test]
    fn the_resume_record_sits_beside_the_partial() {
        let parts = parts_path_for(Path::new("/models/model.gguf.partial"));
        assert_eq!(
            parts,
            PathBuf::from("/models/model.gguf.partial.parts"),
            "the record must not collide with the partial or the final file"
        );
    }

    /// A trailing chunk is short whenever the total is not a whole multiple of
    /// the chunk size, which is the normal case. Getting this wrong writes past
    /// the end of the file or leaves a hole at the tail.
    #[test]
    fn the_last_chunk_covers_only_the_remaining_bytes() {
        let total: u64 = DOWNLOAD_CHUNK_SIZE * 3 + 12345;
        let chunk_count = total.div_ceil(DOWNLOAD_CHUNK_SIZE) as usize;
        assert_eq!(chunk_count, 4);

        let spans: Vec<(u64, u64)> = (0..chunk_count)
            .map(|index| {
                let start = index as u64 * DOWNLOAD_CHUNK_SIZE;
                let end = (start + DOWNLOAD_CHUNK_SIZE).min(total) - 1;
                (start, end)
            })
            .collect();

        assert_eq!(spans[0], (0, DOWNLOAD_CHUNK_SIZE - 1));
        assert_eq!(spans[3], (DOWNLOAD_CHUNK_SIZE * 3, total - 1));
        // Contiguous, no gaps, no overlap, and ending exactly on the last byte.
        for pair in spans.windows(2) {
            assert_eq!(pair[1].0, pair[0].1 + 1);
        }
        let covered: u64 = spans.iter().map(|(s, e)| e - s + 1).sum();
        assert_eq!(covered, total);
    }

    /// A `ModelInfo` for the download-state tests. Built through an existing
    /// constructor so it stays valid as fields are added; only `is_downloading`
    /// matters to the guard under test.
    fn downloading_model_info(model_id: &str) -> ModelInfo {
        let mut info = ModelManager::local_record_to_model_info(&LocalModelRecord {
            id: model_id.to_string(),
            name: "SpeakoFlow Mini".to_string(),
            path: "C:/models/SpeakoFlow-Mini-0.8B-Q8_0.gguf".to_string(),
            engine_type: EngineType::LlamaCpp,
            size_mb: 795,
            mmproj_path: None,
            architecture: Some("qwen35".to_string()),
            folder: None,
        });
        info.is_downloading = true;
        info
    }

    /// Reproduces the observed failure: Cancel, then Download again before the
    /// cancelled transfer has noticed its flag.
    ///
    /// Timeline from a real log (a 795 MB download at ~0.5 MB/s):
    ///   15:03:21  cancel_download  -> flag A set to true
    ///   15:03:23  download again    -> flag B registered, replacing A
    ///   15:03:24  A finally observes its flag and unwinds
    ///
    /// A's guard used to clear `is_downloading` and remove the registered flag
    /// keyed only on the model id, so it wiped B's state: the UI offered a
    /// Download button while B was still transferring, and Cancel had no flag to
    /// set. The superseded guard must be a no-op instead.
    #[test]
    fn a_superseded_download_guard_does_not_clear_its_replacement() {
        let available_models: Mutex<HashMap<String, ModelInfo>> = Mutex::new(HashMap::new());
        let cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let model_id = "speakoflow-mini";

        available_models
            .lock()
            .unwrap()
            .insert(model_id.to_string(), downloading_model_info(model_id));

        // Download A starts and is then cancelled.
        let flag_a = Arc::new(AtomicBool::new(false));
        cancel_flags
            .lock()
            .unwrap()
            .insert(model_id.to_string(), flag_a.clone());
        flag_a.store(true, Ordering::Relaxed);

        // Download B starts before A has unwound, taking over the registry slot.
        let flag_b = Arc::new(AtomicBool::new(false));
        cancel_flags
            .lock()
            .unwrap()
            .insert(model_id.to_string(), flag_b.clone());

        // A unwinds now. Its guard must leave B's state completely alone.
        drop(DownloadCleanup {
            available_models: &available_models,
            cancel_flags: &cancel_flags,
            model_id: model_id.to_string(),
            cancel_flag: flag_a,
            disarmed: false,
        });

        assert!(
            available_models
                .lock()
                .unwrap()
                .get(model_id)
                .map(|m| m.is_downloading)
                .unwrap(),
            "B is still downloading, so the UI must not be told otherwise"
        );
        let flags = cancel_flags.lock().unwrap();
        let registered = flags
            .get(model_id)
            .expect("B's cancel flag must survive so Cancel still works");
        assert!(
            Arc::ptr_eq(registered, &flag_b),
            "the registered flag must still be B's own"
        );
    }

    /// The mirror case: with no replacement, the guard must still clean up.
    #[test]
    fn an_uncontested_download_guard_still_cleans_up() {
        let available_models: Mutex<HashMap<String, ModelInfo>> = Mutex::new(HashMap::new());
        let cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let model_id = "speakoflow-mini";

        available_models
            .lock()
            .unwrap()
            .insert(model_id.to_string(), downloading_model_info(model_id));

        let flag = Arc::new(AtomicBool::new(false));
        cancel_flags
            .lock()
            .unwrap()
            .insert(model_id.to_string(), flag.clone());

        drop(DownloadCleanup {
            available_models: &available_models,
            cancel_flags: &cancel_flags,
            model_id: model_id.to_string(),
            cancel_flag: flag,
            disarmed: false,
        });

        assert!(!available_models
            .lock()
            .unwrap()
            .get(model_id)
            .map(|m| m.is_downloading)
            .unwrap());
        assert!(cancel_flags.lock().unwrap().is_empty());
    }

    #[test]
    fn gemma_4_projectors_use_official_google_artifacts() {
        for id in ["gemma-4-e2b", "gemma-4-e4b", "gemma-4-12b"] {
            let (filename, url) =
                mmproj_for(id).unwrap_or_else(|| panic!("{id} must include a vision projector"));
            assert!(filename.ends_with(".gguf"));
            assert!(
                url.starts_with("https://huggingface.co/google/gemma-4-"),
                "{id} should use an official Google artifact"
            );
        }
    }

    #[test]
    fn catalog_models_are_inserted_as_transcribe_cpp() {
        let mut models = HashMap::new();
        ModelManager::insert_catalog_models(&mut models);

        // The 5 ranked recommended models from PLAN.md §4.
        for slug in [
            "parakeet-unified-en-0.6b",
            "nemotron-3.5-asr-streaming-0.6b",
            "canary-180m-flash",
            "cohere-transcribe-03-2026",
            "whisper-medium",
        ] {
            let id = format!("{}-gguf", slug);
            let m = models
                .get(&id)
                .unwrap_or_else(|| panic!("recommended model {} missing", id));
            assert_eq!(m.engine_type, EngineType::TranscribeCpp);
            assert!(m.is_recommended, "{} should be recommended", id);
            assert!(m.recommended_rank.is_some(), "{} should have a rank", id);
            assert!(!m.is_directory, "{} is a single-file GGUF", id);
            assert!(m.filename.ends_with(".gguf"), "{} filename", id);
            assert!(
                m.url
                    .as_ref()
                    .is_some_and(|u| u.starts_with("https://huggingface.co/handy-computer/")
                        && u.ends_with(".gguf")),
                "{} url",
                id
            );
            assert!(m.size_mb > 0, "{} size", id);
        }

        // Parakeet Unified EN is the streaming rank-1 English model.
        let parakeet = models.get("parakeet-unified-en-0.6b-gguf").unwrap();
        assert!(parakeet.supports_streaming);
        assert_eq!(parakeet.recommended_rank, Some(1));
        assert_eq!(parakeet.supported_languages, vec!["en".to_string()]);
        assert_eq!(parakeet.size_mb, 731_357_568 / (1024 * 1024));

        // The GGUF canary id must be namespaced so it can't shadow the legacy
        // transcribe-rs `canary-180m-flash` entry (N2, side-by-side).
        assert!(models.contains_key("canary-180m-flash-gguf"));
        assert!(!models.contains_key("canary-180m-flash"));

        // A batch-only model reports no streaming.
        assert!(
            !models
                .get("whisper-medium-gguf")
                .unwrap()
                .supports_streaming
        );
    }

    /// Both recommended-default ids must resolve to real, streaming catalog
    /// models — the guarantee behind "fresh onboarding recommends the streaming
    /// model" (PLAN.md Session 6).
    #[test]
    fn recommended_default_ids_resolve_to_streaming_catalog_models() {
        let mut models = HashMap::new();
        ModelManager::insert_catalog_models(&mut models);

        let english = models
            .get(RECOMMENDED_MODEL_ID)
            .expect("recommended English default must be a catalog model");
        assert_eq!(english.engine_type, EngineType::TranscribeCpp);
        assert!(english.is_recommended);
        assert_eq!(english.recommended_rank, Some(1));
        assert!(english.supports_streaming);
        assert_eq!(english.supported_languages, vec!["en".to_string()]);

        let multilingual = models
            .get(RECOMMENDED_MULTILINGUAL_MODEL_ID)
            .expect("recommended multilingual model must be a catalog model");
        assert_eq!(multilingual.engine_type, EngineType::TranscribeCpp);
        assert!(multilingual.is_recommended);
        assert_eq!(multilingual.recommended_rank, Some(2));
        assert!(multilingual.supports_streaming);
        assert!(
            multilingual.supported_languages.len() > 1,
            "the multilingual option must support many languages"
        );
    }

    /// Minimal transcription `ModelInfo` for the picker tests.
    fn make_stt(
        id: &str,
        is_downloaded: bool,
        is_recommended: bool,
        recommended_rank: Option<u32>,
        accuracy_score: f32,
        engine_type: EngineType,
    ) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            filename: format!("{id}.bin"),
            url: None,
            sha256: None,
            size_mb: 100,
            is_downloaded,
            is_downloading: false,
            partial_size: 0,
            is_directory: false,
            engine_type,
            accuracy_score,
            speed_score: 0.5,
            supports_translation: false,
            supports_streaming: false,
            is_recommended,
            recommended_rank,
            supported_languages: vec!["en".to_string()],
            supports_language_selection: false,
            is_custom: false,
            local_path: None,
            local_folder: None,
            is_cleanup_specialist: false,
        }
    }

    #[test]
    fn pick_default_prefers_recommended_rank_and_skips_non_transcription() {
        let mut models = HashMap::new();
        // A very accurate legacy model, the recommended rank-1 GGUF, and a
        // downloaded LLM that must never be chosen as the transcription default.
        models.insert(
            "small".to_string(),
            make_stt("small", true, false, None, 0.95, EngineType::Whisper),
        );
        models.insert(
            RECOMMENDED_MODEL_ID.to_string(),
            make_stt(
                RECOMMENDED_MODEL_ID,
                true,
                true,
                Some(1),
                0.70,
                EngineType::TranscribeCpp,
            ),
        );
        models.insert(
            "gemma-3-1b".to_string(),
            make_stt("gemma-3-1b", true, true, Some(1), 1.0, EngineType::LlamaCpp),
        );

        assert_eq!(
            ModelManager::pick_default_transcription_model(&models).as_deref(),
            Some(RECOMMENDED_MODEL_ID),
            "the recommended rank-1 transcription model wins over a more-accurate legacy one, and LLMs are ignored"
        );
    }

    #[test]
    fn pick_default_falls_back_to_downloaded_when_recommended_absent() {
        // The recommended GGUF exists in the catalog but isn't downloaded; the
        // only downloaded transcription model is a legacy one. The existing
        // default must keep working (PLAN.md Session 6 / N1).
        let mut models = HashMap::new();
        models.insert(
            "parakeet-tdt-0.6b-v3".to_string(),
            make_stt(
                "parakeet-tdt-0.6b-v3",
                true,
                false,
                None,
                0.80,
                EngineType::Parakeet,
            ),
        );
        models.insert(
            RECOMMENDED_MODEL_ID.to_string(),
            make_stt(
                RECOMMENDED_MODEL_ID,
                false, // not downloaded
                true,
                Some(1),
                0.90,
                EngineType::TranscribeCpp,
            ),
        );

        assert_eq!(
            ModelManager::pick_default_transcription_model(&models).as_deref(),
            Some("parakeet-tdt-0.6b-v3"),
        );
    }

    #[test]
    fn pick_default_is_none_when_nothing_downloaded() {
        // Fresh install: the recommended model is known but not on disk, so the
        // picker returns None and the selection is left for onboarding.
        let mut models = HashMap::new();
        models.insert(
            RECOMMENDED_MODEL_ID.to_string(),
            make_stt(
                RECOMMENDED_MODEL_ID,
                false,
                true,
                Some(1),
                0.90,
                EngineType::TranscribeCpp,
            ),
        );
        assert_eq!(
            ModelManager::pick_default_transcription_model(&models),
            None
        );
    }

    #[test]
    fn legacy_parakeet_v3_is_no_longer_recommended() {
        // Guards the Session 6 flip. The catalog GGUF set is the recommended
        // set now; verify that among the catalog-inserted models the recommended
        // ones are all TranscribeCpp (GGUF), i.e. no legacy transcribe-rs engine
        // is marked recommended by the catalog path. (The legacy Parakeet V3's
        // hardcoded `is_recommended: false` is compiled in `ModelManager::new`.)
        let mut models = HashMap::new();
        ModelManager::insert_catalog_models(&mut models);
        for m in models.values().filter(|m| m.is_recommended) {
            assert_eq!(
                m.engine_type,
                EngineType::TranscribeCpp,
                "recommended catalog model {} must be a GGUF transcribe.cpp model",
                m.id
            );
        }
    }

    #[test]
    fn test_discover_custom_whisper_models() {
        let temp_dir = TempDir::new().unwrap();
        let models_dir = temp_dir.path().to_path_buf();

        // Create test .bin files
        let mut custom_file = File::create(models_dir.join("my-custom-model.bin")).unwrap();
        custom_file.write_all(b"fake model data").unwrap();

        let mut another_file = File::create(models_dir.join("whisper_medical_v2.bin")).unwrap();
        another_file.write_all(b"another fake model").unwrap();

        // Create files that should be ignored
        File::create(models_dir.join(".hidden-model.bin")).unwrap(); // Hidden file
        File::create(models_dir.join("readme.txt")).unwrap(); // Non-.bin file
        File::create(models_dir.join("ggml-small.bin")).unwrap(); // Predefined filename
        fs::create_dir(models_dir.join("some-directory.bin")).unwrap(); // Directory

        // Set up available_models with a predefined Whisper model
        let mut models = HashMap::new();
        models.insert(
            "small".to_string(),
            ModelInfo {
                id: "small".to_string(),
                name: "Whisper Small".to_string(),
                description: "Test".to_string(),
                filename: "ggml-small.bin".to_string(),
                url: Some("https://example.com".to_string()),
                sha256: None,
                size_mb: 100,
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: false,
                engine_type: EngineType::Whisper,
                accuracy_score: 0.5,
                speed_score: 0.5,
                supports_translation: true,
                supports_streaming: false,
                is_recommended: false,
                recommended_rank: None,
                supported_languages: vec!["en".to_string()],
                supports_language_selection: true,
                is_custom: false,
                local_path: None,
                local_folder: None,
                is_cleanup_specialist: false,
            },
        );

        // Discover custom models
        ModelManager::discover_custom_whisper_models(&models_dir, &mut models).unwrap();

        // Should have discovered 2 custom models (my-custom-model and whisper_medical_v2)
        assert!(models.contains_key("my-custom-model"));
        assert!(models.contains_key("whisper_medical_v2"));

        // Verify custom model properties
        let custom = models.get("my-custom-model").unwrap();
        assert_eq!(custom.name, "My Custom Model");
        assert_eq!(custom.filename, "my-custom-model.bin");
        assert!(custom.url.is_none()); // Custom models have no URL
        assert!(custom.is_downloaded);
        assert!(custom.is_custom);
        assert_eq!(custom.accuracy_score, 0.0);
        assert_eq!(custom.speed_score, 0.0);
        assert!(custom.supported_languages.is_empty());

        // Verify underscore handling
        let medical = models.get("whisper_medical_v2").unwrap();
        assert_eq!(medical.name, "Whisper Medical V2");

        // Should NOT have discovered hidden, non-.bin, predefined, or directories
        assert!(!models.contains_key(".hidden-model"));
        assert!(!models.contains_key("readme"));
        assert!(!models.contains_key("some-directory"));
    }

    #[test]
    fn test_discover_custom_models_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let models_dir = temp_dir.path().to_path_buf();

        let mut models = HashMap::new();
        let count_before = models.len();

        ModelManager::discover_custom_whisper_models(&models_dir, &mut models).unwrap();

        // No new models should be added
        assert_eq!(models.len(), count_before);
    }

    #[test]
    fn test_discover_custom_models_nonexistent_dir() {
        let models_dir = PathBuf::from("/nonexistent/path/that/does/not/exist");

        let mut models = HashMap::new();
        let count_before = models.len();

        // Should not error, just return Ok
        let result = ModelManager::discover_custom_whisper_models(&models_dir, &mut models);
        assert!(result.is_ok());
        assert_eq!(models.len(), count_before);
    }

    // ── SHA256 verification tests ─────────────────────────────────────────────

    /// Helper: write `data` to a temp file and return (TempDir, path).
    /// TempDir must be kept alive for the duration of the test.
    fn write_temp_file(data: &[u8]) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("model.partial");
        let mut f = File::create(&path).unwrap();
        f.write_all(data).unwrap();
        (dir, path)
    }

    #[test]
    fn test_verify_sha256_skipped_when_none() {
        // Custom models have no expected hash — verification must be a no-op.
        let (_dir, path) = write_temp_file(b"anything");
        assert!(ModelManager::verify_sha256(&path, None, "custom").is_ok());
        assert!(
            path.exists(),
            "file must be untouched when verification is skipped"
        );
    }

    #[test]
    fn test_verify_sha256_passes_on_correct_hash() {
        // Compute the real hash so the test is self-consistent.
        let (_dir, path) = write_temp_file(b"hello world");
        let actual = ModelManager::compute_sha256(&path).unwrap();
        assert!(
            ModelManager::verify_sha256(&path, Some(&actual), "test_model").is_ok(),
            "should pass when hash matches"
        );
        assert!(
            path.exists(),
            "file must be kept on successful verification"
        );
    }

    #[test]
    fn test_verify_sha256_fails_and_deletes_partial_on_mismatch() {
        let (_dir, path) = write_temp_file(b"this is not the real model");
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        let result = ModelManager::verify_sha256(&path, Some(wrong_hash), "bad_model");

        assert!(result.is_err(), "mismatch must return an error");
        assert!(
            result.unwrap_err().to_string().contains("corrupt"),
            "error message should mention corruption"
        );
        assert!(
            !path.exists(),
            "partial file must be deleted after hash mismatch"
        );
    }

    #[test]
    fn test_verify_sha256_fails_and_deletes_partial_when_file_missing() {
        // Simulate a partial file that was already removed (e.g. disk full mid-download).
        let dir = TempDir::new().unwrap();
        let missing_path = dir.path().join("gone.partial");
        // Don't create the file — it should not exist.

        let result =
            ModelManager::verify_sha256(&missing_path, Some("anyexpectedhash"), "missing_model");

        assert!(result.is_err(), "missing file must return an error");
    }

    // -----------------------------------------------------------------
    // Models the user already has on disk
    // -----------------------------------------------------------------

    fn discovered(path: &str, engine: EngineType, mmproj: Option<&str>) -> DiscoveredModel {
        use crate::managers::local_models::LocalModelKind;
        let kind = match engine {
            EngineType::LlamaCpp => LocalModelKind::Llm {
                architecture: Some("qwen3".to_string()),
            },
            engine => LocalModelKind::Transcription {
                engine,
                architecture: Some("whisper".to_string()),
            },
        };
        DiscoveredModel {
            path: PathBuf::from(path),
            kind,
            size_bytes: 3 * 1024 * 1024 * 1024,
            mmproj_path: mmproj.map(PathBuf::from),
        }
    }

    /// The selected transcription model and the assistant's model are persisted
    /// *by id*, so an id that changed between runs would silently deselect the
    /// user's model on restart. Same path must always mean same id.
    #[test]
    fn local_model_id_is_deterministic_for_a_path() {
        let path = Path::new("/home/user/models/my-finetune-Q4_K_M.gguf");
        let first = ModelManager::local_model_id(path);
        let second = ModelManager::local_model_id(path);
        assert_eq!(first, second);
        assert!(
            first.starts_with("local-my-finetune-q4-k-m-"),
            "id should stay readable: {first}"
        );
    }

    /// Two fine-tunes with the same filename in different folders is the normal
    /// case when someone trains iteratively, so they must not collide — and the
    /// id must not depend on which other models happen to be registered.
    #[test]
    fn same_filename_in_different_folders_gets_distinct_ids() {
        let a = ModelManager::local_model_id(Path::new("/models/run-1/model.gguf"));
        let b = ModelManager::local_model_id(Path::new("/models/run-2/model.gguf"));
        assert_ne!(a, b);
    }

    /// A path typed or picked with different separators (or, on Windows,
    /// different case) is the same file and must not produce a second entry.
    #[test]
    fn path_identity_ignores_separator_style() {
        let a = ModelManager::normalized_path_key(Path::new("C:/models/sub/model.gguf"));
        let b = ModelManager::normalized_path_key(Path::new("C:\\models\\sub\\model.gguf"));
        assert_eq!(a, b);

        // Trailing separators must not create a distinct folder identity either.
        assert_eq!(
            ModelManager::normalized_path_key(Path::new("/models/dir/")),
            ModelManager::normalized_path_key(Path::new("/models/dir"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn path_identity_is_case_insensitive_on_windows() {
        assert_eq!(
            ModelManager::local_model_id(Path::new(r"C:\Models\Model.gguf")),
            ModelManager::local_model_id(Path::new(r"c:\models\model.gguf"))
        );
    }

    #[test]
    fn local_entry_points_at_the_users_file_and_never_downloads() {
        let record = ModelManager::record_from_discovered(
            &discovered("/models/chat-Q4_K_M.gguf", EngineType::LlamaCpp, None),
            None,
        );
        let info = ModelManager::local_record_to_model_info(&record);

        assert_eq!(
            info.local_path.as_deref(),
            Some("/models/chat-Q4_K_M.gguf"),
            "the entry must resolve to the user's own path"
        );
        assert!(
            info.url.is_none() && info.sha256.is_none(),
            "there is nothing to download and no expected digest for a user's file"
        );
        assert!(info.is_custom, "local models group with the user's models");
        assert!(
            info.local_folder.is_none(),
            "a picked file belongs to no linked folder"
        );
        assert_eq!(info.engine_type, EngineType::LlamaCpp);
        assert!(
            !info.supports_language_selection,
            "language choice is a transcription concept, not an LLM one"
        );
        assert_eq!(info.size_mb, 3072, "size should be reported in MB");
        assert_eq!(
            info.name, "Chat Q4 K M",
            "name is derived from the filename"
        );
    }

    /// Capabilities must not be invented for a model we haven't inspected past
    /// its architecture: the header probe and a real load fill these in.
    #[test]
    fn local_transcription_entry_starts_with_no_claimed_capabilities() {
        let record = ModelManager::record_from_discovered(
            &discovered("/models/ggml-custom.bin", EngineType::Whisper, None),
            None,
        );
        let info = ModelManager::local_record_to_model_info(&record);

        assert_eq!(info.engine_type, EngineType::Whisper);
        assert!(info.supported_languages.is_empty());
        assert!(!info.supports_streaming);
        assert!(!info.supports_translation);
        assert!(
            info.supports_language_selection,
            "the user can still pick a language for a transcription model"
        );
        assert!(
            !info.is_recommended && info.recommended_rank.is_none(),
            "an unknown local model must not be promoted over the catalog"
        );
    }

    #[test]
    fn folder_derived_entry_records_the_folder_it_came_from() {
        let record = ModelManager::record_from_discovered(
            &discovered("/vault/asr/tuned.gguf", EngineType::TranscribeCpp, None),
            Some(Path::new("/vault")),
        );
        assert_eq!(record.folder.as_deref(), Some("/vault"));

        let info = ModelManager::local_record_to_model_info(&record);
        assert_eq!(
            info.local_folder.as_deref(),
            Some("/vault"),
            "the UI needs this to say 'unlink the folder' instead of 'remove'"
        );
    }

    #[test]
    fn a_paired_projector_is_carried_through_and_described() {
        let record = ModelManager::record_from_discovered(
            &discovered(
                "/models/vlm-Q4_K_M.gguf",
                EngineType::LlamaCpp,
                Some("/models/mmproj-f16.gguf"),
            ),
            None,
        );
        assert_eq!(
            record.mmproj_path.as_deref(),
            Some("/models/mmproj-f16.gguf")
        );

        let description = ModelManager::local_description(&record);
        assert!(
            description.contains("/models/vlm-Q4_K_M.gguf"),
            "the path is the only reliable way to tell fine-tunes apart: {description}"
        );
        assert!(description.contains("Supports vision."), "{description}");
        assert!(description.contains("qwen3"), "{description}");
    }

    /// Only picked files are persisted. Folder finds are re-derived every scan,
    /// so persisting them would resurrect models the user has since deleted.
    #[test]
    fn only_picked_files_are_persisted() {
        let picked = ModelManager::record_from_discovered(
            &discovered("/models/picked.gguf", EngineType::LlamaCpp, None),
            None,
        );
        let scanned = ModelManager::record_from_discovered(
            &discovered("/vault/scanned.gguf", EngineType::LlamaCpp, None),
            Some(Path::new("/vault")),
        );

        let persisted: Vec<&LocalModelRecord> = [&picked, &scanned]
            .into_iter()
            .filter(|record| record.folder.is_none())
            .collect();

        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].path, "/models/picked.gguf");
    }

    #[test]
    fn a_record_survives_a_json_round_trip() {
        let record = ModelManager::record_from_discovered(
            &discovered(
                "/models/vlm.gguf",
                EngineType::LlamaCpp,
                Some("/models/mmproj.gguf"),
            ),
            None,
        );
        let json = serde_json::to_string(&record).unwrap();
        let back: LocalModelRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id, record.id);
        assert_eq!(back.path, record.path);
        assert_eq!(back.engine_type, record.engine_type);
        assert_eq!(back.mmproj_path, record.mmproj_path);
        assert_eq!(back.architecture, record.architecture);
    }

    /// Older `local_models.json` files (and hand-edited ones) omit the optional
    /// fields; they must load rather than invalidate the whole list.
    #[test]
    fn a_minimal_record_json_still_loads() {
        let json = r#"{
            "id": "local-old-1234abcd",
            "name": "Old Entry",
            "path": "/models/old.gguf",
            "engine_type": "LlamaCpp",
            "size_mb": 100
        }"#;
        let record: LocalModelRecord = serde_json::from_str(json).unwrap();
        assert!(record.mmproj_path.is_none());
        assert!(record.architecture.is_none());
        assert!(record.folder.is_none());
    }
}
