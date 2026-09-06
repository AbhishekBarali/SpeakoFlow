//! Cloud speech-to-text: transcription on a hosted API instead of a model on
//! the user's machine.
//!
//! The local path stays the default and the app's identity — this module is the
//! opt-in alternative for people who want a frontier ASR model without a
//! multi-gigabyte download or a GPU. Five services ship configured
//! ([`crate::settings::default_cloud_stt_providers`]) plus a free-form
//! OpenAI-compatible entry, and the whole thing is gated on
//! [`SttEngineMode::Cloud`]; on `Local` not a single line here runs.
//!
//! ## Three protocols, not one
//!
//! "OpenAI-compatible" is close to universal for *chat*, so it is tempting to
//! assume the same for transcription. It is not: ElevenLabs authenticates with
//! an `xi-api-key` header and names the model field `model_id`, and Deepgram
//! takes the audio as a raw request body with every option as a query
//! parameter and no multipart envelope at all. [`CloudSttKind`] is therefore a
//! real dispatch, while the provider *list* stays data.
//!
//! ## Why the requests run on their own thread
//!
//! [`crate::managers::transcription::TranscriptionManager::transcribe`] is a
//! synchronous function, and its callers are a mix: `actions.rs` calls it from
//! inside an async task, while the history and voice-conversation paths come in
//! through `spawn_blocking`. Neither `tauri::async_runtime::block_on` nor
//! `reqwest::blocking` is safe under both — driving a runtime from a thread that
//! already belongs to one panics. [`block_on_request`] sidesteps the question
//! entirely by moving the request to a fresh thread with its own current-thread
//! runtime, which is correct from any caller. One extra thread per dictation is
//! irrelevant next to the network round-trip it is waiting on.
//!
//! Live streaming lives in [`crate::stt_cloud_stream`]; this module is the batch
//! request, the credential check, and the model listing.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Cursor;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use log::{debug, info, warn};
use serde::Deserialize;

use crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE;
use crate::settings::{
    AppSettings, CloudSttKind, CloudSttProvider, CloudSttReadiness, CloudSttResolutionError,
    CloudSttUnavailableReason, ResolvedCloudStt, SttEngineMode,
};

/// Biasing hints are worth sending, but not without limit: ElevenLabs caps
/// keyterms at 50 for realtime and the OpenAI `prompt` field is a token budget
/// shared with the audio. A user with a 400-word custom dictionary should not
/// have every recording rejected for an oversized request.
const MAX_KEYTERMS: usize = 50;

/// Longest keyterm worth forwarding. Beyond this it is a sentence, not a term,
/// and ElevenLabs rejects it outright for realtime.
const MAX_KEYTERM_CHARS: usize = 50;

/// Resolve the active cloud transcription configuration, or explain why it
/// cannot run. Returns `Err` with [`CloudSttUnavailableReason::NotEnabled`] when
/// the user is on the local engine, so callers can treat "not configured" and
/// "not chosen" through one path.
pub(crate) fn resolve_cloud_stt(
    settings: &AppSettings,
) -> Result<ResolvedCloudStt, CloudSttResolutionError> {
    if settings.stt_engine_mode != SttEngineMode::Cloud {
        return Err(CloudSttResolutionError {
            reason: CloudSttUnavailableReason::NotEnabled,
            provider_id: None,
            provider_label: None,
        });
    }

    let provider = settings
        .cloud_stt_providers
        .iter()
        .find(|p| p.id == settings.cloud_stt_provider_id)
        .cloned()
        .ok_or_else(|| CloudSttResolutionError {
            reason: CloudSttUnavailableReason::SelectedProviderMissing,
            provider_id: Some(settings.cloud_stt_provider_id.clone()),
            provider_label: None,
        })?;

    let model = settings
        .cloud_stt_models
        .get(&provider.id)
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| provider.default_model.clone());
    if model.is_empty() {
        return Err(CloudSttResolutionError {
            reason: CloudSttUnavailableReason::NoModelConfigured,
            provider_id: Some(provider.id.clone()),
            provider_label: Some(provider.label.clone()),
        });
    }

    let api_key = settings
        .cloud_stt_api_keys
        .get(&provider.id)
        .map(|k| k.trim().to_string())
        .unwrap_or_default();
    // A self-hosted OpenAI-compatible server on localhost usually needs no key,
    // so an empty key is only fatal for the hosted services.
    if api_key.is_empty() && !allows_anonymous(&provider) {
        return Err(CloudSttResolutionError {
            reason: CloudSttUnavailableReason::MissingApiKey,
            provider_id: Some(provider.id.clone()),
            provider_label: Some(provider.label.clone()),
        });
    }

    let base_url = settings
        .cloud_stt_base_urls
        .get(&provider.id)
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty() && provider.allow_base_url_edit)
        .unwrap_or_else(|| provider.base_url.trim_end_matches('/').to_string());

    let language = if settings.selected_language == "auto" {
        None
    } else {
        Some(settings.selected_language.clone())
    };

    let keyterms = if settings.cloud_stt_send_custom_words && provider.honors_keyterms {
        crate::managers::transcription::recognition_words(settings)
            .into_iter()
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty() && w.chars().count() <= MAX_KEYTERM_CHARS)
            .take(MAX_KEYTERMS)
            .collect()
    } else {
        // Either the user turned biasing off, or this endpoint accepts the field
        // and discards it. Both mean "nothing was sent upstream", which is what
        // keeps the app's own fuzzy custom-word pass switched on.
        Vec::new()
    };

    Ok(ResolvedCloudStt {
        provider,
        model,
        base_url,
        api_key,
        language,
        keyterms,
        no_verbatim: settings.cloud_stt_no_verbatim,
        timeout_secs: settings.cloud_stt_timeout_secs.max(5),
    })
}

/// Whether this provider is usable without an API key. Only the user-editable
/// custom entry qualifies, and only because it is the one that can point at
/// `localhost`.
fn allows_anonymous(provider: &CloudSttProvider) -> bool {
    provider.allow_base_url_edit
}

/// True when the recording pipeline should route to a cloud provider. Both
/// halves matter: the user asked for cloud *and* the configuration is complete.
/// A half-configured cloud setup falls back to the local model rather than
/// failing the dictation.
pub fn cloud_stt_active(settings: &AppSettings) -> bool {
    resolve_cloud_stt(settings).is_ok()
}

/// Whether the next recording will use a realtime WebSocket rather than a batch
/// request once recording stops.
pub fn cloud_stt_streaming_active(settings: &AppSettings) -> bool {
    settings.cloud_stt_streaming
        && resolve_cloud_stt(settings)
            .map(|cfg| crate::stt_cloud_stream::supports_streaming(&cfg))
            .unwrap_or(false)
}

/// Configuration status for display in Settings.
pub fn cloud_stt_readiness(settings: &AppSettings) -> CloudSttReadiness {
    match resolve_cloud_stt(settings) {
        Ok(cfg) => CloudSttReadiness::Ready {
            provider_id: cfg.provider.id.clone(),
            provider_label: cfg.provider.label.clone(),
            model: cfg.model.clone(),
            streaming: settings.cloud_stt_streaming
                && crate::stt_cloud_stream::supports_streaming(&cfg),
        },
        Err(e) => CloudSttReadiness::Unavailable {
            reason: e.reason,
            provider_id: e.provider_id,
            provider_label: e.provider_label,
        },
    }
}

/// Encode 16 kHz mono `f32` samples as a 16-bit PCM WAV in memory.
///
/// Every provider here accepts WAV, and it is the only format reachable without
/// bundling an encoder: the capture pipeline hands over raw `f32` at
/// [`WHISPER_SAMPLE_RATE`], so this is a header plus a scale. Uncompressed costs
/// bandwidth (~32 kB/s) but no quality and no CPU, which is the right trade for
/// dictation-length audio.
pub fn encode_wav_16k_mono(samples: &[f32]) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buffer = Cursor::new(Vec::<u8>::new());
    {
        let mut writer = hound::WavWriter::new(&mut buffer, spec)
            .map_err(|e| format!("Failed to start WAV encoding: {e}"))?;
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            // i16::MIN is -32768 while i16::MAX is 32767, so scaling by 32767
            // keeps a full-scale negative sample from wrapping to positive.
            let value = (clamped * i16::MAX as f32) as i16;
            writer
                .write_sample(value)
                .map_err(|e| format!("Failed to encode WAV sample: {e}"))?;
        }
        writer
            .finalize()
            .map_err(|e| format!("Failed to finalize WAV encoding: {e}"))?;
    }
    Ok(buffer.into_inner())
}

/// Run one async request to completion from a synchronous caller.
///
/// See the module docs: this exists because the callers of the sync
/// `transcribe()` sit on both sides of the runtime boundary. A dedicated thread
/// with its own current-thread runtime is correct from either.
/// The one runtime every cloud request runs on, for the life of the process.
///
/// This used to be a fresh thread and a fresh current-thread runtime per request,
/// which quietly made connection reuse impossible: an idle keep-alive connection
/// belongs to the runtime whose I/O driver services it, so tearing the runtime
/// down after each request threw the connection away with it. Every dictation
/// therefore paid a full DNS + TCP + TLS handshake.
///
/// Measured against OpenRouter with a 5-round interleaved A/B (see
/// `stt_cloud_bench`), median 2.65 s on a fresh connection against 1.93 s on a
/// pooled one — **~715 ms, or 27%, of every dictation** — with all five pooled
/// samples beating their fresh counterpart. Worth noting what this does *not*
/// explain: the first request of a burst is slower still, and pre-opening a
/// socket does not recover that, so the remainder is the provider's own
/// routing/model warm-up plus round-trip distance rather than anything the client
/// controls.
///
/// A multi-threaded runtime (one worker is plenty) rather than a current-thread
/// one, because the worker keeps polling the I/O driver between requests. That is
/// what lets a pooled connection stay alive in the gap between one dictation and
/// the next, which is exactly the gap that matters.
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("cloud-stt")
                .enable_all()
                .build()
                .map_err(|e| warn!("Failed to start the cloud-STT runtime: {e}"))
                .ok()
        })
        .as_ref()
        .ok_or_else(|| "The cloud transcription runtime is unavailable".to_string())
}

/// Cached HTTP clients, keyed by everything that affects the connection.
///
/// A `reqwest::Client` owns the connection pool, so reusing it is the whole point
/// — a new client per request cannot reuse anything. Keyed rather than global
/// because the auth header and the timeout are baked in at build time.
fn client_cache() -> &'static Mutex<HashMap<u64, reqwest::Client>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, reqwest::Client>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drop every pooled client, so the next request pays a full handshake again.
///
/// Test-only, and it exists for one reason: the benefit of the recording-start
/// prewarm can only be measured from a genuinely cold pool, and every earlier
/// request in the same process leaves a warm one behind.
#[cfg(test)]
pub(crate) fn clear_client_cache() {
    if let Ok(mut cache) = client_cache().lock() {
        cache.clear();
    }
}

/// Run one async request to completion from a synchronous caller.
///
/// The future is spawned onto the shared [`runtime`] and awaited over a plain
/// channel, rather than driven with `block_on`. That distinction matters: the
/// callers of the sync `transcribe()` sit on both sides of the runtime boundary
/// (`actions.rs` calls it from inside an async task, history and
/// voice-conversation arrive via `spawn_blocking`), and `block_on` panics when
/// called from within a runtime. A spawn plus a channel receive is safe from
/// either, and keeps the long-lived runtime — and its warm connections — intact.
fn block_on_request<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let runtime = runtime()?;
    let (tx, rx) = std::sync::mpsc::channel();
    runtime.spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv()
        .map_err(|_| "The cloud transcription task was dropped".to_string())?
}

/// Open a connection to the configured provider so the handshake happens while
/// the user is still speaking instead of after they stop.
///
/// Called at recording start (see `actions.rs`), mirroring how the app already
/// prewarms the local cleanup model. The request itself is meaningless — a `HEAD`
/// at the origin, unauthenticated, result discarded — because the connection pool
/// is keyed by scheme/host/port, so any request to the right origin leaves a warm
/// TLS session behind for the real one. Fire-and-forget: a failure here costs
/// nothing, the transcription just pays the handshake as before.
pub(crate) fn prewarm_cloud_stt(settings: &AppSettings) {
    let Ok(cfg) = resolve_cloud_stt(settings) else {
        return;
    };
    // Realtime streaming opens its own socket at recording start already.
    if settings.cloud_stt_streaming && crate::stt_cloud_stream::supports_streaming(&cfg) {
        return;
    }
    let request = CloudRequest::from_config(&cfg);
    let Ok(runtime) = runtime() else { return };
    // Prefer an authenticated GET against the provider's own API path over an
    // anonymous HEAD at the origin: an edge can serve the two from different
    // places, and only a connection opened to the path the transcription will use
    // is certain to be the one it reuses.
    let warm_url = match cfg.provider.models_endpoint.as_deref() {
        Some(endpoint) => format!("{}{}", cfg.base_url, endpoint),
        None => cfg.base_url.clone(),
    };
    let authenticated = cfg.provider.models_endpoint.is_some();
    runtime.spawn(async move {
        let Ok(client) = request.client() else { return };
        let started = std::time::Instant::now();
        let builder = if authenticated {
            client.get(&warm_url).bearer_auth(&request.api_key)
        } else {
            client.head(&warm_url)
        };
        match builder.timeout(Duration::from_secs(5)).send().await {
            Ok(response) => {
                // The body has to be drained for the connection to go back in the
                // pool; an abandoned response is a closed connection.
                let status = response.status();
                let bytes = response.bytes().await.map(|b| b.len()).unwrap_or(0);
                debug!(
                    "Cloud STT connection warmed in {:?} ({status}, {bytes}B, {warm_url})",
                    started.elapsed()
                );
            }
            Err(e) => debug!("Cloud STT prewarm skipped ({warm_url}): {e}"),
        }
    });
}

/// Transcribe a finished recording through the configured cloud provider,
/// blocking until the text is back. This is the batch path — the fallback
/// whenever realtime streaming is off, unsupported, or has failed.
pub(crate) fn transcribe_cloud_blocking(
    cfg: &ResolvedCloudStt,
    samples: &[f32],
) -> Result<String, String> {
    let wav = encode_wav_16k_mono(samples)?;
    let seconds = samples.len() as f32 / WHISPER_SAMPLE_RATE as f32;
    info!(
        "Cloud transcription: {} / {} ({:.1}s, {} kB)",
        cfg.provider.label,
        cfg.model,
        seconds,
        wav.len() / 1024
    );
    let request = CloudRequest::from_config(cfg);
    block_on_request(async move { request.transcribe(wav).await })
}

/// Everything one cloud request needs, owned so it can cross a thread boundary
/// into [`block_on_request`] without borrowing the settings snapshot.
#[derive(Clone)]
struct CloudRequest {
    kind: CloudSttKind,
    label: String,
    base_url: String,
    api_key: String,
    model: String,
    language: Option<String>,
    keyterms: Vec<String>,
    no_verbatim: bool,
    timeout: Duration,
}

impl CloudRequest {
    fn from_config(cfg: &ResolvedCloudStt) -> Self {
        Self {
            kind: cfg.provider.kind,
            label: cfg.provider.label.clone(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            language: cfg.language.clone(),
            keyterms: cfg.keyterms.clone(),
            no_verbatim: cfg.no_verbatim,
            timeout: Duration::from_secs(cfg.timeout_secs),
        }
    }

    /// A pooled client for this endpoint, built once and reused.
    ///
    /// The pool settings are the point of the whole exercise: an idle connection
    /// has to still be there when the user starts their *next* dictation, which
    /// may be a minute later, so the idle timeout is generous and TCP keep-alive
    /// is on to stop a NAT or a load balancer quietly dropping it in between.
    fn client(&self) -> Result<reqwest::Client, String> {
        let key = self.cache_key();
        if let Ok(cache) = client_cache().lock() {
            if let Some(client) = cache.get(&key) {
                return Ok(client.clone());
            }
        }

        // OpenRouter reads `HTTP-Referer` and `X-Title` for app attribution;
        // every other endpoint here ignores both, so they are set unconditionally
        // rather than special-cased. Mirrors `llm_client::build_headers`.
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "HTTP-Referer",
            reqwest::header::HeaderValue::from_static(
                "https://github.com/AbhishekBarali/SpeakoFlow",
            ),
        );
        headers.insert(
            "X-Title",
            reqwest::header::HeaderValue::from_static("SpeakoFlow"),
        );
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .default_headers(headers)
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(2)
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build the HTTP client: {e}"))?;

        if let Ok(mut cache) = client_cache().lock() {
            cache.insert(key, client.clone());
        }
        Ok(client)
    }

    /// Everything that changes the connection or its baked-in headers. The API
    /// key is hashed, never stored in the key.
    fn cache_key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.base_url.hash(&mut hasher);
        self.api_key.hash(&mut hasher);
        self.timeout.hash(&mut hasher);
        hasher.finish()
    }

    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, String> {
        match self.kind {
            CloudSttKind::ElevenLabs => self.transcribe_elevenlabs(wav).await,
            CloudSttKind::OpenAiCompatible => self.transcribe_openai(wav).await,
            CloudSttKind::Deepgram => self.transcribe_deepgram(wav).await,
        }
    }

    /// `POST /v1/speech-to-text` — multipart, `xi-api-key`, `model_id`.
    ///
    /// The realtime model id is rejected by this endpoint, so a user who picked
    /// `scribe_v2_realtime` and then hit a batch fallback would get a 4xx
    /// instead of their words; [`batch_model_id`] maps it back to `scribe_v2`.
    async fn transcribe_elevenlabs(&self, wav: Vec<u8>) -> Result<String, String> {
        let url = format!("{}/v1/speech-to-text", self.base_url);
        let part = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| format!("Failed to attach audio: {e}"))?;
        let mut form = reqwest::multipart::Form::new()
            .text("model_id", batch_model_id(&self.model))
            .part("file", part);
        if let Some(language) = &self.language {
            form = form.text("language_code", language.clone());
        }
        if self.no_verbatim {
            form = form.text("no_verbatim", "true");
        }
        if !self.keyterms.is_empty() {
            // Repeated multipart fields are how this endpoint takes a list.
            for term in &self.keyterms {
                form = form.text("keyterms", term.clone());
            }
        }

        let response = self
            .client()?
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| self.network_error(e))?;

        #[derive(Deserialize)]
        struct ScribeResponse {
            #[serde(default)]
            text: String,
            #[serde(default)]
            language_code: Option<String>,
        }
        let parsed: ScribeResponse = self.parse_json(response).await?;
        if let Some(language) = parsed.language_code {
            debug!("ElevenLabs detected language: {language}");
        }
        Ok(parsed.text)
    }

    /// `POST /audio/transcriptions` — the OpenAI schema, shared by Groq,
    /// Mistral, and self-hosted servers.
    async fn transcribe_openai(&self, wav: Vec<u8>) -> Result<String, String> {
        let url = format!("{}/audio/transcriptions", self.base_url);
        let part = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| format!("Failed to attach audio: {e}"))?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .part("file", part);
        if let Some(language) = &self.language {
            // This schema wants a bare ISO-639-1 code.
            form = form.text("language", language.clone());
        }
        if !self.keyterms.is_empty() {
            // No keyterm field here; the documented way to bias this family is
            // the free-text prompt, which is read as context for the audio.
            form = form.text("prompt", self.keyterms.join(", "));
        }

        let mut request = self.client()?.post(&url).multipart(form);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        let response = request.send().await.map_err(|e| self.network_error(e))?;

        #[derive(Deserialize)]
        struct OpenAiResponse {
            #[serde(default)]
            text: String,
        }
        let parsed: OpenAiResponse = self.parse_json(response).await?;
        Ok(parsed.text)
    }

    /// `POST /v1/listen` — raw audio body, `Authorization: Token`, options as
    /// query parameters.
    async fn transcribe_deepgram(&self, wav: Vec<u8>) -> Result<String, String> {
        let url = format!("{}/v1/listen", self.base_url);
        let mut query: Vec<(String, String)> = vec![
            ("model".to_string(), self.model.clone()),
            // Punctuation and capitalisation are off by default here, which
            // produces an unusable wall of lowercase text for dictation.
            ("smart_format".to_string(), "true".to_string()),
            ("punctuate".to_string(), "true".to_string()),
        ];
        if let Some(language) = &self.language {
            query.push(("language".to_string(), language.clone()));
        }
        if self.no_verbatim {
            query.push(("filler_words".to_string(), "false".to_string()));
        }
        for term in &self.keyterms {
            query.push(("keyterm".to_string(), term.clone()));
        }

        let response = self
            .client()?
            .post(&url)
            .query(&query)
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Content-Type", "audio/wav")
            .body(wav)
            .send()
            .await
            .map_err(|e| self.network_error(e))?;

        #[derive(Deserialize)]
        struct DeepgramResponse {
            results: Option<DeepgramResults>,
        }
        #[derive(Deserialize)]
        struct DeepgramResults {
            #[serde(default)]
            channels: Vec<DeepgramChannel>,
        }
        #[derive(Deserialize)]
        struct DeepgramChannel {
            #[serde(default)]
            alternatives: Vec<DeepgramAlternative>,
        }
        #[derive(Deserialize)]
        struct DeepgramAlternative {
            #[serde(default)]
            transcript: String,
        }
        let parsed: DeepgramResponse = self.parse_json(response).await?;
        Ok(parsed
            .results
            .and_then(|r| r.channels.into_iter().next())
            .and_then(|c| c.alternatives.into_iter().next())
            .map(|a| a.transcript)
            .unwrap_or_default())
    }

    /// Turn a non-2xx response into a message worth showing a user, then decode
    /// the body. Provider error bodies are the only place that distinguishes
    /// "wrong key" from "out of credit" from "no such model", so they are worth
    /// surfacing rather than reporting a bare status code.
    async fn parse_json<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, String> {
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("{} returned an unreadable response: {e}", self.label))?;
        if !status.is_success() {
            return Err(format!(
                "{} returned {}: {}",
                self.label,
                status,
                summarize_error_body(&body)
            ));
        }
        serde_json::from_str(&body).map_err(|e| {
            warn!("Cloud transcription: unparsable success body: {body}");
            format!("{} returned an unexpected response: {e}", self.label)
        })
    }

    fn network_error(&self, e: reqwest::Error) -> String {
        if e.is_timeout() {
            format!(
                "{} did not respond within {}s",
                self.label,
                self.timeout.as_secs()
            )
        } else if e.is_connect() {
            format!("Could not reach {}: {e}", self.label)
        } else {
            format!("{} request failed: {e}", self.label)
        }
    }
}

/// Map a realtime-only model id onto its batch sibling.
///
/// The two ElevenLabs endpoints do not share a model namespace:
/// `scribe_v2_realtime` exists only on the WebSocket. Silently correcting it
/// here means the batch fallback still works for a user configured for
/// streaming, which is exactly when the fallback matters.
fn batch_model_id(model: &str) -> String {
    match model.trim() {
        "scribe_v2_realtime" => {
            // Worth a log line, not a silent substitution: the two models do not
            // produce identical text, so a recording that fell back to batch
            // reads slightly differently from the one before it. When comparing
            // outputs, this is the line that explains the difference.
            info!("Cloud transcription: scribe_v2_realtime has no batch endpoint; using scribe_v2");
            "scribe_v2".to_string()
        }
        other => other.to_string(),
    }
}

/// Pull a human-readable message out of a provider error body, falling back to
/// a truncated raw body. Providers disagree on the envelope
/// (`{"detail": {"message"}}`, `{"error": {"message"}}`, `{"err_msg"}`), so this
/// probes the known shapes rather than modelling each one.
fn summarize_error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty response)".to_string();
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        for pointer in [
            "/detail/message",
            "/error/message",
            "/message",
            "/err_msg",
            "/detail",
            "/error",
        ] {
            if let Some(text) = value.pointer(pointer).and_then(|v| v.as_str()) {
                if !text.trim().is_empty() {
                    return text.trim().to_string();
                }
            }
        }
        // ElevenLabs validation errors arrive as `{"detail": {"status": ...}}`.
        if let Some(status) = value.pointer("/detail/status").and_then(|v| v.as_str()) {
            return status.trim().to_string();
        }
    }
    let mut summary: String = trimmed.chars().take(300).collect();
    if trimmed.chars().count() > 300 {
        summary.push('…');
    }
    summary
}

/// Verify a provider's credentials and model without spending a real dictation.
///
/// This transcribes one second of near-silence, which is the only check that
/// proves the whole path: a plain key probe would pass for a key that is valid
/// but has no speech-to-text permission, no credit, or a model id this endpoint
/// does not know. Empty text is a pass — the request being accepted is the
/// signal, not what a second of silence says.
pub(crate) fn verify_cloud_stt(cfg: &ResolvedCloudStt) -> Result<String, String> {
    let samples = vec![0.0f32; WHISPER_SAMPLE_RATE as usize];
    let wav = encode_wav_16k_mono(&samples)?;
    let request = CloudRequest::from_config(cfg);
    let label = cfg.provider.label.clone();
    let model = cfg.model.clone();
    let text = block_on_request(async move { request.transcribe(wav).await })?;
    debug!("Cloud STT verification transcript: {text:?}");
    Ok(format!("{label} responded — {model} is reachable."))
}

/// Ask a provider which models it offers, for the Settings picker.
///
/// Only providers that publish a listing endpoint are queried
/// ([`CloudSttProvider::models_endpoint`]); ElevenLabs and Deepgram ship fixed
/// model names in the registry, which is more accurate than an API call would be.
///
/// The response is filtered to plausible transcription ids because a generic
/// `/models` mixes chat and audio models. OpenRouter is the exception that proves
/// the field is needed: it excludes transcription models from the default catalog
/// entirely and only returns them for `?output_modalities=transcription`, so a
/// bare `/models` there would list hundreds of chat models and no usable one.
pub(crate) fn list_cloud_stt_models(cfg: &ResolvedCloudStt) -> Result<Vec<String>, String> {
    let Some(endpoint) = cfg.provider.models_endpoint.clone() else {
        return Ok(cfg.provider.models.clone());
    };

    let url = format!("{}{}", cfg.base_url, endpoint);
    let api_key = cfg.api_key.clone();
    let label = cfg.provider.label.clone();
    let timeout = Duration::from_secs(cfg.timeout_secs.min(20));
    let fallback = cfg.provider.models.clone();
    // A listing already scoped to transcription needs no name heuristic, and
    // applying one would drop correctly-listed models whose ids say nothing about
    // the task.
    let prescoped = endpoint.contains("output_modalities=transcription");

    let listed = block_on_request(async move {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("Failed to build the HTTP client: {e}"))?;
        let mut request = client.get(&url);
        if !api_key.is_empty() {
            request = request.bearer_auth(&api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("Could not reach {label}: {e}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("{label} returned an unreadable response: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "{label} returned {status}: {}",
                summarize_error_body(&body)
            ));
        }

        #[derive(Deserialize)]
        struct ModelList {
            #[serde(default)]
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            #[serde(default)]
            id: String,
        }
        let parsed: ModelList = serde_json::from_str(&body)
            .map_err(|e| format!("{label} returned an unexpected model list: {e}"))?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect::<Vec<_>>())
    })?;

    let mut models: Vec<String> = listed
        .into_iter()
        .filter(|id| prescoped || looks_like_transcription_model(id))
        .collect();
    models.sort();
    models.dedup();
    // An endpoint that lists only chat models leaves the picker empty, which
    // reads as a failure; the shipped defaults are a better answer.
    if models.is_empty() {
        return Ok(fallback);
    }
    Ok(models)
}

/// Whether a model id from a generic `/models` listing plausibly transcribes.
/// Deliberately permissive — the field stays editable, so a false negative is
/// only a missing suggestion, while listing every chat model would bury the two
/// entries the user wants.
fn looks_like_transcription_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    ["whisper", "transcribe", "scribe", "voxtral", "stt", "nova"]
        .iter()
        .any(|needle| id.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{get_default_settings, SttEngineMode};

    fn cloud_settings() -> AppSettings {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings
            .cloud_stt_api_keys
            .insert("elevenlabs".to_string(), "test-key".to_string());
        settings
    }

    #[test]
    fn local_mode_is_never_cloud_active() {
        let settings = get_default_settings();
        assert!(!cloud_stt_active(&settings));
        assert!(matches!(
            resolve_cloud_stt(&settings),
            Err(CloudSttResolutionError {
                reason: CloudSttUnavailableReason::NotEnabled,
                ..
            })
        ));
    }

    #[test]
    fn cloud_mode_without_a_key_is_unavailable_not_active() {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        // Falls back to the local model rather than failing the dictation.
        assert!(!cloud_stt_active(&settings));
        assert!(matches!(
            resolve_cloud_stt(&settings),
            Err(CloudSttResolutionError {
                reason: CloudSttUnavailableReason::MissingApiKey,
                ..
            })
        ));
    }

    #[test]
    fn resolves_the_default_provider_and_model() {
        let settings = cloud_settings();
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert_eq!(cfg.provider.id, "elevenlabs");
        assert_eq!(cfg.model, "scribe_v2");
        assert_eq!(cfg.api_key, "test-key");
        assert_eq!(cfg.base_url, "https://api.elevenlabs.io");
        // "auto" means let the provider detect it.
        assert!(cfg.language.is_none());
    }

    #[test]
    fn custom_provider_needs_no_key() {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.cloud_stt_provider_id = "custom".to_string();
        assert!(resolve_cloud_stt(&settings).is_ok());
    }

    #[test]
    fn custom_base_url_override_applies_only_where_editing_is_allowed() {
        let mut settings = cloud_settings();
        settings
            .cloud_stt_base_urls
            .insert("elevenlabs".to_string(), "http://evil.local".to_string());
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert_eq!(cfg.base_url, "https://api.elevenlabs.io");

        settings.cloud_stt_provider_id = "custom".to_string();
        settings.cloud_stt_base_urls.insert(
            "custom".to_string(),
            "http://localhost:9000/v1/".to_string(),
        );
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert_eq!(cfg.base_url, "http://localhost:9000/v1");
    }

    #[test]
    fn keyterms_come_from_custom_words_and_are_bounded() {
        let mut settings = cloud_settings();
        settings.custom_words = (0..80).map(|i| format!("term{i}")).collect();
        settings.custom_words.push("x".repeat(200));
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert_eq!(cfg.keyterms.len(), MAX_KEYTERMS);
        assert!(cfg.keyterms.iter().all(|t| t.chars().count() <= 50));

        settings.cloud_stt_send_custom_words = false;
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert!(cfg.keyterms.is_empty());
    }

    #[test]
    fn realtime_model_maps_to_its_batch_sibling() {
        assert_eq!(batch_model_id("scribe_v2_realtime"), "scribe_v2");
        assert_eq!(batch_model_id("scribe_v2"), "scribe_v2");
        assert_eq!(batch_model_id("whisper-1"), "whisper-1");
    }

    fn provider(id: &str) -> crate::settings::CloudSttProvider {
        crate::settings::default_cloud_stt_providers()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("{id} should ship in the registry"))
    }

    #[test]
    fn openrouter_speaks_the_openai_multipart_shape() {
        let openrouter = provider("openrouter");
        assert_eq!(openrouter.kind, CloudSttKind::OpenAiCompatible);
        assert_eq!(openrouter.base_url, "https://openrouter.ai/api/v1");
        // Slugs are namespaced `vendor/model` and must not be rewritten.
        assert!(openrouter.default_model.contains('/'));
        // No realtime endpoint on this route.
        assert!(!openrouter.supports_streaming);
    }

    /// OpenRouter documents `prompt` as accepted-but-ignored on its multipart
    /// route, so claiming upstream biasing would silently disable both the
    /// provider's correction and the app's own.
    #[test]
    fn openrouter_does_not_claim_keyterm_support() {
        assert!(!provider("openrouter").honors_keyterms);
        for id in ["elevenlabs", "openai", "groq", "deepgram", "mistral"] {
            assert!(provider(id).honors_keyterms, "{id} should honor keyterms");
        }
    }

    #[test]
    fn a_provider_that_ignores_hints_keeps_the_local_word_pass() {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.custom_words = vec!["SpeakoFlow".to_string(), "Kiro".to_string()];
        settings
            .cloud_stt_api_keys
            .insert("openrouter".to_string(), "key".to_string());
        settings.cloud_stt_provider_id = "openrouter".to_string();

        // Empty keyterms is the signal the pipeline reads as "nothing was biased
        // upstream", which is what keeps the fuzzy pass running.
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert!(cfg.keyterms.is_empty());

        // The same words do reach a provider that uses them.
        settings.cloud_stt_provider_id = "elevenlabs".to_string();
        settings
            .cloud_stt_api_keys
            .insert("elevenlabs".to_string(), "key".to_string());
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert!(cfg.keyterms.contains(&"Kiro".to_string()));
    }

    /// OpenRouter hides transcription models from the default catalog, so the
    /// listing path has to carry the scoping query.
    #[test]
    fn openrouter_model_discovery_is_scoped_to_transcription() {
        let endpoint = provider("openrouter")
            .models_endpoint
            .expect("openrouter should publish a listing endpoint");
        assert!(endpoint.contains("output_modalities=transcription"));
        // Fixed-catalog providers must not be queried at all.
        assert!(provider("elevenlabs").models_endpoint.is_none());
        assert!(provider("deepgram").models_endpoint.is_none());
    }

    /// An upgraded store must end up with the same provider order as a fresh
    /// install, not with the new entry appended after "Custom".
    #[test]
    fn migration_restores_the_shipped_provider_order() {
        let mut settings = get_default_settings();
        // Simulate a store written before OpenRouter existed.
        settings
            .cloud_stt_providers
            .retain(|p| p.id != "openrouter");
        settings.cloud_stt_models.remove("openrouter");
        settings.cloud_stt_api_keys.remove("openrouter");

        assert!(crate::settings::ensure_cloud_stt_defaults(&mut settings));

        let ids: Vec<&str> = settings
            .cloud_stt_providers
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        let expected: Vec<String> = crate::settings::default_cloud_stt_providers()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, expected.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        assert_eq!(ids.last(), Some(&"custom"), "custom stays last");
        // And the backfilled slots exist.
        assert_eq!(
            settings
                .cloud_stt_models
                .get("openrouter")
                .map(|m| m.as_str()),
            Some("openai/whisper-large-v3")
        );
        assert!(settings.cloud_stt_api_keys.contains_key("openrouter"));

        // Idempotent: a second pass changes nothing.
        assert!(!crate::settings::ensure_cloud_stt_defaults(&mut settings));
    }

    #[test]
    fn wav_encoding_is_a_44_byte_header_plus_16_bit_samples() {
        let wav = encode_wav_16k_mono(&[0.0, 0.5, -0.5]).expect("should encode");
        assert_eq!(wav.len(), 44 + 3 * 2);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn full_scale_negative_samples_do_not_wrap() {
        let wav = encode_wav_16k_mono(&[-1.0, -2.0]).expect("should encode");
        let first = i16::from_le_bytes([wav[44], wav[45]]);
        let clamped = i16::from_le_bytes([wav[46], wav[47]]);
        assert_eq!(first, -i16::MAX);
        assert_eq!(clamped, -i16::MAX);
    }

    #[test]
    fn error_bodies_are_reduced_to_their_message() {
        assert_eq!(
            summarize_error_body(r#"{"detail":{"message":"Invalid API key"}}"#),
            "Invalid API key"
        );
        assert_eq!(
            summarize_error_body(r#"{"error":{"message":"model_not_found"}}"#),
            "model_not_found"
        );
        assert_eq!(summarize_error_body(""), "(empty response)");
        assert_eq!(summarize_error_body("plain failure"), "plain failure");
    }

    #[test]
    fn model_filter_keeps_transcription_ids_and_drops_chat_ids() {
        assert!(looks_like_transcription_model("whisper-large-v3-turbo"));
        assert!(looks_like_transcription_model("gpt-4o-transcribe"));
        assert!(looks_like_transcription_model("voxtral-mini-latest"));
        assert!(!looks_like_transcription_model("gpt-4o"));
        assert!(!looks_like_transcription_model("llama-3.3-70b-versatile"));
    }
}
