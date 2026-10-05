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
//! ## Four protocols, not one
//!
//! "OpenAI-compatible" is close to universal for *chat*, so it is tempting to
//! assume the same for transcription. It is not: ElevenLabs authenticates with
//! an `xi-api-key` header and names the model field `model_id`, Deepgram
//! takes the audio as a raw request body with every option as a query
//! parameter and no multipart envelope at all, and Azure AI Speech wants an
//! `audio` part plus a JSON `definition` in which the model is a nested field.
//! [`CloudSttKind`] is therefore a real dispatch, while the provider *list*
//! stays data.
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

/// Fast-transcription API version that accepts `enhancedMode.model`, which is
/// how MAI-Transcribe is selected. Earlier versions have no such field.
const AZURE_SPEECH_API_VERSION: &str = "2025-10-15";

/// Extra request time allowed for each second of recorded audio, on top of the
/// configured `cloud_stt_timeout_secs`.
///
/// The client timeout covers the whole exchange: connecting, uploading the
/// body, the provider transcribing it, and the response. The body is
/// uncompressed 16 kHz / 16-bit WAV, ~32 kB per second of speech, so a fixed
/// ceiling that is plenty for a 10-second dictation is not enough for a
/// five-minute one (~9.6 MB) on a slow uplink: on a 1 Mbit/s laptop Wi-Fi link
/// the upload alone takes ~77 s, longer than the 60 s default before the
/// provider has even started. The request then times out, and the recording
/// is quietly finished on the local model ("Cloud transcription didn't
/// respond").
///
/// Half a second per second of audio covers the upload on a ~1 Mbit/s link
/// (0.26 s/s) plus the provider's own processing with room to spare, and only
/// costs anything when a request has genuinely hung.
const TIMEOUT_PER_AUDIO_SECOND: f64 = 0.5;

/// Upper bound on a single batch request however long the recording is, so a
/// hung request on a very long recording still ends and falls back.
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// How long establishing the connection may take.
///
/// Separate from the request timeout because that one now grows with the
/// recording. An unreachable network should not wait out a ten-minute budget
/// meant for uploading audio: it fails here quickly, is retried once as a
/// transient failure, and then falls back.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The timeout for one batch request carrying `audio_secs` of audio.
///
/// `base_secs` is the user's `cloud_stt_timeout_secs`, which stays the floor —
/// short dictations behave exactly as before — and the audio's length adds to
/// it, capped at [`MAX_REQUEST_TIMEOUT`].
fn request_timeout(base_secs: u64, audio_secs: f64) -> Duration {
    let base = Duration::from_secs(base_secs);
    let extra = if audio_secs.is_finite() && audio_secs > 0.0 {
        Duration::from_secs_f64(audio_secs * TIMEOUT_PER_AUDIO_SECOND)
    } else {
        Duration::ZERO
    };
    (base + extra).min(MAX_REQUEST_TIMEOUT.max(base))
}

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

    // Azure's endpoint is the user's own resource, so there is no shipped
    // default to fall back to, and what people paste varies (the portal's
    // resource URL, a Foundry project URL, a region name). Normalised here so the
    // request path only ever sees a bare origin.
    let base_url = if provider.kind == CloudSttKind::AzureSpeech {
        azure_speech_base_url(&base_url).ok_or_else(|| CloudSttResolutionError {
            reason: CloudSttUnavailableReason::MissingEndpoint,
            provider_id: Some(provider.id.clone()),
            provider_label: Some(provider.label.clone()),
        })?
    } else {
        base_url
    };

    let language = cloud_language_code(&settings.selected_language);

    // "Translate to English" is one switch shared with the local engine, where it
    // maps onto Whisper's translate task. On the cloud side only the OpenAI
    // schema has an equivalent, so the flag is resolved against the provider here
    // rather than at the request, and the mismatch is said out loud: a user who
    // turned this on and got their own language back had no way to know the
    // endpoint never offered it.
    let translate = settings.translate_to_english && provider.supports_translation;
    if settings.translate_to_english && !translate {
        // This resolver runs several times per recording (activity checks,
        // streaming checks, prewarm, readiness polls from Settings), so the
        // warning is said once per provider rather than on every call. The
        // transcription path logs its own per-request line.
        static WARNED_FOR: Mutex<Option<String>> = Mutex::new(None);
        let mut warned = WARNED_FOR.lock().unwrap_or_else(|e| e.into_inner());
        if warned.as_deref() != Some(provider.id.as_str()) {
            *warned = Some(provider.id.clone());
            warn!(
                "Translate to English is on, but {} transcribes in the language spoken \
                 and has no translation endpoint; the transcript will not be translated",
                provider.label
            );
        }
    }

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
        translate,
        keyterms,
        no_verbatim: settings.cloud_stt_no_verbatim,
        timeout_secs: settings.cloud_stt_timeout_secs.max(5),
    })
}

/// Turn the app's dictation-language setting into a code a cloud endpoint accepts.
///
/// `None` means auto-detect, which is what every provider here does with no
/// language field at all.
///
/// The app's own list carries script subtags for Chinese (`zh-Hans` / `zh-Hant`)
/// because the local Whisper path uses them to pick a variant conversion. No
/// cloud endpoint here understands that: ElevenLabs wants ISO-639-1/3, the OpenAI
/// schema documents "ISO-639-1", and Deepgram takes BCP-47 tags it publishes
/// itself. Sending `zh-Hans` therefore either errored or was ignored, so the
/// primary subtag is what goes on the wire; the Simplified/Traditional conversion
/// still happens locally afterwards (`maybe_convert_chinese_variant`).
fn cloud_language_code(selected: &str) -> Option<String> {
    let trimmed = selected.trim();
    if trimmed.is_empty() || trimmed == "auto" {
        return None;
    }
    let primary = trimmed.split(['-', '_']).next().unwrap_or(trimmed);
    if primary.is_empty() {
        None
    } else {
        Some(primary.to_ascii_lowercase())
    }
}

/// Whether this provider is usable without an API key. Only the custom entry
/// qualifies, and only because it is the one that can point at `localhost`.
fn allows_anonymous(provider: &CloudSttProvider) -> bool {
    provider.key_optional
}

/// Regions where Azure serves MAI-Transcribe, per Microsoft's Speech region
/// table (LLM speech tab, "Transcribe with MAI-Transcribe").
///
/// Used only to read a bare region name as a region rather than as a resource
/// name. A resource elsewhere still works for plain Speech, but answers every
/// MAI request with "Enhanced mode with model is currently not supported yet".
const AZURE_MAI_REGIONS: &[&str] = &[
    "centralindia",
    "eastus",
    "northeurope",
    "southeastasia",
    "westus",
    "westus2",
];

/// Reduce whatever the user pasted as their Azure endpoint to a bare origin.
///
/// Accepted, because each is what some screen in Azure hands you:
///
/// - the resource endpoint from *Keys and Endpoint*
///   (`https://name.cognitiveservices.azure.com/`), kept as is;
/// - a Foundry project URL (`https://name.services.ai.azure.com/api/projects/p`)
///   or an Azure OpenAI one (`https://name.openai.azure.com/`), both of which
///   are the same resource under another host, so they map to
///   `name.cognitiveservices.azure.com`;
/// - a regional endpoint (`https://centralindia.api.cognitive.microsoft.com`);
/// - a bare region name in [`AZURE_MAI_REGIONS`], expanded to that endpoint;
/// - a bare resource name, expanded to its `cognitiveservices` host.
///
/// Any path or query is dropped, so pasting the full transcription URL also
/// works. `None` means there is nothing usable to call.
pub(crate) fn azure_speech_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    if !trimmed.contains("://") && !trimmed.contains('.') && !trimmed.contains('/') {
        let name = trimmed.to_ascii_lowercase();
        let valid = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !valid {
            return None;
        }
        return Some(if AZURE_MAI_REGIONS.contains(&name.as_str()) {
            format!("https://{name}.api.cognitive.microsoft.com")
        } else {
            format!("https://{name}.cognitiveservices.azure.com")
        });
    }

    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = reqwest::Url::parse(&with_scheme).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let host = [".services.ai.azure.com", ".openai.azure.com"]
        .iter()
        .find_map(|suffix| host.strip_suffix(suffix))
        .map(|name| format!("{name}.cognitiveservices.azure.com"))
        .unwrap_or(host);
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{}://{host}{port}", url.scheme()))
}

/// Whether an Azure MAI model takes `modelOptions.transcribeStyle`.
///
/// Microsoft documents the option for MAI-Transcribe-2 only. Sending it to 1.5
/// risks a rejected request over a cosmetic setting, so anything below version 2
/// (and anything that is not a MAI id at all) goes without it.
fn azure_model_takes_style(model: &str) -> bool {
    model
        .trim()
        .to_ascii_lowercase()
        .strip_prefix("mai-transcribe-")
        .and_then(|version| version.parse::<f32>().ok())
        .is_some_and(|version| version >= 2.0)
}

/// The JSON `definition` part of an Azure fast-transcription request.
///
/// Pulled out of the request so the mapping from settings to Azure's schema is
/// testable without a network: `enhancedMode` is what selects MAI at all,
/// `transcribeStyle` is where the app's filler switch lands (Azure's default is
/// verbatim, so it is always sent explicitly), `locales` carries the spoken
/// language, and `phraseList` the custom words.
fn azure_definition(
    model: &str,
    language: Option<&str>,
    keyterms: &[String],
    no_verbatim: bool,
) -> serde_json::Value {
    let mut enhanced = serde_json::json!({ "enabled": true, "model": model });
    if azure_model_takes_style(model) {
        enhanced["modelOptions"] = serde_json::json!({
            "transcribeStyle": if no_verbatim { "clean" } else { "verbatim" },
        });
    }
    let mut definition = serde_json::json!({ "enhancedMode": enhanced });
    if let Some(language) = language {
        // MAI takes bare language codes here ("en", "ne"), which is exactly what
        // `cloud_language_code` already produces.
        definition["locales"] = serde_json::json!([language]);
    }
    if !keyterms.is_empty() {
        definition["phraseList"] = serde_json::json!({ "phrases": keyterms });
    }
    definition
}

/// Join the text of Azure's `combinedPhrases` into one transcript.
///
/// One entry per channel for stereo audio; the app always sends mono, so this is
/// normally a single string, but joining is correct either way.
fn azure_transcript(body: &serde_json::Value) -> String {
    body.get("combinedPhrases")
        .and_then(|phrases| phrases.as_array())
        .map(|phrases| {
            phrases
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
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

/// The container a recording is uploaded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UploadFormat {
    /// Uncompressed 16-bit PCM: ~32 kB per second of audio.
    Wav,
    /// 48 kbit/s CBR MP3: ~6 kB per second, 5.3x smaller.
    Mp3,
}

impl UploadFormat {
    fn mime(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Mp3 => "audio/mpeg",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Wav => "audio.wav",
            Self::Mp3 => "audio.mp3",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Mp3 => "MP3",
        }
    }
}

/// One encoded recording, ready to attach to a request.
#[derive(Clone)]
pub(crate) struct AudioUpload {
    bytes: Vec<u8>,
    format: UploadFormat,
}

/// Encode a recording for upload in `format`.
///
/// A failure to produce MP3 is not a failure to transcribe: it falls back to
/// WAV, which every provider accepts, so the dictation still goes out.
fn encode_upload(samples: &[f32], format: UploadFormat) -> Result<AudioUpload, String> {
    if format == UploadFormat::Mp3 {
        match encode_mp3_16k_mono(samples) {
            Ok(bytes) => {
                return Ok(AudioUpload {
                    bytes,
                    format: UploadFormat::Mp3,
                })
            }
            Err(e) => warn!("Cloud transcription: {e}; uploading WAV instead"),
        }
    }
    Ok(AudioUpload {
        bytes: encode_wav_16k_mono(samples)?,
        format: UploadFormat::Wav,
    })
}

/// Bitrate for compressed uploads.
///
/// Measured before choosing it, because a smaller upload is only worth having if
/// the transcript does not get worse. Whisper-small over a paired LibriSpeech
/// sample (338 utterances, 6,559 words: `test-clean`, `test-other`, and
/// `test-clean` with pink noise at 15 dB SNR), every utterance transcribed from
/// WAV and from MP3 made by this exact encoder configuration:
///
/// - WAV 4.57% WER, 48 kbit/s MP3 4.62% — 3 more errors in 6,559 words, with a
///   95% bootstrap interval for the difference of [-0.25, +0.34] points, i.e. no
///   measurable change; 32 kbit/s was no different either.
/// - That matches Amazon's study of single-channel ASR (arXiv:2106.07994), where
///   WER stops improving at about 32 kbit/s.
///
/// 48 rather than 32 because the extra 0.5 MB on a five-minute dictation buys
/// headroom over that threshold for free.
const MP3_UPLOAD_BITRATE: mp3lame_encoder::Bitrate = mp3lame_encoder::Bitrate::Kbps48;

/// Encode 16 kHz mono `f32` samples as MP3 in memory.
///
/// The batch upload used to be WAV, and its size was the whole problem with long
/// dictations: a five-minute recording is 9.6 MB, ~77 s to upload at 1 Mbit/s,
/// which ran past the request timeout before the provider had even started and
/// sent the recording to the local model. The same audio as 48 kbit/s MP3 is
/// 1.8 MB, ~14 s. Encoding it takes ~0.5 s on one core for those five minutes —
/// two orders of magnitude less than the upload time it saves.
///
/// MP3 rather than Opus or FLAC because it is the one compressed format every
/// hosted provider here documents: Azure's MAI-Transcribe takes only WAV, MP3 or
/// FLAC, and FLAC is lossless but only 1.8x smaller. Kept at 16 kHz (MPEG-2
/// Layer III) so nothing is resampled, which is also the rate every one of these
/// models works at internally.
pub(crate) fn encode_mp3_16k_mono(samples: &[f32]) -> Result<Vec<u8>, String> {
    use mp3lame_encoder::{Builder, FlushNoGap, Mode, MonoPcm, Quality};

    let mut builder = Builder::new().ok_or("Failed to start MP3 encoding")?;
    builder
        .set_num_channels(1)
        .map_err(|e| format!("MP3 encoder rejected mono: {e}"))?;
    builder
        .set_sample_rate(WHISPER_SAMPLE_RATE)
        .map_err(|e| format!("MP3 encoder rejected 16 kHz: {e}"))?;
    builder
        .set_brate(MP3_UPLOAD_BITRATE)
        .map_err(|e| format!("MP3 encoder rejected the bitrate: {e}"))?;
    builder
        .set_mode(Mode::Mono)
        .map_err(|e| format!("MP3 encoder rejected mono mode: {e}"))?;
    // LAME's middle setting; slower settings did not change the measured WER.
    builder
        .set_quality(Quality::Good)
        .map_err(|e| format!("MP3 encoder rejected the quality: {e}"))?;
    let mut encoder = builder
        .build()
        .map_err(|e| format!("Failed to start MP3 encoding: {e}"))?;

    // Same scaling as the WAV path, so both formats carry identical samples.
    let pcm: Vec<i16> = samples
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    let mut out = Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(pcm.len()));
    encoder
        .encode_to_vec(MonoPcm(pcm.as_slice()), &mut out)
        .map_err(|e| format!("Failed to encode MP3: {e}"))?;
    encoder
        .flush_to_vec::<FlushNoGap>(&mut out)
        .map_err(|e| format!("Failed to finish MP3 encoding: {e}"))?;
    if out.is_empty() {
        return Err("MP3 encoding produced no data".to_string());
    }
    Ok(out)
}

/// Encode 16 kHz mono `f32` samples as a 16-bit PCM WAV in memory.
///
/// The universal format: every provider accepts it, so it is what the custom
/// endpoint gets, and what a compressed upload falls back to if a provider
/// rejects it (see [`is_format_rejection`]). The capture pipeline hands over raw
/// `f32` at [`WHISPER_SAMPLE_RATE`], so this is a header plus a scale.
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

/// How long one completed request is assumed to keep the provider's route warm.
///
/// Deliberately short, because the two costs are wildly asymmetric: a redundant
/// warm-up is a quarter-second of audio (fractions of a cent), while a skipped
/// one that was needed is one to two seconds the user waits. Measured behaviour
/// bears the asymmetry out — back-to-back requests stayed at ~1.2 s, a 4-minute
/// gap cost 3.2–3.9 s, and the 60 s sample in between was slow once and fast
/// once. When in doubt, warm.
const ROUTE_WARM_FOR: Duration = Duration::from_secs(30);

/// When the last cloud request completed, so a redundant warm-up can be skipped.
fn last_request_at() -> &'static Mutex<Option<std::time::Instant>> {
    static AT: OnceLock<Mutex<Option<std::time::Instant>>> = OnceLock::new();
    AT.get_or_init(|| Mutex::new(None))
}

/// Record that a request reached the provider and came back.
fn note_request_completed() {
    if let Ok(mut at) = last_request_at().lock() {
        *at = Some(std::time::Instant::now());
    }
}

/// Whether a recent request has already warmed the path to the provider.
fn route_is_warm() -> bool {
    last_request_at()
        .lock()
        .ok()
        .and_then(|at| *at)
        .is_some_and(|at| at.elapsed() < ROUTE_WARM_FOR)
}

/// Forget any recorded request, so the next prewarm treats the route as cold.
///
/// Test-only, and it exists for the same reason [`clear_client_cache`] does: the
/// effect of the warm-up can only be measured from a genuinely cold start, and
/// every earlier request in the same process leaves the route marked warm.
#[cfg(test)]
pub(crate) fn clear_route_warm_marker() {
    if let Ok(mut at) = last_request_at().lock() {
        *at = None;
    }
}

/// A short, quiet tone: the smallest payload that is still real audio.
///
/// The warm-up has to be a genuine transcription to warm what actually costs the
/// time (see [`prewarm_cloud_stt`]), and a genuine transcription needs something
/// to transcribe. A tone rather than silence because a provider is free to
/// short-circuit an empty request, and a short-circuited request warms nothing.
/// The text that comes back is discarded.
fn warmup_samples() -> Vec<f32> {
    let count = WHISPER_SAMPLE_RATE as usize / 4;
    (0..count)
        .map(|i| {
            let t = i as f32 / WHISPER_SAMPLE_RATE as f32;
            (t * 220.0 * std::f32::consts::TAU).sin() * 0.05
        })
        .collect()
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

/// Warm the path to the configured provider while the user is still speaking, so
/// the dictation does not pay for it after they stop.
///
/// Called at recording start (see `actions.rs`), mirroring how the app already
/// prewarms the local cleanup model — and, like that one, it sends a **real
/// request**, not a ping. That distinction is the whole point, and it was learned
/// the expensive way.
///
/// This used to be an authenticated `GET` on the provider's models path, on the
/// theory that the cost worth hiding was DNS + TCP + TLS. It is not. Measured
/// against OpenRouter / `mai-transcribe-2` with a 4-minute idle gap between
/// samples (`stt_cloud_bench::does_a_tiny_warmup_request_beat_a_cold_provider_route`):
///
/// - cold, connection pooled and prewarmed the old way: **3.85 s and 3.20 s**
/// - after one 0.25 s *transcription* first: **1.81 s and 1.78 s**
///
/// Two arms, no overlap between them, and the cold numbers reproduce what the
/// app's own log showed for two real dictations 35 minutes apart. So the second
/// or two was never the handshake — the connection was already being reused, and
/// the log confirms no new connection was opened before the `POST`. It is the
/// provider's route to the upstream model going cold, and the only thing that
/// warms a model route is asking the model to do its job.
///
/// The cost is a quarter-second of audio per cold dictation: at
/// `mai-transcribe-2`'s $1.7 per 1000 minutes, roughly seven millionths of a
/// dollar, and at the priciest provider here still under three hundredths of a
/// cent. Fire-and-forget — a failure here costs nothing but the second it was
/// meant to save.
///
/// Skipped when a real request finished within [`ROUTE_WARM_FOR`], and skipped
/// entirely for realtime streaming, which opens its own socket at recording start
/// and uploads during speech anyway.
pub(crate) fn prewarm_cloud_stt(settings: &AppSettings) {
    let Ok(cfg) = resolve_cloud_stt(settings) else {
        return;
    };
    // Realtime streaming opens its own socket at recording start already.
    if settings.cloud_stt_streaming && crate::stt_cloud_stream::supports_streaming(&cfg) {
        return;
    }
    if route_is_warm() {
        debug!("Cloud STT prewarm skipped: a request completed within the warm window");
        return;
    }
    let Ok(runtime) = runtime() else { return };
    let request = CloudRequest::from_config(&cfg);
    // The same format the dictation will use, so the warm-up also finds out — on
    // a quarter-second clip rather than on the recording — whether this route
    // rejects compressed uploads.
    let Ok(upload) = encode_upload(&warmup_samples(), upload_format_for(&request)) else {
        return;
    };
    let label = cfg.provider.label.clone();
    let model = cfg.model.clone();
    runtime.spawn(async move {
        let started = std::time::Instant::now();
        let format = upload.format;
        match request.transcribe(upload).await {
            Ok(_) => {
                note_request_completed();
                debug!(
                    "Cloud STT route warmed in {:?} ({label} / {model})",
                    started.elapsed()
                );
            }
            Err(e) if format != UploadFormat::Wav && is_format_rejection(&e) => {
                remember_mp3_rejected(&request);
                info!(
                    "{label} / {model} does not accept {} uploads ({e}); recordings \
                     will be sent as WAV",
                    format.label()
                );
            }
            // Not an error the user should see: the dictation itself will report
            // a real failure with the provider's own message.
            Err(e) => debug!("Cloud STT warm-up skipped ({label} / {model}): {e}"),
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
    let seconds = samples.len() as f64 / WHISPER_SAMPLE_RATE as f64;
    let mut request = CloudRequest::from_config(cfg);
    // Encoded here, on the caller's blocking thread, rather than inside the
    // request future: the shared runtime has one worker, and half a second of
    // MP3 encoding there would stall every other request's I/O.
    let encode_started = std::time::Instant::now();
    let upload = encode_upload(samples, upload_format_for(&request))?;
    let encode_time = encode_started.elapsed();
    // Compression is the fix for long uploads; this is the backstop. A long
    // recording still needs time to upload and be transcribed in proportion to
    // its length, and a fixed ceiling sized for a short dictation would cut
    // off a slow connection however small the body.
    request.timeout = request_timeout(cfg.timeout_secs, seconds);
    info!(
        "Cloud transcription: {} / {} ({:.1}s, {} {} kB in {} ms, timeout {}s){}",
        cfg.provider.label,
        cfg.model,
        seconds,
        upload.format.label(),
        upload.bytes.len() / 1024,
        encode_time.as_millis(),
        request.timeout.as_secs(),
        // Only claimed when the request really is going to the translation route,
        // so this line can be trusted the way the local engine's cannot when the
        // setting is on and the model cannot honour it.
        if request.translate {
            " → English"
        } else {
            ""
        }
    );
    let result = send_with_format_fallback(&request, samples, upload);
    if result.is_ok() {
        // Only a completed round trip proves the route is warm, which is what
        // lets the next recording skip its warm-up.
        note_request_completed();
    }
    result
}

/// The format a request to this endpoint and model should go out in.
///
/// MP3 for every hosted provider — each documents it. The custom entry stays on
/// WAV: it is whatever server the user points it at, often on `localhost`
/// where upload size costs nothing, and not every self-hosted server decodes
/// MP3. A route that has already rejected MP3 in this session also stays on WAV
/// (see [`remember_mp3_rejected`]), so the rejection is paid for once rather than
/// on every recording.
fn upload_format_for(request: &CloudRequest) -> UploadFormat {
    if request.provider_id == "custom" || mp3_was_rejected(request) {
        UploadFormat::Wav
    } else {
        UploadFormat::Mp3
    }
}

/// Routes (endpoint + model) that answered an MP3 upload with a format error.
///
/// Process-lifetime only, on purpose: a provider that adds MP3 support, or a
/// model change in Settings, is picked up again on the next launch.
fn mp3_rejected_routes() -> &'static Mutex<std::collections::HashSet<u64>> {
    static ROUTES: OnceLock<Mutex<std::collections::HashSet<u64>>> = OnceLock::new();
    ROUTES.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

fn route_key(request: &CloudRequest) -> u64 {
    let mut hasher = DefaultHasher::new();
    request.base_url.hash(&mut hasher);
    request.model.hash(&mut hasher);
    hasher.finish()
}

fn mp3_was_rejected(request: &CloudRequest) -> bool {
    mp3_rejected_routes()
        .lock()
        .map(|routes| routes.contains(&route_key(request)))
        .unwrap_or(false)
}

fn remember_mp3_rejected(request: &CloudRequest) {
    if let Ok(mut routes) = mp3_rejected_routes().lock() {
        routes.insert(route_key(request));
    }
}

/// Whether a failed request means "I cannot read this audio format", the one
/// failure that sending the same audio as WAV can fix.
///
/// Every hosted provider here documents MP3, but OpenRouter routes to models of
/// its own choosing and a few of those are WAV-only; a client that compressed
/// its uploads without this check found out the same way (TypeWhisper #802,
/// Groq answering M4A with "could not process file - is it a valid media
/// file?"). So this matches the statuses a provider uses for a body it cannot
/// decode (400, 415, 422) together with a message about the media itself, and
/// deliberately not a length or size limit, which a larger WAV would only make
/// worse.
fn is_format_rejection(message: &str) -> bool {
    let Some((_, rest)) = message.split_once(" returned ") else {
        return false;
    };
    let status = rest.get(..3).and_then(|code| code.parse::<u16>().ok());
    if !matches!(status, Some(400 | 415 | 422)) {
        return false;
    }
    let body = rest.to_ascii_lowercase();
    if ["length", "duration", "too long", "too large", "exceed"]
        .iter()
        .any(|limit| body.contains(limit))
    {
        return false;
    }
    [
        "format",
        "codec",
        "unsupported",
        "decode",
        "corrupt",
        "could not process",
        "invalid file",
        "invalid audio",
        "media",
        "mime",
        "content type",
        "content-type",
    ]
    .iter()
    .any(|hint| body.contains(hint))
}

/// Send one encoded recording, with the single quick retry for a network blip.
fn send_with_transient_retry(
    request: &CloudRequest,
    upload: AudioUpload,
) -> Result<String, String> {
    let result = {
        let request = request.clone();
        let upload = upload.clone();
        block_on_request(async move { request.transcribe(upload).await })
    };
    // One quick second attempt for a failure that is about the network rather
    // than the account. Without it a one-second Wi-Fi drop fell straight
    // through to loading the local model, which costs more time than the retry
    // and gives a different transcript. A timeout is not retried: it has
    // already spent the whole timeout, and a second one would double the wait.
    match result {
        Err(e) if is_transient_failure(&e) => {
            warn!("Cloud transcription failed ({e}); retrying once");
            std::thread::sleep(TRANSIENT_RETRY_DELAY);
            let request = request.clone();
            block_on_request(async move { request.transcribe(upload).await })
        }
        other => other,
    }
}

/// Send `upload`, and if the provider rejects a compressed upload's format, send
/// the same recording again as WAV.
fn send_with_format_fallback(
    request: &CloudRequest,
    samples: &[f32],
    upload: AudioUpload,
) -> Result<String, String> {
    let format = upload.format;
    match send_with_transient_retry(request, upload) {
        Err(e) if format != UploadFormat::Wav && is_format_rejection(&e) => {
            warn!(
                "{} did not accept the {} upload ({e}); sending WAV instead, and \
                 using WAV for {} for the rest of this session",
                request.label,
                format.label(),
                request.model
            );
            remember_mp3_rejected(request);
            let wav = encode_upload(samples, UploadFormat::Wav)?;
            send_with_transient_retry(request, wav)
        }
        other => other,
    }
}

/// Pause before the one retry of a transient failure, long enough for a
/// dropped Wi-Fi link or a connection reset to recover.
const TRANSIENT_RETRY_DELAY: Duration = Duration::from_millis(750);

/// Whether a failed request is worth sending again straight away.
///
/// Matches the messages this module builds in `network_error` and `parse_json`:
/// a connection that could not be made or broke mid-request, a body that could
/// not be read, and the statuses a provider uses for "try again" (408 and
/// 5xx). A 429 is left out on purpose: a rate limit rarely clears in under a
/// second, and the local model finishes the recording sooner. Anything else — a rejected key, no credit, an unknown model — fails the
/// same way every time, and a timeout has already cost the full wait.
fn is_transient_failure(message: &str) -> bool {
    if message.starts_with("Could not reach ")
        || message.contains(" request failed: ")
        || message.contains(" returned an unreadable response")
    {
        return true;
    }
    // "<label> returned <status>: <body>" — the label never contains " returned ".
    message
        .split_once(" returned ")
        .and_then(|(_, rest)| rest.get(..3))
        .and_then(|code| code.parse::<u16>().ok())
        .is_some_and(|code| code == 408 || (500..600).contains(&code))
}

/// Everything one cloud request needs, owned so it can cross a thread boundary
/// into [`block_on_request`] without borrowing the settings snapshot.
#[derive(Clone)]
struct CloudRequest {
    kind: CloudSttKind,
    provider_id: String,
    label: String,
    base_url: String,
    api_key: String,
    model: String,
    language: Option<String>,
    translate: bool,
    keyterms: Vec<String>,
    no_verbatim: bool,
    timeout: Duration,
}

/// The multipart file part for an upload, named and typed for its real format.
///
/// The MIME type and extension matter: the OpenAI-schema servers pick a decoder
/// from the file name, and a part labelled `audio/wav` that holds MP3 is exactly
/// the "is it a valid media file?" error the WAV fallback exists to recover from.
fn audio_part(audio: AudioUpload) -> Result<reqwest::multipart::Part, String> {
    reqwest::multipart::Part::bytes(audio.bytes)
        .file_name(audio.format.file_name())
        .mime_str(audio.format.mime())
        .map_err(|e| format!("Failed to attach audio: {e}"))
}

impl CloudRequest {
    fn from_config(cfg: &ResolvedCloudStt) -> Self {
        Self {
            kind: cfg.provider.kind,
            provider_id: cfg.provider.id.clone(),
            label: cfg.provider.label.clone(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            language: cfg.language.clone(),
            translate: cfg.translate,
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
        // No overall timeout here: it differs per request (it grows with the
        // recording, see `request_timeout`) and is set in `post`. Baking it in
        // would mint a separate client — and a separate, cold connection pool —
        // for every recording length.
        let builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .default_headers(headers)
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(2)
            .tcp_keepalive(Duration::from_secs(30));
        // TLS 1.2 on Windows. With TLS 1.3, schannel (through native-tls) lets
        // the first large request on a new connection arrive short: Azure
        // answered every upload over ~90 kB sent as the first request with 400
        // "Failed to read the request form. Unexpected end of Stream", while the
        // same bytes worked as a second request on the same connection, from
        // curl (also schannel) and from Python, and from this client capped at
        // TLS 1.2 (measured 2026-10-05: 0 of 12 cold uploads accepted on 1.3,
        // 4 of 4 on 1.2). Short dictations rarely saw it because the warm-up
        // leaves a connection open, but Azure closes an idle one after about
        // two minutes, so any dictation longer than that went out on a fresh
        // connection, failed, and was finished on the local model. TLS 1.2 is
        // supported by every provider here and costs one extra round trip,
        // only when a connection is opened.
        #[cfg(windows)]
        let builder = builder.max_tls_version(reqwest::tls::Version::TLS_1_2);
        let client = builder
            .build()
            .map_err(|e| format!("Failed to build the HTTP client: {e}"))?;

        if let Ok(mut cache) = client_cache().lock() {
            // Every edit to the key, endpoint or timeout mints a new entry, and
            // each holds an idle pool open for minutes. Same cap as `llm_client`.
            if cache.len() >= 8 {
                cache.clear();
            }
            cache.insert(key, client.clone());
        }
        Ok(client)
    }

    /// Everything that changes the connection or its baked-in headers. The API
    /// key is hashed, never stored in the key. The timeout is deliberately not
    /// part of it: it is applied per request.
    fn cache_key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.base_url.hash(&mut hasher);
        self.api_key.hash(&mut hasher);
        hasher.finish()
    }

    /// A `POST` on the pooled client, bounded by this request's own timeout.
    ///
    /// `RequestBuilder::timeout` covers the same span the client-level one did
    /// (connect through the end of the response body) and overrides it for
    /// this request only.
    fn post(&self, url: &str) -> Result<reqwest::RequestBuilder, String> {
        Ok(self.client()?.post(url).timeout(self.timeout))
    }

    async fn transcribe(&self, audio: AudioUpload) -> Result<String, String> {
        match self.kind {
            CloudSttKind::ElevenLabs => self.transcribe_elevenlabs(audio).await,
            CloudSttKind::OpenAiCompatible => self.transcribe_openai(audio).await,
            CloudSttKind::Deepgram => self.transcribe_deepgram(audio).await,
            CloudSttKind::AzureSpeech => self.transcribe_azure(audio).await,
        }
    }

    /// `POST /speechtotext/transcriptions:transcribe` — Azure AI Speech's fast
    /// transcription, with MAI-Transcribe selected through `enhancedMode`.
    ///
    /// Multipart like the others, but the parts are `audio` and a JSON
    /// `definition` (see [`azure_definition`]), and the key goes in
    /// `Ocp-Apim-Subscription-Key`. The one error worth rewording is the region
    /// one: a resource outside [`AZURE_MAI_REGIONS`] rejects MAI with a message
    /// that says nothing about regions, which is how a working key looks broken.
    async fn transcribe_azure(&self, audio: AudioUpload) -> Result<String, String> {
        let url = format!(
            "{}/speechtotext/transcriptions:transcribe?api-version={AZURE_SPEECH_API_VERSION}",
            self.base_url
        );
        let definition = azure_definition(
            &self.model,
            self.language.as_deref(),
            &self.keyterms,
            self.no_verbatim,
        );
        let part = audio_part(audio)?;
        let form = reqwest::multipart::Form::new()
            .part("audio", part)
            .text("definition", definition.to_string());

        let response = self
            .post(&url)?
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| self.network_error(e))?;

        let parsed: serde_json::Value = self.parse_json(response).await.map_err(|e| {
            if e.contains("not supported yet") {
                format!(
                    "{e}. MAI-Transcribe only runs on Azure resources in {}; \
                     this resource is in another region.",
                    AZURE_MAI_REGIONS.join(", ")
                )
            } else {
                e
            }
        })?;
        Ok(azure_transcript(&parsed))
    }

    /// `POST /v1/speech-to-text` — multipart, `xi-api-key`, `model_id`.
    ///
    /// The realtime model id is rejected by this endpoint, so a user who picked
    /// `scribe_v2_realtime` and then hit a batch fallback would get a 4xx
    /// instead of their words; [`batch_model_id`] maps it back to `scribe_v2`.
    async fn transcribe_elevenlabs(&self, audio: AudioUpload) -> Result<String, String> {
        let url = format!("{}/v1/speech-to-text", self.base_url);
        let part = audio_part(audio)?;
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
            .post(&url)?
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
    ///
    /// With translation asked for, the same multipart body goes to
    /// `/audio/translations` instead. That route is the only translation this
    /// family offers and its output language is fixed to English, so it takes no
    /// `language` field — sending one would be describing a target it does not
    /// have. The spoken-language hint is dropped for exactly that reason:
    /// Whisper's translate task detects the source itself.
    async fn transcribe_openai(&self, audio: AudioUpload) -> Result<String, String> {
        let path = if self.translate {
            "/audio/translations"
        } else {
            "/audio/transcriptions"
        };
        let url = format!("{}{}", self.base_url, path);
        let part = audio_part(audio)?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .part("file", part);
        if let Some(language) = &self.language {
            if self.translate {
                debug!(
                    "Cloud transcription: translating to English, so the {language} \
                     source hint is not sent"
                );
            } else {
                // This schema wants a bare ISO-639-1 code.
                form = form.text("language", language.clone());
            }
        }
        if !self.keyterms.is_empty() {
            // No keyterm field here; the documented way to bias this family is
            // the free-text prompt, which is read as context for the audio.
            form = form.text("prompt", self.keyterms.join(", "));
        }

        let mut request = self.post(&url)?.multipart(form);
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
    async fn transcribe_deepgram(&self, audio: AudioUpload) -> Result<String, String> {
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
            .post(&url)?
            .query(&query)
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Content-Type", audio.format.mime())
            .body(audio.bytes)
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
        let body = response.text().await.map_err(|e| {
            // The client timeout covers the whole exchange, so a slow body
            // times out here rather than at `send`. Reported as a timeout so it
            // is not retried: it has already cost the full wait.
            if e.is_timeout() {
                format!(
                    "{} did not respond within {}s",
                    self.label,
                    self.timeout.as_secs()
                )
            } else {
                format!("{} returned an unreadable response: {e}", self.label)
            }
        })?;
        if !status.is_success() {
            return Err(format!(
                "{} returned {}: {}",
                self.label,
                status,
                summarize_error_body(&body)
            ));
        }
        serde_json::from_str(&body).map_err(|e| {
            warn!(
                "Cloud transcription: unparsable success body: {}",
                crate::utils::redact_text(&body)
            );
            format!("{} returned an unexpected response: {e}", self.label)
        })
    }

    fn network_error(&self, e: reqwest::Error) -> String {
        // Connect first: a connect timeout is both, and it is about the network
        // (cut off at `CONNECT_TIMEOUT`), not about the request's own budget —
        // so it is reported as unreachable, which is also what gets it retried.
        if e.is_connect() {
            format!("Could not reach {}: {e}", self.label)
        } else if e.is_timeout() {
            format!(
                "{} did not respond within {}s",
                self.label,
                self.timeout.as_secs()
            )
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
    let request = CloudRequest::from_config(cfg);
    // In the format dictations will use, so a pass means a real recording will
    // be accepted too, and with the same WAV fallback if it is not.
    let upload = encode_upload(&samples, upload_format_for(&request))?;
    let label = cfg.provider.label.clone();
    let model = cfg.model.clone();
    let text = send_with_format_fallback(&request, &samples, upload)?;
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

    #[test]
    fn only_network_and_retryable_status_failures_are_retried() {
        // Network-side: retried.
        assert!(is_transient_failure(
            "Could not reach Azure AI Speech: error sending request"
        ));
        assert!(is_transient_failure(
            "OpenAI request failed: connection reset by peer"
        ));
        assert!(is_transient_failure(
            "Deepgram returned an unreadable response: unexpected EOF"
        ));
        assert!(is_transient_failure(
            "OpenRouter returned 503 Service Unavailable: overloaded"
        ));
        assert!(!is_transient_failure(
            "ElevenLabs returned 429 Too Many Requests: slow down"
        ));
        // Account-side, or already slow: not retried.
        assert!(!is_transient_failure(
            "Azure AI Speech returned 401 Unauthorized: invalid key"
        ));
        assert!(!is_transient_failure(
            "OpenAI returned 400 Bad Request: unknown model"
        ));
        assert!(!is_transient_failure("Groq did not respond within 30s"));
        assert!(!is_transient_failure(
            "OpenAI returned an unexpected response: missing field"
        ));
    }

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
    fn auto_and_blank_languages_mean_provider_detection() {
        assert_eq!(cloud_language_code("auto"), None);
        assert_eq!(cloud_language_code("   "), None);
    }

    #[test]
    fn a_chosen_language_reaches_the_provider_as_a_bare_iso_code() {
        let mut settings = cloud_settings();
        settings.selected_language = "ne".to_string();
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert_eq!(cfg.language.as_deref(), Some("ne"));
    }

    #[test]
    fn script_subtags_are_dropped_because_no_endpoint_here_takes_them() {
        // The local path needs `zh-Hans` to pick a variant conversion; every
        // cloud endpoint wants the primary subtag and rejects or ignores the rest.
        assert_eq!(cloud_language_code("zh-Hans"), Some("zh".to_string()));
        assert_eq!(cloud_language_code("zh-Hant"), Some("zh".to_string()));
        assert_eq!(cloud_language_code("pt_BR"), Some("pt".to_string()));
        assert_eq!(cloud_language_code("EN"), Some("en".to_string()));
    }

    #[test]
    fn translation_is_refused_where_the_provider_has_no_route_for_it() {
        // ElevenLabs takes a source-language hint and answers in that language.
        let mut settings = cloud_settings();
        settings.translate_to_english = true;
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert!(!cfg.provider.supports_translation);
        assert!(!cfg.translate);
    }

    #[test]
    fn translation_is_honoured_on_the_openai_schema() {
        let mut settings = cloud_settings();
        settings.translate_to_english = true;
        settings.cloud_stt_provider_id = "openai".to_string();
        settings
            .cloud_stt_api_keys
            .insert("openai".to_string(), "test-key".to_string());
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert!(cfg.translate);
    }

    #[test]
    fn translation_stays_off_until_the_user_asks_for_it() {
        let mut settings = cloud_settings();
        settings.cloud_stt_provider_id = "openai".to_string();
        settings
            .cloud_stt_api_keys
            .insert("openai".to_string(), "test-key".to_string());
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert!(!cfg.translate);
    }

    /// Guards the claim the UI makes for each shipped provider. A wrong entry
    /// here is a switch that reads as working and silently does nothing, which is
    /// the exact failure this flag exists to prevent.
    #[test]
    fn only_the_openai_schema_providers_claim_translation() {
        for provider in crate::settings::default_cloud_stt_providers() {
            let expected = matches!(provider.id.as_str(), "openai" | "groq" | "custom");
            assert_eq!(
                provider.supports_translation, expected,
                "{} claims the wrong translation capability",
                provider.id
            );
        }
    }

    #[test]
    fn custom_provider_needs_no_key() {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.cloud_stt_provider_id = "custom".to_string();
        assert!(resolve_cloud_stt(&settings).is_ok());
    }

    fn azure_settings(endpoint: &str) -> AppSettings {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.cloud_stt_provider_id = "azure".to_string();
        settings
            .cloud_stt_api_keys
            .insert("azure".to_string(), "test-key".to_string());
        if !endpoint.is_empty() {
            settings
                .cloud_stt_base_urls
                .insert("azure".to_string(), endpoint.to_string());
        }
        settings
    }

    /// Azure's URL is editable because it is the user's own resource — which is
    /// exactly why "editable URL" can no longer mean "no key needed".
    #[test]
    fn azure_needs_a_key_even_though_its_endpoint_is_editable() {
        let mut settings = azure_settings("https://name.cognitiveservices.azure.com");
        settings
            .cloud_stt_api_keys
            .insert("azure".to_string(), String::new());
        assert!(matches!(
            resolve_cloud_stt(&settings),
            Err(CloudSttResolutionError {
                reason: CloudSttUnavailableReason::MissingApiKey,
                ..
            })
        ));
        for provider in crate::settings::default_cloud_stt_providers() {
            assert_eq!(
                provider.key_optional,
                provider.id == "custom",
                "{} has the wrong key requirement",
                provider.id
            );
        }
    }

    #[test]
    fn azure_without_an_endpoint_is_unavailable_not_active() {
        let settings = azure_settings("");
        assert!(!cloud_stt_active(&settings));
        assert!(matches!(
            resolve_cloud_stt(&settings),
            Err(CloudSttResolutionError {
                reason: CloudSttUnavailableReason::MissingEndpoint,
                ..
            })
        ));
    }

    #[test]
    fn azure_resolves_to_mai_on_the_users_resource() {
        let settings = azure_settings("https://contoso-speech.cognitiveservices.azure.com/");
        let cfg = resolve_cloud_stt(&settings).expect("should resolve");
        assert_eq!(cfg.provider.kind, CloudSttKind::AzureSpeech);
        assert_eq!(
            cfg.base_url,
            "https://contoso-speech.cognitiveservices.azure.com"
        );
        assert_eq!(cfg.model, "MAI-Transcribe-2");
        assert!(!cfg.translate);
    }

    /// Every shape Azure's own screens hand out has to land on the same origin.
    #[test]
    fn azure_endpoints_are_normalised_from_whatever_the_portal_shows() {
        let resource = Some("https://contoso-speech.cognitiveservices.azure.com".to_string());
        for pasted in [
            "https://contoso-speech.cognitiveservices.azure.com/",
            "contoso-speech.cognitiveservices.azure.com",
            "https://contoso-speech.services.ai.azure.com/api/projects/contoso-speech-project",
            "https://contoso-speech.openai.azure.com/",
            "https://contoso-speech.cognitiveservices.azure.com/speechtotext/transcriptions:transcribe?api-version=2025-10-15",
            "contoso-speech",
            "  Contoso-Speech  ",
        ] {
            assert_eq!(azure_speech_base_url(pasted), resource, "pasted {pasted:?}");
        }
        assert_eq!(
            azure_speech_base_url("centralindia").as_deref(),
            Some("https://centralindia.api.cognitive.microsoft.com")
        );
        assert_eq!(
            azure_speech_base_url("https://eastus.api.cognitive.microsoft.com/").as_deref(),
            Some("https://eastus.api.cognitive.microsoft.com")
        );
        assert_eq!(azure_speech_base_url(""), None);
        assert_eq!(azure_speech_base_url("   "), None);
        assert_eq!(azure_speech_base_url("not a name!"), None);
    }

    #[test]
    fn azure_definition_selects_mai_and_maps_every_setting() {
        let words = vec!["SpeakoFlow".to_string(), "Kiro".to_string()];
        let definition = azure_definition("MAI-Transcribe-2", Some("ne"), &words, true);
        assert_eq!(definition["enhancedMode"]["enabled"], true);
        assert_eq!(definition["enhancedMode"]["model"], "MAI-Transcribe-2");
        assert_eq!(
            definition["enhancedMode"]["modelOptions"]["transcribeStyle"],
            "clean"
        );
        assert_eq!(definition["locales"], serde_json::json!(["ne"]));
        assert_eq!(
            definition["phraseList"]["phrases"],
            serde_json::json!(["SpeakoFlow", "Kiro"])
        );

        // Azure's default style is verbatim, so "off" is sent explicitly too,
        // and auto-detect / no words send no field at all.
        let plain = azure_definition("MAI-Transcribe-2", None, &[], false);
        assert_eq!(
            plain["enhancedMode"]["modelOptions"]["transcribeStyle"],
            "verbatim"
        );
        assert!(plain.get("locales").is_none());
        assert!(plain.get("phraseList").is_none());
    }

    #[test]
    fn transcribe_style_is_only_sent_where_it_is_documented() {
        assert!(azure_model_takes_style("MAI-Transcribe-2"));
        assert!(azure_model_takes_style("mai-transcribe-2.5"));
        assert!(!azure_model_takes_style("MAI-Transcribe-1.5"));
        assert!(!azure_model_takes_style("something-else"));
        let older = azure_definition("MAI-Transcribe-1.5", None, &[], true);
        assert!(older["enhancedMode"].get("modelOptions").is_none());
    }

    #[test]
    fn azure_transcript_joins_combined_phrases() {
        let body = serde_json::json!({
            "durationMilliseconds": 3000,
            "combinedPhrases": [{ "text": " Send the report by Friday. " }],
            "phrases": [{ "text": "ignored" }]
        });
        assert_eq!(azure_transcript(&body), "Send the report by Friday.");
        assert_eq!(azure_transcript(&serde_json::json!({})), "");
        assert_eq!(
            azure_transcript(&serde_json::json!({ "combinedPhrases": [] })),
            ""
        );
    }

    /// Azure's error envelopes differ between the Speech route and the gateway
    /// in front of it; both have to reduce to the sentence that explains them.
    #[test]
    fn azure_error_bodies_are_reduced_to_their_message() {
        assert_eq!(
            summarize_error_body(
                r#"{"code":"InvalidRequest","message":"Enhanced mode with model is currently not supported yet."}"#
            ),
            "Enhanced mode with model is currently not supported yet."
        );
        assert_eq!(
            summarize_error_body(
                r#"{"error":{"code":"401","message":"Access denied due to invalid subscription key."}}"#
            ),
            "Access denied due to invalid subscription key."
        );
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

    /// The warm-up has to be real audio, and small. If it ever becomes silence
    /// the whole mechanism quietly stops working: a provider is free to
    /// short-circuit an empty request, and a short-circuited request warms
    /// nothing while still looking like a success in the log.
    #[test]
    fn the_warmup_payload_is_short_real_audio() {
        let samples = warmup_samples();
        assert_eq!(samples.len(), WHISPER_SAMPLE_RATE as usize / 4);
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        assert!(rms > 0.01, "warm-up audio must not be silence (rms {rms})");
        assert!(samples.iter().all(|s| s.abs() <= 1.0));
        // Small enough that the cost per cold dictation stays negligible.
        let wav = encode_wav_16k_mono(&samples).expect("should encode");
        assert!(wav.len() < 16_000, "warm-up payload is {} bytes", wav.len());
    }

    /// The route-warm marker is what stops a burst of dictations paying for a
    /// warm-up each time, and it must only ever be set by a completed round trip.
    #[test]
    fn only_a_completed_request_marks_the_route_warm() {
        clear_route_warm_marker();
        assert!(!route_is_warm());
        note_request_completed();
        assert!(route_is_warm());
        clear_route_warm_marker();
        assert!(!route_is_warm());
    }

    fn tone(seconds: usize) -> Vec<f32> {
        (0..WHISPER_SAMPLE_RATE as usize * seconds)
            .map(|i| {
                (i as f32 / WHISPER_SAMPLE_RATE as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.3
            })
            .collect()
    }

    /// The upload is what the long-dictation fix rests on, so pin both halves of
    /// it: the bytes are MPEG-2 Layer III at 16 kHz (no resampling), and the
    /// bitrate really is ~48 kbit/s — a regression to WAV-sized output would
    /// bring back the timeouts with no other symptom.
    #[test]
    fn mp3_uploads_are_16_khz_mono_mpeg2_at_about_48_kbps() {
        let mp3 = encode_mp3_16k_mono(&tone(10)).expect("should encode");
        let frame = mp3
            .windows(2)
            .position(|w| w[0] == 0xFF && (w[1] & 0xE0) == 0xE0)
            .expect("an MPEG frame sync");
        let header = &mp3[frame..frame + 4];
        assert_eq!(
            (header[1] >> 3) & 0b11,
            0b10,
            "MPEG-2, i.e. 16/22.05/24 kHz"
        );
        assert_eq!((header[1] >> 1) & 0b11, 0b01, "Layer III");
        assert_eq!((header[2] >> 2) & 0b11, 0b10, "16 kHz in MPEG-2");
        assert_eq!(header[3] >> 6, 0b11, "mono");
        let kbps = mp3.len() as f64 * 8.0 / 10.0 / 1000.0;
        assert!((44.0..=53.0).contains(&kbps), "{kbps:.1} kbit/s");
        let wav = encode_wav_16k_mono(&tone(10)).expect("should encode");
        assert!(wav.len() > mp3.len() * 5, "MP3 should be over 5x smaller");
    }

    #[test]
    fn mp3_encoding_handles_short_and_full_scale_input() {
        // The warm-up clip and Verify's silence are both short.
        assert!(!encode_mp3_16k_mono(&warmup_samples())
            .expect("warm-up")
            .is_empty());
        assert!(!encode_mp3_16k_mono(&[0.0; 16]).expect("tiny").is_empty());
        assert!(!encode_mp3_16k_mono(&[2.0, -2.0, 1.0, -1.0])
            .expect("clipped")
            .is_empty());
    }

    fn request_for(provider_id: &str) -> CloudRequest {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.cloud_stt_provider_id = provider_id.to_string();
        settings
            .cloud_stt_api_keys
            .insert(provider_id.to_string(), "test-key".to_string());
        settings.cloud_stt_base_urls.insert(
            provider_id.to_string(),
            "https://contoso-speech.cognitiveservices.azure.com".to_string(),
        );
        CloudRequest::from_config(&resolve_cloud_stt(&settings).expect("should resolve"))
    }

    #[test]
    fn hosted_providers_get_mp3_and_the_custom_endpoint_keeps_wav() {
        for id in [
            "elevenlabs",
            "openai",
            "groq",
            "openrouter",
            "deepgram",
            "mistral",
            "azure",
        ] {
            assert_eq!(
                upload_format_for(&request_for(id)),
                UploadFormat::Mp3,
                "{id}"
            );
        }
        assert_eq!(upload_format_for(&request_for("custom")), UploadFormat::Wav);
    }

    #[test]
    fn a_route_that_rejected_mp3_stays_on_wav() {
        let mut request = request_for("openrouter");
        request.model = "test/wav-only-model".to_string();
        assert_eq!(upload_format_for(&request), UploadFormat::Mp3);
        remember_mp3_rejected(&request);
        assert_eq!(upload_format_for(&request), UploadFormat::Wav);
        // Only that route: the same provider with another model is unaffected.
        let mut other = request.clone();
        other.model = "test/another-model".to_string();
        assert_eq!(upload_format_for(&other), UploadFormat::Mp3);
    }

    #[test]
    fn only_media_format_errors_trigger_the_wav_fallback() {
        for message in [
            "Groq returned 400 Bad Request: could not process file - is it a valid media file?",
            "Deepgram returned 400 Bad Request: failed to process audio: corrupt or unsupported data",
            "OpenRouter returned 415 Unsupported Media Type: (empty response)",
            "OpenRouter returned 422 Unprocessable Entity: unsupported audio format mp3",
            "Custom returned 400 Bad Request: Invalid file format.",
        ] {
            assert!(is_format_rejection(message), "{message}");
        }
        for message in [
            // Account and request problems: WAV fails the same way.
            "Azure AI Speech returned 401 Unauthorized: invalid key",
            "OpenAI returned 400 Bad Request: model_not_found",
            "ElevenLabs returned 429 Too Many Requests: slow down",
            "OpenRouter returned 503 Service Unavailable: unsupported right now",
            // Size and length limits: a WAV would be bigger still.
            "Azure AI Speech returned 400 Bad Request: AudioLengthLimitExceeded: the audio file is longer than the maximum allowed duration",
            "OpenAI returned 400 Bad Request: file too large for this format",
            // Network failures are the transient path's job.
            "Could not reach Groq: unsupported protocol",
            "Groq did not respond within 60s",
        ] {
            assert!(!is_format_rejection(message), "{message}");
        }
    }

    /// What a local stand-in for a provider saw in one request.
    struct Seen {
        content_type: String,
        body: Vec<u8>,
    }

    /// Serve `responses` in order on a local port, one request per connection,
    /// recording each request. Returns the base URL and the recordings.
    fn fake_provider(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::sync::Arc<Mutex<Vec<Seen>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let Ok((mut socket, _)) = listener.accept() else {
                    return;
                };
                let mut raw = Vec::new();
                let mut byte = [0u8; 1];
                while !raw.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap_or(0) == 1 {
                    raw.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&raw).to_ascii_lowercase();
                let header = |name: &str| {
                    head.lines()
                        .find_map(|l| l.strip_prefix(name))
                        .map(|v| v.trim().to_string())
                        .unwrap_or_default()
                };
                let len: usize = header("content-length:").parse().unwrap_or(0);
                let mut request_body = vec![0u8; len];
                let _ = socket.read_exact(&mut request_body);
                log.lock().unwrap().push(Seen {
                    content_type: header("content-type:"),
                    body: request_body,
                });
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (url, seen)
    }

    fn config_against(url: &str, provider_id: &str, model: &str) -> ResolvedCloudStt {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.cloud_stt_provider_id = provider_id.to_string();
        settings
            .cloud_stt_api_keys
            .insert(provider_id.to_string(), "test-key".to_string());
        let mut cfg = resolve_cloud_stt(&settings).expect("should resolve");
        cfg.base_url = url.to_string();
        cfg.model = model.to_string();
        cfg
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// The whole batch path against a local server: the recording leaves as an
    /// MP3 file part, and a provider that answers it with a media-format error
    /// gets the same recording again as WAV, which then succeeds.
    #[test]
    fn a_rejected_mp3_upload_is_resent_as_wav_and_remembered() {
        let (url, seen) = fake_provider(vec![
            (
                400,
                r#"{"error":{"message":"could not process file - is it a valid media file?"}}"#,
            ),
            (200, r#"{"text":"hello from wav"}"#),
            (200, r#"{"text":"second recording"}"#),
        ]);
        let cfg = config_against(&url, "openai", "test/fallback-model");
        let text = transcribe_cloud_blocking(&cfg, &tone(3)).expect("the WAV retry succeeds");
        assert_eq!(text, "hello from wav");
        // A later recording on the same route goes straight to WAV.
        let text = transcribe_cloud_blocking(&cfg, &tone(3)).expect("second recording");
        assert_eq!(text, "second recording");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(contains(&seen[0].body, b"filename=\"audio.mp3\""));
        assert!(contains(&seen[0].body, b"Content-Type: audio/mpeg"));
        for later in &seen[1..] {
            assert!(contains(&later.body, b"filename=\"audio.wav\""));
            assert!(contains(&later.body, b"Content-Type: audio/wav"));
        }
        // The MP3 body really was the small one.
        assert!(seen[1].body.len() > seen[0].body.len() * 4);
    }

    /// Deepgram takes the audio as the raw body, so its Content-Type header is
    /// the only thing telling it the format.
    #[test]
    fn deepgram_gets_a_raw_mp3_body_labelled_as_mp3() {
        let (url, seen) = fake_provider(vec![(
            200,
            r#"{"results":{"channels":[{"alternatives":[{"transcript":"raw body"}]}]}}"#,
        )]);
        let cfg = config_against(&url, "deepgram", "nova-3");
        let text = transcribe_cloud_blocking(&cfg, &tone(2)).expect("should transcribe");
        assert_eq!(text, "raw body");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].content_type, "audio/mpeg");
        assert!(seen[0].body.starts_with(&[0xFF]) || seen[0].body.starts_with(b"ID3"));
    }

    /// An account-side 400 is not a format problem: no WAV re-upload, and the
    /// provider's own message is what comes back.
    #[test]
    fn a_non_format_error_is_not_retried_as_wav() {
        let (url, seen) = fake_provider(vec![(400, r#"{"error":{"message":"model_not_found"}}"#)]);
        let cfg = config_against(&url, "openai", "test/unknown-model");
        let err = transcribe_cloud_blocking(&cfg, &tone(1)).expect_err("should fail");
        assert!(err.contains("model_not_found"), "{err}");
        assert_eq!(seen.lock().unwrap().len(), 1);
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
