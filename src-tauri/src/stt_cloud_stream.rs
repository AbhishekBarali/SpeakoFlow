//! Realtime cloud speech-to-text over WebSockets.
//!
//! The batch path in [`crate::stt_cloud`] cannot start work until the user stops
//! talking, so the whole transcription latency lands after the key is released.
//! A realtime endpoint moves that cost under the recording: audio goes up while
//! it is being spoken, text comes back as it is recognised, and releasing the
//! key only has to flush a tail. This is also what makes the recording overlay's
//! Live card work with a cloud provider — the same `stream-text` event the local
//! streaming engines emit.
//!
//! ## A synchronous facade over an async socket
//!
//! [`CloudStreamSession`] is deliberately blocking. Its caller is the
//! transcription manager's stream worker, an ordinary OS thread that already
//! drives the local streaming engines through `feed`/`finalize`, so a session
//! that owns its own current-thread runtime drops straight into that shape
//! instead of forcing an async rewrite of the worker. The socket does not need a
//! dedicated pump: [`feed`](CloudStreamSession::feed) is called about thirty
//! times a second by the audio path, and each call sends its chunk and then
//! drains whatever has already arrived without blocking, which is frequent
//! enough that transcripts appear as promptly as they would under a background
//! reader.
//!
//! ## Committed versus partial
//!
//! Every provider here distinguishes a stable prefix from a revisable tail,
//! which maps exactly onto the overlay's existing `committed` / `tentative`
//! split — so a partial result can be shown immediately and still be corrected
//! without the text ever visibly rewriting itself behind the cursor.
//!
//! Every failure here is non-fatal: a session that cannot connect, stalls, or
//! errors mid-recording returns `None`, and the manager re-transcribes the
//! complete recording through the batch path. The user waits slightly longer and
//! loses nothing, which is the only acceptable behaviour for dictation.
//!
//! ## Amazon Transcribe, the provider with no batch path
//!
//! AWS speaks binary event-stream frames over a SigV4-presigned URL
//! ([`crate::stt_aws`]) where the others speak JSON with an auth header, and its
//! "batch path" is this same session run again over the finished recording
//! ([`transcribe_by_streaming`]). AWS processes a stream at speaking speed and
//! asks for recorded audio to be sent at that rate, so a replay takes as long as
//! the recording — which is why this provider always streams live, and why the
//! replay is only the fallback.

use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use log::{debug, info, warn};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE;
use crate::settings::{CloudSttKind, ResolvedCloudStt};

/// Audio is batched to this many milliseconds before being sent.
///
/// The capture callback delivers ~30 ms frames, and one WebSocket message per
/// frame is 33 messages a second of pure overhead — each carrying a JSON
/// envelope and base64 expansion around 960 bytes of audio. Batching to 100 ms
/// cuts that to ten while staying far below the ~150 ms the model itself takes,
/// so it costs no perceptible latency.
const CHUNK_MS: usize = 100;

/// Samples per outgoing chunk at the capture rate.
const CHUNK_SAMPLES: usize = (WHISPER_SAMPLE_RATE as usize * CHUNK_MS) / 1000;

/// How long to wait for the handshake and the session-started acknowledgement.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait after the final commit for the closing transcript.
///
/// Generous because this is the one wait the user actually feels — but bounded,
/// because a provider that has stopped answering must not hold a dictation
/// hostage when a complete batch transcription is one request away.
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);

/// Quiet period after a committed transcript that marks the end of the flush.
/// A commit can arrive as two messages (text, then the timestamped variant), so
/// the session waits briefly for a follow-up rather than returning on the first.
const FINALIZE_QUIET_PERIOD: Duration = Duration::from_millis(400);

/// How long a single mid-recording read is allowed to wait for data.
///
/// This is small but deliberately **not zero**. Polling with `now_or_never()`
/// looks cheaper, but a future that is never `Pending` means `block_on` never
/// parks, and a current-thread runtime only ticks its I/O driver on park — so a
/// non-blocking poll can sit there reporting "nothing yet" while the socket has
/// data waiting, and partial transcripts arrive in clumps whenever some other
/// call happens to park. A tiny timeout gives the driver its tick.
const DRAIN_POLL: Duration = Duration::from_millis(2);

/// Ceiling on one [`drain_ready`](CloudStreamSession::drain_ready) pass, so a
/// chatty provider cannot stall the audio path. Anything left over is read on
/// the next frame, ~30 ms later.
const DRAIN_BUDGET: Duration = Duration::from_millis(20);

/// Ceiling on a single socket write, and on the closing handshake.
///
/// Every wait in this module has to be bounded: `feed()` runs on the worker
/// thread that drains the audio channel, and a peer that stops reading would
/// otherwise block it forever, leaking the thread and the socket.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Ceiling on the closing handshake. Shorter than [`SEND_TIMEOUT`] because by
/// this point the transcript is already in hand and nothing is waiting on a
/// clean close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Where an Amazon Transcribe session connects in tests: a local stand-in for
/// AWS. The real URL is signed for AWS's own host, so the whole URL is swapped.
#[cfg(test)]
static AMAZON_TEST_URL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Quiet period that ends Amazon Transcribe's finalize once a segment has been
/// committed. AWS closes the socket itself after the last transcript of a
/// stream, so this is only the backstop for a close that never comes — and it
/// is longer than [`FINALIZE_QUIET_PERIOD`] because AWS can finalize two
/// segments a second or more apart, and breaking between them would drop the
/// second.
const AMAZON_FINALIZE_QUIET_PERIOD: Duration = Duration::from_millis(2000);

/// Whether this provider has a realtime endpoint wired up here.
///
/// Checks the model too, not just the provider: ElevenLabs' realtime and batch
/// endpoints have separate model namespaces, and a user on `scribe_v2` is asking
/// for the batch model by name.
pub(crate) fn supports_streaming(cfg: &ResolvedCloudStt) -> bool {
    match cfg.provider.kind {
        CloudSttKind::ElevenLabs => cfg.model.contains("realtime"),
        CloudSttKind::Deepgram | CloudSttKind::AmazonTranscribe => true,
        CloudSttKind::OpenAiCompatible | CloudSttKind::AzureSpeech => false,
    }
}

/// Whether streaming is this provider's *only* way to transcribe, so a finished
/// recording has to be replayed through a stream ([`transcribe_by_streaming`])
/// instead of sent as one request.
pub(crate) fn streams_only(cfg: &ResolvedCloudStt) -> bool {
    cfg.provider.kind == CloudSttKind::AmazonTranscribe
}

/// Where a session publishes its running transcript.
///
/// A closure rather than the `AppHandle` itself so the protocol logic has no
/// dependency on the window layer — which is what lets a live test drive a real
/// session against a real endpoint without standing up a Tauri app.
pub(crate) type LiveTextSink = Box<dyn Fn(&str, &str) + Send>;

/// A live transcription session against a cloud provider.
pub(crate) struct CloudStreamSession {
    runtime: tokio::runtime::Runtime,
    socket: WebSocket,
    protocol: Protocol,
    sink: LiveTextSink,
    /// Samples not yet sent, held until a full [`CHUNK_MS`] chunk is available.
    pending: Vec<f32>,
    /// The running committed + tentative transcript.
    transcript: LiveTranscript,
    /// Set once the socket has failed, so later calls stop trying.
    failed: bool,
    /// Why it failed, for a caller that reports the reason rather than falling
    /// back silently (the replay, and the Settings test).
    last_error: Option<String>,
}

/// The running transcript of one session: an append-only committed prefix plus
/// the revisable tail after it.
///
/// Separate from the socket so its rules can be tested without one, and the rule
/// that most needs it is the replayed partial. ElevenLabs can deliver a
/// `partial_transcript` for a segment *after* the `committed_transcript` that
/// closed it. Taken at face value that draft becomes the new tail, so the overlay
/// shows the sentence twice — and at the end of a recording `text()` appends the
/// tail as the last thing the user said, which pasted two consecutive dictations
/// as the whole transcript twice over, the second copy ending in an earlier guess
/// ("…satisfying our" after "…satisfying or good.").
#[derive(Default)]
struct LiveTranscript {
    committed: String,
    tentative: String,
    /// Words of the segment committed last, kept until the first partial that is
    /// clearly the start of a new segment rather than a replay of this one.
    last_segment: Option<Vec<String>>,
}

impl LiveTranscript {
    /// Take a new tail. Returns whether the visible transcript changed.
    fn apply_partial(&mut self, text: String) -> bool {
        let words = transcript_words(&text);
        if let Some(segment) = &self.last_segment {
            if is_replay_of(segment, &words) {
                debug!(
                    "Realtime cloud STT: dropped a partial that repeats the segment just committed"
                );
                return false;
            }
            // An empty partial says nothing about which segment it belongs to,
            // so only real words end the watch for a replay.
            if !words.is_empty() {
                self.last_segment = None;
            }
        }
        if self.tentative == text {
            return false;
        }
        self.tentative = text;
        true
    }

    /// Close a segment. Returns whether the visible transcript changed.
    ///
    /// Commits are never de-duplicated, only partials: someone who says the same
    /// sentence twice means it twice, and dropping a commit loses their words
    /// where dropping a replayed partial only delays a preview.
    fn apply_commit(&mut self, text: &str) -> bool {
        let text = text.trim();
        let had_tail = !self.tentative.is_empty();
        self.tentative.clear();
        if text.is_empty() {
            return had_tail;
        }
        if !self.committed.is_empty() && !self.committed.ends_with(' ') {
            self.committed.push(' ');
        }
        self.committed.push_str(text);
        self.last_segment = Some(transcript_words(text));
        true
    }

    /// The tail as the overlay draws it: immediately after `committed`, with no
    /// separator of its own. A new segment's partial does not start with a
    /// space, so without one the overlay ran two sentences together
    /// ("…right now.and then") while `text()` joined them correctly.
    fn display_tentative(&self) -> std::borrow::Cow<'_, str> {
        let needs_space = !self.committed.is_empty()
            && !self.committed.ends_with(char::is_whitespace)
            && !self.tentative.is_empty()
            && !self.tentative.starts_with(char::is_whitespace);
        if needs_space {
            std::borrow::Cow::Owned(format!(" {}", self.tentative))
        } else {
            std::borrow::Cow::Borrowed(&self.tentative)
        }
    }

    /// The whole transcript, or `None` when nothing was recognised.
    fn text(&self) -> Option<String> {
        let mut text = self.committed.clone();
        // The tail is unstable, but at the end of a recording it is the last
        // thing the user said; dropping it would silently truncate them.
        if !self.tentative.trim().is_empty() {
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(self.tentative.trim());
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

/// Lowercased words with punctuation removed, so a draft ("satisfying our") and
/// its commit ("satisfying, or good.") compare on what was said.
fn transcript_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// Whether `partial` is a draft of `segment` arriving late, rather than the
/// opening of the next segment.
///
/// A replay is a draft of the same sentence, so nearly all of its words appear
/// in the committed segment in the same order; the order-aware overlap (longest
/// common subsequence) tolerates the words a draft got wrong. A short partial is
/// only a replay when it *is* the segment, because a new segment can reasonably
/// open with the same word or two as the last one ("and then… and then").
fn is_replay_of(segment: &[String], partial: &[String]) -> bool {
    if partial.is_empty() || segment.is_empty() {
        return false;
    }
    if partial.len() < 3 {
        return partial == segment;
    }
    common_subsequence_len(segment, partial) * 5 >= partial.len() * 4
}

/// Length of the longest common subsequence of two word lists.
fn common_subsequence_len(a: &[String], b: &[String]) -> usize {
    let mut previous = vec![0usize; b.len() + 1];
    let mut current = vec![0usize; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            current[j + 1] = if x == y {
                previous[j] + 1
            } else {
                previous[j + 1].max(current[j])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

type WebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Protocol {
    ElevenLabs,
    Deepgram,
    AmazonTranscribe,
}

impl CloudStreamSession {
    /// Open a realtime session, or return why it could not be opened. A failure
    /// here is expected to be handled by falling back to batch.
    ///
    /// `sink` receives the running `(committed, tentative)` transcript. The
    /// caller supplies it rather than an `AppHandle` for two reasons: this module
    /// then has no dependency on the window layer, and the caller is the only
    /// place that knows whether the recording these events belong to is still
    /// the current one.
    pub fn connect(sink: LiveTextSink, cfg: &ResolvedCloudStt) -> Result<Self, String> {
        let protocol = match cfg.provider.kind {
            CloudSttKind::ElevenLabs => Protocol::ElevenLabs,
            CloudSttKind::Deepgram => Protocol::Deepgram,
            CloudSttKind::AmazonTranscribe => Protocol::AmazonTranscribe,
            CloudSttKind::OpenAiCompatible | CloudSttKind::AzureSpeech => {
                return Err("This provider has no realtime endpoint".to_string())
            }
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to start the realtime runtime: {e}"))?;

        let url = match protocol {
            Protocol::ElevenLabs => elevenlabs_url(cfg),
            Protocol::Deepgram => deepgram_url(cfg),
            Protocol::AmazonTranscribe => {
                let amazon = cfg
                    .amazon
                    .as_ref()
                    .ok_or_else(|| format!("{} is not fully configured", cfg.provider.label))?;
                if !crate::stt_aws::STREAMING_REGIONS.contains(&amazon.region.as_str()) {
                    warn!(
                        "{} is not a region AWS lists for Transcribe streaming; trying it anyway",
                        amazon.region
                    );
                }
                // Signed fresh for each session: the signature is only valid
                // for five minutes, and it is in the URL, so the URL itself is
                // never logged — only its host.
                debug!(
                    "Realtime cloud STT connecting: wss://{} (presigned)",
                    crate::stt_aws::streaming_host(&amazon.region)
                );
                let signed =
                    crate::stt_aws::streaming_url(amazon, WHISPER_SAMPLE_RATE, chrono::Utc::now());
                #[cfg(test)]
                let signed = AMAZON_TEST_URL
                    .lock()
                    .ok()
                    .and_then(|url| url.clone())
                    .unwrap_or(signed);
                signed
            }
        };
        if protocol != Protocol::AmazonTranscribe {
            debug!("Realtime cloud STT connecting: {url}");
        }

        let mut request = url
            .into_client_request()
            .map_err(|e| format!("Invalid realtime URL: {e}"))?;
        let auth = match protocol {
            Protocol::ElevenLabs => Some(("xi-api-key", cfg.api_key.clone())),
            Protocol::Deepgram => Some(("Authorization", format!("Token {}", cfg.api_key))),
            // The credentials are signed into the URL instead.
            Protocol::AmazonTranscribe => None,
        };
        if let Some((auth_header, auth_value)) = auth {
            request.headers_mut().insert(
                auth_header,
                auth_value
                    .parse()
                    .map_err(|_| "API key contains characters that cannot be sent".to_string())?,
            );
        }

        let socket = runtime.block_on(async {
            match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
                .await
            {
                Ok(Ok((socket, _response))) => Ok(socket),
                Ok(Err(e)) => Err(describe_connect_error(protocol, e)),
                Err(_) => Err("Realtime connection timed out".to_string()),
            }
        })?;

        let mut session = Self {
            runtime,
            socket,
            protocol,
            sink,
            pending: Vec::with_capacity(CHUNK_SAMPLES * 2),
            transcript: LiveTranscript::default(),
            failed: false,
            last_error: None,
        };

        // ElevenLabs acknowledges a session before it will accept audio, and
        // that acknowledgement is also where an auth or quota rejection shows
        // up. Reading it now means a bad key fails before recording rather than
        // silently producing no text.
        if session.protocol == Protocol::ElevenLabs {
            session.await_session_started()?;
        }

        info!(
            "Realtime cloud STT session open: {} / {}",
            cfg.provider.label, cfg.model
        );
        Ok(session)
    }

    /// Wait for `session_started`, surfacing any error message the server sends
    /// instead of it.
    fn await_session_started(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Realtime session was never acknowledged".to_string());
            }
            let socket = &mut self.socket;
            let received = self
                .runtime
                .block_on(async { tokio::time::timeout(remaining, socket.next()).await });
            match received {
                Err(_) => return Err("Realtime session was never acknowledged".to_string()),
                Ok(None) => return Err("Realtime connection closed during setup".to_string()),
                Ok(Some(Err(e))) => return Err(format!("Realtime connection failed: {e}")),
                Ok(Some(Ok(Message::Text(text)))) => {
                    match parse_elevenlabs(text.as_str()) {
                        Some(ServerEvent::SessionStarted) => return Ok(()),
                        Some(ServerEvent::Error(message)) => {
                            return Err(format!("Realtime session rejected: {message}"))
                        }
                        // A transcript before the acknowledgement is impossible,
                        // but tolerating it is cheaper than failing on it.
                        _ => continue,
                    }
                }
                Ok(Some(Ok(Message::Close(_)))) => {
                    return Err("Realtime connection closed during setup".to_string())
                }
                Ok(Some(Ok(_))) => continue,
            }
        }
    }

    /// Push captured audio. Sends whole chunks and drains any transcripts that
    /// have already arrived. Errors are recorded and swallowed: the recording
    /// must keep going so the batch fallback still has complete audio.
    pub fn feed(&mut self, frame: &[f32]) {
        if self.failed {
            return;
        }
        self.pending.extend_from_slice(frame);
        while self.pending.len() >= CHUNK_SAMPLES {
            let chunk: Vec<f32> = self.pending.drain(..CHUNK_SAMPLES).collect();
            if let Err(e) = self.send_audio(&chunk, false) {
                warn!("Realtime cloud STT send failed: {e}");
                self.fail_after_send(format!("sending audio failed: {e}"));
                return;
            }
        }
        self.drain_ready();
    }

    /// Flush the tail, ask for a final commit, and return the full transcript.
    ///
    /// `None` means the session produced nothing usable and the caller should
    /// batch-transcribe instead.
    pub fn finish(mut self) -> Option<String> {
        if self.failed {
            debug!("Realtime cloud STT: session already failed; using batch instead");
            return None;
        }
        match self.finalize() {
            Ok(text) => text,
            Err(e) => {
                // The session cannot vouch for a complete transcript. Answering
                // `None` costs the user one extra batch request; answering with a
                // truncated transcript costs them the end of their sentence,
                // silently.
                warn!("Realtime cloud STT failed during finalize ({e}); using the complete batch result");
                None
            }
        }
    }

    /// [`finish`](Self::finish) for a caller with no batch path behind it: the
    /// transcript (empty when nothing was said), or why the session failed.
    fn finish_result(mut self) -> Result<String, String> {
        self.finalize().map(Option::unwrap_or_default)
    }

    /// Whether the session has failed. A replay checks this between chunks so a
    /// dead socket is not fed for the rest of the recording.
    fn has_failed(&self) -> bool {
        self.failed
    }

    /// Mark the session dead. The first reason is kept: later errors are
    /// usually consequences of it (a send after the provider closed).
    fn mark_failed(&mut self, reason: String) {
        self.failed = true;
        if self.last_error.is_none() {
            self.last_error = Some(reason);
        }
    }

    /// A send failed, which usually means the provider already closed the
    /// socket — and Amazon Transcribe says why in a frame just before closing.
    /// Read what has arrived first, so that reason wins over "connection
    /// closed".
    fn fail_after_send(&mut self, reason: String) {
        self.drain_ready();
        self.mark_failed(reason);
    }

    fn failure_reason(&self) -> String {
        self.last_error
            .clone()
            .unwrap_or_else(|| "the realtime session failed".to_string())
    }

    /// Send the tail, end the stream, and wait for the closing transcript.
    /// `Err` means the session cannot vouch for a complete transcript.
    fn finalize(&mut self) -> Result<Option<String>, String> {
        if self.failed {
            return Err(self.failure_reason());
        }

        // Send whatever is left with the commit flag, so a recording that ends
        // mid-sentence still gets its last words closed out.
        let tail: Vec<f32> = std::mem::take(&mut self.pending);
        if let Err(e) = self.send_audio(&tail, true) {
            // The tail never reached the provider, so the transcript stops
            // short of what the user said.
            self.fail_after_send(format!("the final commit failed: {e}"));
            self.close_socket();
            return Err(self.failure_reason());
        }
        if self.protocol == Protocol::Deepgram {
            // Deepgram has no per-chunk commit flag; closing the stream is what
            // flushes it.
            let socket = &mut self.socket;
            let _ = self.runtime.block_on(async {
                tokio::time::timeout(
                    SEND_TIMEOUT,
                    socket.send(Message::Text(r#"{"type":"CloseStream"}"#.into())),
                )
                .await
            });
        }

        let quiet_period = match self.protocol {
            Protocol::AmazonTranscribe => AMAZON_FINALIZE_QUIET_PERIOD,
            Protocol::ElevenLabs | Protocol::Deepgram => FINALIZE_QUIET_PERIOD,
        };
        let deadline = Instant::now() + FINALIZE_TIMEOUT;
        let mut last_commit: Option<Instant> = None;
        loop {
            if let Some(at) = last_commit {
                if at.elapsed() >= quiet_period {
                    break;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                warn!("Realtime cloud STT finalize timed out; using whatever was committed");
                break;
            }
            // Bounded so the quiet-period check runs even while messages flow.
            let wait = remaining.min(quiet_period);
            let socket = &mut self.socket;
            let received = self
                .runtime
                .block_on(async { tokio::time::timeout(wait, socket.next()).await });
            match received {
                // No message within the slice: if a commit has landed we are
                // done, otherwise keep waiting until the deadline.
                Err(_) => {
                    if last_commit.is_some() {
                        break;
                    }
                }
                Ok(None) | Ok(Some(Ok(Message::Close(_)))) => break,
                Ok(Some(Err(e))) => {
                    warn!("Realtime cloud STT closed during finalize: {e}");
                    // The socket died before the flush completed, so whatever is
                    // in hand may be missing the end of the sentence. Treat it
                    // as a failed session rather than pasting a truncated
                    // transcript as if it were complete.
                    self.mark_failed(format!("the connection dropped before the last words: {e}"));
                    break;
                }
                Ok(Some(Ok(message))) => {
                    for event in self.decode(&message) {
                        if self.apply(event) {
                            last_commit = Some(Instant::now());
                        }
                    }
                    // `apply` sets this on a provider error event.
                    if self.failed {
                        break;
                    }
                }
            }
        }

        self.close_socket();

        if self.failed {
            return Err(self.failure_reason());
        }
        Ok(self.transcript.text())
    }

    /// Abandon the session without producing a result (the user cancelled).
    pub fn abort(mut self) {
        self.close_socket();
    }

    /// Close the socket, bounded. By this point the transcript is already in
    /// hand, so a peer that never answers the handshake must not hold the
    /// pipeline.
    fn close_socket(&mut self) {
        let socket = &mut self.socket;
        let _ = self
            .runtime
            .block_on(async { tokio::time::timeout(CLOSE_TIMEOUT, socket.close(None)).await });
    }

    /// Encode and send one audio chunk. `commit` asks the provider to close the
    /// current segment (ElevenLabs), or ends the stream (Amazon Transcribe);
    /// Deepgram commits on stream close.
    fn send_audio(&mut self, samples: &[f32], commit: bool) -> Result<(), String> {
        if samples.is_empty() && !commit {
            return Ok(());
        }
        let pcm = to_pcm16_le(samples);
        let messages = match self.protocol {
            Protocol::ElevenLabs => {
                let payload = serde_json::json!({
                    "message_type": "input_audio_chunk",
                    "audio_base_64": base64::engine::general_purpose::STANDARD.encode(&pcm),
                    "commit": commit,
                    "sample_rate": WHISPER_SAMPLE_RATE,
                });
                vec![Message::Text(payload.to_string().into())]
            }
            // Raw PCM frames; an empty frame is how this protocol signals the
            // end of audio, so it must not be skipped.
            Protocol::Deepgram => vec![Message::Binary(pcm.into())],
            // One `AudioEvent` per chunk. The commit is an empty `AudioEvent`
            // after the tail: that is how AWS is told the stream is over, and it
            // answers with the last transcripts and closes the socket itself.
            Protocol::AmazonTranscribe => {
                let mut frames = Vec::with_capacity(2);
                if !pcm.is_empty() {
                    frames.push(Message::Binary(crate::stt_aws::audio_event(&pcm).into()));
                }
                if commit {
                    frames.push(Message::Binary(crate::stt_aws::audio_event(&[]).into()));
                }
                frames
            }
        };
        for message in messages {
            let socket = &mut self.socket;
            match self
                .runtime
                .block_on(async { tokio::time::timeout(SEND_TIMEOUT, socket.send(message)).await })
            {
                Ok(result) => result.map_err(|e| e.to_string())?,
                Err(_) => {
                    return Err(format!(
                        "the provider stopped accepting audio within {}s",
                        SEND_TIMEOUT.as_secs()
                    ))
                }
            }
        }
        Ok(())
    }

    /// Consume messages that have already arrived. Bounded by [`DRAIN_BUDGET`],
    /// with each individual read bounded by [`DRAIN_POLL`], so this returns
    /// promptly whether the socket is silent or busy.
    fn drain_ready(&mut self) {
        let deadline = Instant::now() + DRAIN_BUDGET;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            let poll = DRAIN_POLL.min(remaining);
            let socket = &mut self.socket;
            let received = self
                .runtime
                .block_on(async { tokio::time::timeout(poll, socket.next()).await });
            match received {
                // Nothing arrived in the poll window — the normal case.
                Err(_) => return,
                Ok(None) => {
                    debug!("Realtime cloud STT stream ended early");
                    self.mark_failed("the provider closed the stream".to_string());
                    return;
                }
                Ok(Some(Err(e))) => {
                    warn!("Realtime cloud STT receive failed: {e}");
                    self.mark_failed(format!("receiving failed: {e}"));
                    return;
                }
                Ok(Some(Ok(message))) => {
                    for event in self.decode(&message) {
                        self.apply(event);
                        // A protocol-level error is terminal; stop reading.
                        if self.failed {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// The events in one socket message. ElevenLabs and Deepgram send JSON text;
    /// Amazon Transcribe sends binary event-stream frames, which can carry more
    /// than one result. Pings are answered by the library.
    fn decode(&mut self, message: &Message) -> Vec<ServerEvent> {
        match (self.protocol, message) {
            (Protocol::ElevenLabs, Message::Text(text)) => {
                parse_elevenlabs(text.as_str()).into_iter().collect()
            }
            (Protocol::Deepgram, Message::Text(text)) => {
                parse_deepgram(text.as_str()).into_iter().collect()
            }
            (Protocol::AmazonTranscribe, Message::Binary(bytes)) => parse_amazon(bytes),
            _ => Vec::new(),
        }
    }

    /// Fold an event into the running transcript and push it to the overlay.
    /// Returns whether this event committed text.
    fn apply(&mut self, event: ServerEvent) -> bool {
        let (changed, did_commit) = match event {
            ServerEvent::SessionStarted => return false,
            ServerEvent::Partial(text) => (self.transcript.apply_partial(text), false),
            ServerEvent::Committed(text) => (self.transcript.apply_commit(&text), true),
            ServerEvent::Error(message) => {
                // Errors here are terminal for the socket but not for the
                // dictation: mark the session dead and let the caller batch.
                warn!("Realtime cloud STT error: {message}");
                self.mark_failed(message);
                return false;
            }
        };
        if changed {
            self.emit();
        }
        did_commit
    }

    /// Publish the running transcript. In the app this lands on the same untyped
    /// `stream-text` event the local streaming engines use, so the overlay needs
    /// no cloud-specific handling.
    fn emit(&self) {
        let tentative = self.transcript.display_tentative();
        (self.sink)(&self.transcript.committed, &tentative);
    }
}

/// Normalised view of the events every provider sends.
enum ServerEvent {
    SessionStarted,
    Partial(String),
    Committed(String),
    Error(String),
}

/// A refused or failed connection, in words. Amazon Transcribe rejects a bad
/// signature, an unknown key or a missing permission at the upgrade, with an
/// HTTP status and a JSON body, so that case gets AWS's own reason; the others
/// keep the library's message.
fn describe_connect_error(
    protocol: Protocol,
    error: tokio_tungstenite::tungstenite::Error,
) -> String {
    match (protocol, &error) {
        (Protocol::AmazonTranscribe, tokio_tungstenite::tungstenite::Error::Http(response)) => {
            crate::stt_aws::describe_handshake_failure(
                response.status().as_u16(),
                response.body().as_deref().unwrap_or_default(),
            )
        }
        _ => format!("Realtime connection refused: {error}"),
    }
}

/// Transcribe a finished recording through a fresh realtime session: the batch
/// path of a provider that has no batch endpoint ([`streams_only`]).
///
/// The audio is paced at speaking speed. AWS's own streaming SDK says the
/// service processes a stream in real time and that pre-recorded audio "should
/// be rate limited to match the appropriate realtime bitrate"; sending it faster
/// only queues it on AWS's side. So this takes as long as the recording, plus the
/// connection and the closing transcript. Every wait inside is bounded (connect,
/// each send, the finalize), so the call is bounded by the recording's length.
///
/// The session runs on a thread of its own because it drives its own runtime,
/// and this is called from inside async tasks as well as from blocking threads.
pub(crate) fn transcribe_by_streaming(
    cfg: &ResolvedCloudStt,
    samples: &[f32],
) -> Result<String, String> {
    let cfg = cfg.clone();
    let samples = samples.to_vec();
    std::thread::Builder::new()
        .name("cloud-stt-replay".to_string())
        .spawn(move || replay(&cfg, &samples))
        .map_err(|e| format!("Could not start the transcription thread: {e}"))?
        .join()
        .map_err(|_| "The transcription thread stopped unexpectedly".to_string())?
}

fn replay(cfg: &ResolvedCloudStt, samples: &[f32]) -> Result<String, String> {
    let label = cfg.provider.label.clone();
    // Errors that already name the provider are AWS's own, described; the rest
    // come from the socket and need saying whose socket it was.
    let named = move |error: String| {
        if error.contains(&label) {
            error
        } else {
            format!("{label}: {error}")
        }
    };
    let mut session = CloudStreamSession::connect(Box::new(|_, _| {}), cfg).map_err(&named)?;
    let started = Instant::now();
    for (index, chunk) in samples.chunks(CHUNK_SAMPLES).enumerate() {
        let due = started + Duration::from_millis((index * CHUNK_MS) as u64);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        session.feed(chunk);
        if session.has_failed() {
            let reason = session.failure_reason();
            session.abort();
            return Err(named(reason));
        }
    }
    session.finish_result().map_err(named)
}

/// `wss://api.elevenlabs.io/v1/speech-to-text/realtime`
///
/// `commit_strategy=vad` lets the provider close a segment at each natural
/// pause, which is what produces an append-only committed prefix instead of one
/// enormous commit at the end. The final flush still forces a commit for the
/// tail after the last pause.
fn elevenlabs_url(cfg: &ResolvedCloudStt) -> String {
    let ws_base = cfg
        .base_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let mut url = format!(
        "{ws_base}/v1/speech-to-text/realtime?model_id={}&audio_format=pcm_{}&commit_strategy=vad",
        urlencode(&cfg.model),
        WHISPER_SAMPLE_RATE
    );
    if let Some(language) = &cfg.language {
        url.push_str(&format!("&language_code={}", urlencode(language)));
    }
    if cfg.no_verbatim {
        url.push_str("&no_verbatim=true");
    }
    for term in &cfg.keyterms {
        url.push_str(&format!("&keyterms={}", urlencode(term)));
    }
    url
}

/// `wss://api.deepgram.com/v1/listen`
fn deepgram_url(cfg: &ResolvedCloudStt) -> String {
    let ws_base = cfg
        .base_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let mut url = format!(
        "{ws_base}/v1/listen?model={}&encoding=linear16&sample_rate={}&channels=1\
         &smart_format=true&punctuate=true&interim_results=true",
        urlencode(&cfg.model),
        WHISPER_SAMPLE_RATE
    );
    if let Some(language) = &cfg.language {
        url.push_str(&format!("&language={}", urlencode(language)));
    }
    if cfg.no_verbatim {
        url.push_str("&filler_words=false");
    }
    for term in &cfg.keyterms {
        url.push_str(&format!("&keyterm={}", urlencode(term)));
    }
    url
}

/// Percent-encode a query-parameter value. Keyterms are user-supplied and can
/// contain spaces, ampersands, and non-ASCII text, any of which would otherwise
/// corrupt the query string.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 16 kHz mono `f32` to little-endian 16-bit PCM, the encoding both realtime
/// endpoints are configured for above.
fn to_pcm16_le(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn parse_elevenlabs(text: &str) -> Option<ServerEvent> {
    #[derive(Deserialize)]
    struct Envelope {
        message_type: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        error: Option<String>,
    }
    let envelope: Envelope = serde_json::from_str(text).ok()?;
    match envelope.message_type.as_str() {
        "session_started" => Some(ServerEvent::SessionStarted),
        "partial_transcript" => Some(ServerEvent::Partial(envelope.text.unwrap_or_default())),
        "committed_transcript" => Some(ServerEvent::Committed(envelope.text.unwrap_or_default())),
        // The timestamped variant repeats text already committed by the plain
        // message; folding it in again would duplicate every segment.
        "committed_transcript_with_timestamps" | "committed_transcript_entities" => None,
        // Non-fatal notice, not an error.
        "warning" => {
            debug!("Realtime cloud STT warning: {text}");
            None
        }
        other
            if other.contains("error")
                || matches!(
                    other,
                    "quota_exceeded"
                        | "commit_throttled"
                        | "unaccepted_terms"
                        | "rate_limited"
                        | "queue_overflow"
                        | "resource_exhausted"
                        | "session_time_limit_exceeded"
                        | "invalid_request"
                        | "chunk_size_exceeded"
                        | "insufficient_audio_activity"
                ) =>
        {
            Some(ServerEvent::Error(
                envelope
                    .error
                    .unwrap_or_else(|| envelope.message_type.clone()),
            ))
        }
        _ => None,
    }
}

fn parse_deepgram(text: &str) -> Option<ServerEvent> {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(rename = "type", default)]
        kind: Option<String>,
        #[serde(default)]
        is_final: bool,
        #[serde(default)]
        channel: Option<Channel>,
        #[serde(default)]
        description: Option<String>,
    }
    #[derive(Deserialize)]
    struct Channel {
        #[serde(default)]
        alternatives: Vec<Alternative>,
    }
    #[derive(Deserialize)]
    struct Alternative {
        #[serde(default)]
        transcript: String,
    }
    let envelope: Envelope = serde_json::from_str(text).ok()?;
    match envelope.kind.as_deref() {
        Some("Results") | None => {
            let transcript = envelope
                .channel?
                .alternatives
                .into_iter()
                .next()
                .map(|a| a.transcript)
                .unwrap_or_default();
            // An empty interim result is a heartbeat, not a correction.
            if transcript.trim().is_empty() && !envelope.is_final {
                return None;
            }
            if envelope.is_final {
                Some(ServerEvent::Committed(transcript))
            } else {
                Some(ServerEvent::Partial(transcript))
            }
        }
        Some("Error") => Some(ServerEvent::Error(
            envelope.description.unwrap_or_else(|| text.to_string()),
        )),
        // Metadata / SpeechStarted / UtteranceEnd carry no transcript.
        _ => None,
    }
}

/// Amazon Transcribe's binary frames, as the same events the JSON providers
/// produce. A result's `IsPartial` draft is the segment's tentative tail and its
/// final version is the commit, exactly the split [`LiveTranscript`] keeps. One
/// frame can carry several results — a segment closing and the next one's first
/// draft — and they are applied in the order AWS listed them.
fn parse_amazon(bytes: &[u8]) -> Vec<ServerEvent> {
    use crate::stt_aws::ServerMessage;
    let messages = match crate::stt_aws::decode_messages(bytes) {
        Ok(messages) => messages,
        // A frame that fails its checksum cannot be trusted for anything,
        // including the transcript it seems to carry.
        Err(e) => {
            return vec![ServerEvent::Error(format!(
                "Amazon Transcribe sent an unreadable frame: {e}"
            ))]
        }
    };
    let mut events = Vec::new();
    for message in &messages {
        match crate::stt_aws::interpret(message) {
            ServerMessage::Results(results) => {
                for result in results {
                    if result.is_partial {
                        // An empty draft is a heartbeat, not a correction.
                        if !result.text.is_empty() {
                            events.push(ServerEvent::Partial(result.text));
                        }
                    } else {
                        if let Some(language) = &result.language {
                            debug!("Amazon Transcribe identified the language as {language}");
                        }
                        events.push(ServerEvent::Committed(result.text));
                    }
                }
            }
            ServerMessage::Exception { kind, message } => events.push(ServerEvent::Error(
                crate::stt_aws::describe_failure(&kind, &message),
            )),
            ServerMessage::Ignored => {}
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{get_default_settings, SttEngineMode};

    fn config_for(provider_id: &str, model: &str) -> ResolvedCloudStt {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.cloud_stt_provider_id = provider_id.to_string();
        settings
            .cloud_stt_api_keys
            .insert(provider_id.to_string(), "key".to_string());
        settings
            .cloud_stt_models
            .insert(provider_id.to_string(), model.to_string());
        crate::stt_cloud::resolve_cloud_stt(&settings).expect("should resolve")
    }

    #[test]
    fn streaming_requires_a_realtime_elevenlabs_model() {
        assert!(supports_streaming(&config_for(
            "elevenlabs",
            "scribe_v2_realtime"
        )));
        // The batch model has no realtime endpoint, so streaming must not claim
        // support and send audio that gets rejected.
        assert!(!supports_streaming(&config_for("elevenlabs", "scribe_v2")));
    }

    #[test]
    fn openai_compatible_providers_never_stream() {
        assert!(!supports_streaming(&config_for("groq", "whisper-large-v3")));
        assert!(!supports_streaming(&config_for(
            "openai",
            "gpt-4o-transcribe"
        )));
    }

    #[test]
    fn deepgram_always_streams() {
        assert!(supports_streaming(&config_for("deepgram", "nova-3")));
    }

    #[test]
    fn elevenlabs_url_is_wss_with_pcm_and_vad_commits() {
        let url = elevenlabs_url(&config_for("elevenlabs", "scribe_v2_realtime"));
        assert!(url.starts_with("wss://api.elevenlabs.io/v1/speech-to-text/realtime?"));
        assert!(url.contains("model_id=scribe_v2_realtime"));
        assert!(url.contains("audio_format=pcm_16000"));
        assert!(url.contains("commit_strategy=vad"));
    }

    #[test]
    fn deepgram_url_declares_the_raw_pcm_it_will_receive() {
        let url = deepgram_url(&config_for("deepgram", "nova-3"));
        assert!(url.starts_with("wss://api.deepgram.com/v1/listen?"));
        assert!(url.contains("encoding=linear16"));
        assert!(url.contains("sample_rate=16000"));
        assert!(url.contains("interim_results=true"));
    }

    #[test]
    fn keyterms_are_percent_encoded_into_the_query() {
        let mut cfg = config_for("elevenlabs", "scribe_v2_realtime");
        cfg.keyterms = vec!["Kiro & Co".to_string()];
        let url = elevenlabs_url(&cfg);
        assert!(url.contains("keyterms=Kiro%20%26%20Co"), "got {url}");
        assert!(!url.contains("Kiro & Co"));
    }

    #[test]
    fn language_is_omitted_on_auto_detect() {
        let cfg = config_for("elevenlabs", "scribe_v2_realtime");
        assert!(cfg.language.is_none());
        assert!(!elevenlabs_url(&cfg).contains("language_code"));
    }

    #[test]
    fn elevenlabs_events_map_onto_partial_and_committed() {
        assert!(matches!(
            parse_elevenlabs(r#"{"message_type":"session_started","session_id":"s","config":{}}"#),
            Some(ServerEvent::SessionStarted)
        ));
        assert!(matches!(
            parse_elevenlabs(r#"{"message_type":"partial_transcript","text":"hello"}"#),
            Some(ServerEvent::Partial(t)) if t == "hello"
        ));
        assert!(matches!(
            parse_elevenlabs(r#"{"message_type":"committed_transcript","text":"hello there"}"#),
            Some(ServerEvent::Committed(t)) if t == "hello there"
        ));
        assert!(matches!(
            parse_elevenlabs(r#"{"message_type":"auth_error","error":"bad key"}"#),
            Some(ServerEvent::Error(m)) if m == "bad key"
        ));
        assert!(matches!(
            parse_elevenlabs(r#"{"message_type":"quota_exceeded","error":"no credit"}"#),
            Some(ServerEvent::Error(m)) if m == "no credit"
        ));
    }

    #[test]
    fn timestamped_commits_are_ignored_so_text_is_not_duplicated() {
        assert!(parse_elevenlabs(
            r#"{"message_type":"committed_transcript_with_timestamps","text":"hello","words":[]}"#
        )
        .is_none());
    }

    #[test]
    fn deepgram_interim_and_final_results_are_distinguished() {
        let interim = r#"{"type":"Results","is_final":false,
            "channel":{"alternatives":[{"transcript":"hello"}]}}"#;
        assert!(matches!(
            parse_deepgram(interim),
            Some(ServerEvent::Partial(t)) if t == "hello"
        ));
        let final_result = r#"{"type":"Results","is_final":true,
            "channel":{"alternatives":[{"transcript":"hello there"}]}}"#;
        assert!(matches!(
            parse_deepgram(final_result),
            Some(ServerEvent::Committed(t)) if t == "hello there"
        ));
        // Empty interim results are heartbeats.
        let heartbeat = r#"{"type":"Results","is_final":false,
            "channel":{"alternatives":[{"transcript":""}]}}"#;
        assert!(parse_deepgram(heartbeat).is_none());
        assert!(parse_deepgram(r#"{"type":"Metadata"}"#).is_none());
    }

    #[test]
    fn pcm_conversion_is_little_endian_and_clamped() {
        assert_eq!(to_pcm16_le(&[0.0]), vec![0, 0]);
        assert_eq!(to_pcm16_le(&[1.0]), i16::MAX.to_le_bytes().to_vec());
        assert_eq!(to_pcm16_le(&[-1.0]), (-i16::MAX).to_le_bytes().to_vec());
        assert_eq!(to_pcm16_le(&[9.0]), i16::MAX.to_le_bytes().to_vec());
        assert_eq!(to_pcm16_le(&[]).len(), 0);
    }

    #[test]
    fn chunking_targets_100ms_at_the_capture_rate() {
        assert_eq!(CHUNK_SAMPLES, 1600);
    }

    fn folded(events: &[(&str, &str)]) -> LiveTranscript {
        let mut transcript = LiveTranscript::default();
        for (kind, text) in events {
            match *kind {
                "partial" => {
                    transcript.apply_partial(text.to_string());
                }
                "commit" => {
                    transcript.apply_commit(text);
                }
                other => panic!("unknown event {other}"),
            }
        }
        transcript
    }

    #[test]
    fn a_partial_replayed_after_its_commit_is_not_pasted_twice() {
        // Both sequences are from real dictations, where the pasted text came
        // back as the whole transcript twice.
        let exact = folded(&[
            ("partial", "Yeah, so this is the new window"),
            (
                "commit",
                "Yeah, so this is the new window, and I still don't like the colors.",
            ),
            (
                "partial",
                "Yeah, so this is the new window, and I still don't like the colors",
            ),
        ]);
        assert_eq!(
            exact.text().as_deref(),
            Some("Yeah, so this is the new window, and I still don't like the colors.")
        );
        let earlier_draft = folded(&[
            (
                "commit",
                "The colors are not something that's satisfying or good.",
            ),
            (
                "partial",
                "The colors are not something that's satisfying our",
            ),
        ]);
        assert_eq!(
            earlier_draft.text().as_deref(),
            Some("The colors are not something that's satisfying or good.")
        );
        assert!(earlier_draft.tentative.is_empty());
    }

    #[test]
    fn the_next_segment_is_still_shown_and_kept() {
        let transcript = folded(&[
            ("commit", "Thank you."),
            ("partial", ""),
            ("partial", "and then I left"),
        ]);
        assert_eq!(transcript.tentative, "and then I left");
        // Drawn straight after the committed text, so it carries the space.
        assert_eq!(transcript.display_tentative(), " and then I left");
        assert_eq!(
            transcript.text().as_deref(),
            Some("Thank you. and then I left")
        );
        // A new segment may open with the last one's first word.
        let echo = folded(&[("commit", "Okay."), ("partial", "okay so")]);
        assert_eq!(echo.tentative, "okay so");
    }

    #[test]
    fn once_a_new_segment_starts_a_repeated_sentence_is_kept() {
        // Only the window straight after a commit is watched; saying the same
        // thing twice on purpose must survive.
        let transcript = folded(&[
            ("commit", "Testing one two three."),
            ("partial", "hello"),
            ("partial", "testing one two three"),
            ("commit", "Testing one two three."),
        ]);
        assert_eq!(
            transcript.text().as_deref(),
            Some("Testing one two three. Testing one two three.")
        );
    }

    #[test]
    fn the_tail_is_spaced_once_from_the_committed_text() {
        let mut transcript = LiveTranscript::default();
        transcript.apply_partial("hello".to_string());
        // Nothing committed yet, so nothing to separate it from.
        assert_eq!(transcript.display_tentative(), "hello");
        transcript.apply_commit("Hello.");
        transcript.apply_partial(" again".to_string());
        assert_eq!(transcript.display_tentative(), " again");
    }

    #[test]
    fn replay_detection_needs_most_of_the_words_in_order() {
        let segment = transcript_words("So this is going to be how it looks.");
        assert!(is_replay_of(
            &segment,
            &transcript_words("so this is going to be")
        ));
        assert!(!is_replay_of(
            &segment,
            &transcript_words("and something else entirely")
        ));
        assert!(is_replay_of(
            &transcript_words("Thank you."),
            &transcript_words("thank you")
        ));
        assert!(!is_replay_of(
            &transcript_words("Thank you."),
            &transcript_words("thank")
        ));
        assert!(!is_replay_of(&segment, &[]));
    }

    fn amazon_config() -> ResolvedCloudStt {
        let mut settings = get_default_settings();
        settings.stt_engine_mode = SttEngineMode::Cloud;
        settings.cloud_stt_provider_id = crate::settings::CLOUD_STT_AMAZON.to_string();
        settings.cloud_stt_api_keys.insert(
            crate::settings::CLOUD_STT_AMAZON.to_string(),
            "AKIAIOSFODNN7EXAMPLE:wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        );
        settings.selected_language = "en".to_string();
        crate::stt_cloud::resolve_cloud_stt(&settings).expect("should resolve")
    }

    fn transcript_frame(json: &str) -> Vec<u8> {
        crate::stt_aws::encode_message(
            &[
                (":content-type", "application/json"),
                (":event-type", "TranscriptEvent"),
                (":message-type", "event"),
            ],
            json.as_bytes(),
        )
    }

    fn exception_frame(kind: &str, message: &str) -> Vec<u8> {
        crate::stt_aws::encode_message(
            &[
                (":content-type", "application/json"),
                (":exception-type", kind),
                (":message-type", "exception"),
            ],
            serde_json::json!({ "Message": message })
                .to_string()
                .as_bytes(),
        )
    }

    #[test]
    fn amazon_transcribe_streams_and_has_nothing_else() {
        let amazon = amazon_config();
        assert!(supports_streaming(&amazon));
        assert!(streams_only(&amazon));
        assert!(!streams_only(&config_for("deepgram", "nova-3")));
        assert!(!streams_only(&config_for(
            "elevenlabs",
            "scribe_v2_realtime"
        )));
    }

    #[test]
    fn amazon_results_fold_into_the_live_transcript() {
        let frames = [
            r#"{"Transcript":{"Results":[{"Alternatives":[{"Transcript":"The Amazon"}],"IsPartial":true,"ResultId":"a"}]}}"#,
            r#"{"Transcript":{"Results":[]}}"#,
            r#"{"Transcript":{"Results":[
                {"Alternatives":[{"Transcript":"The Amazon is the largest rainforest."}],"IsPartial":false,"ResultId":"a"},
                {"Alternatives":[{"Transcript":"On"}],"IsPartial":true,"ResultId":"b"}
            ]}}"#,
            r#"{"Transcript":{"Results":[{"Alternatives":[{"Transcript":"On the planet."}],"IsPartial":false,"ResultId":"b"}]}}"#,
        ];
        let mut transcript = LiveTranscript::default();
        for frame in frames {
            for event in parse_amazon(&transcript_frame(frame)) {
                match event {
                    ServerEvent::Partial(text) => {
                        transcript.apply_partial(text);
                    }
                    ServerEvent::Committed(text) => {
                        transcript.apply_commit(&text);
                    }
                    _ => panic!("only transcript events were sent"),
                }
            }
        }
        assert_eq!(
            transcript.text().as_deref(),
            Some("The Amazon is the largest rainforest. On the planet.")
        );
    }

    #[test]
    fn an_amazon_exception_or_a_corrupt_frame_ends_the_session() {
        let events = parse_amazon(&exception_frame(
            "UnrecognizedClientException",
            "The security token included in the request is invalid.",
        ));
        assert!(
            matches!(events.as_slice(), [ServerEvent::Error(message)] if message.contains("access key")),
            "an exception should end the session with a reason"
        );

        let mut corrupt = transcript_frame(r#"{"Transcript":{"Results":[]}}"#);
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xFF;
        assert!(matches!(
            parse_amazon(&corrupt).as_slice(),
            [ServerEvent::Error(message)] if message.contains("unreadable")
        ));

        // An empty draft is a heartbeat, not a correction.
        assert!(parse_amazon(&transcript_frame(
            r#"{"Transcript":{"Results":[{"Alternatives":[{"Transcript":""}],"IsPartial":true}]}}"#
        ))
        .is_empty());
    }

    /// What a stand-in AWS server received.
    struct StandInReport {
        audio_bytes: usize,
        ended: bool,
    }

    /// The stand-in tests share [`AMAZON_TEST_URL`], so they take turns.
    static STAND_IN: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Serve one Amazon Transcribe session on a local port, the way AWS does:
    /// every client frame is decoded with the real event-stream decoder and must
    /// be an `AudioEvent`; the stream's end (an empty one) is answered with
    /// `replies` and a close. With `reply_at_once`, the replies go back after the
    /// first audio frame instead — how AWS ends a session with an exception.
    fn stand_in_aws(
        replies: Vec<Vec<u8>>,
        reply_at_once: bool,
    ) -> (String, std::thread::JoinHandle<StandInReport>) {
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                port_tx.send(listener.local_addr().unwrap().port()).unwrap();
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let mut report = StandInReport {
                    audio_bytes: 0,
                    ended: false,
                };
                while let Some(Ok(message)) = socket.next().await {
                    let Message::Binary(bytes) = message else {
                        continue;
                    };
                    for frame in crate::stt_aws::decode_messages(&bytes)
                        .expect("every frame the client sends must decode")
                    {
                        assert_eq!(frame.header_str(":event-type"), Some("AudioEvent"));
                        assert_eq!(frame.header_str(":message-type"), Some("event"));
                        report.audio_bytes += frame.payload.len();
                        report.ended |= frame.payload.is_empty();
                        if report.ended || reply_at_once {
                            for reply in &replies {
                                socket
                                    .send(Message::Binary(reply.clone().into()))
                                    .await
                                    .unwrap();
                            }
                            let _ = socket.close(None).await;
                            return report;
                        }
                    }
                }
                report
            })
        });
        let port = port_rx.recv().unwrap();
        (
            format!("ws://127.0.0.1:{port}/stream-transcription-websocket"),
            server,
        )
    }

    /// The whole replay path, offline: framing, pacing, the end of the stream,
    /// and AWS closing the socket after the last transcript.
    #[test]
    fn a_recording_replays_through_a_stand_in_aws_session() {
        let _turn = STAND_IN.lock().unwrap_or_else(|e| e.into_inner());
        let (url, server) = stand_in_aws(
            vec![
                transcript_frame(
                    r#"{"Transcript":{"Results":[{"Alternatives":[{"Transcript":"Hello"}],"IsPartial":true}]}}"#,
                ),
                transcript_frame(
                    r#"{"Transcript":{"Results":[{"Alternatives":[{"Transcript":"Hello there."}],"IsPartial":false}]}}"#,
                ),
            ],
            false,
        );
        *AMAZON_TEST_URL.lock().unwrap() = Some(url);

        // Half a second of audio: five 100 ms chunks.
        let samples = vec![0.1f32; 8_000];
        let started = Instant::now();
        let text = transcribe_by_streaming(&amazon_config(), &samples);
        let elapsed = started.elapsed();
        *AMAZON_TEST_URL.lock().unwrap() = None;
        let report = server.join().unwrap();

        assert_eq!(text.as_deref(), Ok("Hello there."));
        assert_eq!(report.audio_bytes, 16_000, "every sample, as 16-bit PCM");
        assert!(
            report.ended,
            "the stream has to end with an empty AudioEvent"
        );
        // Paced at speaking speed: the fifth chunk is due at 400 ms.
        assert!(elapsed >= Duration::from_millis(400), "took {elapsed:?}");
    }

    #[test]
    fn an_exception_from_aws_is_what_a_replay_reports() {
        let _turn = STAND_IN.lock().unwrap_or_else(|e| e.into_inner());
        let (url, server) = stand_in_aws(
            vec![exception_frame(
                "UnrecognizedClientException",
                "The security token included in the request is invalid.",
            )],
            true,
        );
        *AMAZON_TEST_URL.lock().unwrap() = Some(url);
        let result = transcribe_by_streaming(&amazon_config(), &vec![0.1f32; 16_000]);
        *AMAZON_TEST_URL.lock().unwrap() = None;
        server.join().unwrap();

        let error = result.expect_err("an exception must not read as a transcript");
        assert!(error.contains("did not accept the access key"), "{error}");
    }
}
