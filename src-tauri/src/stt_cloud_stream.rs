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
//! Both providers distinguish a stable prefix from a revisable tail, which maps
//! exactly onto the overlay's existing `committed` / `tentative` split — so a
//! partial result can be shown immediately and still be corrected without the
//! text ever visibly rewriting itself behind the cursor.
//!
//! Every failure here is non-fatal: a session that cannot connect, stalls, or
//! errors mid-recording returns `None`, and the manager re-transcribes the
//! complete recording through the batch path. The user waits slightly longer and
//! loses nothing, which is the only acceptable behaviour for dictation.

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

/// Whether this provider has a realtime endpoint wired up here.
///
/// Checks the model too, not just the provider: ElevenLabs' realtime and batch
/// endpoints have separate model namespaces, and a user on `scribe_v2` is asking
/// for the batch model by name.
pub(crate) fn supports_streaming(cfg: &ResolvedCloudStt) -> bool {
    match cfg.provider.kind {
        CloudSttKind::ElevenLabs => cfg.model.contains("realtime"),
        CloudSttKind::Deepgram => true,
        CloudSttKind::OpenAiCompatible | CloudSttKind::AzureSpeech => false,
    }
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
        };
        debug!("Realtime cloud STT connecting: {url}");

        let mut request = url
            .into_client_request()
            .map_err(|e| format!("Invalid realtime URL: {e}"))?;
        let auth_value = match protocol {
            Protocol::ElevenLabs => cfg.api_key.clone(),
            Protocol::Deepgram => format!("Token {}", cfg.api_key),
        };
        let auth_header = match protocol {
            Protocol::ElevenLabs => "xi-api-key",
            Protocol::Deepgram => "Authorization",
        };
        request.headers_mut().insert(
            auth_header,
            auth_value
                .parse()
                .map_err(|_| "API key contains characters that cannot be sent".to_string())?,
        );

        let socket = runtime.block_on(async {
            match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
                .await
            {
                Ok(Ok((socket, _response))) => Ok(socket),
                Ok(Err(e)) => Err(format!("Realtime connection refused: {e}")),
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
                self.failed = true;
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

        // Send whatever is left with the commit flag, so a recording that ends
        // mid-sentence still gets its last words closed out.
        let tail: Vec<f32> = std::mem::take(&mut self.pending);
        if let Err(e) = self.send_audio(&tail, true) {
            warn!("Realtime cloud STT final commit failed: {e}");
            // The tail never reached the provider, so the transcript stops
            // short of what the user said. Batch the whole recording instead.
            self.close_socket();
            return None;
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

        let deadline = Instant::now() + FINALIZE_TIMEOUT;
        let mut last_commit: Option<Instant> = None;
        loop {
            if let Some(at) = last_commit {
                if at.elapsed() >= FINALIZE_QUIET_PERIOD {
                    break;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                warn!("Realtime cloud STT finalize timed out; using whatever was committed");
                break;
            }
            // Bounded so the quiet-period check runs even while messages flow.
            let wait = remaining.min(FINALIZE_QUIET_PERIOD);
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
                    self.failed = true;
                    break;
                }
                Ok(Some(Ok(message))) => {
                    if let Some(event) = self.decode(&message) {
                        let committed = self.apply(event);
                        if committed {
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

        // A session that broke during the flush cannot promise a complete
        // transcript. Answering `None` costs the user one extra batch request;
        // answering with a truncated transcript costs them the end of their
        // sentence, silently.
        if self.failed {
            warn!("Realtime cloud STT failed during finalize; using the complete batch result");
            return None;
        }
        self.transcript.text()
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
    /// current segment (ElevenLabs only; Deepgram commits on stream close).
    fn send_audio(&mut self, samples: &[f32], commit: bool) -> Result<(), String> {
        if samples.is_empty() && !commit {
            return Ok(());
        }
        let pcm = to_pcm16_le(samples);
        let message = match self.protocol {
            Protocol::ElevenLabs => {
                let payload = serde_json::json!({
                    "message_type": "input_audio_chunk",
                    "audio_base_64": base64::engine::general_purpose::STANDARD.encode(&pcm),
                    "commit": commit,
                    "sample_rate": WHISPER_SAMPLE_RATE,
                });
                Message::Text(payload.to_string().into())
            }
            // Raw PCM frames; an empty frame is how this protocol signals the
            // end of audio, so it must not be skipped.
            Protocol::Deepgram => Message::Binary(pcm.into()),
        };
        let socket = &mut self.socket;
        match self
            .runtime
            .block_on(async { tokio::time::timeout(SEND_TIMEOUT, socket.send(message)).await })
        {
            Ok(result) => result.map_err(|e| e.to_string()),
            Err(_) => Err(format!(
                "the provider stopped accepting audio within {}s",
                SEND_TIMEOUT.as_secs()
            )),
        }
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
                    self.failed = true;
                    return;
                }
                Ok(Some(Err(e))) => {
                    warn!("Realtime cloud STT receive failed: {e}");
                    self.failed = true;
                    return;
                }
                Ok(Some(Ok(message))) => {
                    if let Some(event) = self.decode(&message) {
                        self.apply(event);
                    }
                    // A protocol-level error is terminal; stop reading.
                    if self.failed {
                        return;
                    }
                }
            }
        }
    }

    fn decode(&mut self, message: &Message) -> Option<ServerEvent> {
        let text = match message {
            Message::Text(text) => text.as_str(),
            // Neither protocol sends binary payloads to the client; pings are
            // answered by the library.
            _ => return None,
        };
        match self.protocol {
            Protocol::ElevenLabs => parse_elevenlabs(text),
            Protocol::Deepgram => parse_deepgram(text),
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
                self.failed = true;
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

/// Normalised view of the events both providers send.
enum ServerEvent {
    SessionStarted,
    Partial(String),
    Committed(String),
    Error(String),
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
}
