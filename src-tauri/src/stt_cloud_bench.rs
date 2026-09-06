//! Where cloud transcription latency actually goes.
//!
//! `#[ignore]`d, like the other live tests. This exists because "it feels slow"
//! and "the provider quotes 0.64 s" are both true at once: the number a provider
//! publishes is model time, and everything else — DNS, TCP, TLS, uploading the
//! audio — happens before the model sees a byte. This measures each part so the
//! fix targets the dominant one instead of the most obvious one.
//!
//! ```text
//! cargo test --lib stt_cloud_bench -- --ignored --nocapture
//! ```
//!
//! ## What these measured (OpenRouter, `openai/whisper-large-v3`, from Nepal)
//!
//! - **WAV encoding and client construction are free.** 12 ms and 0.4 ms.
//!   Not worth touching.
//! - **Latency does not scale with payload.** 2 s of audio (62 kB) took 1.16 s and
//!   10 s (312 kB) took 1.43 s on the same warm connection. Upload is not the
//!   bottleneck, which is why the app still sends uncompressed WAV: adding an MP3
//!   or Opus encoder would buy nothing and cost a dependency.
//! - **Connection reuse is worth ~715 ms, or 27%.** Interleaved 5-round A/B,
//!   median 2.65 s fresh against 1.93 s pooled, with every pooled sample beating
//!   its fresh counterpart. This is what the shared runtime and the client cache
//!   in `stt_cloud` buy.
//! - **The warm-up request has to hit the API path, authenticated.** An anonymous
//!   `HEAD` at the origin returns `200 keep-alive` and yet left nothing the
//!   transcription POST would reuse — measured across three rounds as no gain at
//!   all. An authenticated `GET` on the provider's own models path does leave a
//!   reusable connection.
//! - **What is left is not ours.** Beyond the handshake, the first request of a
//!   burst stays slower than later ones, and pre-opening a socket does not recover
//!   it. That residue is the provider's routing and model warm-up plus round-trip
//!   distance. A realtime WebSocket sidesteps all of it by uploading during
//!   speech, which is why ElevenLabs streaming feels immediate and OpenRouter —
//!   which has no realtime endpoint — cannot.
//!
//! Numbers on this path vary by a second run to run, so anything measured here
//! needs repeats and medians. Single samples have already produced two wrong
//! conclusions.

#![cfg(test)]

use std::time::{Duration, Instant};

use crate::settings::{get_default_settings, ResolvedCloudStt, SttEngineMode};
use crate::stt_cloud::encode_wav_16k_mono;

/// Reuse the live-test helpers for keys and audio.
use super::stt_cloud_live_tests::{openrouter_config, read_wav, sample_wav};

/// Build the same multipart body the app sends, independently of the app's own
/// client, so a cold and a warm client can be compared on equal terms.
async fn one_request(
    client: &reqwest::Client,
    cfg: &ResolvedCloudStt,
    wav: Vec<u8>,
) -> Result<(String, Duration), String> {
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .text("model", cfg.model.clone())
        .text("response_format", "json")
        .part("file", part);

    let started = Instant::now();
    let response = client
        .post(format!("{}/audio/transcriptions", cfg.base_url))
        .bearer_auth(&cfg.api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();
    if !status.is_success() {
        return Err(format!("{status}: {body}"));
    }
    let text = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["text"].as_str().map(str::to_string))
        .unwrap_or_default();
    Ok((text, elapsed))
}

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

/// Does reusing the connection actually help? Interleaved A/B, medians compared.
///
/// This is the experiment that decides whether the client cache earns its place.
/// Arm A clears the cache before every request, which is exactly what the app did
/// before — a fresh client, a fresh handshake. Arm B reuses the pooled client.
/// Interleaved so a drifting network hits both arms equally, and medians rather
/// than best-of because a single fast sample proves nothing on a path that varies
/// by a second.
#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn is_connection_reuse_actually_worth_it() {
    let cfg = require!(
        openrouter_config("openai/whisper-large-v3"),
        "no OpenRouter API key available"
    );
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    let clip: Vec<f32> = samples.iter().copied().take(16_000 * 5).collect();

    const ROUNDS: usize = 5;
    let mut fresh = Vec::new();
    let mut pooled = Vec::new();

    // Prime once so neither arm eats a provider cold start.
    let _ = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip);

    for _ in 0..ROUNDS {
        // Arm A: the old behaviour — new client, new handshake, every time.
        crate::stt_cloud::clear_client_cache();
        let started = Instant::now();
        if crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip).is_ok() {
            fresh.push(started.elapsed());
        }
        // Arm B: reuse whatever is in the pool.
        let started = Instant::now();
        if crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip).is_ok() {
            pooled.push(started.elapsed());
        }
    }

    let median = |mut v: Vec<Duration>| {
        v.sort();
        v[v.len() / 2]
    };
    eprintln!("\n=== connection reuse A/B ({ROUNDS} rounds, interleaved) ===");
    eprintln!("fresh connection each time: {fresh:?}");
    eprintln!("reused pooled connection:   {pooled:?}");
    let fresh_median = median(fresh.clone());
    let pooled_median = median(pooled.clone());
    eprintln!("median fresh:  {fresh_median:?}");
    eprintln!("median pooled: {pooled_median:?}");
    if fresh_median > pooled_median {
        let saved = fresh_median - pooled_median;
        eprintln!(
            "=> reuse saves ~{saved:?} per dictation ({:.0}%)",
            saved.as_secs_f64() / fresh_median.as_secs_f64() * 100.0
        );
    } else {
        eprintln!("=> reuse makes no measurable difference on this path");
    }
}

/// What does the warm-up request actually get back?
///
/// Prewarming only works if the response leaves a connection in the pool, which
/// requires a status the server is willing to keep alive for. This prints the
/// answer instead of guessing at it.
#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn what_does_the_warmup_request_return() {
    let cfg = require!(
        openrouter_config("openai/whisper-large-v3"),
        "no OpenRouter API key available"
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("client");

        for (label, method, url) in [
            ("HEAD origin", "HEAD", cfg.base_url.clone()),
            (
                "GET scoped models",
                "GET",
                format!("{}/models?output_modalities=transcription", cfg.base_url),
            ),
        ] {
            let builder = if method == "HEAD" {
                client.head(&url)
            } else {
                client.get(&url).bearer_auth(&cfg.api_key)
            };
            match builder.send().await {
                Ok(response) => {
                    let status = response.status();
                    let connection = response
                        .headers()
                        .get("connection")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("(absent)")
                        .to_string();
                    let version = format!("{:?}", response.version());
                    let bytes = response.bytes().await.map(|b| b.len()).unwrap_or(0);
                    eprintln!(
                        "{label:<18} -> {status}  http={version}  connection={connection}  body={bytes}B"
                    );
                }
                Err(e) => eprintln!("{label:<18} -> failed: {e}"),
            }
        }
    });
}

/// The fix, measured through the app's own code path.
///
/// [`transcribe_cloud_blocking`] is exactly what a dictation calls. Run three
/// times in a row it shows whether the client cache and the shared runtime are
/// actually keeping a connection alive between recordings: before the fix every
/// call built a new runtime and a new client, so all three were cold and
/// identical; after it, only the first pays the handshake.
///
/// Then it warms the connection the way recording start does and measures again —
/// which is what a real dictation experiences, since the handshake happens while
/// the user is still talking.
#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn the_app_path_reuses_its_connection() {
    let cfg = require!(
        openrouter_config("openai/whisper-large-v3"),
        "no OpenRouter API key available"
    );
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    // A short slice: the point is the per-request overhead, not the model.
    let clip: Vec<f32> = samples.iter().copied().take(16_000 * 5).collect();

    eprintln!("\n=== transcribe_cloud_blocking, three consecutive dictations ===");
    let mut timings = Vec::new();
    for attempt in 1..=3 {
        let started = Instant::now();
        match crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip) {
            Ok(_) => {
                let elapsed = started.elapsed();
                eprintln!("dictation {attempt}: {elapsed:?}");
                timings.push(elapsed);
            }
            Err(e) => {
                eprintln!("dictation {attempt} failed: {e}");
                return;
            }
        }
    }

    let first = timings[0];
    let rest_best = timings[1..].iter().copied().min().expect("two more");
    eprintln!("\nfirst: {first:?}   best repeat: {rest_best:?}");
    assert!(
        rest_best < first,
        "a repeat dictation must reuse the connection and beat the first \
         (first {first:?}, best repeat {rest_best:?}) — if these are equal the \
         client cache or the shared runtime has regressed"
    );

    eprintln!("\n=== recording-start prewarm, A/B from a cold pool ===");
    let mut settings = get_default_settings();
    settings.stt_engine_mode = SttEngineMode::Cloud;
    settings.cloud_stt_provider_id = "openrouter".to_string();
    settings
        .cloud_stt_api_keys
        .insert("openrouter".to_string(), cfg.api_key.clone());
    settings
        .cloud_stt_models
        .insert("openrouter".to_string(), cfg.model.clone());

    // Single samples are hopelessly noisy on this path (warm requests already
    // vary 1.3–1.7 s), so alternate A/B and compare the best of each.
    let mut without = Vec::new();
    let mut with = Vec::new();
    for _ in 0..3 {
        crate::stt_cloud::clear_client_cache();
        let started = Instant::now();
        let _ = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip);
        without.push(started.elapsed());

        crate::stt_cloud::clear_client_cache();
        crate::stt_cloud::prewarm_cloud_stt(&settings);
        std::thread::sleep(Duration::from_millis(900));
        let started = Instant::now();
        let _ = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip);
        with.push(started.elapsed());
    }
    let best_without = without.iter().copied().min().expect("three runs");
    let best_with = with.iter().copied().min().expect("three runs");
    eprintln!("cold, no prewarm: {without:?}  best {best_without:?}");
    eprintln!("cold, prewarmed:  {with:?}  best {best_with:?}");
    if best_without > best_with {
        eprintln!(
            "prewarm saves ~{:?} off a first dictation",
            best_without - best_with
        );
    } else {
        eprintln!(
            "prewarm did NOT help (best without {best_without:?} <= best with {best_with:?}); \
             the warm-up request is not leaving a reusable connection behind"
        );
    }
}

/// Cold vs warm connection, on one runtime, against the real endpoint.
///
/// The gap between request 1 and request 2 on the *same* client is the
/// per-dictation handshake tax the app currently pays, because it builds a fresh
/// client and a fresh runtime for every recording.
#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn where_does_the_latency_go() {
    let cfg = require!(
        openrouter_config("openai/whisper-large-v3"),
        "no OpenRouter API key available"
    );
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    let seconds = samples.len() as f32 / 16_000.0;

    let encode_started = Instant::now();
    let wav = encode_wav_16k_mono(&samples).expect("encode");
    let encode = encode_started.elapsed();

    eprintln!("\n=== audio ===");
    eprintln!(
        "{:.1}s of speech, {} kB uncompressed WAV, encoded in {:?}",
        seconds,
        wav.len() / 1024,
        encode
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        // A fresh client, exactly like the app builds today.
        let build_started = Instant::now();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("client");
        eprintln!("\n=== requests on ONE client ===");
        eprintln!("client construction: {:?}", build_started.elapsed());

        let mut timings = Vec::new();
        for attempt in 1..=3 {
            match one_request(&client, &cfg, wav.clone()).await {
                Ok((text, elapsed)) => {
                    eprintln!(
                        "request {attempt}: {:?}{}",
                        elapsed,
                        if attempt == 1 {
                            "  (cold: DNS + TCP + TLS)"
                        } else {
                            "  (warm: pooled connection)"
                        }
                    );
                    if attempt == 1 {
                        eprintln!("  -> {:?}", text.chars().take(60).collect::<String>());
                    }
                    timings.push(elapsed);
                }
                Err(e) => {
                    eprintln!("request {attempt} failed: {e}");
                    return;
                }
            }
        }

        let cold = timings[0];
        let warm = timings[1..].iter().copied().min().unwrap_or(cold);
        eprintln!("\n=== verdict ===");
        eprintln!("cold: {cold:?}   best warm: {warm:?}");
        if cold > warm {
            let saved = cold - warm;
            eprintln!(
                "handshake tax paid on EVERY dictation today: ~{:?} ({:.0}% of the cold request)",
                saved,
                saved.as_secs_f64() / cold.as_secs_f64() * 100.0
            );
        }
        eprintln!(
            "upload share of the warm request: {} kB at whatever the uplink is; \
             model time is what the provider quotes",
            wav.len() / 1024
        );
    });
}

/// Does latency scale with payload (upload-bound) or stay flat (RTT/model-bound)?
///
/// This is the question that decides whether compressing the audio is worth a new
/// dependency. Same endpoint, same warm client, three lengths of the same
/// recording.
#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn does_latency_scale_with_payload_size() {
    let cfg = require!(
        openrouter_config("openai/whisper-large-v3"),
        "no OpenRouter API key available"
    );
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    if samples.len() < 16_000 * 4 {
        eprintln!("SKIPPED: recording too short to slice");
        return;
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("client");
        // Warm the connection first so the handshake isn't counted.
        let warmup = encode_wav_16k_mono(&samples[..16_000]).expect("encode");
        let _ = one_request(&client, &cfg, warmup).await;

        eprintln!("\n=== latency vs payload (warm connection) ===");
        for secs in [2usize, 5, 10] {
            let take = (16_000 * secs).min(samples.len());
            let wav = encode_wav_16k_mono(&samples[..take]).expect("encode");
            match one_request(&client, &cfg, wav.clone()).await {
                Ok((_, elapsed)) => eprintln!(
                    "{secs:>3}s audio, {:>4} kB -> {:?}",
                    wav.len() / 1024,
                    elapsed
                ),
                Err(e) => eprintln!("{secs}s failed: {e}"),
            }
        }
    });
}
