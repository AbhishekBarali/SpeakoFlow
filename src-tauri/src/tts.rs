//! Remote TTS engines for the assistant's spoken replies.
//!
//! Every engine except Kokoro is handled here in Rust: audio is fetched and
//! played natively via rodio, so playback works even when the panel webview is
//! hidden. The engines are described once, in [`TTS_PROVIDERS`], and the
//! engine id stored in `assistant_tts_engine` is a key into that table.
//!
//! "OpenAI-compatible" is near-universal for chat but not for speech. Only
//! OpenAI, OpenRouter, Groq, Inworld and self-hosted servers accept the
//! `/audio/speech` shape; everyone else gets its own [`TtsProtocol`] variant,
//! because the differences are not cosmetic — Deepgram takes the model as a
//! query parameter and the voice as part of the model id, Google and Mistral
//! answer with base64 inside JSON, xAI requires a `language`, and Cartesia
//! rejects a request without its version header.
//!
//! The "custom" engine is what makes the list open-ended: any server that
//! implements `POST /v1/audio/speech` (Kokoro-FastAPI, Speaches, Chatterbox,
//! Orpheus-FastAPI, LocalAI, AllTalk, openai-edge-tts, vLLM-Omni, Azure
//! OpenAI…) is reachable by pasting its address, with no key unless the server
//! asks for one.
//!
//! The "kokoro" engine runs fully locally in the panel webview
//! (kokoro-js, WebGPU) and never reaches this module.

use crate::settings::{AppSettings, OPENROUTER_TTS_BASE_URL};
use log::{debug, error};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Provider registry
// ---------------------------------------------------------------------------

/// The request shape an engine speaks. Each variant is a genuinely different
/// wire format; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TtsProtocol {
    /// Kokoro, synthesized inside the assistant webview.
    Local,
    /// `POST {base}/audio/speech` with `{model, input, voice, response_format, speed}`.
    OpenAiCompatible,
    /// `POST /v1/text-to-speech/{voice_id}` with an `xi-api-key` header.
    ElevenLabs,
    /// SSML to `{region}.tts.speech.microsoft.com/cognitiveservices/v1`.
    AzureSpeech,
    /// `POST /v1/speak?model=…` with `{text}`. The voice is part of the model id.
    Deepgram,
    /// `POST /tts/bytes` with a `Cartesia-Version` header.
    Cartesia,
    /// `POST /v1/text:synthesize`, answered with base64 audio inside JSON.
    GoogleCloud,
    /// `POST /v1/tts` with `{text, voice_id, language}`.
    Xai,
    /// `POST /v1/audio/speech` with `voice_id`, answered with base64 audio inside JSON.
    Mistral,
}

/// How an OpenAI-compatible engine takes its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TtsAuth {
    /// `Authorization: Bearer <key>`.
    Bearer,
    /// `Authorization: Basic <key>`. Inworld hands out a ready-encoded Basic
    /// credential, and its voice listing accepts nothing else.
    Basic,
}

/// One voice engine, described once so the request path, the voice and model
/// pickers, the call precondition and the chunk sizing cannot disagree.
#[derive(Debug)]
pub(crate) struct TtsProvider {
    pub id: &'static str,
    pub label: &'static str,
    pub protocol: TtsProtocol,
    /// Fixed API root. `None` means the user supplies it (Azure, custom).
    pub base_url: Option<&'static str>,
    /// Model used when the user has not chosen one. Empty where the engine has
    /// no model field (the voice implies the model, or there is only one).
    pub default_model: &'static str,
    /// Voice used when the user has not chosen one. Empty where no voice is
    /// safe to assume (ElevenLabs and Mistral voices belong to an account).
    pub default_voice: &'static str,
    /// Suggested models for the picker when the engine has no listing of its
    /// own. Not a whitelist — the field stays free text.
    pub models: &'static [&'static str],
    /// Whether a request can succeed without a key. False only for the custom
    /// engine: a self-hosted server legitimately needs none.
    pub requires_key: bool,
    /// Longest text one request accepts, in characters. Speech is split below
    /// this, which is what lets a 200-character engine (Groq) read a long reply.
    pub max_chars: usize,
    /// Speed range the API accepts, or `None` when it has no speed control.
    pub speed_range: Option<(f64, f64)>,
    /// OpenAI-compatible only: the `response_format` to ask for first.
    pub response_format: &'static str,
    /// OpenAI-compatible only.
    pub auth: TtsAuth,
}

/// `Cartesia-Version` sent with every Cartesia request. The API rejects a
/// request without one; this is the version whose schema the request below is
/// written against.
const CARTESIA_VERSION: &str = "2026-08-14";

/// Every voice engine the app ships, in the order the settings UI shows them.
pub(crate) const TTS_PROVIDERS: &[TtsProvider] = &[
    TtsProvider {
        id: "kokoro",
        label: "Kokoro",
        protocol: TtsProtocol::Local,
        base_url: None,
        default_model: "",
        default_voice: "af_heart",
        models: &[],
        requires_key: false,
        max_chars: usize::MAX,
        speed_range: Some((0.25, 4.0)),
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "openai",
        label: "OpenAI",
        protocol: TtsProtocol::OpenAiCompatible,
        base_url: Some("https://api.openai.com/v1"),
        default_model: "gpt-4o-mini-tts",
        default_voice: "alloy",
        models: &["gpt-4o-mini-tts", "tts-1", "tts-1-hd"],
        requires_key: true,
        max_chars: 4096,
        speed_range: Some((0.25, 4.0)),
        response_format: "mp3",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "elevenlabs",
        label: "ElevenLabs",
        protocol: TtsProtocol::ElevenLabs,
        base_url: Some("https://api.elevenlabs.io"),
        default_model: "eleven_flash_v2_5",
        default_voice: "",
        models: &[],
        requires_key: true,
        // eleven_v3 caps a request at 5,000 characters; the other models allow
        // more, so the smallest limit is the safe one.
        max_chars: 5000,
        speed_range: Some((0.7, 1.2)),
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "openrouter",
        label: "OpenRouter",
        protocol: TtsProtocol::OpenAiCompatible,
        base_url: Some(OPENROUTER_TTS_BASE_URL),
        // OpenRouter slugs are namespaced, so OpenAI's bare `gpt-4o-mini-tts`
        // (the old fallback) does not exist there. Gemini TTS is listed on the
        // live speech catalog and has a documented voice set.
        default_model: "google/gemini-3.1-flash-tts-preview",
        default_voice: "Kore",
        models: &[],
        requires_key: true,
        max_chars: 4000,
        speed_range: Some((0.25, 4.0)),
        response_format: "mp3",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "deepgram",
        label: "Deepgram",
        protocol: TtsProtocol::Deepgram,
        base_url: Some("https://api.deepgram.com"),
        default_model: "",
        default_voice: "aura-2-thalia-en",
        models: &[],
        requires_key: true,
        // Aura rejects anything longer with HTTP 413.
        max_chars: 2000,
        speed_range: Some((0.7, 1.5)),
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "cartesia",
        label: "Cartesia",
        protocol: TtsProtocol::Cartesia,
        base_url: Some("https://api.cartesia.ai"),
        default_model: "sonic-3.6",
        // "Skylar", the voice Cartesia's own examples use.
        default_voice: "db6b0ed5-d5d3-463d-ae85-518a07d3c2b4",
        models: &["sonic-3.6", "sonic-3.5", "sonic-3", "sonic-latest"],
        requires_key: true,
        // Undocumented; generous for speech and far above a streamed chunk.
        max_chars: 2000,
        speed_range: Some((0.6, 1.5)),
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "google",
        label: "Google Cloud",
        protocol: TtsProtocol::GoogleCloud,
        base_url: Some("https://texttospeech.googleapis.com"),
        default_model: "",
        default_voice: "en-US-Chirp3-HD-Kore",
        models: &[],
        requires_key: true,
        // The documented limit is 5,000 *bytes*. Characters are what the
        // splitter counts, so this leaves room for three-byte scripts.
        max_chars: 1600,
        speed_range: Some((0.25, 2.0)),
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "azure",
        label: "Azure AI Speech",
        protocol: TtsProtocol::AzureSpeech,
        base_url: None,
        default_model: "",
        default_voice: "en-US-JennyNeural",
        models: &[],
        requires_key: true,
        max_chars: 4000,
        speed_range: Some((0.5, 2.0)),
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "groq",
        label: "Groq",
        protocol: TtsProtocol::OpenAiCompatible,
        base_url: Some("https://api.groq.com/openai/v1"),
        default_model: "canopylabs/orpheus-v1-english",
        default_voice: "troy",
        models: &[
            "canopylabs/orpheus-v1-english",
            "canopylabs/orpheus-arabic-saudi",
        ],
        requires_key: true,
        // Orpheus on Groq accepts at most 200 characters per request, answers
        // only in WAV, and documents no speed control.
        max_chars: 200,
        speed_range: None,
        response_format: "wav",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "xai",
        label: "xAI",
        protocol: TtsProtocol::Xai,
        base_url: Some("https://api.x.ai/v1"),
        default_model: "",
        default_voice: "eve",
        models: &[],
        requires_key: true,
        max_chars: 4000,
        speed_range: Some((0.7, 1.5)),
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "mistral",
        label: "Mistral",
        protocol: TtsProtocol::Mistral,
        base_url: Some("https://api.mistral.ai/v1"),
        default_model: "voxtral-mini-tts-2603",
        // `voice_id` is required and the preset ids are opaque, so the user
        // picks one from the loaded list.
        default_voice: "",
        models: &["voxtral-mini-tts-2603"],
        requires_key: true,
        // Mistral recommends staying under 300 words.
        max_chars: 1500,
        speed_range: None,
        response_format: "",
        auth: TtsAuth::Bearer,
    },
    TtsProvider {
        id: "inworld",
        label: "Inworld",
        protocol: TtsProtocol::OpenAiCompatible,
        base_url: Some("https://api.inworld.ai/v1"),
        default_model: "inworld-tts-2",
        default_voice: "Dennis",
        models: &["inworld-tts-2", "inworld-tts-2-flash"],
        requires_key: true,
        max_chars: 4000,
        // A speed outside this range is a 400, not a clamp.
        speed_range: Some((0.5, 1.5)),
        response_format: "mp3",
        auth: TtsAuth::Basic,
    },
    TtsProvider {
        id: "custom",
        label: "Custom server",
        protocol: TtsProtocol::OpenAiCompatible,
        base_url: None,
        // `tts-1` is the one model name nearly every self-hosted server
        // accepts (Kokoro-FastAPI, Speaches, openedai-speech, openai-edge-tts)
        // or ignores (Chatterbox, Orpheus-FastAPI, AllTalk).
        default_model: "tts-1",
        default_voice: "alloy",
        models: &[],
        requires_key: false,
        max_chars: 4000,
        speed_range: Some((0.25, 4.0)),
        response_format: "mp3",
        auth: TtsAuth::Bearer,
    },
];

/// The registry entry for an engine id.
pub(crate) fn provider(id: &str) -> Option<&'static TtsProvider> {
    TTS_PROVIDERS.iter().find(|p| p.id == id)
}

/// Whether `id` names an engine this build knows how to drive.
pub fn is_known_engine(id: &str) -> bool {
    provider(id).is_some()
}

/// The fixed API root of an engine, or `None` when the user supplies it.
pub fn fixed_base_url(engine: &str) -> Option<&'static str> {
    provider(engine).and_then(|p| p.base_url)
}

/// Longest text one request to the active engine may carry.
pub(crate) fn max_chars_for(settings: &AppSettings) -> usize {
    provider(&settings.assistant_tts_engine)
        .map(|p| p.max_chars)
        .unwrap_or(usize::MAX)
}

fn active_provider(settings: &AppSettings) -> Result<&'static TtsProvider, String> {
    provider(&settings.assistant_tts_engine)
        .ok_or_else(|| format!("Unknown TTS engine: {}", settings.assistant_tts_engine))
}

/// The user's value, or the engine's default when they left the field empty.
fn or_default<'a>(value: &'a str, default: &'a str) -> &'a str {
    let value = value.trim();
    if value.is_empty() {
        default
    } else {
        value
    }
}

/// The speed to request, clamped to what the engine accepts. `None` when the
/// engine has no speed control, or when the rate is normal and there is no
/// reason to send it.
fn speed_for(provider: &TtsProvider, settings: &AppSettings) -> Option<f64> {
    let (min, max) = provider.speed_range?;
    let speed = settings.assistant_tts_speed.clamp(min, max);
    ((speed - 1.0).abs() > f64::EPSILON).then_some(speed)
}

/// The engine's API root: the registry's fixed URL, or the one the user typed.
fn api_root(provider: &TtsProvider, settings: &AppSettings) -> Result<String, String> {
    if let Some(fixed) = provider.base_url {
        return Ok(fixed.to_string());
    }
    let raw = settings.assistant_tts_base_url.trim();
    if raw.is_empty() {
        return Err(format!(
            "{} needs a server address. Add it in the Voice settings.",
            provider.label
        ));
    }
    Ok(normalize_server_url(raw))
}

/// A bare `http://host:port` gets `/v1` appended, because every OpenAI-compatible
/// speech server serves its routes there and people paste the address a
/// server's startup log prints, which usually has no path. Anything with a path
/// is taken as given.
fn normalize_server_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    match reqwest::Url::parse(trimmed) {
        Ok(url) if url.path().is_empty() || url.path() == "/" => format!("{trimmed}/v1"),
        _ => trimmed.to_string(),
    }
}

/// Uniform message for a non-2xx response.
fn http_error(status: reqwest::StatusCode, body: &str) -> String {
    format!("{}: {}", status, truncate(body, 300))
}

/// Read a successful response body as raw audio bytes.
async fn audio_body(response: reqwest::Response) -> Result<Vec<u8>, String> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(http_error(status, &body));
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read audio: {}", e))
}

/// Read a successful response body as JSON.
async fn json_body(response: reqwest::Response) -> Result<serde_json::Value, String> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(http_error(status, &body));
    }
    response
        .json()
        .await
        .map_err(|e| format!("Unexpected response: {}", e))
}

/// Decode base64 audio held in a JSON field (Google `audioContent`, Mistral
/// `audio_data`).
fn decode_base64_audio(value: &serde_json::Value, field: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let encoded = value
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("The response had no `{field}` field"))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| format!("Couldn't decode the audio: {}", e))
}

/// Split `text` into pieces no longer than `max_chars` characters, preferring
/// sentence ends, then clause marks, then word gaps, and only as a last resort
/// a hard cut. Pieces are trimmed and never empty.
///
/// The streaming chunker already aims below an engine's limit, but it can emit
/// one long sentence whole, and the one-shot paths (a spoken summary, a replay)
/// never pass through it. An engine that rejects long input — Groq caps a
/// request at 200 characters — would otherwise fail on exactly those.
pub(crate) fn split_to_limit(text: &str, max_chars: usize) -> Vec<String> {
    const SENTENCE: &[char] = &['.', '!', '?', '…', '。', '！', '？'];
    const CLAUSE: &[char] = &[',', ';', ':', '，', '；', '：', '、'];

    let mut pieces = Vec::new();
    let mut rest = text.trim();
    if max_chars == 0 {
        if !rest.is_empty() {
            pieces.push(rest.to_string());
        }
        return pieces;
    }
    while rest.chars().count() > max_chars {
        // Byte offset just past the `max_chars`-th character.
        let limit = rest
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let window = &rest[..limit];
        // A mark only counts as a boundary when whitespace follows it, so
        // "3.5" or "e.g.x" is never split.
        let boundary_after = |marks: &[char]| -> Option<usize> {
            let mut found = None;
            for (i, c) in window.char_indices() {
                if !marks.contains(&c) {
                    continue;
                }
                let end = i + c.len_utf8();
                let next = rest[end..].chars().next();
                if next.is_none_or(char::is_whitespace) {
                    found = Some(end);
                }
            }
            found
        };
        let cut = boundary_after(SENTENCE)
            .or_else(|| boundary_after(CLAUSE))
            .or_else(|| {
                window
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map(|(i, _)| i)
            })
            .filter(|&cut| cut > 0)
            .unwrap_or(limit);
        let (head, tail) = rest.split_at(cut);
        let head = head.trim();
        if !head.is_empty() {
            pieces.push(head.to_string());
        }
        rest = tail.trim_start();
    }
    if !rest.is_empty() {
        pieces.push(rest.to_string());
    }
    pieces
}

/// A neural voice returned by the Azure Speech `voices/list` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AzureVoice {
    /// e.g. "en-US-JennyNeural" — this is what goes in the Voice name field.
    pub short_name: String,
    /// Friendly display name, e.g. "Jenny".
    pub local_name: String,
    /// e.g. "en-US".
    pub locale: String,
    /// "Male" / "Female".
    pub gender: String,
}

/// A voice option handed to the settings UI for any remote TTS engine, so the
/// user can pick from a loaded list instead of typing an opaque id.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TtsVoice {
    /// Value written to settings (OpenAI voice name / ElevenLabs voice_id /
    /// Azure short name).
    pub id: String,
    /// Friendly label for the picker.
    pub label: String,
}

/// Built-in OpenAI TTS voices (current Audio API set). Used as the fallback
/// voice list for the "openai" engine when the configured endpoint has no
/// `/audio/voices` listing (e.g. api.openai.com itself, which serves a fixed
/// set). Local OpenAI-compatible servers (Kokoro-FastAPI, openai-edge-tts) that
/// do expose `/audio/voices` return their own list instead.
const OPENAI_TTS_VOICES: &[&str] = &[
    "alloy", "ash", "ballad", "coral", "echo", "fable", "onyx", "nova", "sage", "shimmer", "verse",
    "marin", "cedar",
];

/// Monotonic playback epoch. Incremented whenever in-flight TTS should be
/// cancelled (e.g. the user disables voice summaries). A request or playback
/// tagged with an older epoch aborts instead of starting/continuing.
static PLAYBACK_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Snapshot the current epoch. Capture this before kicking off a TTS request
/// so a cancel that happens *during* generation still supersedes it.
pub fn current_epoch() -> u64 {
    PLAYBACK_EPOCH.load(Ordering::SeqCst)
}

/// Why the configured voice engine cannot speak, or `None` when it can.
///
/// A hands-free call forces spoken replies on regardless of the
/// `assistant_tts_enabled` switch, which means it also inherits a voice engine
/// the user configured but never finished setting up — and a remote engine with
/// no API key fails once per turn, forever, with a generic provider error. This
/// is the check that turns that into one clear message before the call starts.
///
/// The custom engine is exempt from the key check: a self-hosted speech server
/// legitimately needs no key, and refusing it would break a working setup. It
/// does need an address, which no default can supply.
pub fn voice_engine_blocker(settings: &AppSettings) -> Option<String> {
    let engine = settings.assistant_tts_engine.trim();
    // The in-webview engine needs nothing configured.
    if engine.is_empty() || engine == "kokoro" {
        return None;
    }
    let Some(provider) = provider(engine) else {
        return Some(format!(
            "The assistant's voice is set to an engine this version doesn't know ({engine}). Pick another in Models → Voice."
        ));
    };
    let label = provider.label;
    if provider.base_url.is_none() && settings.assistant_tts_base_url.trim().is_empty() {
        return Some(format!(
            "The assistant's voice is set to {label}, which needs a server address. Add it in Models → Voice, or switch the voice to Kokoro."
        ));
    }
    if provider.requires_key && settings.assistant_tts_api_key.0.trim().is_empty() {
        return Some(format!(
            "The assistant's voice is set to {label}, which needs an API key. Add one in Models → Voice, or switch the voice to Kokoro."
        ));
    }
    None
}

/// Cancel any in-flight or queued remote TTS: native playback stops within
/// ~50ms and any superseded request aborts before it can play.
pub fn stop_remote() {
    PLAYBACK_EPOCH.fetch_add(1, Ordering::SeqCst);
    // Release the streaming sink too. Its thread already stops on the epoch
    // change; dropping the sender here means queued chunks are discarded
    // immediately instead of waiting for the next reply to displace them.
    if let Ok(mut guard) = PLAYBACK_SESSION.lock() {
        *guard = None;
    }
}

/// Stop ALL assistant speech — remote playback and the in-webview Kokoro
/// engine. Used when a new recording starts so old speech never talks over
/// the user's next dictation or question.
pub fn stop_all(app: &AppHandle) {
    stop_remote();
    use tauri::Emitter;
    let _ = app.emit("assistant-tts-stop", ());
}

// ---------------------------------------------------------------------------
// Streaming playback: one output stream per spoken reply
// ---------------------------------------------------------------------------

/// How long the playback thread lingers with an empty sink before releasing the
/// audio device. Purely a safety net for a caller that never signals the end of
/// a reply (a closed panel, a failed synthesis); a later chunk simply opens a
/// fresh session. Generous because a slow local synthesis on CPU can leave a
/// real gap between sentences, and tearing down mid-reply would cost a
/// device-open right when the next chunk arrives.
const PLAYBACK_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Queue depth before [`enqueue_speech_chunk`] applies backpressure. Bounded so
/// a long reply cannot accumulate unbounded decoded audio in memory; synthesis
/// waits instead, which is harmless because playback is the slower side.
const PLAYBACK_QUEUE_DEPTH: usize = 16;

/// A live playback session: one output stream and one sink serving every chunk
/// of a single spoken reply.
///
/// Streamed speech arrives as a series of clips. Playing each one through
/// [`play_audio_bytes`] would open and close the audio device per sentence,
/// which costs latency and inserts an audible gap exactly where the sentences
/// should flow together. A rodio `Sink` plays appended sources back-to-back
/// without a break, so the sink is kept alive for the whole reply and chunks are
/// appended as they are synthesized.
struct PlaybackSession {
    /// Chunks are handed to the playback thread; `Sink`/`OutputStream` are not
    /// `Send`, so they never leave that thread.
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
    /// The cancellation epoch this session belongs to. A chunk from a newer
    /// epoch replaces the session; one from an older epoch is dropped.
    epoch: u64,
}

static PLAYBACK_SESSION: Lazy<std::sync::Mutex<Option<PlaybackSession>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

/// Number of playback sessions currently producing audio.
///
/// The panel's "audio is playing" flag is a single boolean, but sessions overlap:
/// a follow-up question starts its session while the previous one is still
/// winding down. Emitting `true`/`false` per session let a stale `false` land
/// after a fresh `true` and strand the panel showing silence during playback (or
/// the reverse). Counting instead means the event is emitted only on the real
/// 0↔1 transitions, so the flag always ends up matching reality.
static PLAYING_SESSIONS: AtomicUsize = AtomicUsize::new(0);

/// Announce that this session has begun producing audio, emitting the event only
/// if nothing else was already playing.
///
/// Returns a guard that balances the count on drop, so an early return — or a
/// panic in the playback thread — cannot leave the count above zero and strand
/// the panel showing a Stop button for audio that is not playing.
#[must_use = "the count is only balanced when the guard is dropped"]
fn announce_playing(app: &AppHandle) -> PlayingGuard {
    use tauri::Emitter;
    if PLAYING_SESSIONS.fetch_add(1, Ordering::SeqCst) == 0 {
        let _ = app.emit("assistant-tts-playing", true);
    }
    PlayingGuard { app: app.clone() }
}

/// Balances [`announce_playing`]; emits the event once the last session ends.
struct PlayingGuard {
    app: AppHandle,
}

impl Drop for PlayingGuard {
    fn drop(&mut self) {
        use tauri::Emitter;
        if PLAYING_SESSIONS.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _ = self.app.emit("assistant-tts-playing", false);
        }
    }
}

/// Queue one synthesized clip for gapless playback.
///
/// Chunks play strictly in the order they are enqueued, so callers must enqueue
/// in reading order. Returns once the clip is *queued*, not once it has been
/// heard — that is what lets synthesis of the next sentence overlap playback of
/// this one. Blocks only when playback has fallen [`PLAYBACK_QUEUE_DEPTH`] clips
/// behind, so callers must not hold a lock across it.
pub(crate) fn enqueue_speech_chunk(
    app: &AppHandle,
    bytes: Vec<u8>,
    device: Option<String>,
    volume: f32,
    epoch: u64,
) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    // Superseded by a Stop while this chunk was being synthesized.
    if current_epoch() != epoch {
        debug!("speech chunk superseded before playback; dropping");
        return Ok(());
    }

    // The sender is cloned out and the lock released before sending. Sending can
    // block when playback is far behind, and holding the lock across that would
    // make `stop_remote` wait on it — Stop has to stay immediate.
    let tx = session_sender(app, &device, volume, epoch)?;
    let payload = match tx.send(bytes) {
        Ok(()) => return Ok(()),
        // The thread retired on its idle timeout since the last chunk, so this
        // channel is dead. Start a fresh session and retry once.
        Err(std::sync::mpsc::SendError(payload)) => payload,
    };

    debug!("playback session had retired; starting a new one");
    // A send can also fail because a Stop dropped the receiver. Re-check before
    // respawning, or Stop would pointlessly reopen the audio device — audible as
    // a click on some hardware, and a profile switch on a Bluetooth headset.
    if current_epoch() != epoch {
        return Ok(());
    }
    if let Ok(mut guard) = PLAYBACK_SESSION.lock() {
        *guard = None;
    }
    let tx = session_sender(app, &device, volume, epoch)?;
    tx.send(payload)
        .map_err(|_| "playback thread stopped unexpectedly".to_string())
}

/// Get the sender for the current reply's playback session, starting one if
/// needed. Holds the session lock only long enough to look up or create it.
fn session_sender(
    app: &AppHandle,
    device: &Option<String>,
    volume: f32,
    epoch: u64,
) -> Result<std::sync::mpsc::SyncSender<Vec<u8>>, String> {
    let mut guard = PLAYBACK_SESSION
        .lock()
        .map_err(|_| "playback session lock poisoned".to_string())?;

    // A new reply supersedes the previous session's thread, which notices the
    // epoch change and stops within one poll interval.
    if guard.as_ref().is_some_and(|s| s.epoch != epoch) {
        *guard = None;
    }
    if guard.is_none() {
        *guard = Some(spawn_playback_session(
            app.clone(),
            device.clone(),
            volume,
            epoch,
        )?);
    }
    Ok(guard.as_ref().expect("session just created").tx.clone())
}

/// Signal that no further chunks belong to the current reply, so the sink can
/// drain and release the audio device.
///
/// Safe to call more than once, and safe to call for an epoch that has already
/// been superseded — both are no-ops.
pub(crate) fn finish_speech_stream(epoch: u64) {
    if let Ok(mut guard) = PLAYBACK_SESSION.lock() {
        if guard.as_ref().is_some_and(|s| s.epoch == epoch) {
            // Dropping the sender closes the channel; the thread then plays out
            // whatever is queued and exits.
            *guard = None;
        }
    }
}

/// Start the dedicated playback thread for one reply.
fn spawn_playback_session(
    app: AppHandle,
    device: Option<String>,
    volume: f32,
    epoch: u64,
) -> Result<PlaybackSession, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(PLAYBACK_QUEUE_DEPTH);
    // A plain OS thread, not a runtime worker: this thread owns non-`Send` audio
    // handles and blocks for the length of the reply.
    std::thread::Builder::new()
        .name("assistant-speech".into())
        .spawn(move || run_playback_session(app, rx, device, volume, epoch))
        .map_err(|e| format!("Failed to start speech playback thread: {}", e))?;
    Ok(PlaybackSession { tx, epoch })
}

/// Playback thread body: append every chunk to a single sink, then drain.
fn run_playback_session(
    app: AppHandle,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    device: Option<String>,
    volume: f32,
    epoch: u64,
) {
    use std::sync::mpsc::RecvTimeoutError;

    let stream_handle = match open_output_stream(device) {
        Ok(handle) => handle,
        Err(e) => {
            error!("TTS playback device unavailable: {}", e);
            if current_epoch() == epoch {
                crate::assistant::emit_error(
                    &app,
                    "tts",
                    format!("Couldn't play the voice on your output device: {}", e),
                );
            }
            return;
        }
    };
    let sink = rodio::Sink::connect_new(stream_handle.mixer());
    // No floor: this is the assistant's own voice volume, so 0 means silent.
    // The old `.max(0.1)` existed because the value came from the feedback-sound
    // slider, where 0 meant "no beeps" rather than "no spoken replies".
    sink.set_volume(volume.clamp(0.0, 1.0));

    // "Playing" is announced when audio actually starts, not when the device
    // opens, so a session that is superseded before its first chunk never makes
    // the panel flash a Stop affordance for silence. The guard balances the
    // announcement however this thread exits.
    let mut playing: Option<PlayingGuard> = None;
    let mut idle_since: Option<std::time::Instant> = None;
    let mut decode_failures = 0usize;

    loop {
        if current_epoch() != epoch {
            sink.stop();
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => {
                idle_since = None;
                match rodio::Decoder::new(Cursor::new(bytes)) {
                    Ok(source) => {
                        sink.append(source);
                        if playing.is_none() {
                            playing = Some(announce_playing(&app));
                        }
                    }
                    Err(e) => {
                        // Skip the bad clip rather than abandoning the reply; one
                        // truncated response shouldn't silence the rest.
                        decode_failures += 1;
                        error!("Failed to decode speech chunk: {}", e);
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if sink.empty() {
                    let since = idle_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= PLAYBACK_IDLE_TIMEOUT {
                        debug!("speech playback idle; releasing the output device");
                        break;
                    }
                } else {
                    idle_since = None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // The reply is complete: play out what is queued, still honouring
                // a Stop.
                while !sink.empty() {
                    if current_epoch() != epoch {
                        sink.stop();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                break;
            }
        }
    }

    // Dropping the guard emits the balancing event once no session is left
    // playing, on every exit path including an unexpected panic.
    drop(playing);
    if decode_failures > 0 && current_epoch() == epoch {
        crate::assistant::emit_error(
            &app,
            "tts",
            "Part of the spoken reply couldn't be played.".to_string(),
        );
    }
}

/// One synthesis request, plus the context needed to make it sound like a
/// continuation rather than a fresh utterance.
pub(crate) struct SpeechRequest<'a> {
    pub text: &'a str,
    /// ElevenLabs request ids from earlier chunks of the same reply, oldest
    /// first. Empty for a one-shot request.
    pub previous_request_ids: &'a [String],
}

/// Audio for one chunk, plus anything later chunks need from it.
pub(crate) struct SynthesizedSpeech {
    pub bytes: Vec<u8>,
    /// ElevenLabs `request-id`, carried into the next chunk so the voice keeps
    /// its prosody across the seam. `None` for every other engine.
    pub request_id: Option<String>,
}

/// Synthesize one piece of speech with the configured engine.
///
/// This is the single place that maps the engine setting onto a provider call,
/// used both for whole replies and for individual streamed chunks.
pub(crate) async fn synthesize_speech(
    settings: &AppSettings,
    request: SpeechRequest<'_>,
) -> Result<SynthesizedSpeech, String> {
    let provider = active_provider(settings)?;
    let plain = |bytes: Vec<u8>| SynthesizedSpeech {
        bytes,
        request_id: None,
    };
    let text = request.text;
    match provider.protocol {
        TtsProtocol::Local => {
            Err("Kokoro speaks inside the assistant panel, not through this path".to_string())
        }
        TtsProtocol::OpenAiCompatible => fetch_openai_speech(settings, provider, text)
            .await
            .map(plain),
        TtsProtocol::ElevenLabs => fetch_elevenlabs_speech(settings, &request).await,
        TtsProtocol::AzureSpeech => fetch_azure_speech(settings, text).await.map(plain),
        TtsProtocol::Deepgram => fetch_deepgram_speech(settings, provider, text)
            .await
            .map(plain),
        TtsProtocol::Cartesia => fetch_cartesia_speech(settings, provider, text)
            .await
            .map(plain),
        TtsProtocol::GoogleCloud => fetch_google_speech(settings, provider, text)
            .await
            .map(plain),
        TtsProtocol::Xai => fetch_xai_speech(settings, provider, text).await.map(plain),
        TtsProtocol::Mistral => fetch_mistral_speech(settings, provider, text)
            .await
            .map(plain),
    }
}

/// Fetch speech audio for `text` using the configured remote engine and play
/// it on the selected output device. Returns after playback finishes.
pub async fn speak_remote(app: &AppHandle, settings: &AppSettings, text: String) {
    speak_remote_epoch(app, settings, text, current_epoch()).await;
}

/// Like [`speak_remote`] but tagged with a caller-captured epoch, so a cancel
/// that occurred while the spoken summary was still being generated also
/// suppresses playback.
pub async fn speak_remote_epoch(app: &AppHandle, settings: &AppSettings, text: String, epoch: u64) {
    // Superseded before we even started (e.g. disabled during generation).
    if current_epoch() != epoch {
        debug!("TTS request superseded before fetch; skipping");
        return;
    }

    // Longer than one request may carry: speak it as a gapless series instead
    // of letting the provider reject the whole thing.
    let pieces = split_to_limit(&text, max_chars_for(settings));
    if pieces.len() > 1 {
        speak_pieces(app, settings, pieces, epoch).await;
        return;
    }
    let Some(text) = pieces.into_iter().next() else {
        return;
    };

    let result = synthesize_speech(
        settings,
        SpeechRequest {
            text: &text,
            previous_request_ids: &[],
        },
    )
    .await
    .map(|speech| speech.bytes);

    match result {
        Ok(audio_bytes) => {
            // Cancelled while the audio was being fetched?
            if current_epoch() != epoch {
                debug!("TTS request superseded during fetch; not playing");
                return;
            }
            debug!("TTS audio fetched: {} KB", audio_bytes.len() / 1024);
            let volume = settings.assistant_tts_volume;
            let device = settings.selected_output_device.clone();
            // Let the panel know audio is playing so it can show a Stop button
            // even though the turn itself is already idle. Counted, so a
            // one-shot clip and a streamed reply can't leave the flag wrong.
            let playing = announce_playing(app);
            // rodio playback blocks; run it off the async runtime. Map the
            // error to a String so it can cross the spawn_blocking boundary
            // (the boxed error isn't Send).
            let play_result = tauri::async_runtime::spawn_blocking(move || {
                play_audio_bytes(audio_bytes, device, volume, epoch).map_err(|e| e.to_string())
            })
            .await;
            drop(playing); // Surface a real playback failure (bad/removed output device, decode
                           // error) instead of failing silently — but stay quiet when the clip
                           // was simply superseded by a Stop (which returns Ok, not Err).
            if let Ok(Err(e)) = play_result {
                error!("TTS playback failed: {}", e);
                if current_epoch() == epoch {
                    crate::assistant::emit_error(
                        app,
                        "tts",
                        format!("Couldn't play the voice on your output device: {}", e),
                    );
                }
            }
        }
        Err(e) => {
            error!("TTS request failed: {}", e);
            crate::assistant::emit_error(app, "tts", e);
        }
    }
}

/// Speak several pieces of one reply back to back through a single playback
/// session, so the seams between them are gapless. Synthesis of the next piece
/// overlaps playback of the current one, because queueing returns before the
/// audio is heard.
async fn speak_pieces(app: &AppHandle, settings: &AppSettings, pieces: Vec<String>, epoch: u64) {
    let device = settings.selected_output_device.clone();
    let volume = settings.assistant_tts_volume;
    let mut request_ids: Vec<String> = Vec::new();
    for piece in pieces {
        if current_epoch() != epoch {
            break;
        }
        let speech = match synthesize_speech(
            settings,
            SpeechRequest {
                text: &piece,
                previous_request_ids: &request_ids,
            },
        )
        .await
        {
            Ok(speech) => speech,
            Err(e) => {
                error!("TTS request failed: {}", e);
                if current_epoch() == epoch {
                    crate::assistant::emit_error(app, "tts", e);
                }
                break;
            }
        };
        if let Some(id) = speech.request_id {
            request_ids.push(id);
        }
        let app_play = app.clone();
        let device = device.clone();
        let queued = tauri::async_runtime::spawn_blocking(move || {
            enqueue_speech_chunk(&app_play, speech.bytes, device, volume, epoch)
        })
        .await;
        if let Ok(Err(e)) | Err(e) = queued.map_err(|e| e.to_string()) {
            error!("TTS playback failed: {}", e);
            break;
        }
    }
    finish_speech_stream(epoch);
}

/// HTTP client for remote TTS. Forces HTTP/1.1 — some hosted TTS gateways/
/// proxies emit "upstream connect error / reset before headers / protocol
/// error" during HTTP/2 negotiation (the same reason the LLM client pins h1) —
/// and sets connect/overall timeouts so a stalled upstream can't wedge playback.
///
/// Built once and shared. A `reqwest::Client` owns its connection pool, so
/// building one per request re-paid DNS + TCP + TLS every time; that was
/// tolerable when a reply was a single request, but streaming speech sends one
/// request per sentence, where a fresh handshake per chunk would show up as
/// audible stalling. Clones are cheap and share the pool.
static TTS_CLIENT: Lazy<Result<reqwest::Client, String>> = Lazy::new(|| {
    reqwest::Client::builder()
        .http1_only()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        // Hold sockets open between sentences so only the first chunk of a reply
        // pays for connection setup.
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| format!("Failed to build TTS HTTP client: {}", e))
});

fn tts_client() -> Result<reqwest::Client, String> {
    TTS_CLIENT.clone()
}

/// Send a TTS request with a few retries. Transient upstream hiccups — 5xx
/// gateway errors (502/503/504) and connection resets / protocol errors — are
/// common with hosted TTS proxies and usually clear on a quick retry, so a
/// one-off blip no longer surfaces as a hard "TTS failed" to the user. Up to 3
/// attempts with a short linear backoff; anything else returns immediately.
async fn send_tts_with_retries(
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut attempt = 0;
    loop {
        attempt += 1;
        // Clone so the request can be replayed; a non-cloneable body (none of
        // ours) falls back to a single send.
        let Some(try_req) = request.try_clone() else {
            return request
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {}", e));
        };
        match try_req.send().await {
            Ok(resp) => {
                if resp.status().is_server_error() && attempt < MAX_ATTEMPTS {
                    debug!(
                        "TTS upstream {} (attempt {}/{}); retrying",
                        resp.status(),
                        attempt,
                        MAX_ATTEMPTS
                    );
                    tokio::time::sleep(Duration::from_millis(300 * attempt as u64)).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                if attempt < MAX_ATTEMPTS {
                    debug!(
                        "TTS request error (attempt {}/{}): {}; retrying",
                        attempt, MAX_ATTEMPTS, e
                    );
                    tokio::time::sleep(Duration::from_millis(300 * attempt as u64)).await;
                    continue;
                }
                return Err(format!("HTTP request failed: {}", e));
            }
        }
    }
}

/// Default PCM parameters for OpenAI-compatible `pcm` output. Both OpenAI's TTS
/// and Gemini TTS (via OpenRouter) emit signed 16-bit little-endian mono at
/// 24 kHz, so these are safe defaults when the response omits an explicit rate.
const PCM_DEFAULT_SAMPLE_RATE: u32 = 24_000;
const PCM_DEFAULT_CHANNELS: u16 = 1;
const PCM_BITS_PER_SAMPLE: u16 = 16;

/// Outcome of a single `/audio/speech` attempt: either the audio (with its
/// reported `Content-Type`) or a non-success HTTP status plus body. HTTP errors
/// are kept separate from transport errors so the caller can inspect the body
/// and decide whether to retry with a different `response_format`.
enum SpeechAttempt {
    Ok {
        content_type: String,
        bytes: Vec<u8>,
    },
    HttpError {
        status: reqwest::StatusCode,
        body: String,
    },
}

/// Perform one `/audio/speech` POST for a specific `response_format`. Transport
/// failures return `Err`; a non-2xx response returns `Ok(SpeechAttempt::HttpError)`
/// so the caller can look at the body (e.g. to detect a pcm-only model).
async fn openai_speech_attempt(
    settings: &AppSettings,
    provider: &TtsProvider,
    text: &str,
    url: &str,
    response_format: &str,
) -> Result<SpeechAttempt, String> {
    let client = tts_client()?;
    // The model/voice fields start empty (they're loadable pickers). Fall back to
    // the engine's own defaults so synthesis works out of the box without
    // forcing a pre-filled value into the settings UI.
    let model = or_default(&settings.assistant_tts_model, provider.default_model);
    let voice = or_default(&settings.assistant_tts_remote_voice, provider.default_voice);
    let mut body = serde_json::json!({
        "model": model,
        "input": text,
        "voice": voice,
        "response_format": response_format,
    });
    // Sent only when it differs from normal, and never to an engine without
    // speed control (Groq's Orpheus rejects unknown fields it doesn't document
    // less predictably than it ignores their absence).
    if let Some(speed) = speed_for(provider, settings) {
        body["speed"] = serde_json::json!(speed);
    }
    let request = with_openai_auth(
        client.post(url).json(&body),
        provider,
        settings.assistant_tts_api_key.0.trim(),
    );

    let response = send_tts_with_retries(request).await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Ok(SpeechAttempt::HttpError { status, body });
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read audio: {}", e))?;
    Ok(SpeechAttempt::Ok {
        content_type,
        bytes,
    })
}

/// Attach an OpenAI-compatible engine's key.
///
/// The custom engine also gets the `api-key` header: that is how classic Azure
/// OpenAI deployment endpoints authenticate, and a server that uses Bearer
/// ignores it.
fn with_openai_auth(
    request: reqwest::RequestBuilder,
    provider: &TtsProvider,
    api_key: &str,
) -> reqwest::RequestBuilder {
    if api_key.is_empty() {
        return request;
    }
    match provider.auth {
        TtsAuth::Basic => {
            request.header(reqwest::header::AUTHORIZATION, format!("Basic {api_key}"))
        }
        TtsAuth::Bearer if provider.id == "custom" => {
            request.bearer_auth(api_key).header("api-key", api_key)
        }
        TtsAuth::Bearer => request.bearer_auth(api_key),
    }
}

/// The `/audio/speech` URL for an OpenAI-compatible root.
///
/// If the configured address already contains `/audio/speech`, it is used
/// verbatim (matching SillyTavern's "Provider Endpoint" behaviour). This lets
/// users paste a full Azure endpoint such as
/// `https://{res}.cognitiveservices.azure.com/openai/deployments/{dep}/audio/speech?api-version=2025-03-01-preview`,
/// including the `?api-version=` query string, which a base-plus-suffix scheme
/// cannot express.
fn openai_speech_url(root: &str) -> String {
    if root.contains("/audio/speech") {
        root.to_string()
    } else {
        format!("{}/audio/speech", root.trim_end_matches('/'))
    }
}

/// The API root of an OpenAI-compatible address that may point straight at
/// `/audio/speech` (with a query string), for deriving sibling routes.
fn openai_root(root: &str) -> String {
    let trimmed = root.trim_end_matches('/');
    match trimmed.split_once("/audio/speech") {
        Some((prefix, _)) => prefix.trim_end_matches('/').to_string(),
        None => trimmed.to_string(),
    }
}

/// Endpoint + model pairs already known to reject `mp3` and accept only `pcm`.
///
/// Discovering that costs a rejected request, and streamed speech sends one
/// request per sentence, so without this every sentence of a Gemini-TTS reply
/// paid a full extra round trip. Process-lifetime is the right scope: the
/// answer only changes if the provider changes the model.
static PCM_ONLY: Lazy<std::sync::Mutex<HashSet<String>>> =
    Lazy::new(|| std::sync::Mutex::new(HashSet::new()));

/// POST {base}/audio/speech — OpenAI-compatible shape.
///
/// Requests the engine's preferred format first (mp3 for most, wav for Groq,
/// which accepts nothing else). Some models are pcm-only — notably Gemini TTS
/// via OpenRouter, which rejects mp3 with a 400 — so on that specific error we
/// transparently retry as `pcm` and wrap the raw samples in a WAV container
/// (see [`pcm_to_wav`]) so playback still works. This self-heals for any
/// pcm-only model without a hardcoded model list.
async fn fetch_openai_speech(
    settings: &AppSettings,
    provider: &TtsProvider,
    text: &str,
) -> Result<Vec<u8>, String> {
    if provider.requires_key && settings.assistant_tts_api_key.0.trim().is_empty() {
        return Err(format!(
            "{} API key is required for voice output",
            provider.label
        ));
    }

    let url = openai_speech_url(&api_root(provider, settings)?);
    let model = or_default(&settings.assistant_tts_model, provider.default_model);
    let pcm_key = format!("{url}|{model}");
    let known_pcm_only = PCM_ONLY
        .lock()
        .map(|set| set.contains(&pcm_key))
        .unwrap_or(false);
    let first_format = if known_pcm_only {
        "pcm"
    } else {
        provider.response_format
    };

    match openai_speech_attempt(settings, provider, text, &url, first_format).await? {
        SpeechAttempt::Ok {
            content_type,
            bytes,
        } => Ok(if first_format == "pcm" {
            wrap_requested_pcm(&content_type, bytes)
        } else {
            maybe_wrap_pcm(&content_type, bytes)
        }),
        SpeechAttempt::HttpError { status, body } => {
            // Only retry as pcm for the specific "this model needs pcm" 400 so
            // unrelated 4xx/5xx errors (bad key, missing model, gateway) still
            // surface to the user immediately instead of doubling the latency.
            let pcm_only = first_format != "pcm"
                && status == reqwest::StatusCode::BAD_REQUEST
                && body.to_lowercase().contains("pcm");
            if !pcm_only {
                return Err(http_error(status, &body));
            }
            debug!(
                "TTS model rejected {first_format} (pcm-only); retrying as pcm and wrapping to WAV"
            );
            match openai_speech_attempt(settings, provider, text, &url, "pcm").await? {
                SpeechAttempt::Ok {
                    content_type,
                    bytes,
                } => {
                    if let Ok(mut set) = PCM_ONLY.lock() {
                        set.insert(pcm_key);
                    }
                    Ok(wrap_requested_pcm(&content_type, bytes))
                }
                SpeechAttempt::HttpError { status, body } => Err(http_error(status, &body)),
            }
        }
    }
}

/// Audio from a request that explicitly asked for `pcm`. The bytes are raw
/// samples even if the endpoint omits a Content-Type — unless they carry a
/// container header after all, which some servers send regardless of the
/// requested format.
fn wrap_requested_pcm(content_type: &str, bytes: Vec<u8>) -> Vec<u8> {
    if sniff_container(&bytes).is_some() {
        return bytes;
    }
    let sample_rate = parse_pcm_rate(content_type).unwrap_or(PCM_DEFAULT_SAMPLE_RATE);
    pcm_to_wav(
        &bytes,
        sample_rate,
        PCM_DEFAULT_CHANNELS,
        PCM_BITS_PER_SAMPLE,
    )
}

/// The container format of an audio payload, read from its first bytes, or
/// `None` when it has no recognisable header (i.e. is raw PCM).
///
/// The Content-Type cannot be trusted for this: openai-edge-tts without ffmpeg
/// answers every format request with MP3 while labelling it as the format that
/// was asked for, and Chatterbox and Orpheus servers always answer in WAV.
fn sniff_container(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"RIFF") {
        Some("wav")
    } else if bytes.starts_with(b"ID3")
        || (bytes.len() > 1 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        Some("mp3")
    } else if bytes.starts_with(b"OggS") {
        Some("ogg")
    } else if bytes.starts_with(b"fLaC") {
        Some("flac")
    } else {
        None
    }
}

/// Wrap raw PCM in a WAV container when the audio has no decodable header;
/// otherwise pass it through unchanged. rodio decodes container formats
/// (mp3/wav/ogg/flac) but not headerless PCM, so any `audio/pcm` / `audio/L16`
/// response is wrapped while mp3 and everything else is returned as-is.
fn maybe_wrap_pcm(content_type: &str, bytes: Vec<u8>) -> Vec<u8> {
    let ct = content_type.to_ascii_lowercase();
    let is_pcm = ct.contains("audio/pcm") || ct.contains("audio/l16") || ct.contains("codec=pcm");
    if !is_pcm || sniff_container(&bytes).is_some() {
        return bytes;
    }
    let sample_rate = parse_pcm_rate(&ct).unwrap_or(PCM_DEFAULT_SAMPLE_RATE);
    pcm_to_wav(
        &bytes,
        sample_rate,
        PCM_DEFAULT_CHANNELS,
        PCM_BITS_PER_SAMPLE,
    )
}

/// Parse a sample rate from a PCM `Content-Type` such as `audio/L16;rate=24000`
/// or `audio/pcm;rate=16000`. Returns `None` when no `rate=` parameter present.
fn parse_pcm_rate(content_type: &str) -> Option<u32> {
    let lower = content_type.to_ascii_lowercase();
    lower
        .split(';')
        .map(|part| part.trim())
        .find_map(|part| part.strip_prefix("rate="))
        .and_then(|r| r.trim().parse::<u32>().ok())
}

/// Wrap raw little-endian PCM samples in a canonical 44-byte WAV header so a
/// container-based decoder (rodio) can play them. Assumes `bits_per_sample` is
/// a multiple of 8 (16 for all current OpenAI-compatible pcm output).
fn pcm_to_wav(pcm: &[u8], sample_rate: u32, channels: u16, bits_per_sample: u16) -> Vec<u8> {
    let bytes_per_sample = (bits_per_sample / 8) as u32;
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample;
    let block_align = channels * (bits_per_sample / 8);
    let data_len = pcm.len() as u32;
    let riff_len = 36u32.saturating_add(data_len);

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    // "fmt " subchunk (PCM).
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // subchunk size for PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format = 1 (PCM)
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    // "data" subchunk.
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Most recent ElevenLabs request ids to condition a chunk on. ElevenLabs
/// accepts at most three, and only ids from the last two hours.
const ELEVENLABS_MAX_STITCH_IDS: usize = 3;

/// Whether this ElevenLabs model supports Request Stitching. The `eleven_v3`
/// family does not, and sending the field there is pointless, so it is skipped.
fn elevenlabs_supports_stitching(model: &str) -> bool {
    !model.to_ascii_lowercase().contains("v3")
}

/// POST https://api.elevenlabs.io/v1/text-to-speech/{voice_id}
///
/// When earlier chunks of the same reply are supplied via
/// [`SpeechRequest::previous_request_ids`], ElevenLabs' Request Stitching is
/// used: the model conditions on what it already generated, so a reply split
/// into sentences keeps one continuous voice and intonation instead of restarting
/// its delivery at every seam. This is the main defence against streamed speech
/// sounding choppier than a single-shot reply.
async fn fetch_elevenlabs_speech(
    settings: &AppSettings,
    request: &SpeechRequest<'_>,
) -> Result<SynthesizedSpeech, String> {
    let text = request.text;
    let voice_id = settings.assistant_tts_remote_voice.trim();
    if voice_id.is_empty() {
        return Err("No ElevenLabs voice ID configured".to_string());
    }
    let url = format!(
        "https://api.elevenlabs.io/v1/text-to-speech/{}?output_format=mp3_44100_64",
        voice_id
    );

    let model = if settings.assistant_tts_model.trim().is_empty()
        || settings.assistant_tts_model == "gpt-4o-mini-tts"
    {
        // Sensible default when the user hasn't set an ElevenLabs model.
        "eleven_flash_v2_5".to_string()
    } else {
        settings.assistant_tts_model.clone()
    };

    let client = tts_client()?;
    let mut body = serde_json::json!({
        "text": text,
        "model_id": model,
    });
    // ElevenLabs exposes speed inside `voice_settings`, limited to 0.7x–1.2x.
    // Only send it when the user actually changed the rate so the voice's own
    // saved settings (stability, similarity) are otherwise left untouched.
    let speed = settings.assistant_tts_speed.clamp(0.7, 1.2);
    if (speed - 1.0).abs() > f64::EPSILON {
        body["voice_settings"] = serde_json::json!({ "speed": speed });
    }
    if !request.previous_request_ids.is_empty() && elevenlabs_supports_stitching(&model) {
        let ids: Vec<&String> = request
            .previous_request_ids
            .iter()
            .rev()
            .take(ELEVENLABS_MAX_STITCH_IDS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        body["previous_request_ids"] = serde_json::json!(ids);
    }
    let request_builder = client
        .post(&url)
        .header("xi-api-key", settings.assistant_tts_api_key.0.trim())
        .json(&body);
    let response = send_tts_with_retries(request_builder).await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("{}: {}", status, truncate(&body, 300)));
    }

    // Captured before the body is consumed; conditioning the next chunk needs it.
    let request_id = response
        .headers()
        .get("request-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
        .filter(|v| !v.is_empty());

    let bytes = response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read audio: {}", e))?;
    Ok(SynthesizedSpeech { bytes, request_id })
}

/// The key for an engine that cannot work without one, or a clear error.
fn required_key<'a>(settings: &'a AppSettings, provider: &TtsProvider) -> Result<&'a str, String> {
    let key = settings.assistant_tts_api_key.0.trim();
    if key.is_empty() {
        return Err(format!("No {} API key configured", provider.label));
    }
    Ok(key)
}

/// The locale a voice name starts with — `en-US` from `en-US-JennyNeural` or
/// `en-US-Chirp3-HD-Kore` — falling back to `en-US`. Azure and Google both
/// name voices this way, and both need the locale sent alongside the voice.
fn voice_locale(voice: &str) -> String {
    let prefix: Vec<&str> = voice.splitn(3, '-').take(2).collect();
    if prefix.len() == 2 && !prefix[0].is_empty() && !prefix[1].is_empty() {
        format!("{}-{}", prefix[0], prefix[1])
    } else {
        "en-US".to_string()
    }
}

/// POST https://api.deepgram.com/v1/speak?model=…
///
/// Deepgram folds the voice into the model id (`aura-2-thalia-en`), so the
/// "voice" the user picks *is* the model, and everything except the text rides
/// in the query string. Flux voices live on the newer `/v2/speak` route.
async fn fetch_deepgram_speech(
    settings: &AppSettings,
    provider: &TtsProvider,
    text: &str,
) -> Result<Vec<u8>, String> {
    let key = required_key(settings, provider)?;
    let voice = or_default(
        &settings.assistant_tts_remote_voice,
        or_default(&settings.assistant_tts_model, provider.default_voice),
    );
    let route = if voice.starts_with("flux-") {
        "/v2/speak"
    } else {
        "/v1/speak"
    };
    let mut url = reqwest::Url::parse(&format!("{}{route}", api_root(provider, settings)?))
        .map_err(|e| format!("Invalid Deepgram URL: {e}"))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("model", voice);
        // The docs disagree on the default encoding, so ask for mp3 explicitly.
        query.append_pair("encoding", "mp3");
        if let Some(speed) = speed_for(provider, settings) {
            query.append_pair("speed", &format!("{speed:.2}"));
        }
    }
    let request = tts_client()?
        .post(url)
        .header(reqwest::header::AUTHORIZATION, format!("Token {key}"))
        .json(&serde_json::json!({ "text": text }));
    audio_body(send_tts_with_retries(request).await?).await
}

/// POST https://api.cartesia.ai/tts/bytes
///
/// Asks for 16-bit WAV: it is the one output shape the current API reference
/// documents unambiguously, and rodio plays it without transcoding.
async fn fetch_cartesia_speech(
    settings: &AppSettings,
    provider: &TtsProvider,
    text: &str,
) -> Result<Vec<u8>, String> {
    let key = required_key(settings, provider)?;
    let mut body = serde_json::json!({
        "model_id": or_default(&settings.assistant_tts_model, provider.default_model),
        "transcript": text,
        "voice": or_default(&settings.assistant_tts_remote_voice, provider.default_voice),
        "output_format": {
            "container": "wav",
            "encoding": "pcm_s16le",
            "sample_rate": 44100,
        },
    });
    if let Some(speed) = speed_for(provider, settings) {
        body["generation_config"] = serde_json::json!({ "speed": speed });
    }
    let request = tts_client()?
        .post(format!("{}/tts/bytes", api_root(provider, settings)?))
        .bearer_auth(key)
        .header("Cartesia-Version", CARTESIA_VERSION)
        .json(&body);
    audio_body(send_tts_with_retries(request).await?).await
}

/// POST https://texttospeech.googleapis.com/v1/text:synthesize
///
/// Authenticated with a plain API key in `x-goog-api-key` (the key's project
/// needs the Text-to-Speech API enabled). `LINEAR16` comes back with a WAV
/// header, which is higher quality than Google's fixed 32 kbps MP3.
async fn fetch_google_speech(
    settings: &AppSettings,
    provider: &TtsProvider,
    text: &str,
) -> Result<Vec<u8>, String> {
    let key = required_key(settings, provider)?;
    let voice = or_default(&settings.assistant_tts_remote_voice, provider.default_voice);
    let mut audio_config = serde_json::json!({ "audioEncoding": "LINEAR16" });
    if let Some(speed) = speed_for(provider, settings) {
        audio_config["speakingRate"] = serde_json::json!(speed);
    }
    let request = tts_client()?
        .post(format!(
            "{}/v1/text:synthesize",
            api_root(provider, settings)?
        ))
        .header("x-goog-api-key", key)
        .json(&serde_json::json!({
            "input": { "text": text },
            "voice": { "languageCode": voice_locale(voice), "name": voice },
            "audioConfig": audio_config,
        }));
    let value = json_body(send_tts_with_retries(request).await?).await?;
    decode_base64_audio(&value, "audioContent")
}

/// POST https://api.x.ai/v1/tts
///
/// `language` is required; `auto` lets the model detect it, which matches the
/// assistant replying in whatever language the user spoke.
async fn fetch_xai_speech(
    settings: &AppSettings,
    provider: &TtsProvider,
    text: &str,
) -> Result<Vec<u8>, String> {
    let key = required_key(settings, provider)?;
    let mut body = serde_json::json!({
        "text": text,
        "voice_id": or_default(&settings.assistant_tts_remote_voice, provider.default_voice),
        "language": "auto",
        "output_format": { "codec": "mp3", "sample_rate": 24000, "bit_rate": 128000 },
    });
    if let Some(speed) = speed_for(provider, settings) {
        body["speed"] = serde_json::json!(speed);
    }
    let request = tts_client()?
        .post(format!("{}/tts", api_root(provider, settings)?))
        .bearer_auth(key)
        .json(&body);
    audio_body(send_tts_with_retries(request).await?).await
}

/// POST https://api.mistral.ai/v1/audio/speech
///
/// Same path as OpenAI's, different contract: the voice is `voice_id`, there is
/// no speed, and the audio comes back base64-encoded inside JSON.
async fn fetch_mistral_speech(
    settings: &AppSettings,
    provider: &TtsProvider,
    text: &str,
) -> Result<Vec<u8>, String> {
    let key = required_key(settings, provider)?;
    let voice = settings.assistant_tts_remote_voice.trim();
    if voice.is_empty() {
        return Err(
            "No Mistral voice chosen. Press Load voices in the Voice settings and pick one."
                .to_string(),
        );
    }
    let request = tts_client()?
        .post(format!("{}/audio/speech", api_root(provider, settings)?))
        .bearer_auth(key)
        .json(&serde_json::json!({
            "model": or_default(&settings.assistant_tts_model, provider.default_model),
            "input": text,
            "voice_id": voice,
            "response_format": "mp3",
        }));
    let value = json_body(send_tts_with_retries(request).await?).await?;
    decode_base64_audio(&value, "audio_data")
}

/// Resolve the Azure Speech regional TTS host from a user-provided endpoint.
///
/// Azure synthesis and the voices list live on `{region}.tts.speech.microsoft.com`,
/// but the portal "Endpoint" field shows `{region}.api.cognitive.microsoft.com`.
/// We accept either (and the tts host directly) and normalize to the tts host.
fn azure_tts_host(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = without_scheme.split('/').next().unwrap_or(without_scheme);

    if host.is_empty() {
        return trimmed.to_string();
    }
    if host.ends_with(".tts.speech.microsoft.com") {
        return format!("https://{}", host);
    }
    // `{region}.api.cognitive.microsoft.com` → region is the first label.
    if host.ends_with(".api.cognitive.microsoft.com") {
        if let Some(region) = host.split('.').next() {
            if !region.is_empty() {
                return format!("https://{}.tts.speech.microsoft.com", region);
            }
        }
    }
    // Unknown form (e.g. a custom cognitiveservices.azure.com domain): use the
    // host as given and let any error surface to the user.
    format!("https://{}", host)
}

/// True when the resolved Azure host is a regional Speech host
/// (`{region}.tts.speech.microsoft.com` / `.azure.us`). Regional hosts use the
/// un-prefixed `/cognitiveservices/...` paths; custom-domain resources
/// (`{res}.cognitiveservices.azure.com`, AI Foundry `services.ai.azure.com`)
/// use the `/tts/`-prefixed voices path instead.
fn azure_is_regional_host(host_url: &str) -> bool {
    host_url.ends_with(".tts.speech.microsoft.com") || host_url.ends_with(".tts.speech.azure.us")
}

/// Build the Azure `voices/list` URL for a configured endpoint, choosing the
/// right path prefix for the resolved host type.
fn azure_voices_url(raw: &str) -> String {
    let host = azure_tts_host(raw);
    if azure_is_regional_host(&host) {
        format!("{}/cognitiveservices/voices/list", host)
    } else {
        format!("{}/tts/cognitiveservices/voices/list", host)
    }
}

/// GET {host}/cognitiveservices/voices/list — all neural voices available to
/// the configured Azure Speech resource. Errors are returned for display.
pub async fn list_azure_voices(settings: &AppSettings) -> Result<Vec<AzureVoice>, String> {
    if settings.assistant_tts_base_url.trim().is_empty() {
        return Err(
            "No Azure Speech endpoint configured. Set the TTS Base URL first, \
             e.g. https://eastus.tts.speech.microsoft.com"
                .to_string(),
        );
    }
    let api_key = settings.assistant_tts_api_key.0.trim();
    if api_key.is_empty() {
        return Err("No Azure Speech API key configured".to_string());
    }
    let url = azure_voices_url(&settings.assistant_tts_base_url);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Ocp-Apim-Subscription-Key", api_key)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("{}: {}", status, truncate(&body, 300)));
    }

    let raw: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse voices list: {}", e))?;

    let mut voices: Vec<AzureVoice> = raw
        .into_iter()
        .filter_map(|v| {
            let short_name = v.get("ShortName")?.as_str()?.to_string();
            let local_name = v
                .get("LocalName")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let locale = v
                .get("Locale")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let gender = v
                .get("Gender")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            Some(AzureVoice {
                short_name,
                local_name,
                locale,
                gender,
            })
        })
        .collect();

    // Group by locale, then by name, for a predictable picker order.
    voices.sort_by(|a, b| {
        a.locale
            .cmp(&b.locale)
            .then_with(|| a.short_name.cmp(&b.short_name))
    });

    if voices.is_empty() {
        return Err("The endpoint returned no voices".to_string());
    }
    Ok(voices)
}

// ---------------------------------------------------------------------------
// Remote TTS voice / model discovery (settings pickers)
// ---------------------------------------------------------------------------

/// List available voices for the configured remote TTS engine, for the settings
/// voice picker. Errors are returned for inline display.
pub async fn list_tts_voices(settings: &AppSettings) -> Result<Vec<TtsVoice>, String> {
    let provider = active_provider(settings)?;
    match provider.protocol {
        TtsProtocol::Local => Err("Kokoro's voices are built in".to_string()),
        TtsProtocol::OpenAiCompatible => list_openai_tts_voices(settings, provider).await,
        TtsProtocol::ElevenLabs => list_elevenlabs_voices(settings).await,
        TtsProtocol::AzureSpeech => {
            let voices = list_azure_voices(settings).await?;
            Ok(voices
                .into_iter()
                .map(|v| TtsVoice {
                    label: format!("{} · {} {}", v.short_name, v.locale, v.gender)
                        .trim()
                        .to_string(),
                    id: v.short_name,
                })
                .collect())
        }
        TtsProtocol::Deepgram => list_deepgram_voices(settings, provider).await,
        TtsProtocol::Cartesia => list_cartesia_voices(settings, provider).await,
        TtsProtocol::GoogleCloud => list_google_voices(settings, provider).await,
        TtsProtocol::Xai => list_xai_voices(settings, provider).await,
        TtsProtocol::Mistral => list_mistral_voices(settings, provider).await,
    }
    .and_then(|voices| {
        if voices.is_empty() {
            Err("The endpoint returned no voices".to_string())
        } else {
            Ok(voices)
        }
    })
}

/// Title-case a lowercase voice slug for display (`thalia` → `Thalia`).
fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Join the non-empty parts of a picker label with the separator the other
/// engines use.
fn voice_label(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Deepgram voices via `GET /v1/models`. Each `tts` entry's `canonical_name`
/// (`aura-2-thalia-en`) is what `/v1/speak` takes as its model.
async fn list_deepgram_voices(
    settings: &AppSettings,
    provider: &TtsProvider,
) -> Result<Vec<TtsVoice>, String> {
    let key = required_key(settings, provider)?;
    let request = tts_client()?
        .get(format!("{}/v1/models", api_root(provider, settings)?))
        .header(reqwest::header::AUTHORIZATION, format!("Token {key}"));
    let value = json_body(
        request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?,
    )
    .await?;
    let mut voices: Vec<TtsVoice> = value
        .get("tts")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("canonical_name")?.as_str()?.to_string();
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(&id);
            let accent = item
                .pointer("/metadata/accent")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = item
                .get("languages")
                .and_then(|v| v.as_array())
                .and_then(|langs| langs.first())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let label = voice_label(&[&capitalized(name), accent, language, &id]);
            Some(TtsVoice { id, label })
        })
        .collect();
    // 102 voices across languages; English first so the common case is at the
    // top of the picker instead of behind every French and Spanish voice.
    voices.sort_by(|a, b| (!a.id.ends_with("-en"), &a.id).cmp(&(!b.id.ends_with("-en"), &b.id)));
    voices.dedup_by(|a, b| a.id == b.id);
    Ok(voices)
}

/// Cartesia voices via `GET /voices`, following its cursor for a few pages so
/// the list is useful without downloading the entire public library.
async fn list_cartesia_voices(
    settings: &AppSettings,
    provider: &TtsProvider,
) -> Result<Vec<TtsVoice>, String> {
    const MAX_PAGES: usize = 5;
    let key = required_key(settings, provider)?;
    let root = api_root(provider, settings)?;
    let client = tts_client()?;
    let mut voices = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut request = client
            .get(format!("{root}/voices"))
            .bearer_auth(key)
            .header("Cartesia-Version", CARTESIA_VERSION)
            .query(&[("limit", "100")]);
        if let Some(after) = &cursor {
            request = request.query(&[("starting_after", after.as_str())]);
        }
        let value = json_body(
            request
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?,
        )
        .await?;
        for item in value
            .get("data")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            let language = item.get("language").and_then(|v| v.as_str()).unwrap_or("");
            let gender = item.get("gender").and_then(|v| v.as_str()).unwrap_or("");
            voices.push(TtsVoice {
                id: id.to_string(),
                label: voice_label(&[name, language, gender]),
            });
        }
        let more = value
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        cursor = value
            .get("next_page")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| voices.last().map(|voice| voice.id.clone()));
        if !more {
            break;
        }
    }
    Ok(voices)
}

/// Google Cloud voices via `GET /v1/voices`, sorted so each locale's voices
/// sit together.
async fn list_google_voices(
    settings: &AppSettings,
    provider: &TtsProvider,
) -> Result<Vec<TtsVoice>, String> {
    let key = required_key(settings, provider)?;
    let request = tts_client()?
        .get(format!("{}/v1/voices", api_root(provider, settings)?))
        .header("x-goog-api-key", key);
    let value = json_body(
        request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?,
    )
    .await?;
    let mut voices: Vec<TtsVoice> = value
        .get("voices")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("name")?.as_str()?.to_string();
            let gender = item
                .get("ssmlGender")
                .and_then(|v| v.as_str())
                .map(|g| capitalized(&g.to_ascii_lowercase()))
                .unwrap_or_default();
            let label = voice_label(&[&id, &gender]);
            Some(TtsVoice { id, label })
        })
        .collect();
    // Hundreds of voices in every locale; English first, then each locale's
    // voices together.
    voices
        .sort_by(|a, b| (!a.id.starts_with("en-"), &a.id).cmp(&(!b.id.starts_with("en-"), &b.id)));
    Ok(voices)
}

/// xAI voices via `GET /v1/tts/voices`.
async fn list_xai_voices(
    settings: &AppSettings,
    provider: &TtsProvider,
) -> Result<Vec<TtsVoice>, String> {
    let key = required_key(settings, provider)?;
    let request = tts_client()?
        .get(format!("{}/tts/voices", api_root(provider, settings)?))
        .bearer_auth(key);
    let value = json_body(
        request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?,
    )
    .await?;
    Ok(value
        .get("voices")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("voice_id")?.as_str()?.to_string();
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(&id);
            let language = item.get("language").and_then(|v| v.as_str()).unwrap_or("");
            let label = voice_label(&[name, language]);
            Some(TtsVoice { id, label })
        })
        .collect())
}

/// Mistral voices via `GET /v1/audio/voices`: the presets plus any voices the
/// account has cloned.
async fn list_mistral_voices(
    settings: &AppSettings,
    provider: &TtsProvider,
) -> Result<Vec<TtsVoice>, String> {
    let key = required_key(settings, provider)?;
    let request = tts_client()?
        .get(format!("{}/audio/voices", api_root(provider, settings)?))
        .bearer_auth(key)
        .query(&[("type", "all"), ("limit", "100")]);
    let value = json_body(
        request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?,
    )
    .await?;
    Ok(value
        .get("items")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?.to_string();
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(&id);
            let kind = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let label = voice_label(&[name, kind]);
            Some(TtsVoice { id, label })
        })
        .collect())
}

/// Inworld voices via its TTS voice listing, which takes the portal's Basic
/// credential.
async fn list_inworld_voices(settings: &AppSettings) -> Result<Vec<TtsVoice>, String> {
    let key = settings.assistant_tts_api_key.0.trim();
    if key.is_empty() {
        return Err("No Inworld API key configured".to_string());
    }
    let request = tts_client()?
        .get("https://api.inworld.ai/tts/v1/voices")
        .header(reqwest::header::AUTHORIZATION, format!("Basic {key}"));
    let value = json_body(
        request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?,
    )
    .await?;
    Ok(value
        .get("voices")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("voiceId")?.as_str()?.to_string();
            let name = item
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or(&id);
            let languages = item
                .get("languages")
                .and_then(|v| v.as_array())
                .map(|langs| {
                    langs
                        .iter()
                        .filter_map(|l| l.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let label = voice_label(&[name, &languages]);
            Some(TtsVoice { id, label })
        })
        .collect())
}

/// Groq's Orpheus voices, documented per model rather than listed by the API.
const GROQ_ENGLISH_VOICES: &[&str] = &["autumn", "diana", "hannah", "austin", "daniel", "troy"];
const GROQ_ARABIC_VOICES: &[&str] = &["abdullah", "fahad", "sultan", "lulwa", "noura", "aisha"];

/// xAI Grok Voice TTS built-in voices. Grok's `/audio/speech` route rejects
/// OpenAI voice names, so offering `alloy`/`verse` here would only 404 — these
/// are the five voices it actually accepts.
const GROK_TTS_VOICES: &[&str] = &["Eve", "Ara", "Rex", "Sal", "Leo"];

/// The Kokoro v1.0 English voices, for Kokoro served over the OpenAI schema.
const KOKORO_VOICES: &[&str] = &[
    "af_heart",
    "af_bella",
    "af_nicole",
    "af_sky",
    "af_sarah",
    "af_nova",
    "am_adam",
    "am_michael",
    "am_fenrir",
    "am_puck",
    "bf_emma",
    "bf_isabella",
    "bm_george",
    "bm_fable",
];

/// Gemini TTS voices and Google's documented style labels. Shared by Gemini
/// 3.1 Flash TTS Preview and Gemini 2.5 Flash/Pro Preview TTS.
const GEMINI_TTS_VOICES: &[(&str, &str)] = &[
    ("Zephyr", "Bright"),
    ("Puck", "Upbeat"),
    ("Charon", "Informative"),
    ("Kore", "Firm"),
    ("Fenrir", "Excitable"),
    ("Leda", "Youthful"),
    ("Orus", "Firm"),
    ("Aoede", "Breezy"),
    ("Callirrhoe", "Easy-going"),
    ("Autonoe", "Bright"),
    ("Enceladus", "Breathy"),
    ("Iapetus", "Clear"),
    ("Umbriel", "Easy-going"),
    ("Algieba", "Smooth"),
    ("Despina", "Smooth"),
    ("Erinome", "Clear"),
    ("Algenib", "Gravelly"),
    ("Rasalgethi", "Informative"),
    ("Laomedeia", "Upbeat"),
    ("Achernar", "Soft"),
    ("Alnilam", "Firm"),
    ("Schedar", "Even"),
    ("Gacrux", "Mature"),
    ("Pulcherrima", "Forward"),
    ("Achird", "Friendly"),
    ("Zubenelgenubi", "Casual"),
    ("Vindemiatrix", "Gentle"),
    ("Sadachbia", "Lively"),
    ("Sadaltager", "Knowledgeable"),
    ("Sulafat", "Warm"),
];

/// Turn a slice of voice tokens into pickable [`TtsVoice`]s (id == label).
fn voices_from(names: &[&str]) -> Vec<TtsVoice> {
    names
        .iter()
        .map(|v| TtsVoice {
            id: v.to_string(),
            label: v.to_string(),
        })
        .collect()
}

fn labeled_voices_from(voices: &[(&str, &str)]) -> Vec<TtsVoice> {
    voices
        .iter()
        .map(|(name, style)| TtsVoice {
            id: name.to_string(),
            label: format!("{} · {}", name, style),
        })
        .collect()
}

/// Best-effort curated voice set for hosted OpenAI-compatible providers that
/// don't publish a `/audio/voices` listing, chosen by the selected MODEL rather
/// than a blanket OpenAI default.
///
/// This is what stops the picker from always showing OpenAI's `alloy…verse` for
/// every model: those names are only valid on OpenAI speech models and 404 on a
/// non-OpenAI model (Grok, Gemini, MAI-Voice, …). Returns `None` when the
/// model/provider isn't recognized so the caller can guide the user to type the
/// model's own voice instead of guessing.
fn curated_tts_voices(model: &str, base_url: &str) -> Option<Vec<TtsVoice>> {
    let m = model.to_ascii_lowercase();
    let url = base_url.to_ascii_lowercase();

    // xAI Grok Voice TTS — five named voices (e.g. `x-ai/grok-voice-tts-1.0`).
    if m.contains("grok") {
        return Some(voices_from(GROK_TTS_VOICES));
    }
    // Gemini 3.1/2.5 TTS models all use Google's shared named voice set.
    if m.contains("gemini") && m.contains("tts") {
        return Some(labeled_voices_from(GEMINI_TTS_VOICES));
    }
    // Kokoro served over the OpenAI schema (OpenRouter's `hexgrad/kokoro-82m`,
    // or a local server whose model is named after it) takes Kokoro's own ids.
    if m.contains("kokoro") {
        return Some(voices_from(KOKORO_VOICES));
    }
    // OpenAI speech models (`openai/gpt-4o-mini-tts…`, bare `gpt-4o-mini-tts`,
    // `tts-1`, `tts-1-hd`) or OpenAI's own endpoint use the standard voice set.
    let is_openai_model = m.starts_with("openai/")
        || m.starts_with("gpt-")
        || m.starts_with("tts-1")
        || (m.contains("gpt") && m.contains("tts"));
    if is_openai_model || url.contains("api.openai.com") {
        return Some(voices_from(OPENAI_TTS_VOICES));
    }
    None
}

/// Parse a self-hosted server's voice listing. There is no standard, so every
/// shape the popular servers use is accepted: `{voices:[…]}` (Kokoro-FastAPI,
/// Speaches, Orpheus-FastAPI, vLLM-Omni, Chatterbox), `{data:[…]}`, or a bare
/// array — each item a string, or an object with an `id` / `voice_id` / `name`.
fn parse_voice_listing(value: &serde_json::Value) -> Vec<TtsVoice> {
    let items = value
        .get("voices")
        .and_then(|v| v.as_array())
        .or_else(|| value.get("data").and_then(|v| v.as_array()))
        .or_else(|| value.as_array());
    let mut voices = Vec::new();
    for item in items.into_iter().flatten() {
        if let Some(s) = item.as_str() {
            voices.push(TtsVoice {
                id: s.to_string(),
                label: s.to_string(),
            });
        } else if let Some(id) = item
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("voice_id").and_then(|v| v.as_str()))
            .or_else(|| item.get("name").and_then(|v| v.as_str()))
        {
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            let language = item.get("language").and_then(|v| v.as_str()).unwrap_or("");
            // openai-edge-tts reports `{id: "alloy", name: "en-US-JennyNeural"}`,
            // where the name is the Edge voice the alias maps to — worth
            // showing, but it must not replace the id.
            let label = if name == id {
                voice_label(&[id, language])
            } else {
                voice_label(&[name, language, id])
            };
            voices.push(TtsVoice {
                id: id.to_string(),
                label,
            });
        }
    }
    voices
}

/// A self-hosted server's own voice list. `GET {root}/audio/voices` is the
/// common route; Chatterbox (travisvn) serves `GET {root}/voices` instead.
/// `None` when neither answers with a usable list.
async fn fetch_server_voices(
    settings: &AppSettings,
    provider: &TtsProvider,
    root: &str,
) -> Option<Vec<TtsVoice>> {
    let client = tts_client().ok()?;
    let key = settings.assistant_tts_api_key.0.trim();
    for path in ["/audio/voices", "/voices"] {
        let request = with_openai_auth(client.get(format!("{root}{path}")), provider, key);
        let Ok(response) = request.send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(value) = response.json::<serde_json::Value>().await else {
            continue;
        };
        let voices = parse_voice_listing(&value);
        if !voices.is_empty() {
            return Some(voices);
        }
    }
    None
}

/// Voices for an OpenAI-compatible engine.
///
/// Hosted engines publish no voice listing on this schema, so each gets the
/// set its own docs name, chosen by the selected MODEL where the engine fronts
/// several (the root of "the voice name is the same for every model" — OpenAI's
/// `alloy` 404s under Grok or Gemini). A custom server is asked for its own
/// list first. When nothing applies the error says what to type instead; the
/// field is free text, so an unrecognized model is never a dead end.
async fn list_openai_tts_voices(
    settings: &AppSettings,
    provider: &TtsProvider,
) -> Result<Vec<TtsVoice>, String> {
    let model = or_default(&settings.assistant_tts_model, provider.default_model);
    match provider.id {
        "openai" => return Ok(voices_from(OPENAI_TTS_VOICES)),
        "groq" => {
            return Ok(voices_from(if model.contains("arabic") {
                GROQ_ARABIC_VOICES
            } else {
                GROQ_ENGLISH_VOICES
            }))
        }
        "inworld" => return list_inworld_voices(settings).await,
        _ => {}
    }

    let root = openai_root(&api_root(provider, settings)?);
    if provider.base_url.is_none() {
        if let Some(voices) = fetch_server_voices(settings, provider, &root).await {
            return Ok(voices);
        }
    }

    if let Some(voices) = curated_tts_voices(model, &root) {
        return Ok(voices);
    }

    Err(
        "Voices vary by model, and this endpoint doesn't publish a list. \
         Type the voice from the model's page — e.g. Grok TTS uses \
         Eve/Ara/Rex/Sal/Leo, OpenAI models use alloy/echo/nova/…, Kokoro \
         uses af_heart/am_adam/…, and other models use the names from their \
         own page."
            .to_string(),
    )
}

/// ElevenLabs voices via `GET /v2/voices` (auth `xi-api-key`).
async fn list_elevenlabs_voices(settings: &AppSettings) -> Result<Vec<TtsVoice>, String> {
    let api_key = settings.assistant_tts_api_key.0.trim();
    if api_key.is_empty() {
        return Err("No ElevenLabs API key configured".to_string());
    }
    let client = tts_client()?;
    let resp = client
        .get("https://api.elevenlabs.io/v2/voices?page_size=100")
        .header("xi-api-key", api_key)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{}: {}", status, truncate(&body, 300)));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse voices list: {}", e))?;

    let mut voices = Vec::new();
    if let Some(items) = value.get("voices").and_then(|v| v.as_array()) {
        for item in items {
            let Some(id) = item.get("voice_id").and_then(|v| v.as_str()) else {
                continue;
            };
            if id.is_empty() {
                continue;
            }
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            let category = item.get("category").and_then(|v| v.as_str()).unwrap_or("");
            let label = if category.is_empty() {
                name.to_string()
            } else {
                format!("{} · {}", name, category)
            };
            voices.push(TtsVoice {
                id: id.to_string(),
                label,
            });
        }
    }

    if voices.is_empty() {
        return Err("The endpoint returned no voices".to_string());
    }
    Ok(voices)
}

/// List available models for the configured remote TTS engine, for the settings
/// model picker. Engines with a live listing are asked; engines with a small
/// documented set answer from the registry; engines whose voice implies the
/// model (Deepgram, Google, xAI, Azure) have no model field and say so.
pub async fn list_tts_models(settings: &AppSettings) -> Result<Vec<String>, String> {
    let provider = active_provider(settings)?;
    match provider.protocol {
        TtsProtocol::ElevenLabs => list_elevenlabs_models(settings).await,
        TtsProtocol::OpenAiCompatible
            if matches!(provider.id, "openai" | "openrouter" | "custom") =>
        {
            list_openai_tts_models(settings, provider).await
        }
        _ if !provider.models.is_empty() => {
            Ok(provider.models.iter().map(|m| m.to_string()).collect())
        }
        _ => Err(format!(
            "{} picks its model from the voice, so there is no model to choose.",
            provider.label
        )),
    }
}

/// Model ids from an OpenAI-compatible `/models` listing: the standard
/// `{data:[{id}]}`, openai-edge-tts' `{models:[{id}]}`, or a bare array.
fn parse_model_listing(value: &serde_json::Value) -> Vec<String> {
    let items = value
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| value.get("models").and_then(|v| v.as_array()))
        .or_else(|| value.as_array());
    items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("id").and_then(|v| v.as_str()))
                .map(str::to_string)
        })
        .collect()
}

/// OpenAI-compatible models via `GET {base}/models`.
async fn list_openai_tts_models(
    settings: &AppSettings,
    provider: &TtsProvider,
) -> Result<Vec<String>, String> {
    let base = openai_root(&api_root(provider, settings)?);
    // OpenRouter's /models returns thousands of chat models; ask it for only
    // speech-capable ones so the TTS model picker is actually usable.
    let url = if provider.id == "openrouter" {
        format!("{base}/models?output_modalities=speech")
    } else {
        format!("{base}/models")
    };

    let request = with_openai_auth(
        tts_client()?.get(&url),
        provider,
        settings.assistant_tts_api_key.0.trim(),
    );
    let resp = request
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(http_error(status, &body));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse models list: {}", e))?;
    let mut models = parse_model_listing(&value);
    // OpenAI's listing is every model on the account, chat included; only the
    // speech ones belong in this picker.
    if provider.id == "openai" {
        models.retain(|id| id.contains("tts"));
        if models.is_empty() {
            models = provider.models.iter().map(|m| m.to_string()).collect();
        }
    }
    Ok(models)
}

/// ElevenLabs TTS models via `GET /v1/models`, filtered to those that can do
/// text-to-speech.
async fn list_elevenlabs_models(settings: &AppSettings) -> Result<Vec<String>, String> {
    let api_key = settings.assistant_tts_api_key.0.trim();
    if api_key.is_empty() {
        return Err("No ElevenLabs API key configured".to_string());
    }
    let client = tts_client()?;
    let resp = client
        .get("https://api.elevenlabs.io/v1/models")
        .header("xi-api-key", api_key)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{}: {}", status, truncate(&body, 300)));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse models list: {}", e))?;

    let mut models = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            let can_tts = item
                .get("can_do_text_to_speech")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if !can_tts {
                continue;
            }
            if let Some(id) = item.get("model_id").and_then(|v| v.as_str()) {
                models.push(id.to_string());
            }
        }
    }

    if models.is_empty() {
        return Err("The endpoint returned no text-to-speech models".to_string());
    }
    Ok(models)
}

/// POST {base}/cognitiveservices/v1 — Azure AI Speech (Neural TTS) SSML API.
async fn fetch_azure_speech(settings: &AppSettings, text: &str) -> Result<Vec<u8>, String> {
    if settings.assistant_tts_base_url.trim().is_empty() {
        return Err(
            "No Azure Speech endpoint configured. Set the TTS Base URL to your regional \
             endpoint, e.g. https://eastus.tts.speech.microsoft.com"
                .to_string(),
        );
    }
    let url = format!(
        "{}/cognitiveservices/v1",
        azure_tts_host(&settings.assistant_tts_base_url)
    );

    let api_key = settings.assistant_tts_api_key.0.trim();
    if api_key.is_empty() {
        return Err("No Azure Speech API key configured".to_string());
    }

    let voice = settings.assistant_tts_remote_voice.trim();
    let voice = if voice.is_empty() {
        "en-US-JennyNeural"
    } else {
        voice
    };

    // Derive the locale (xml:lang) from the voice name prefix, e.g. a voice
    // named "en-US-JennyNeural" yields "en-US". Fall back to en-US otherwise.
    let lang = voice_locale(voice);

    // Apply playback speed via SSML <prosody rate>. Azure takes a relative
    // percentage (e.g. +100% ≈ 2x, -50% ≈ 0.5x) and preserves pitch, within
    // 0.5x–2x. Wrap only when the rate actually differs from normal.
    let escaped_text = xml_escape(text);
    let speed = provider("azure").and_then(|azure| speed_for(azure, settings));
    let inner = if let Some(speed) = speed {
        let rate = format!("{:+.0}%", (speed - 1.0) * 100.0);
        format!("<prosody rate='{}'>{}</prosody>", rate, escaped_text)
    } else {
        escaped_text
    };

    let ssml = format!(
        "<speak version='1.0' xml:lang='{lang}'><voice xml:lang='{lang}' name='{voice}'>{inner}</voice></speak>",
        lang = lang,
        voice = xml_escape(voice),
        inner = inner,
    );

    let client = tts_client()?;
    let request = client
        .post(&url)
        .header("Ocp-Apim-Subscription-Key", api_key)
        .header("Content-Type", "application/ssml+xml")
        // Highest-quality MP3 Azure offers: 48 kHz, 192 kbps. The previous
        // 24 kHz/48 kbps profile sounded crunchy on speech.
        .header(
            "X-Microsoft-OutputFormat",
            "audio-48khz-192kbitrate-mono-mp3",
        )
        .header("User-Agent", "SpeakoFlow")
        .body(ssml);
    let response = send_tts_with_retries(request).await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("{}: {}", status, truncate(&body, 300)));
    }

    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read audio: {}", e))
}

/// Synthesize and play a short sample for the settings "Test voice" button.
/// Unlike [`speak_remote`], errors are returned to the caller (so the UI can
/// show them inline) instead of being emitted as assistant errors.
pub async fn test_remote(settings: &AppSettings, text: String) -> Result<(), String> {
    let epoch = current_epoch();
    let audio_bytes = synthesize_speech(
        settings,
        SpeechRequest {
            text: &text,
            previous_request_ids: &[],
        },
    )
    .await?
    .bytes;
    let volume = settings.assistant_tts_volume;
    let device = settings.selected_output_device.clone();
    tauri::async_runtime::spawn_blocking(move || {
        play_audio_bytes(audio_bytes, device, volume, epoch).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("playback task failed: {}", e))?
}

/// Escape the five XML predefined entities so user/model text is safe in SSML.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Open an output stream on the user's selected device, falling back to the
/// system default when the named device is gone (unplugged headset, changed
/// profile) so playback degrades to "wrong speaker" rather than silence.
fn open_output_stream(
    selected_device: Option<String>,
) -> Result<rodio::OutputStream, Box<dyn std::error::Error>> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use rodio::OutputStreamBuilder;

    let stream_builder = match selected_device {
        Some(name) if name != "Default" => {
            let host = crate::audio_toolkit::get_cpal_host();
            let device = host
                .output_devices()?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false));
            match device {
                Some(device) => OutputStreamBuilder::from_device(device)?,
                None => OutputStreamBuilder::from_default_device()?,
            }
        }
        _ => OutputStreamBuilder::from_default_device()?,
    };
    Ok(stream_builder.open_stream()?)
}

/// Decode and play audio bytes (mp3/wav/ogg) on the selected output device.
/// Polls the playback epoch so a `stop_remote()` cancels playback promptly.
///
/// One-shot: opens a stream, plays a single clip, returns when it finishes. Used
/// where that is exactly the intent (the settings "Test voice" button, replaying
/// a saved reply). Streamed replies use [`enqueue_speech_chunk`] instead, which
/// keeps one stream open across every chunk of the reply.
pub(crate) fn play_audio_bytes(
    bytes: Vec<u8>,
    selected_device: Option<String>,
    volume: f32,
    epoch: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let stream_handle = open_output_stream(selected_device)?;
    let sink = rodio::play(stream_handle.mixer(), Cursor::new(bytes))?;
    sink.set_volume(volume.clamp(0.0, 1.0));

    // Poll rather than `sink.sleep_until_end()` so cancellation is responsive.
    // The OutputStream/Sink are not Send, so they stay on this thread while the
    // cancel signal crosses threads via the atomic epoch.
    while !sink.empty() {
        if current_epoch() != epoch {
            sink.stop();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

/// Clean LLM / Markdown text so it reads naturally through any TTS engine.
///
/// The assistant only ever speaks a short summary, never the on-screen answer,
/// but that summary can still carry Markdown, inline code, links or emojis that
/// sound terrible read aloud (or get spelled out symbol by symbol). This strips
/// the *formatting* while keeping the *words*:
///
/// - fenced code blocks are dropped entirely (we never read code out loud)
/// - inline code keeps its text, only the backticks go (`map()` -> map())
/// - headings, blockquotes, list bullets, emphasis (`*` `_` `~`) and table
///   pipes lose their markers but keep their content
/// - `[text](url)` keeps `text`; bare URLs are dropped
/// - emoji / pictographs and stray symbol runs (`----`, `====`) are removed
/// - `_` becomes a space so identifiers like `snake_case` read as two words
///
/// It is deliberately conservative — only known noise is touched, normal
/// punctuation and tokens like `C#` or `foo()` are preserved. If cleaning would
/// leave nothing speakable, the original (whitespace-collapsed) text is returned
/// when it still contains pronounceable characters, otherwise an empty string so
/// the caller can skip playback instead of voicing garbage.
pub fn sanitize_for_speech(input: &str) -> String {
    sanitize_speech_inner(input, true)
}

/// [`sanitize_for_speech`] for a single streamed chunk rather than a whole reply.
///
/// The only difference is the raw-text fallback: whole replies keep it, because a
/// reply that cleans away to nothing is better spoken imperfectly than not at
/// all. A *chunk* that cleans away to nothing is usually a fenced code block or
/// a divider, and falling back would read the code aloud — exactly what the
/// filter exists to prevent. Callers skip empty results instead.
pub fn sanitize_for_speech_chunk(input: &str) -> String {
    sanitize_speech_inner(input, false)
}

fn sanitize_speech_inner(input: &str, allow_raw_fallback: bool) -> String {
    // Fenced code blocks (``` … ``` or ~~~ … ~~~), including the info string.
    static FENCED_CODE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?s)```[^\n]*\n?.*?```|~~~[^\n]*\n?.*?~~~").unwrap());
    // Images first (drop alt + url), then links (keep the visible text). The URL
    // half allows one level of nested parentheses, so Wikipedia-style targets
    // (`.../Foo_(bar)`) are consumed whole instead of leaving a stray `)` to be
    // read aloud.
    static IMAGE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"!\[[^\]]*\]\((?:[^()]|\([^()]*\))*\)").unwrap());
    static LINK: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\[([^\]]+)\]\((?:[^()]|\([^()]*\))*\)").unwrap());
    static URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(?:https?://|www\.)\S+").unwrap());
    // Inline code: keep the inner text, drop the backticks.
    static INLINE_CODE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`+([^`]*)`+").unwrap());
    static HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^\s{0,3}#{1,6}[ \t]*").unwrap());
    static BLOCKQUOTE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^[ \t]*>+[ \t]?").unwrap());
    static LIST_BULLET: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^[ \t]*[-*+][ \t]+").unwrap());
    // A Markdown table separator row, e.g. `|---|:--:|`.
    static TABLE_SEP: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?m)^[ \t]*\|?[ \t]*:?-{2,}:?[ \t]*(\|[ \t]*:?-{2,}:?[ \t]*)+\|?[ \t]*$")
            .unwrap()
    });
    // Horizontal-rule / divider runs of dashes or equals signs.
    static DASH_RUN: Lazy<Regex> = Lazy::new(|| Regex::new(r"-{2,}|={2,}").unwrap());
    // Leftover emphasis / table markers. Note: `#` is intentionally NOT here so
    // tokens like `C#` / `F#` survive (heading `#` is already handled above).
    static EMPHASIS: Lazy<Regex> = Lazy::new(|| Regex::new(r"[*~`|]+").unwrap());
    // Emoji & common pictographs / dingbats / arrows / flags / selectors.
    static EMOJI: Lazy<Regex> = Lazy::new(|| {
        Regex::new(concat!(
            "[",
            r"\x{1F000}-\x{1FAFF}", // emoticons, transport, pictographs, symbols ext-A …
            r"\x{2600}-\x{27BF}",   // misc symbols + dingbats
            r"\x{2300}-\x{23FF}",   // misc technical (⌚ ⏰ …)
            r"\x{2B00}-\x{2BFF}",   // misc symbols and arrows
            r"\x{2190}-\x{21FF}",   // arrows
            r"\x{1F1E6}-\x{1F1FF}", // regional indicators (flag letters)
            r"\x{FE00}-\x{FE0F}",   // variation selectors
            r"\x{200D}",            // zero-width joiner
            r"\x{20E3}",            // combining enclosing keycap
            r"\x{2122}\x{2139}\x{3030}\x{303D}\x{3297}\x{3299}",
            "]"
        ))
        .unwrap()
    });
    static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

    let mut text = FENCED_CODE.replace_all(input, " ").into_owned();
    text = IMAGE.replace_all(&text, " ").into_owned();
    text = LINK.replace_all(&text, "$1").into_owned();
    text = URL.replace_all(&text, " ").into_owned();
    text = INLINE_CODE.replace_all(&text, "$1").into_owned();
    text = HEADING.replace_all(&text, "").into_owned();
    text = BLOCKQUOTE.replace_all(&text, "").into_owned();
    text = TABLE_SEP.replace_all(&text, " ").into_owned();
    text = LIST_BULLET.replace_all(&text, "").into_owned();
    text = DASH_RUN.replace_all(&text, " ").into_owned();
    text = text.replace('_', " ");
    text = EMPHASIS.replace_all(&text, "").into_owned();
    text = EMOJI.replace_all(&text, "").into_owned();

    let cleaned = WS.replace_all(text.trim(), " ").into_owned();
    let cleaned = cleaned.trim();

    if cleaned.is_empty() {
        // Over-aggressive strip: only fall back to the raw text if it actually
        // contains something pronounceable, otherwise let the caller skip.
        if allow_raw_fallback && input.chars().any(|c| c.is_alphanumeric()) {
            return WS.replace_all(input.trim(), " ").into_owned();
        }
        return String::new();
    }
    cleaned.to_string()
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_speech;
    use super::voice_engine_blocker;
    use crate::settings::{get_default_settings, AppSettings};

    fn with_voice(engine: &str, api_key: &str, base_url: &str) -> AppSettings {
        let mut settings = get_default_settings();
        settings.assistant_tts_engine = engine.to_string();
        settings.assistant_tts_api_key = crate::settings::SecretString(api_key.to_string());
        settings.assistant_tts_base_url = base_url.to_string();
        settings
    }

    /// A call speaks every reply whether or not spoken replies are switched on,
    /// so it inherits a voice engine the user configured but never finished. A
    /// remote engine with no key used to fail once per utterance, forever, with a
    /// generic provider error on a surface the orb view hides.
    #[test]
    fn a_remote_voice_with_no_key_blocks_a_call_before_it_starts() {
        for provider in super::TTS_PROVIDERS.iter().filter(|p| p.requires_key) {
            let blocker =
                voice_engine_blocker(&with_voice(provider.id, "", "https://api.example.com"));
            assert!(
                blocker.is_some_and(|m| m.contains("API key")),
                "{} with no key should block a call",
                provider.id
            );
        }
    }

    #[test]
    fn a_configured_or_on_device_voice_does_not_block_a_call() {
        assert!(voice_engine_blocker(&with_voice("kokoro", "", "")).is_none());
        assert!(voice_engine_blocker(&get_default_settings()).is_none());
        for provider in super::TTS_PROVIDERS {
            assert!(
                voice_engine_blocker(&with_voice(
                    provider.id,
                    "sk-test",
                    "https://api.example.com"
                ))
                .is_none(),
                "{} with a key should be allowed",
                provider.id
            );
        }
    }

    /// A self-hosted speech server legitimately needs no key; refusing it would
    /// break a working offline setup. It does need an address.
    #[test]
    fn a_custom_speech_server_needs_an_address_but_no_key() {
        for base_url in [
            "http://127.0.0.1:8880/v1",
            "http://localhost:5005/v1",
            "http://[::1]:8000/v1",
            "http://192.168.1.20:4123/v1",
        ] {
            assert!(
                voice_engine_blocker(&with_voice("custom", "", base_url)).is_none(),
                "{base_url} should be allowed without a key"
            );
        }
        assert!(voice_engine_blocker(&with_voice("custom", "", ""))
            .is_some_and(|m| m.contains("server address")));
        // OpenAI itself is a hosted engine now, whatever URL is lying around.
        assert!(
            voice_engine_blocker(&with_voice("openai", "", "http://localhost:8880/v1")).is_some()
        );
    }

    #[test]
    fn an_unknown_engine_blocks_with_a_readable_message() {
        assert!(voice_engine_blocker(&with_voice("not-an-engine", "k", ""))
            .is_some_and(|m| m.contains("not-an-engine")));
    }

    #[test]
    fn registry_ids_are_unique_and_fixed_endpoints_are_https() {
        let mut seen = std::collections::HashSet::new();
        for provider in super::TTS_PROVIDERS {
            assert!(seen.insert(provider.id), "duplicate id {}", provider.id);
            assert!(
                provider.max_chars > 0,
                "{} has no length limit",
                provider.id
            );
            if let Some(url) = provider.base_url {
                assert!(url.starts_with("https://"), "{} is not https", provider.id);
                assert!(!url.ends_with('/'), "{} has a trailing slash", provider.id);
            }
            if let Some((min, max)) = provider.speed_range {
                assert!(min < 1.0 && max > 1.0, "{} can't speak at 1x", provider.id);
            }
        }
        // The two engines people configure themselves have no fixed endpoint.
        assert!(super::fixed_base_url("custom").is_none());
        assert!(super::fixed_base_url("azure").is_none());
        assert!(super::is_known_engine("kokoro"));
        assert!(!super::is_known_engine("playai"));
    }

    #[test]
    fn split_to_limit_keeps_short_text_whole() {
        assert_eq!(
            super::split_to_limit("  Hello there.  ", 200),
            vec!["Hello there.".to_string()]
        );
        assert!(super::split_to_limit("   ", 200).is_empty());
    }

    #[test]
    fn split_to_limit_prefers_sentence_ends_and_never_exceeds_the_limit() {
        let text = "The weather is mild today. Expect light rain after noon, \
                    so bring a jacket. Tomorrow clears up nicely, with a high of 3.5 degrees above normal.";
        let pieces = super::split_to_limit(text, 60);
        assert!(pieces.len() > 1);
        for piece in &pieces {
            assert!(piece.chars().count() <= 60, "{piece:?} is too long");
            assert!(!piece.is_empty());
        }
        assert_eq!(pieces[0], "The weather is mild today.");
        // A decimal point is not a sentence end.
        assert!(pieces.iter().all(|p| !p.ends_with("3.")));
        // Nothing is lost or duplicated.
        let rejoined = pieces.join(" ");
        let words = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(words(&rejoined), words(text));
    }

    #[test]
    fn split_to_limit_falls_back_to_a_hard_cut_for_an_unbroken_run() {
        let run = "a".repeat(450);
        let pieces = super::split_to_limit(&run, 200);
        assert_eq!(
            pieces.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![200, 200, 50]
        );
        // Multi-byte text is cut on character boundaries, not bytes.
        let wide = "語".repeat(250);
        for piece in super::split_to_limit(&wide, 200) {
            assert!(piece.chars().count() <= 200);
        }
    }

    #[test]
    fn a_bare_server_address_gets_the_v1_root() {
        use super::normalize_server_url;
        assert_eq!(
            normalize_server_url("http://localhost:8880"),
            "http://localhost:8880/v1"
        );
        assert_eq!(
            normalize_server_url("http://localhost:8880/"),
            "http://localhost:8880/v1"
        );
        assert_eq!(
            normalize_server_url("http://localhost:8880/v1/"),
            "http://localhost:8880/v1"
        );
        let azure = "https://res.cognitiveservices.azure.com/openai/deployments/tts/audio/speech?api-version=2025-03-01-preview";
        assert_eq!(normalize_server_url(azure), azure);
        assert_eq!(super::openai_speech_url(azure), azure);
        assert_eq!(
            super::openai_root(azure),
            "https://res.cognitiveservices.azure.com/openai/deployments/tts"
        );
    }

    #[test]
    fn speed_is_clamped_per_engine_and_omitted_where_unsupported() {
        use super::{provider, speed_for};
        let mut settings = get_default_settings();
        settings.assistant_tts_speed = 3.0;
        assert_eq!(speed_for(provider("openai").unwrap(), &settings), Some(3.0));
        assert_eq!(
            speed_for(provider("deepgram").unwrap(), &settings),
            Some(1.5)
        );
        assert_eq!(speed_for(provider("groq").unwrap(), &settings), None);
        settings.assistant_tts_speed = 1.0;
        assert_eq!(speed_for(provider("openai").unwrap(), &settings), None);
    }

    #[test]
    fn server_voice_listings_in_every_known_shape_parse() {
        use super::parse_voice_listing;
        let ids = |v: serde_json::Value| {
            parse_voice_listing(&v)
                .into_iter()
                .map(|x| x.id)
                .collect::<Vec<_>>()
        };
        // Kokoro-FastAPI / Speaches: objects with ids.
        assert_eq!(
            ids(serde_json::json!({"voices":[{"id":"af_heart","name":"af_heart"}]})),
            vec!["af_heart"]
        );
        // Orpheus-FastAPI / devnen Chatterbox: bare strings beside a status.
        assert_eq!(
            ids(serde_json::json!({"status":"ok","voices":["tara","leo"]})),
            vec!["tara", "leo"]
        );
        // travisvn Chatterbox: objects with only a name.
        assert_eq!(
            ids(serde_json::json!({"voices":[{"name":"narrator","filename":"n.wav"}],"count":1})),
            vec!["narrator"]
        );
        // openai-edge-tts: the id is the alias, the name is the Edge voice.
        let edge = parse_voice_listing(
            &serde_json::json!({"voices":[{"id":"alloy","name":"en-US-JennyNeural"}]}),
        );
        assert_eq!(edge[0].id, "alloy");
        assert!(edge[0].label.contains("en-US-JennyNeural"));
        // A top-level array.
        assert_eq!(ids(serde_json::json!(["a", "b"])), vec!["a", "b"]);
    }

    #[test]
    fn server_model_listings_in_every_known_shape_parse() {
        use super::parse_model_listing;
        assert_eq!(
            parse_model_listing(&serde_json::json!({"object":"list","data":[{"id":"tts-1"}]})),
            vec!["tts-1"]
        );
        // openai-edge-tts answers `{models:[…]}` rather than `{data:[…]}`.
        assert_eq!(
            parse_model_listing(&serde_json::json!({"models":[{"id":"tts-1-hd"}]})),
            vec!["tts-1-hd"]
        );
        assert_eq!(
            parse_model_listing(&serde_json::json!(["kokoro"])),
            vec!["kokoro"]
        );
    }

    #[test]
    fn audio_is_recognised_by_its_header_not_its_label() {
        use super::{sniff_container, wrap_requested_pcm};
        assert_eq!(sniff_container(b"RIFF\x00\x00\x00\x00WAVE"), Some("wav"));
        assert_eq!(sniff_container(b"ID3\x04"), Some("mp3"));
        assert_eq!(sniff_container(&[0xFF, 0xFB, 0x90]), Some("mp3"));
        assert_eq!(sniff_container(b"OggS"), Some("ogg"));
        assert_eq!(sniff_container(&[0x00, 0x01, 0x02]), None);
        // A server that answers a pcm request with a WAV file must not have a
        // second header stacked on top.
        let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
        assert_eq!(wrap_requested_pcm("audio/pcm", wav.clone()), wav);
        assert_eq!(&wrap_requested_pcm("", vec![0, 0])[0..4], b"RIFF");
    }

    #[test]
    fn voice_locale_comes_from_the_voice_name() {
        use super::voice_locale;
        assert_eq!(voice_locale("en-US-JennyNeural"), "en-US");
        assert_eq!(voice_locale("en-GB-Chirp3-HD-Kore"), "en-GB");
        assert_eq!(voice_locale("cmn-CN-Wavenet-A"), "cmn-CN");
        assert_eq!(voice_locale("Kore"), "en-US");
    }

    #[test]
    fn keeps_plain_prose_unchanged() {
        let s = "Use the map function to double each item.";
        assert_eq!(sanitize_for_speech(s), s);
    }

    #[test]
    fn strips_emphasis_markers_keeps_words() {
        assert_eq!(
            sanitize_for_speech("This is **bold** and *italic* text."),
            "This is bold and italic text."
        );
    }

    #[test]
    fn keeps_inline_code_text() {
        assert_eq!(sanitize_for_speech("Call `foo()` now."), "Call foo() now.");
    }

    #[test]
    fn removes_fenced_code_block_but_keeps_surrounding_prose() {
        let input = "Here is how:\n```rust\nlet x = 1;\n```\nThat's it.";
        let out = sanitize_for_speech(input);
        assert!(!out.contains("let x"), "code leaked: {out}");
        assert!(out.contains("Here is how"));
        assert!(out.contains("That's it."));
    }

    #[test]
    fn keeps_link_text_drops_url() {
        assert_eq!(
            sanitize_for_speech("See [the docs](https://example.com/page) here."),
            "See the docs here."
        );
    }

    #[test]
    fn drops_bare_urls() {
        assert_eq!(
            sanitize_for_speech("Go to https://example.com now"),
            "Go to now"
        );
    }

    #[test]
    fn removes_emoji() {
        assert_eq!(sanitize_for_speech("Nice 👍 work 🎉"), "Nice work");
    }

    #[test]
    fn underscores_become_spaces() {
        assert_eq!(sanitize_for_speech("Set my_var here"), "Set my var here");
    }

    #[test]
    fn preserves_c_sharp_token() {
        assert_eq!(sanitize_for_speech("Use C# for this"), "Use C# for this");
    }

    #[test]
    fn strips_heading_marker() {
        assert_eq!(sanitize_for_speech("# Title"), "Title");
    }

    #[test]
    fn symbols_only_returns_empty() {
        assert_eq!(sanitize_for_speech("***"), "");
        assert_eq!(sanitize_for_speech("```\n```"), "");
    }

    #[test]
    fn azure_voices_url_regional_vs_custom_domain() {
        use super::azure_voices_url;
        // Regional Speech host: un-prefixed path.
        assert_eq!(
            azure_voices_url("https://eastus2.tts.speech.microsoft.com"),
            "https://eastus2.tts.speech.microsoft.com/cognitiveservices/voices/list"
        );
        // Portal "endpoint" form is converted to the regional tts host.
        assert_eq!(
            azure_voices_url("https://eastus.api.cognitive.microsoft.com/"),
            "https://eastus.tts.speech.microsoft.com/cognitiveservices/voices/list"
        );
        // Custom-domain / AI Foundry resource: /tts/-prefixed path.
        assert_eq!(
            azure_voices_url("https://myres.cognitiveservices.azure.com/"),
            "https://myres.cognitiveservices.azure.com/tts/cognitiveservices/voices/list"
        );
    }

    #[test]
    fn pcm_to_wav_writes_canonical_header() {
        use super::pcm_to_wav;
        // 4 bytes of PCM = two 16-bit mono samples.
        let pcm = [0x01u8, 0x00, 0xff, 0x7f];
        let wav = pcm_to_wav(&pcm, 24_000, 1, 16);

        // 44-byte header + data.
        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        // RIFF chunk size = 36 + data_len.
        assert_eq!(u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]), 40);
        // fmt subchunk size = 16, audio format = 1 (PCM).
        assert_eq!(u32::from_le_bytes([wav[16], wav[17], wav[18], wav[19]]), 16);
        assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1);
        // channels = 1, sample rate = 24000.
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            24_000
        );
        // byte rate = 24000 * 1 * 2, block align = 2, bits = 16.
        assert_eq!(
            u32::from_le_bytes([wav[28], wav[29], wav[30], wav[31]]),
            48_000
        );
        assert_eq!(u16::from_le_bytes([wav[32], wav[33]]), 2);
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16);
        // data subchunk header + payload preserved verbatim.
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 4);
        assert_eq!(&wav[44..], &pcm);
    }

    #[test]
    fn curated_tts_voices_are_model_aware() {
        use super::curated_tts_voices;
        let ids = |v: Vec<super::TtsVoice>| v.into_iter().map(|x| x.id).collect::<Vec<_>>();

        // Grok model → Grok's own voices, never OpenAI's.
        let grok = ids(curated_tts_voices(
            "x-ai/grok-voice-tts-1.0",
            "https://openrouter.ai/api/v1",
        )
        .unwrap());
        assert!(grok.contains(&"Eve".to_string()));
        assert!(!grok.contains(&"alloy".to_string()));

        // OpenAI model on OpenRouter → OpenAI voice set.
        let oai = ids(curated_tts_voices(
            "openai/gpt-4o-mini-tts-2025-12-15",
            "https://openrouter.ai/api/v1",
        )
        .unwrap());
        assert!(oai.contains(&"alloy".to_string()));

        // OpenAI's own endpoint, no model set → OpenAI voice set.
        assert!(
            ids(curated_tts_voices("", "https://api.openai.com/v1").unwrap())
                .contains(&"alloy".to_string())
        );

        // Gemini TTS → Google's complete shared voice set with style labels.
        let gemini = curated_tts_voices(
            "google/gemini-3.1-flash-tts-preview",
            "https://openrouter.ai/api/v1",
        )
        .unwrap();
        assert_eq!(gemini.len(), 30);
        assert!(gemini
            .iter()
            .any(|voice| voice.id == "Puck" && voice.label == "Puck · Upbeat"));
        assert!(!gemini.iter().any(|voice| voice.id == "alloy"));
    }

    #[test]
    fn fixed_engines_ignore_a_stale_saved_url() {
        use super::{api_root, provider, OPENROUTER_TTS_BASE_URL};

        let mut settings = crate::settings::get_default_settings();
        settings.assistant_tts_base_url = "https://wrong.example/v1".to_string();
        assert_eq!(
            api_root(provider("openrouter").unwrap(), &settings).unwrap(),
            OPENROUTER_TTS_BASE_URL
        );
        assert_eq!(
            api_root(provider("openai").unwrap(), &settings).unwrap(),
            "https://api.openai.com/v1"
        );
        // The custom engine is the one that takes it.
        assert_eq!(
            api_root(provider("custom").unwrap(), &settings).unwrap(),
            "https://wrong.example/v1"
        );
    }

    #[test]
    fn parse_pcm_rate_reads_rate_param() {
        use super::parse_pcm_rate;
        assert_eq!(parse_pcm_rate("audio/L16;rate=24000"), Some(24_000));
        assert_eq!(parse_pcm_rate("audio/pcm; rate=16000"), Some(16_000));
        assert_eq!(parse_pcm_rate("AUDIO/L16;RATE=8000"), Some(8_000));
        assert_eq!(parse_pcm_rate("audio/pcm"), None);
        assert_eq!(parse_pcm_rate("audio/mpeg"), None);
    }

    #[test]
    fn maybe_wrap_pcm_wraps_only_raw_pcm() {
        use super::maybe_wrap_pcm;
        // mp3 (or any container) passes through untouched.
        let mp3 = vec![0xff, 0xfb, 0x90, 0x00];
        assert_eq!(maybe_wrap_pcm("audio/mpeg", mp3.clone()), mp3);
        // Raw pcm gets a WAV header prepended.
        let pcm = vec![0x00u8, 0x00, 0x00, 0x00];
        let wrapped = maybe_wrap_pcm("audio/pcm", pcm.clone());
        assert_eq!(&wrapped[0..4], b"RIFF");
        assert_eq!(wrapped.len(), 44 + pcm.len());
    }

    #[test]
    fn chunk_sanitizer_never_falls_back_to_raw_code() {
        // A streamed chunk can be nothing but a code block. The whole-reply
        // sanitizer keeps a raw fallback so a reply is spoken imperfectly rather
        // than not at all; for a chunk that fallback would read code aloud, so it
        // must return empty and let the caller skip it.
        let code_only = "```rust\nlet x = 1;\n```";
        assert_eq!(super::sanitize_for_speech_chunk(code_only), "");
        assert!(!super::sanitize_for_speech(code_only).is_empty());

        // Ordinary prose is treated identically by both.
        let prose = "This is a normal sentence.";
        assert_eq!(super::sanitize_for_speech_chunk(prose), prose);
        assert_eq!(super::sanitize_for_speech(prose), prose);
    }

    #[test]
    fn request_stitching_is_skipped_for_models_that_reject_it() {
        use super::elevenlabs_supports_stitching;
        // ElevenLabs documents Request Stitching as unavailable on eleven_v3.
        assert!(!elevenlabs_supports_stitching("eleven_v3"));
        assert!(!elevenlabs_supports_stitching("eleven_v3_alpha"));
        // Everything else supports it.
        assert!(elevenlabs_supports_stitching("eleven_flash_v2_5"));
        assert!(elevenlabs_supports_stitching("eleven_multilingual_v2"));
        assert!(elevenlabs_supports_stitching("eleven_turbo_v2_5"));
    }
}
