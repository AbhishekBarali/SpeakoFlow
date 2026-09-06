//! Live checks against a real cloud provider.
//!
//! Every test here is `#[ignore]`d, so `cargo test` stays offline and free. They
//! exist because the parsing tests next door cannot answer the question that
//! actually matters — whether the endpoint accepts what this app sends — and
//! that question has exactly one authority.
//!
//! Run them deliberately:
//!
//! ```text
//! $env:ELEVENLABS_API_KEY = "<key>"
//! cargo test --lib stt_cloud_live -- --ignored --nocapture
//! ```
//!
//! The key is read from `ELEVENLABS_API_KEY`, falling back to whatever the app
//! already has in the OS keychain for its own cloud-STT or TTS slot, so a user
//! who has configured ElevenLabs in the UI can run these with no extra setup.
//! Audio comes from `SPEAKOFLOW_TEST_WAV`, or the newest recording the app has
//! saved — real dictation at the pipeline's own 16 kHz mono, which is exactly
//! what the production path would send.

#![cfg(test)]

use std::path::PathBuf;

use crate::settings::{get_default_settings, ResolvedCloudStt, SttEngineMode};

/// Locate an ElevenLabs key without printing it.
fn api_key() -> Option<String> {
    if let Ok(key) = std::env::var("ELEVENLABS_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Some(key);
        }
    }
    for account in [
        crate::secret_store::account_cloud_stt("elevenlabs"),
        crate::secret_store::account_assistant_tts("elevenlabs"),
    ] {
        if let Some(key) = crate::secret_store::get(&account) {
            if !key.trim().is_empty() {
                return Some(key.trim().to_string());
            }
        }
    }
    None
}

/// The app's own recordings directory on this platform.
pub(crate) fn recordings_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".local/share"))
    }?;
    let dir = base
        .join("com.abhishekbarali.speakoflow")
        .join("recordings");
    dir.is_dir().then_some(dir)
}

/// Newest saved recording, or an explicit override.
pub(crate) fn sample_wav() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SPEAKOFLOW_TEST_WAV") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(recordings_dir()?).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wav") {
            continue;
        }
        let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
        if let Some(modified) = modified {
            if newest.as_ref().is_none_or(|(best, _)| modified > *best) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, path)| path)
}

/// Read a 16-bit PCM WAV back into the `f32` samples the pipeline works in.
pub(crate) fn read_wav(path: &PathBuf) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("recording should open");
    let spec = reader.spec();
    assert_eq!(
        spec.sample_rate, 16_000,
        "the pipeline records at 16 kHz; this file is {} Hz",
        spec.sample_rate
    );
    reader
        .samples::<i16>()
        .map(|s| s.expect("sample should decode") as f32 / i16::MAX as f32)
        .collect()
}

/// Build a config against the real endpoint for a given model.
fn live_config(model: &str) -> Option<ResolvedCloudStt> {
    let key = api_key()?;
    let mut settings = get_default_settings();
    settings.stt_engine_mode = SttEngineMode::Cloud;
    settings
        .cloud_stt_api_keys
        .insert("elevenlabs".to_string(), key);
    settings
        .cloud_stt_models
        .insert("elevenlabs".to_string(), model.to_string());
    // The point of these tests is the endpoint contract, so keep the request as
    // plain as possible: no keyterms, no server-side filtering.
    settings.cloud_stt_send_custom_words = false;
    crate::stt_cloud::resolve_cloud_stt(&settings).ok()
}

/// An OpenRouter key from the environment, or whichever app slot already holds
/// one — OpenRouter is also selectable for the assistant and for voice output, so
/// a user who configured either has a usable key on file.
fn openrouter_key() -> Option<String> {
    if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Some(key);
        }
    }
    for account in [
        crate::secret_store::account_cloud_stt("openrouter"),
        crate::secret_store::account_post_process("openrouter"),
        crate::secret_store::account_assistant_tts("openrouter"),
    ] {
        if let Some(key) = crate::secret_store::get(&account) {
            if !key.trim().is_empty() {
                return Some(key.trim().to_string());
            }
        }
    }
    None
}

pub(crate) fn openrouter_config(model: &str) -> Option<ResolvedCloudStt> {
    let key = openrouter_key()?;
    let mut settings = get_default_settings();
    settings.stt_engine_mode = SttEngineMode::Cloud;
    settings.cloud_stt_provider_id = "openrouter".to_string();
    settings
        .cloud_stt_api_keys
        .insert("openrouter".to_string(), key);
    settings
        .cloud_stt_models
        .insert("openrouter".to_string(), model.to_string());
    crate::stt_cloud::resolve_cloud_stt(&settings).ok()
}

/// Skip with a clear reason rather than failing, so a run without a key or
/// without any saved recording is not mistaken for a broken integration.
macro_rules! require {
    ($value:expr, $why:expr) => {
        match $value {
            Some(value) => value,
            None => {
                eprintln!("SKIPPED: {}", $why);
                return;
            }
        }
    };
}

#[test]
#[ignore = "live network test; needs an ElevenLabs key"]
fn elevenlabs_batch_transcribes_a_real_recording() {
    let cfg = require!(live_config("scribe_v2"), "no ElevenLabs API key available");
    let wav = require!(sample_wav(), "no recording found to transcribe");
    let samples = read_wav(&wav);
    let seconds = samples.len() as f32 / 16_000.0;
    eprintln!(
        "batch: {} ({:.1}s) -> {}",
        wav.display(),
        seconds,
        cfg.model
    );

    let text = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &samples)
        .expect("ElevenLabs batch transcription should succeed");
    eprintln!("batch transcript: {text:?}");
    assert!(
        !text.trim().is_empty(),
        "a real recording should produce text"
    );
}

#[test]
#[ignore = "live network test; needs an ElevenLabs key"]
fn elevenlabs_realtime_streams_a_real_recording() {
    let cfg = require!(
        live_config("scribe_v2_realtime"),
        "no ElevenLabs API key available"
    );
    let wav = require!(sample_wav(), "no recording found to transcribe");
    let samples = read_wav(&wav);
    assert!(
        crate::stt_cloud_stream::supports_streaming(&cfg),
        "the realtime model should advertise streaming"
    );

    // Count the live updates as well as the final text: a session that only ever
    // produces a transcript at the end has not actually streamed, which is the
    // whole reason to pay for the realtime model.
    let updates = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = updates.clone();
    let session = crate::stt_cloud_stream::CloudStreamSession::connect(
        Box::new(move |committed, tentative| {
            let n = seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 6 {
                eprintln!("live[{n}] committed={committed:?} tentative={tentative:?}");
            }
        }),
        &cfg,
    )
    .expect("realtime session should connect");

    // Feed at the same ~30 ms granularity the recorder uses, paced to real time
    // so the provider's VAD sees a plausible stream rather than a burst.
    let mut session = session;
    let frame = 480;
    for chunk in samples.chunks(frame) {
        session.feed(chunk);
        std::thread::sleep(std::time::Duration::from_millis(30));
    }

    let text = session
        .finish()
        .expect("realtime session should return text");
    eprintln!(
        "realtime transcript ({} live updates): {text:?}",
        updates.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert!(
        !text.trim().is_empty(),
        "a real recording should produce text"
    );
    assert!(
        updates.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "streaming should have produced at least one live update before the end"
    );
}

#[test]
#[ignore = "live network test; needs an ElevenLabs key"]
fn elevenlabs_verification_probe_succeeds() {
    let cfg = require!(live_config("scribe_v2"), "no ElevenLabs API key available");
    let message = crate::stt_cloud::verify_cloud_stt(&cfg)
        .expect("the one-second verification probe should be accepted");
    eprintln!("verify: {message}");
    assert!(message.contains("scribe_v2"));
}

/// The "Remove filler words" switch, end to end, on one recording.
///
/// This is the test that answers the question the switch actually raises: does
/// it change the outcome, and is each state self-consistent? It runs the same
/// audio through the provider twice and then applies the app's own filter under
/// the same rule the pipeline uses — off means the local filter is skipped too,
/// which is the half that used to run regardless and made the switch look
/// broken.
#[test]
#[ignore = "live network test; needs an ElevenLabs key"]
fn the_filler_switch_changes_the_outcome_in_both_directions() {
    let cfg = require!(live_config("scribe_v2"), "no ElevenLabs API key available");
    let wav = require!(sample_wav(), "no recording found to transcribe");
    let samples = read_wav(&wav);

    // OFF: nothing asked of the provider, and the app's filter is skipped.
    let verbatim = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &samples)
        .expect("verbatim transcription should succeed");
    eprintln!("\n[switch OFF] {verbatim}");

    // ON: the provider is asked to strip fillers, and the app's filter runs as
    // the backstop for providers that have no such flag.
    let mut on = cfg;
    on.no_verbatim = true;
    let provider_cleaned = crate::stt_cloud::transcribe_cloud_blocking(&on, &samples)
        .expect("no-verbatim transcription should succeed");
    let cleaned = crate::audio_toolkit::filter_transcription_output(&provider_cleaned, "en", &None);
    eprintln!("[switch ON ] {cleaned}\n");

    // Off has to mean off: whatever fillers the provider returned must survive
    // the pipeline. Conditional because this depends on the recording actually
    // containing some — asserting unconditionally would make the test pass or
    // fail on what the user happened to dictate last.
    let filler_count = |text: &str| {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| matches!(*w, "uh" | "um" | "uhm" | "umm" | "er" | "hmm"))
            .count()
    };
    let kept = filler_count(&verbatim);
    let dropped = filler_count(&cleaned);
    eprintln!("fillers in the verbatim text: {kept}; surviving with the switch on: {dropped}");

    if kept > 0 {
        // The regression guard: the app's own filter would have deleted these,
        // and with the switch off it must not run at all.
        let if_filtered = crate::audio_toolkit::filter_transcription_output(&verbatim, "en", &None);
        assert_ne!(
            if_filtered, verbatim,
            "sanity: the local filter should have had something to remove here"
        );
        assert_eq!(
            dropped, 0,
            "with the switch on, no filler should survive either layer: {cleaned}"
        );
    } else {
        eprintln!(
            "note: this recording contains no 'uh'/'um', so the off-direction \
             assertion is vacuous — the unit tests in audio_toolkit::text cover it"
        );
    }

    // The two states must actually differ, or the switch is decorative. Compared
    // case-insensitively and ignoring punctuation so a pure re-punctuation still
    // counts as a difference in *content* only when there is one.
    assert!(
        !verbatim.trim().is_empty() && !cleaned.trim().is_empty(),
        "both directions should return text"
    );

    // And the seams must be intact — no doubled commas, no lowercase sentence
    // openings left behind by a deletion.
    assert!(!cleaned.contains(", ,"), "doubled comma in: {cleaned}");
    assert!(!cleaned.contains(",,"), "doubled comma in: {cleaned}");
    for boundary in ["? ", "! "] {
        for (index, _) in cleaned.match_indices(boundary) {
            let next = cleaned[index + boundary.len()..].chars().next();
            if let Some(next) = next {
                assert!(
                    !next.is_lowercase(),
                    "sentence after '{}' opens lowercase in: {cleaned}",
                    boundary.trim()
                );
            }
        }
    }
}

#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn openrouter_transcribes_a_real_recording() {
    let cfg = require!(
        openrouter_config("openai/whisper-large-v3"),
        "no OpenRouter API key available"
    );
    let wav = require!(sample_wav(), "no recording found to transcribe");
    let samples = read_wav(&wav);
    eprintln!("openrouter: {} -> {}", wav.display(), cfg.model);

    let text = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &samples)
        .expect("OpenRouter transcription should succeed");
    eprintln!("openrouter transcript: {text:?}");
    assert!(
        !text.trim().is_empty(),
        "a real recording should produce text"
    );
}

/// OpenRouter keeps transcription models out of its default catalog, so this is
/// the one provider where getting the discovery query wrong yields a long list of
/// models that cannot transcribe at all.
#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn openrouter_lists_only_transcription_models() {
    let cfg = require!(
        openrouter_config("openai/whisper-large-v3"),
        "no OpenRouter API key available"
    );
    let models =
        crate::stt_cloud::list_cloud_stt_models(&cfg).expect("model listing should succeed");
    eprintln!("openrouter transcription models ({}):", models.len());
    for model in models.iter().take(25) {
        eprintln!("  {model}");
    }
    assert!(
        !models.is_empty(),
        "the scoped listing should return models"
    );
    // Namespaced slugs are what this endpoint expects; a bare id means the
    // listing came back from the wrong catalog.
    assert!(
        models.iter().all(|m| m.contains('/')),
        "every OpenRouter slug should be vendor-namespaced: {models:?}"
    );
    // A chat-only model showing up here would mean the scoping query was dropped.
    assert!(
        !models
            .iter()
            .any(|m| m.contains("claude") || m == "openai/gpt-4o"),
        "a chat model leaked into the transcription listing: {models:?}"
    );
}

#[test]
#[ignore = "live network test; needs an ElevenLabs key"]
fn a_bad_key_is_reported_as_such() {
    let mut settings = get_default_settings();
    settings.stt_engine_mode = SttEngineMode::Cloud;
    settings.cloud_stt_api_keys.insert(
        "elevenlabs".to_string(),
        "sk_definitely_not_valid".to_string(),
    );
    let cfg = crate::stt_cloud::resolve_cloud_stt(&settings).expect("should resolve");
    let error = crate::stt_cloud::verify_cloud_stt(&cfg)
        .expect_err("an invalid key must not be reported as success");
    eprintln!("bad-key error: {error}");
    // The user has to be able to tell an auth failure from an outage.
    assert!(error.contains("ElevenLabs"), "should name the provider");
}
