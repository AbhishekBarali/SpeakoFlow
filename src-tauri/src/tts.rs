//! Remote TTS engines for the assistant's spoken summaries.
//!
//! Two engines are handled here in Rust (audio fetched and played natively
//! via rodio, so playback works even when the panel webview is hidden):
//! - "openai": any OpenAI-compatible `/audio/speech` endpoint — OpenAI,
//!   Azure OpenAI (`https://{res}.openai.azure.com/openai/v1` or
//!   `cognitiveservices.azure.com/openai/v1`, model = deployment name),
//!   Groq, LocalAI, Kokoro-FastAPI, openai-edge-tts, etc.
//! - "elevenlabs": ElevenLabs `text-to-speech/{voice_id}` API.
//! - "azure": Azure AI Speech (Neural TTS) `cognitiveservices/v1` SSML API —
//!   base URL is the regional TTS endpoint
//!   (`https://{region}.tts.speech.microsoft.com`), auth via the
//!   `Ocp-Apim-Subscription-Key` header, voice = a neural voice name such as
//!   `en-US-JennyNeural`. This is distinct from Azure OpenAI (use "openai"
//!   for `*.openai.azure.com` / `*.cognitiveservices.azure.com/openai/v1`).
//!
//! The "kokoro" engine runs fully locally in the panel webview
//! (kokoro-js, WebGPU) and never reaches this module.

use crate::settings::AppSettings;
use log::{debug, error};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::AppHandle;

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

/// Cancel any in-flight or queued remote TTS: native playback stops within
/// ~50ms and any superseded request aborts before it can play.
pub fn stop_remote() {
    PLAYBACK_EPOCH.fetch_add(1, Ordering::SeqCst);
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

    let result = match settings.assistant_tts_engine.as_str() {
        "openai" => fetch_openai_speech(settings, &text).await,
        "elevenlabs" => fetch_elevenlabs_speech(settings, &text).await,
        "azure" => fetch_azure_speech(settings, &text).await,
        other => Err(format!("Unknown TTS engine: {}", other)),
    };

    match result {
        Ok(audio_bytes) => {
            // Cancelled while the audio was being fetched?
            if current_epoch() != epoch {
                debug!("TTS request superseded during fetch; not playing");
                return;
            }
            debug!("TTS audio fetched: {} KB", audio_bytes.len() / 1024);
            let volume = settings.audio_feedback_volume;
            let device = settings.selected_output_device.clone();
            // Let the panel know audio is playing so it can show a Stop button
            // even though the turn itself is already idle.
            use tauri::Emitter;
            let _ = app.emit("assistant-tts-playing", true);
            // rodio playback blocks; run it off the async runtime.
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = play_audio_bytes(audio_bytes, device, volume, epoch) {
                    error!("TTS playback failed: {}", e);
                }
            })
            .await;
            let _ = app.emit("assistant-tts-playing", false);
        }
        Err(e) => {
            error!("TTS request failed: {}", e);
            use tauri::Emitter;
            let _ = app.emit("assistant-error", format!("TTS failed: {}", e));
        }
    }
}

/// HTTP client for remote TTS. Forces HTTP/1.1 — some hosted TTS gateways/
/// proxies emit "upstream connect error / reset before headers / protocol
/// error" during HTTP/2 negotiation (the same reason the LLM client pins h1) —
/// and sets connect/overall timeouts so a stalled upstream can't wedge playback.
fn tts_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .http1_only()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build TTS HTTP client: {}", e))
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

/// POST {base}/audio/speech — OpenAI-compatible shape.
///
/// If the configured base URL already contains `/audio/speech`, it is used
/// verbatim (matching SillyTavern's "Provider Endpoint" behaviour). This lets
/// users paste a full Azure endpoint such as
/// `https://{res}.cognitiveservices.azure.com/openai/deployments/{dep}/audio/speech?api-version=2025-03-01-preview`,
/// including the `?api-version=` query string, which a base-plus-suffix scheme
/// cannot express.
async fn fetch_openai_speech(settings: &AppSettings, text: &str) -> Result<Vec<u8>, String> {
    let raw = settings.assistant_tts_base_url.trim();
    let url = if raw.contains("/audio/speech") {
        raw.to_string()
    } else {
        format!("{}/audio/speech", raw.trim_end_matches('/'))
    };

    let client = tts_client()?;
    // OpenAI-compatible `speed` (0.25x–4x). Pitch is preserved by the service.
    let speed = settings.assistant_tts_speed.clamp(0.25, 4.0);
    let mut request = client.post(&url).json(&serde_json::json!({
        "model": settings.assistant_tts_model,
        "input": text,
        "voice": settings.assistant_tts_remote_voice,
        "response_format": "mp3",
        "speed": speed,
    }));

    let api_key = settings.assistant_tts_api_key.0.trim();
    if !api_key.is_empty() {
        // Bearer covers OpenAI, Groq, and Azure's v1 API; the `api-key` header
        // covers classic Azure OpenAI deployment endpoints. Sending both is
        // harmless — endpoints ignore the header they don't use.
        request = request.bearer_auth(api_key).header("api-key", api_key);
    }

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

/// POST https://api.elevenlabs.io/v1/text-to-speech/{voice_id}
async fn fetch_elevenlabs_speech(settings: &AppSettings, text: &str) -> Result<Vec<u8>, String> {
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
    let request = client
        .post(&url)
        .header("xi-api-key", settings.assistant_tts_api_key.0.trim())
        .json(&body);
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
    match settings.assistant_tts_engine.as_str() {
        "openai" => list_openai_tts_voices(settings).await,
        "elevenlabs" => list_elevenlabs_voices(settings).await,
        "azure" => {
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
        other => Err(format!("Voice listing isn't supported for engine: {}", other)),
    }
}

/// OpenAI-compatible voices. Local servers (Kokoro-FastAPI, openai-edge-tts)
/// expose `GET {base}/audio/voices`; OpenAI proper does not, so we fall back to
/// the known built-in voice set. Never errors — a failed lookup degrades to the
/// built-in list so the picker is always usable.
async fn list_openai_tts_voices(settings: &AppSettings) -> Result<Vec<TtsVoice>, String> {
    let builtin = || -> Vec<TtsVoice> {
        OPENAI_TTS_VOICES
            .iter()
            .map(|v| TtsVoice {
                id: v.to_string(),
                label: v.to_string(),
            })
            .collect()
    };

    let raw = settings.assistant_tts_base_url.trim();
    if raw.is_empty() || raw.contains("api.openai.com") {
        return Ok(builtin());
    }

    // Derive an `/audio/voices` URL from the configured base (which may already
    // point straight at `/audio/speech`, possibly with a query string).
    let base = raw.trim_end_matches('/');
    let voices_url = match base.split_once("/audio/speech") {
        Some((prefix, _)) => format!("{}/audio/voices", prefix.trim_end_matches('/')),
        None => format!("{}/audio/voices", base),
    };

    let client = tts_client()?;
    let mut req = client.get(&voices_url);
    let api_key = settings.assistant_tts_api_key.0.trim();
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key).header("api-key", api_key);
    }

    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return Ok(builtin()),
    };
    let value: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return Ok(builtin()),
    };

    // Accept several shapes: {voices:[...]} / {data:[...]} / top-level array,
    // where each item is a bare string or an object with an id/name.
    let arr = value
        .get("voices")
        .and_then(|v| v.as_array())
        .or_else(|| value.get("data").and_then(|v| v.as_array()))
        .or_else(|| value.as_array());

    let mut voices = Vec::new();
    if let Some(items) = arr {
        for item in items {
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
                let label = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(id)
                    .to_string();
                voices.push(TtsVoice {
                    id: id.to_string(),
                    label,
                });
            }
        }
    }

    if voices.is_empty() {
        return Ok(builtin());
    }
    Ok(voices)
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
/// model picker. Only the OpenAI-compatible and ElevenLabs engines expose a
/// model list; Azure/Kokoro return an error the UI surfaces as "not supported".
pub async fn list_tts_models(settings: &AppSettings) -> Result<Vec<String>, String> {
    match settings.assistant_tts_engine.as_str() {
        "openai" => list_openai_tts_models(settings).await,
        "elevenlabs" => list_elevenlabs_models(settings).await,
        other => Err(format!("Model listing isn't supported for engine: {}", other)),
    }
}

/// OpenAI-compatible models via `GET {base}/models`.
async fn list_openai_tts_models(settings: &AppSettings) -> Result<Vec<String>, String> {
    let raw = settings.assistant_tts_base_url.trim();
    let base = if raw.is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        let b = raw.trim_end_matches('/');
        match b.split_once("/audio/speech") {
            Some((prefix, _)) => prefix.trim_end_matches('/').to_string(),
            None => b.to_string(),
        }
    };
    let url = format!("{}/models", base);

    let client = tts_client()?;
    let mut req = client.get(&url);
    let api_key = settings.assistant_tts_api_key.0.trim();
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key).header("api-key", api_key);
    }

    let resp = req
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
    if let Some(items) = value.get("data").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                models.push(id.to_string());
            }
        }
    } else if let Some(items) = value.as_array() {
        for item in items {
            if let Some(id) = item.as_str() {
                models.push(id.to_string());
            }
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
    let prefix: Vec<&str> = voice.splitn(3, '-').take(2).collect();
    let lang = if prefix.len() == 2 {
        format!("{}-{}", prefix[0], prefix[1])
    } else {
        "en-US".to_string()
    };

    // Apply playback speed via SSML <prosody rate>. Azure takes a relative
    // percentage (e.g. +100% ≈ 2x, -50% ≈ 0.5x) and preserves pitch. Wrap only
    // when the rate actually differs from normal.
    let escaped_text = xml_escape(text);
    let inner = if (settings.assistant_tts_speed - 1.0).abs() > f64::EPSILON {
        let rate = format!("{:+.0}%", (settings.assistant_tts_speed - 1.0) * 100.0);
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
    let audio_bytes = match settings.assistant_tts_engine.as_str() {
        "openai" => fetch_openai_speech(settings, &text).await?,
        "elevenlabs" => fetch_elevenlabs_speech(settings, &text).await?,
        "azure" => fetch_azure_speech(settings, &text).await?,
        other => return Err(format!("Unknown TTS engine: {}", other)),
    };
    let volume = settings.audio_feedback_volume;
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

/// Decode and play audio bytes (mp3/wav/ogg) on the selected output device.
/// Polls the playback epoch so a `stop_remote()` cancels playback promptly.
fn play_audio_bytes(
    bytes: Vec<u8>,
    selected_device: Option<String>,
    volume: f32,
    epoch: u64,
) -> Result<(), Box<dyn std::error::Error>> {
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

    let stream_handle = stream_builder.open_stream()?;
    let sink = rodio::play(stream_handle.mixer(), Cursor::new(bytes))?;
    sink.set_volume(volume.max(0.1));

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
    // Fenced code blocks (``` … ``` or ~~~ … ~~~), including the info string.
    static FENCED_CODE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?s)```[^\n]*\n?.*?```|~~~[^\n]*\n?.*?~~~").unwrap());
    // Images first (drop alt + url), then links (keep the visible text).
    static IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap());
    static LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());
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
        if input.chars().any(|c| c.is_alphanumeric()) {
            return WS.replace_all(input.trim(), " ").into_owned();
        }
        return String::new();
    }
    cleaned.to_string()
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_speech;

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
}
