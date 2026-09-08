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
//! ## What these measured (OpenRouter, from Nepal)
//!
//! - **WAV encoding and client construction are free.** 12 ms and 0.4 ms.
//!   Not worth touching.
//! - **Latency does not scale with payload.** A six-length least-squares fit
//!   ([`how_much_of_the_request_is_upload`]) on `mai-transcribe-2` put the uplink
//!   at ~1.5 MB/s, which is 0.02 s of latency per second of speech: even a
//!   60-second dictation spends about a second uploading. Compressing the audio
//!   would buy that back and cost a dependency, so the app still sends
//!   uncompressed WAV.
//! - **Connection reuse is worth ~715 ms, or 27%.** Interleaved 5-round A/B,
//!   median 2.65 s fresh against 1.93 s pooled. This is what the shared runtime
//!   and the client cache in `stt_cloud` buy.
//! - **The app's own request is not slow.** Ten interleaved rounds against a bare
//!   `reqwest` client with none of the app's headers: p50 1.19 s for the app,
//!   1.22 s bare, and the occasional ~3 s spike hit *both* arms in the same round
//!   — which is also why request hedging was not added, since a correlated
//!   provider-side stall is not something a duplicate request escapes.
//! - **The missing seconds were a cold provider route, and a real warm-up
//!   recovers them.** This is the finding that superseded the earlier
//!   "what is left is not ours". A benchmark loop keeps the provider's route to
//!   the upstream model hot; a person dictates once every few minutes and pays
//!   for it. Across three runs with 4-minute idle gaps, no warm-up gave 1.82,
//!   3.17, 3.20, 3.28, 3.80 and 3.85 s (median 3.24 s) while a single 0.25 s
//!   *transcription* beforehand gave 1.75, 1.78, 1.81, 1.88 and 2.89 s (median
//!   1.81 s). ~1.4 s, ~44%, and never worse. An authenticated `GET` on the models
//!   path — what the prewarm used to send — does not do this: it warms DNS, TCP
//!   and TLS, none of which was the problem, and the app's log confirms the
//!   connection was already being reused before the `POST`.
//! - **The warm-up has to fit inside the recording.** It is itself a cold request
//!   (~2.3 s), so a 4-second dictation sometimes stops before it lands: at 4 s of
//!   speech one round gained 1.9 s and the other only 0.4 s.
//!
//! Numbers on this path vary by a second run to run, so anything measured here
//! needs repeats and medians. Single samples have already produced three wrong
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

/// The shipped fix, measured end to end through `prewarm_cloud_stt`.
///
/// [`does_a_tiny_warmup_request_beat_a_cold_provider_route`] proved the mechanism
/// with a hand-rolled warm-up on real speech. This proves the thing that actually
/// ships: the tone payload, the route-warm marker, the spawn onto the shared
/// runtime, called exactly the way `TranscribeAction::start` calls it.
///
/// Both arms start from a genuinely cold route — the marker is cleared and the
/// gap is real — because a warm route is precisely the condition under which
/// there is nothing to measure.
#[test]
#[ignore = "live network test; needs an OpenRouter key; takes ~20 minutes"]
fn the_shipped_prewarm_warms_the_provider_route() {
    let model = std::env::var("SPEAKOFLOW_BENCH_MODEL")
        .unwrap_or_else(|_| "microsoft/mai-transcribe-2".into());
    let cfg = require!(openrouter_config(&model), "no OpenRouter API key available");
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    let clip: Vec<f32> = samples.iter().copied().take(16_000 * 8).collect();

    let mut settings = get_default_settings();
    settings.stt_engine_mode = SttEngineMode::Cloud;
    settings.cloud_stt_provider_id = "openrouter".to_string();
    settings
        .cloud_stt_api_keys
        .insert("openrouter".to_string(), cfg.api_key.clone());
    settings
        .cloud_stt_models
        .insert("openrouter".to_string(), cfg.model.clone());

    let gap = std::env::var("SPEAKOFLOW_BENCH_GAP_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(240u64);
    // How long the user speaks for. The warm-up only helps if it lands inside the
    // recording, so this is a real parameter of the fix, not test scaffolding.
    let speech = std::env::var("SPEAKOFLOW_BENCH_SPEECH_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4u64);
    const ROUNDS: usize = 2;
    eprintln!(
        "\n=== shipped prewarm: {model}, {gap}s idle, {speech}s of speech, {ROUNDS} rounds ==="
    );

    let mut without = Vec::new();
    let mut with = Vec::new();
    for round in 1..=ROUNDS {
        for warmed in [false, true] {
            eprintln!(
                "round {round} ({}): idling {gap}s...",
                if warmed {
                    "prewarmed"
                } else {
                    "as shipped before"
                }
            );
            std::thread::sleep(Duration::from_secs(gap));
            crate::stt_cloud::clear_route_warm_marker();
            if warmed {
                // Exactly what recording start does, followed by the time the
                // user spends talking.
                crate::stt_cloud::prewarm_cloud_stt(&settings);
            }
            std::thread::sleep(Duration::from_secs(speech));
            let started = Instant::now();
            let elapsed = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip)
                .ok()
                .map(|_| started.elapsed());
            eprintln!(
                "  {}: {elapsed:?}",
                if warmed { "with " } else { "without" }
            );
            if warmed {
                with.extend(elapsed);
            } else {
                without.extend(elapsed);
            }
        }
    }

    eprintln!("\nwithout prewarm: {without:?}");
    eprintln!("with prewarm:    {with:?}");
    let best = |v: &Vec<Duration>| v.iter().copied().min();
    match (best(&without), best(&with)) {
        (Some(a), Some(b)) if a > b => eprintln!(
            "=> the shipped prewarm saves ~{:.2?} off a dictation after an idle gap",
            a - b
        ),
        (Some(a), Some(b)) => eprintln!(
            "=> NO GAIN (without {a:.2?} <= with {b:.2?}); the warm-up is not warming \
             what the transcription pays for"
        ),
        _ => eprintln!("not enough successful samples"),
    }
}

/// Is a cold *provider route* the missing latency, and does a real warm-up fix it?
///
/// This is the experiment that matters, because everything cheaper has been ruled
/// out: the payload is free on this uplink, connection pooling is already working
/// (the app's log shows no new connection before the `POST`), and
/// [`is_the_apps_request_slower_than_a_bare_one`] measured the app's own request
/// at the same p50 as a bare one. What is left is that a benchmark loop keeps the
/// provider's route to the upstream model hot while a person dictates once every
/// few minutes — and the app's log shows 3.5 s and 4.3 s for two dictations 35
/// minutes apart, against a 1.2 s p50 measured back to back.
///
/// If that is the cause, the fix is not a faster connection but a *real* warm-up:
/// a tiny transcription at recording start, which is a different thing from the
/// `GET /models` the prewarm sends today. A TCP connection is not a warm model
/// route.
///
/// Arm A: long idle, then transcribe — what the user experiences.
/// Arm B: long idle, then a 0.25 s warm-up transcription, then transcribe — what
/// the app could do while the user is still speaking.
///
/// Interleaved, so a drifting provider hits both arms equally. Slow by
/// construction: the idle gap is the independent variable.
#[test]
#[ignore = "live network test; needs an OpenRouter key; takes ~20 minutes"]
fn does_a_tiny_warmup_request_beat_a_cold_provider_route() {
    let model = std::env::var("SPEAKOFLOW_BENCH_MODEL")
        .unwrap_or_else(|_| "microsoft/mai-transcribe-2".into());
    let cfg = require!(openrouter_config(&model), "no OpenRouter API key available");
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    let clip: Vec<f32> = samples.iter().copied().take(16_000 * 8).collect();
    // The warm-up has to be a real transcription — that is the whole point — but
    // it should be as small as the endpoint will accept.
    let warm: Vec<f32> = samples.iter().copied().take(16_000 / 4).collect();

    let gap = std::env::var("SPEAKOFLOW_BENCH_GAP_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(240u64);
    const ROUNDS: usize = 2;
    eprintln!(
        "\n=== cold provider route vs tiny warm-up: {model}, {gap}s idle, {ROUNDS} rounds ==="
    );

    let mut cold = Vec::new();
    let mut warmed = Vec::new();
    for round in 1..=ROUNDS {
        // Arm A: cold.
        eprintln!("round {round}: idling {gap}s (cold arm)...");
        std::thread::sleep(Duration::from_secs(gap));
        let started = Instant::now();
        let a = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip)
            .ok()
            .map(|_| started.elapsed());
        eprintln!("  cold:   {a:?}");

        // Arm B: cold, then warmed with a real (tiny) transcription.
        eprintln!("round {round}: idling {gap}s (warmed arm)...");
        std::thread::sleep(Duration::from_secs(gap));
        let warm_started = Instant::now();
        let warm_ok = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &warm).is_ok();
        eprintln!(
            "  warm-up request ({}): {:?}",
            if warm_ok { "ok" } else { "failed" },
            warm_started.elapsed()
        );
        let started = Instant::now();
        let b = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip)
            .ok()
            .map(|_| started.elapsed());
        eprintln!("  warmed: {b:?}");

        cold.extend(a);
        warmed.extend(b);
    }

    eprintln!("\ncold arm:   {cold:?}");
    eprintln!("warmed arm: {warmed:?}");
    let best = |v: &Vec<Duration>| v.iter().copied().min();
    if let (Some(c), Some(w)) = (best(&cold), best(&warmed)) {
        eprintln!("best cold {c:.2?} vs best warmed {w:.2?}");
        if c > w {
            eprintln!(
                "=> a real warm-up request saves ~{:.2?} off a dictation that follows an idle gap",
                c - w
            );
        } else {
            eprintln!(
                "=> the warm-up does not help; the cold arm is not paying for a cold model route"
            );
        }
    }
}

/// Is the app's request slower than a bare one, or is the provider just noisy?
///
/// The app's log showed 3.5 s and 4.3 s on `mai-transcribe-2` while
/// [`how_much_of_the_request_is_upload`] measured 1.2 s for a larger clip on the
/// same key. The only differences in the app's request are the two OpenRouter
/// attribution headers (`HTTP-Referer`, `X-Title`) and the fact that its warm
/// connection was opened by the prewarm `GET` rather than by a `POST`.
///
/// Interleaved A/B, ten rounds, so a drifting provider hits both arms equally.
/// Arm A is [`transcribe_cloud_blocking`] — the app, headers and all. Arm B is
/// the bare client. Reports both medians and the spread, because the spread is
/// the thing worth designing against: a p50 the user rarely feels is not what
/// makes an app feel slow.
#[test]
#[ignore = "live network test; needs an OpenRouter key; takes ~1 minute"]
fn is_the_apps_request_slower_than_a_bare_one() {
    let model = std::env::var("SPEAKOFLOW_BENCH_MODEL")
        .unwrap_or_else(|_| "microsoft/mai-transcribe-2".into());
    let cfg = require!(openrouter_config(&model), "no OpenRouter API key available");
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    let clip: Vec<f32> = samples.iter().copied().take(16_000 * 8).collect();
    let wav = encode_wav_16k_mono(&clip).expect("encode");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let bare = runtime.block_on(async {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(300))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .expect("client")
    });

    const ROUNDS: usize = 10;
    let mut app = Vec::new();
    let mut plain = Vec::new();
    // Prime both arms.
    let _ = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip);
    let _ = runtime.block_on(one_request(&bare, &cfg, wav.clone()));

    eprintln!(
        "\n=== app request vs bare request: {model}, 8s clip, {ROUNDS} interleaved rounds ==="
    );
    for round in 1..=ROUNDS {
        let started = Instant::now();
        let a = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip)
            .ok()
            .map(|_| started.elapsed());
        let b = runtime
            .block_on(one_request(&bare, &cfg, wav.clone()))
            .ok()
            .map(|(_, e)| e);
        eprintln!(
            "round {round:>2}: app {:>9}   bare {:>9}",
            a.map(|d| format!("{:.0?}", d)).unwrap_or("FAIL".into()),
            b.map(|d| format!("{:.0?}", d)).unwrap_or("FAIL".into())
        );
        app.extend(a);
        plain.extend(b);
    }

    let stats = |label: &str, mut v: Vec<Duration>| {
        if v.is_empty() {
            eprintln!("{label}: no samples");
            return;
        }
        v.sort();
        eprintln!(
            "{label:<6} n={:<3} min {:.2?}  p50 {:.2?}  max {:.2?}  spread {:.2?}",
            v.len(),
            v[0],
            v[v.len() / 2],
            v[v.len() - 1],
            v[v.len() - 1] - v[0]
        );
    };
    eprintln!();
    stats("app", app);
    stats("bare", plain);
    eprintln!(
        "\nIf the two medians match, the app's request is not the problem and the \
         spread is: design for the tail, not the median."
    );
}

/// Why a real dictation is slower than a benchmarked request.
///
/// [`how_much_of_the_request_is_upload`] fires its requests back to back and sees
/// ~1.1 s of fixed cost. The app's own log, on the same model and key, shows
/// 3.5–4.3 s. The difference between the two situations is *idleness*: a
/// benchmark loop keeps both the TCP connection and the provider's route to the
/// upstream model hot, while a person dictates once every few minutes.
///
/// This measures the request at increasing idle gaps, through
/// [`transcribe_cloud_blocking`] so the app's own client, headers and pooling are
/// what is under test. If latency climbs with the gap, the gap is the bug and
/// something has to keep the path warm. If it does not, the app is doing
/// something the benchmark is not.
#[test]
#[ignore = "live network test; needs an OpenRouter key; takes ~3 minutes"]
fn does_an_idle_gap_cost_latency() {
    let model = std::env::var("SPEAKOFLOW_BENCH_MODEL")
        .unwrap_or_else(|_| "microsoft/mai-transcribe-2".into());
    let cfg = require!(openrouter_config(&model), "no OpenRouter API key available");
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    let clip: Vec<f32> = samples.iter().copied().take(16_000 * 10).collect();

    eprintln!("\n=== idle-gap sensitivity: {model}, 10s clip, app code path ===");
    // Two priming requests, so the first measurement is not paying for anything
    // the later ones will not.
    for _ in 0..2 {
        let _ = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip);
    }

    for gap in [0u64, 10, 30, 60, 120] {
        if gap > 0 {
            eprintln!("  ... idling {gap}s");
            std::thread::sleep(Duration::from_secs(gap));
        }
        let started = Instant::now();
        match crate::stt_cloud::transcribe_cloud_blocking(&cfg, &clip) {
            Ok(_) => eprintln!("after {gap:>3}s idle: {:?}", started.elapsed()),
            Err(e) => eprintln!("after {gap:>3}s idle: FAILED {e}"),
        }
    }
}

/// Split the request time into a fixed cost and a per-byte cost, by regression.
///
/// This is the measurement that decides whether compressing the audio is worth
/// anything, and it supersedes [`does_latency_scale_with_payload_size`] below —
/// that one printed three timings and left the reader to eyeball a trend, which
/// is how "latency does not scale with payload" got written down from a 2 s vs
/// 10 s pair on one lucky evening.
///
/// Six lengths, warm connection, least-squares fit of `elapsed = a + b·bytes`.
/// `b` is the uplink; `a` is RTT plus the provider's own routing and model time.
/// If `b · 32 kB/s` is a meaningful fraction of a normal dictation, the payload
/// is the thing to attack.
#[test]
#[ignore = "live network test; needs an OpenRouter key"]
fn how_much_of_the_request_is_upload() {
    let model = std::env::var("SPEAKOFLOW_BENCH_MODEL")
        .unwrap_or_else(|_| "microsoft/mai-transcribe-2".into());
    let cfg = require!(openrouter_config(&model), "no OpenRouter API key available");
    let wav_path = require!(sample_wav(), "no recording found");
    let samples = read_wav(&wav_path);
    let have = samples.len() / 16_000;
    eprintln!("\n=== payload regression: {model} ===");
    eprintln!("source recording: {have}s available");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(300))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .expect("client");

        // Warm twice: the first request pays the handshake, the second absorbs
        // any provider-side cold start for this model.
        let warmup = encode_wav_16k_mono(&samples[..16_000.min(samples.len())]).expect("encode");
        let _ = one_request(&client, &cfg, warmup.clone()).await;
        let _ = one_request(&client, &cfg, warmup).await;

        // Repeat each length so a single network hiccup cannot set the slope.
        let lengths: Vec<usize> = [2usize, 4, 6, 10, 15, 20]
            .into_iter()
            .filter(|s| s * 16_000 <= samples.len())
            .collect();
        if lengths.len() < 3 {
            eprintln!("SKIPPED: recording is only {have}s; need ~10s+ to fit a slope");
            return;
        }

        // (bytes, elapsed) pairs, best-of-two per length: the minimum is the
        // sample least polluted by unrelated congestion.
        let mut points: Vec<(f64, f64)> = Vec::new();
        for secs in &lengths {
            let take = secs * 16_000;
            let wav = encode_wav_16k_mono(&samples[..take]).expect("encode");
            let bytes = wav.len() as f64;
            let mut best: Option<Duration> = None;
            for _ in 0..2 {
                match one_request(&client, &cfg, wav.clone()).await {
                    Ok((_, elapsed)) => {
                        best = Some(best.map_or(elapsed, |b: Duration| b.min(elapsed)));
                    }
                    Err(e) => eprintln!("{secs}s failed: {e}"),
                }
            }
            if let Some(elapsed) = best {
                eprintln!(
                    "{secs:>3}s audio, {:>4} kB -> {:>8.0?}   ({:.0} kB/s effective)",
                    wav.len() / 1024,
                    elapsed,
                    bytes / 1024.0 / elapsed.as_secs_f64()
                );
                points.push((bytes, elapsed.as_secs_f64()));
            }
        }

        if points.len() < 3 {
            eprintln!("not enough successful samples to fit");
            return;
        }

        let n = points.len() as f64;
        let mean_x = points.iter().map(|p| p.0).sum::<f64>() / n;
        let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
        let cov: f64 = points
            .iter()
            .map(|(x, y)| (x - mean_x) * (y - mean_y))
            .sum();
        let var: f64 = points.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
        let slope = if var > 0.0 { cov / var } else { 0.0 };
        let intercept = mean_y - slope * mean_x;

        eprintln!("\n=== fit: elapsed = a + b*bytes ===");
        eprintln!(
            "a (fixed: RTT + provider routing + model) = {:.2}s",
            intercept
        );
        if slope > 0.0 {
            let throughput_kbps = 1.0 / slope / 1024.0;
            eprintln!(
                "b (per byte) = {:.3} ms/kB  =>  effective uplink ~{:.0} kB/s ({:.1} Mbit/s)",
                slope * 1024.0 * 1000.0,
                throughput_kbps,
                throughput_kbps * 8.0 / 1024.0
            );
            // 16 kHz mono 16-bit PCM is 32 kB per second of speech.
            let per_audio_second = slope * 32_000.0;
            eprintln!(
                "=> uncompressed WAV costs {:.2}s of latency per second of speech",
                per_audio_second
            );
            for dictation in [5.0f64, 15.0, 30.0, 60.0] {
                eprintln!(
                    "   {dictation:>4.0}s dictation: {:.1}s upload + {:.1}s fixed = {:.1}s total; \
                     at Opus 24 kbps the upload term becomes {:.2}s",
                    per_audio_second * dictation,
                    intercept,
                    per_audio_second * dictation + intercept,
                    slope * 3_000.0 * dictation
                );
            }
        } else {
            eprintln!("b <= 0: no measurable payload term on this network right now");
        }
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
