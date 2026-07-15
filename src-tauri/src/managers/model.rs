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
struct DownloadCleanup<'a> {
    available_models: &'a Mutex<HashMap<String, ModelInfo>>,
    cancel_flags: &'a Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    model_id: String,
    disarmed: bool,
}

impl<'a> Drop for DownloadCleanup<'a> {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(self.model_id.as_str()) {
                model.is_downloading = false;
            }
        }
        self.cancel_flags.lock().unwrap().remove(&self.model_id);
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
            },
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
            },
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
            },
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
            },
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
            },
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
            },
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
            },
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
            },
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
            },
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
        };

        // Migrate any bundled models to user directory
        manager.migrate_bundled_models()?;

        // Migrate GigaAM from single-file to directory format
        manager.migrate_gigaam_to_directory()?;

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
        models.values().cloned().collect()
    }

    pub fn get_model_info(&self, model_id: &str) -> Option<ModelInfo> {
        let models = self.available_models.lock().unwrap();
        models.get(model_id).cloned()
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

        let filename = {
            let models = self.available_models.lock().unwrap();
            match models.get(model_id) {
                Some(m)
                    if matches!(m.engine_type, EngineType::TranscribeCpp) && m.is_downloaded =>
                {
                    m.filename.clone()
                }
                _ => return,
            }
        };
        let path = self.models_dir.join(&filename);
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
        let projector_ready = |model_id: &str| {
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
                    model.partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0);
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
                    model.partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0);
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
    /// Full steps + per-model checklist: docs/TODO_BEFORE_RELEASE.md §2.
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
    async fn attempt_download(
        &self,
        model_id: &str,
        url: &str,
        partial_path: &Path,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<AttemptOutcome> {
        // Resume from the current partial size, if present.
        let mut resume_from = if partial_path.exists() {
            partial_path.metadata()?.len()
        } else {
            0
        };

        // A tuned client: a connect timeout stops a dead endpoint from hanging
        // forever, and a User-Agent keeps hosts like Hugging Face from rejecting
        // the request. Redirects (HF → CDN) are followed by default.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .user_agent(concat!("SpeakoFlow/", env!("CARGO_PKG_VERSION")))
            .build()?;

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
        let mut file = if resume_from > 0 {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(partial_path)?
        } else {
            std::fs::File::create(partial_path)?
        };

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
                let _ = fs::remove_file(&partial_path);
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
                flags.remove(model_id);
            }
            cleanup.disarmed = true;
            self.update_download_status()?;
            let _ = self.app_handle.emit("model-download-complete", model_id);
            return Ok(());
        }

        // Mark as downloading
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = true;
            }
        }

        // Create cancellation flag for this download
        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut flags = self.cancel_flags.lock().unwrap();
            flags.insert(model_id.to_string(), cancel_flag.clone());
        }

        // Guard ensures is_downloading and cancel_flags are cleaned up on every
        // error path. Disarmed only on success (which sets is_downloaded = true).
        let mut cleanup = DownloadCleanup {
            available_models: &self.available_models,
            cancel_flags: &self.cancel_flags,
            model_id: model_id.to_string(),
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

                match self
                    .attempt_download(model_id, url, &partial_path, &cancel_flag)
                    .await
                {
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
                let _ = fs::remove_file(&partial_path);
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
            let _ = fs::remove_file(&partial_path);
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
        {
            let mut flags = self.cancel_flags.lock().unwrap();
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(anyhow::anyhow!(DOWNLOAD_CANCELLED_ERROR));
            }
            flags.remove(model_id);
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
}
