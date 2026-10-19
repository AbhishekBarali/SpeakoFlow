use crate::audio_toolkit::chunking::{Chunker, MakeScorer};
use crate::audio_toolkit::{
    apply_custom_words, filter_transcription_output, normalize_peak, SileroVad,
};
use crate::engine_supervisor::{EngineError, EngineSupervisor, LoadedInfo, StreamProgress};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::language;
use crate::managers::legacy_models::is_retired_engine;
use crate::managers::model::{EngineType, ModelManager};
use crate::settings::{
    get_settings, ModelUnloadTimeout, ResolvedCloudStt, TranscribeAcceleratorSetting,
};
use anyhow::Result;
use chinese_script::LiveScript;
use log::{debug, error, info, warn};
use policy::{batch_decode, BatchDecode, LiveDecode, ModelTraits};
use serde::Serialize;
use specta::Type;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};
use transcribe_cpp::CancelToken;

#[cfg(test)]
use crate::engine_supervisor::device_key;

pub mod chinese_script;
mod chunked;
pub mod policy;

#[derive(Clone, Debug, Serialize)]
pub struct ModelStateEvent {
    pub event_type: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub error: Option<String>,
}

/// Whether a meeting needs the speech engine (recording, or draining its queue).
/// Meetings capture through their own recorder rather than
/// `AudioRecordingManager`, so the dictation `is_recording` check does not see
/// them.
fn meeting_is_recording(app: &AppHandle) -> bool {
    app.try_state::<Arc<crate::meetings::session::MeetingRecorder>>()
        .is_some_and(|recorder| recorder.uses_speech_engine())
}

thread_local! {
    /// Set when the last `transcribe` on this thread fell back from the cloud
    /// to the local model. Thread-local because `transcribe` is synchronous and
    /// runs on its caller's thread, so a dictation reads its own answer and a
    /// meeting chunk transcribing on another thread cannot leak into it.
    static CLOUD_FELL_BACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the last `transcribe` on this thread finished on the local model
/// because the cloud request failed. Reading clears it. Call it on the same
/// thread as `transcribe`, with no `.await` in between.
pub fn take_cloud_fallback() -> bool {
    CLOUD_FELL_BACK.with(|flag| flag.replace(false))
}

/// Incremental live-transcription text pushed to the recording overlay while a
/// streaming recording is in progress. Emitted as an UNTYPED Tauri event
/// ("stream-text") to match the existing "mic-level" pattern — intentionally
/// NOT a tauri-specta typed Event, so bindings.ts doesn't need regenerating.
#[derive(Clone, Debug, Serialize)]
struct StreamTextPayload {
    /// The append-only, flicker-free committed prefix: `StreamText::committed`,
    /// the stable prefix the commit policy has locked in. Cloud streaming maps
    /// its committed segments onto the same field.
    committed: String,
    /// The volatile, not-yet-committed tail (`StreamText::tentative`). Empty for
    /// cache-aware streaming models (e.g. Parakeet) that only ever commit.
    tentative: String,
}

/// How a native stream's live text is shown: in the script the paste will be
/// in, with the safe custom-word correction. Shared between the engine's
/// progress callback (every feed) and the stream worker (the final text).
struct LiveDisplay {
    script: LiveScript,
    hints: Vec<String>,
    threshold: f64,
    flow_phrase: Option<String>,
}

impl LiveDisplay {
    /// The overlay's committed and tentative text for one stream snapshot.
    fn show(&mut self, text: &transcribe_cpp::StreamText) -> (String, String) {
        let (committed, tentative) = self.script.apply(&text.committed, &text.tentative);
        let committed = correct_live_display(
            &committed,
            &self.hints,
            self.threshold,
            self.flow_phrase.as_deref(),
        );
        (committed, tentative)
    }
}

fn lock_display(display: &Mutex<LiveDisplay>) -> std::sync::MutexGuard<'_, LiveDisplay> {
    display.lock().unwrap_or_else(|e| e.into_inner())
}

/// The error a cancelled transcription (Esc) ends with, so the dictation
/// pipeline can tell "the user stopped it" from "it failed" and offer it back
/// as dismissed instead of as a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptionCancelled;

impl std::fmt::Display for TranscriptionCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the transcription was cancelled")
    }
}

impl std::error::Error for TranscriptionCancelled {}

/// Whether `error` is a cancelled transcription.
pub fn is_cancellation(error: &anyhow::Error) -> bool {
    error.downcast_ref::<TranscriptionCancelled>().is_some()
}

/// The one transcription engine of this process, for the device list. Every
/// transcribe.cpp call goes through it, device enumeration included, and the
/// free functions below (`gpu_devices`, which `local_llm` reads without an
/// `AppHandle`) need to find it.
static ENGINE: OnceLock<EngineSupervisor> = OnceLock::new();

/// The message shown when the selected model belongs to an engine SpeakoFlow no
/// longer ships (the ONNX models `transcribe-rs` ran; see
/// [`crate::managers::legacy_models`]). Such a model stays on disk but is never
/// loaded, so this is what the Models page and the loading-failed toast say.
pub fn retired_engine_message(model_name: &str) -> String {
    format!(
        "{model_name} used an older engine that SpeakoFlow no longer includes. \
         Download its new version on the Models page."
    )
}

/// Replace NaN and infinite samples with silence.
///
/// transcribe.cpp 0.4 rejects a whole run (or a stream feed) when any sample is
/// not finite, which would turn one bad sample from a misbehaving audio driver
/// into a failed dictation. Silence is the value that changes nothing about
/// what was said. Returns how many samples were replaced, for the log.
fn sanitize_pcm(pcm: &mut [f32]) -> usize {
    let mut replaced = 0;
    for sample in pcm.iter_mut() {
        if !sample.is_finite() {
            *sample = 0.0;
            replaced += 1;
        }
    }
    replaced
}

/// Command sent to the live-transcription worker over a single FIFO channel.
/// Frames and the terminal Finalize/Cancel share the channel, so every fed
/// frame is guaranteed to be processed before finalization.
enum StreamCmd {
    /// A 16 kHz mono frame: every frame for a stream, the kept speech for
    /// long-form chunks (see [`FeedKind`]).
    Feed(Vec<f32>),
    /// Flush the tail, merge, and reply with the full transcript (or `None` to
    /// send the caller to the batch path). `recorded_samples` is the length of
    /// the recording the batch path would transcribe, so a decode that saw
    /// different audio can tell.
    Finalize {
        recorded_samples: usize,
        reply: mpsc::Sender<Option<LiveResult>>,
    },
    /// Abort the session without producing a result.
    Cancel,
}

/// A live session's result, and what its text post-processing needs to know.
struct LiveResult {
    text: String,
    facts: RunFacts,
}

/// Which of the recorder's two feeds a live session takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeedKind {
    /// Every frame, silence included: a stream decides where speech ends from
    /// the pauses it hears.
    Raw,
    /// Only what the VAD keeps, which is exactly the recording the batch path
    /// would transcribe, so live chunks and a batch run agree.
    Speech,
}

/// What a session's worker gets from [`StreamRouter::open`].
struct OpenedStream {
    rx: mpsc::Receiver<StreamCmd>,
    generation: u64,
    give_up: Arc<AtomicBool>,
    cancel: transcribe_cpp::CancelToken,
}

/// What [`StreamRouter::take`] hands back to end a session.
struct TakenStream {
    tx: mpsc::Sender<StreamCmd>,
    give_up: Arc<AtomicBool>,
    cancel: transcribe_cpp::CancelToken,
}

/// Routes raw audio frames from the recorder into the live-transcription
/// worker. `feed()` is a cheap, atomic-gated no-op when no stream is active, so
/// it is safe to call from the audio hot path unconditionally. Created once at
/// startup and shared (via `Arc`) by both the `TranscriptionManager` (which
/// starts/finalizes streams) and the `AudioRecordingManager` (whose recorder
/// feeds frames).
/// Hard cap on the number of un-consumed `Feed` frames allowed to sit in the
/// live-transcription channel at once. The recorder feeds ~30 ms frames, so
/// ~33/sec; 1000 frames ≈ 30 seconds of backlog.
///
/// The channel is FIFO and fed from the real-time audio callback. If inference
/// can't keep up with real-time (a slow/overloaded GPU, a stalled or hung
/// engine, or a runaway multi-minute recording), an *unbounded* channel would
/// grow without limit for as long as recording continues — a memory leak that
/// scales with recording length and starves the very inference that's already
/// behind. Once the worker is >~30 s behind, the live transcript is already
/// unusable, so we drop new frames (the batch `transcribe()` fallback still has
/// the complete audio) rather than let the queue grow without bound.
const MAX_QUEUED_FEED_FRAMES: usize = 1000;

/// How long the end of a dictation waits for the live stream before asking it
/// to stop. A stream that keeps up answers within about half a second even on a
/// busy machine; one still working after this is behind, and on a GPU the whole
/// recording is transcribed again in one pass faster than the backlog drains.
const STREAM_FINALIZE_PATIENCE: Duration = Duration::from_secs(2);

/// Silence fed to a buffered (chunked attention with right context) stream
/// before it finalizes. That stream decodes its last chunk with no audio after
/// it, and Parakeet Unified, trained to look about a second ahead, then drops
/// the last word ("extra inform", "going"). 560 ms brought the word back on
/// every cut-off recording tried, and changed nothing on the complete ones.
const BUFFERED_STREAM_TAIL_PAD_SAMPLES: usize = 16_000 * 560 / 1000;

pub struct StreamRouter {
    open: Arc<AtomicBool>,
    /// Whether the open session takes the kept speech rather than every frame
    /// (see [`FeedKind`]). Read on the audio path, so an atomic.
    takes_speech: AtomicBool,
    tx: Mutex<Option<mpsc::Sender<StreamCmd>>>,
    /// Raised by [`TranscriptionManager::finalize_stream`] when the worker has
    /// not answered within [`STREAM_FINALIZE_PATIENCE`]. Fresh for every
    /// session, so a late raise can never reach the next recording's worker.
    give_up: Mutex<Arc<AtomicBool>>,
    /// Cancels a decode the session's worker is running, so a cancelled
    /// recording does not wait for a long-form chunk to finish. Fresh for
    /// every session, like `give_up`.
    cancel: Mutex<transcribe_cpp::CancelToken>,
    /// Monotonic stream session identity. Opening starts a fresh generation;
    /// finalize/cancel invalidates it immediately so queued old audio cannot
    /// affect a later recording's Flow prewarm watcher.
    generation: AtomicU64,
    /// Number of `Feed` frames enqueued but not yet consumed by the worker.
    /// Gates `feed()` so the channel can never grow past
    /// [`MAX_QUEUED_FEED_FRAMES`]. Reset to 0 on each [`open`](Self::open).
    queued: AtomicUsize,
    /// Cumulative frames dropped because the worker fell behind. Surfaced in a
    /// periodic warning so a chronically-overloaded stream is visible instead
    /// of silently discarding audio. Reset on each [`open`](Self::open).
    dropped: AtomicU64,
}

impl StreamRouter {
    pub fn new() -> Self {
        Self {
            open: Arc::new(AtomicBool::new(false)),
            takes_speech: AtomicBool::new(false),
            tx: Mutex::new(None),
            give_up: Mutex::new(Arc::new(AtomicBool::new(false))),
            cancel: Mutex::new(transcribe_cpp::CancelToken::new()),
            generation: AtomicU64::new(0),
            queued: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Whether a streaming session is currently accepting frames.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    /// Frames discarded due to backpressure during the current (or most-recent)
    /// session — non-zero only when inference couldn't keep up with real-time
    /// and the bounded queue hit its cap. Reset on each [`open`](Self::open). A
    /// non-zero count means the live transcript is missing audio, so
    /// [`finalize_stream`](TranscriptionManager::finalize_stream) prefers the
    /// complete batch result instead of the truncated streamed one.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Feed one raw 16 kHz mono frame. Returns immediately (relaxed atomic
    /// load, no allocation, no lock) when no stream is active.
    ///
    /// Never blocks the audio hot path: if the worker has fallen more than
    /// [`MAX_QUEUED_FEED_FRAMES`] behind, the frame is dropped (and counted)
    /// instead of enqueued, so the channel is strictly bounded regardless of
    /// how long recording runs or how slow inference is.
    pub fn feed(&self, frame: &[f32]) {
        if !self.open.load(Ordering::Relaxed) || self.takes_speech.load(Ordering::Relaxed) {
            return;
        }
        self.enqueue(frame);
    }

    /// Feed audio the VAD kept, for a session that decodes long-form chunks
    /// while recording. The same no-op and the same bound as [`feed`].
    ///
    /// [`feed`]: Self::feed
    pub fn feed_speech(&self, kept: &[f32]) {
        if !self.open.load(Ordering::Relaxed) || !self.takes_speech.load(Ordering::Relaxed) {
            return;
        }
        self.enqueue(kept);
    }

    fn enqueue(&self, frame: &[f32]) {
        // Backpressure: bound the in-flight queue. Dropping when the worker is
        // this far behind is the correct degradation — a >30 s-late live
        // transcript is worthless anyway, and the batch fallback keeps the full
        // audio — versus an unbounded queue that leaks memory for the whole
        // recording and can never catch up.
        if self.queued.load(Ordering::Relaxed) >= MAX_QUEUED_FEED_FRAMES {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            // Log on the first drop and then sparsely (~every 6 s of drops) so a
            // struggling stream is visible without flooding the log.
            if n == 1 || n.is_multiple_of(200) {
                warn!(
                    "Live transcription can't keep up with real-time; dropping frames \
                     (backlog cap {} frames hit, {} dropped so far). The final transcript \
                     falls back to batch, so no audio is lost.",
                    MAX_QUEUED_FEED_FRAMES, n
                );
            }
            return;
        }
        if let Ok(guard) = self.tx.lock() {
            if let Some(tx) = guard.as_ref() {
                // Reserve the slot before sending so a concurrent feed sees the
                // updated backlog; roll back if the worker is already gone.
                self.queued.fetch_add(1, Ordering::Relaxed);
                if tx.send(StreamCmd::Feed(frame.to_vec())).is_err() {
                    self.queued.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Account for one `Feed` frame consumed by the worker, freeing a slot in
    /// the bounded queue. Saturating so a stale frame drained after a reset can
    /// never underflow the counter.
    fn note_feed_consumed(&self) {
        let _ = self
            .queued
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |q| q.checked_sub(1));
    }

    /// Install a fresh FIFO command channel and mark the router open for a
    /// session taking `kind` of audio, returning what its worker needs. Resets
    /// the backlog/drop counters so each session starts clean.
    fn open(&self, kind: FeedKind) -> OpenedStream {
        let (tx, rx) = mpsc::channel();
        self.queued.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
        let give_up = Arc::new(AtomicBool::new(false));
        *self.give_up.lock().unwrap() = give_up.clone();
        let cancel = transcribe_cpp::CancelToken::new();
        *self.cancel.lock().unwrap() = cancel.clone();
        *self.tx.lock().unwrap() = Some(tx);
        self.takes_speech
            .store(kind == FeedKind::Speech, Ordering::Relaxed);
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.open.store(true, Ordering::Release);
        OpenedStream {
            rx,
            generation,
            give_up,
            cancel,
        }
    }

    /// Stop accepting new frames and take the sender so the caller can push a
    /// terminal Finalize/Cancel through the same channel (after all buffered
    /// frames have been enqueued), along with that session's give-up flag and
    /// cancel token. Invalidates the worker immediately so stale queued
    /// transcript events cannot affect the next recording.
    fn take(&self) -> Option<TakenStream> {
        self.open.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        let give_up = self.give_up.lock().unwrap().clone();
        let cancel = self.cancel.lock().unwrap().clone();
        self.tx.lock().unwrap().take().map(|tx| TakenStream {
            tx,
            give_up,
            cancel,
        })
    }

    fn is_generation_current(&self, generation: u64) -> bool {
        self.open.load(Ordering::Acquire) && self.generation.load(Ordering::Acquire) == generation
    }

    /// Mark closed and drop any channel (defensive cleanup on worker exit).
    fn clear(&self) {
        self.open.store(false, Ordering::Relaxed);
        *self.tx.lock().unwrap() = None;
    }

    /// Cancel the decode of the most recent session, whether it is still open
    /// or already being finalized (by then [`take`](Self::take) has closed it
    /// to everything else).
    fn cancel_latest(&self) {
        self.cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel();
    }
}

impl Default for StreamRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard that clears the `is_loading` flag and notifies waiters on drop.
/// Ensures the loading flag is always reset, even on early returns or panics.
pub struct LoadingGuard {
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
}

impl Drop for LoadingGuard {
    fn drop(&mut self) {
        // Recover from a poisoned mutex instead of panicking — a panic inside
        // Drop calls abort(), which would kill the app on quit (Handy #1354).
        let mut is_loading = match self.is_loading.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *is_loading = false;
        self.loading_condvar.notify_all();
    }
}

#[derive(Clone)]
pub struct TranscriptionManager {
    /// The transcribe.cpp engine: owns the worker process, its model, and
    /// recovery from crashes and hangs. Two decodes never run at once on it,
    /// because it works through one queue; a batch run that arrives during a
    /// live stream waits for the stream to end.
    engine: EngineSupervisor,
    model_manager: Arc<ModelManager>,
    app_handle: AppHandle,
    current_model_id: Arc<Mutex<Option<String>>>,
    last_activity: Arc<AtomicU64>,
    shutdown_signal: Arc<AtomicBool>,
    watcher_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
    /// Cancels the dictation transcription in progress (Esc). Replaced by a
    /// fresh token on every cancel, so a cancel never reaches a dictation that
    /// starts after it. Meetings, History and calls do not use it: Esc during a
    /// dictation must not cut a hole in a meeting transcript.
    dictation_cancel: Arc<Mutex<CancelToken>>,
    /// Router for the opt-in live/streaming transcription path. Shared with the
    /// `AudioRecordingManager` so the recorder can feed frames into an active
    /// stream. When no stream is active, feeding is a cheap no-op.
    stream_router: Arc<StreamRouter>,
}

impl TranscriptionManager {
    pub fn new(
        app_handle: &AppHandle,
        model_manager: Arc<ModelManager>,
        stream_router: Arc<StreamRouter>,
    ) -> Result<Self> {
        // One supervisor per process. Device listing (which `local_llm` reads
        // through the free functions below) goes through the same one, so a
        // second manager, which the app never builds, would list through the
        // first's engine.
        let engine = ENGINE
            .get_or_init(|| {
                let gpu_allowed = !transcribe_gpu_disabled_for_host();
                if !gpu_allowed {
                    // Every worker is then CPU-only, so the emulated GPU
                    // driver is never loaded at all.
                    warn!(
                        "Windows x64 build is running under emulation on an ARM64 PC; \
                         speech runs on the processor"
                    );
                }
                EngineSupervisor::new(gpu_allowed)
            })
            .clone();
        let manager = Self {
            engine,
            model_manager,
            app_handle: app_handle.clone(),
            current_model_id: Arc::new(Mutex::new(None)),
            last_activity: Arc::new(AtomicU64::new(Self::now_ms())),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            watcher_handle: Arc::new(Mutex::new(None)),
            is_loading: Arc::new(Mutex::new(false)),
            loading_condvar: Arc::new(Condvar::new()),
            dictation_cancel: Arc::new(Mutex::new(CancelToken::new())),
            stream_router,
        };

        // Start the idle watcher
        {
            let app_handle_cloned = app_handle.clone();
            let manager_cloned = manager.clone();
            let shutdown_signal = manager.shutdown_signal.clone();
            let handle = thread::spawn(move || {
                debug!("Idle watcher thread started");
                while !shutdown_signal.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_secs(10)); // Check every 10 seconds

                    // Check shutdown signal again after sleep
                    if shutdown_signal.load(Ordering::Relaxed) {
                        break;
                    }

                    // Cloud-only sessions have no local engine to unload.
                    // Avoid deserializing/migrating/hydrating the full settings
                    // snapshot on every idle tick.
                    if !manager_cloned.is_model_loaded() {
                        continue;
                    }

                    let settings = get_settings(&app_handle_cloned);
                    let timeout = settings.model_unload_timeout;

                    // Skip Immediately — that variant is handled by
                    // maybe_unload_immediately() after each transcription.
                    // Treating it as 0s here would unload the model mid-recording.
                    if timeout == ModelUnloadTimeout::Immediately {
                        continue;
                    }

                    // While recording, keep the idle timer fresh so the
                    // model is never unloaded mid-session. A meeting records
                    // through its own recorder, not the dictation one, and
                    // transcribes a chunk every ~30 s, so it counts too.
                    //
                    // With cloud transcription active the local model is only
                    // a fallback and no recording will use it, so it is left
                    // to idle out instead of being kept warm by every cloud
                    // dictation.
                    let is_recording = app_handle_cloned
                        .try_state::<Arc<AudioRecordingManager>>()
                        .is_some_and(|a| a.is_recording())
                        || meeting_is_recording(&app_handle_cloned);
                    if is_recording && !crate::stt_cloud::cloud_stt_active(&settings) {
                        manager_cloned.touch_activity();
                        continue;
                    }

                    if let Some(limit_seconds) = timeout.to_seconds() {
                        let last = manager_cloned.last_activity.load(Ordering::Relaxed);
                        let now_ms = TranscriptionManager::now_ms();
                        let idle_ms = now_ms.saturating_sub(last);
                        let limit_ms = limit_seconds * 1000;

                        if idle_ms > limit_ms {
                            // idle -> unload
                            if manager_cloned.is_model_loaded() {
                                let unload_start = std::time::Instant::now();
                                info!(
                                    "Model idle for {}s (limit: {}s), unloading",
                                    idle_ms / 1000,
                                    limit_seconds
                                );
                                match manager_cloned.unload_model() {
                                    Ok(()) => {
                                        let unload_duration = unload_start.elapsed();
                                        info!(
                                            "Model unloaded due to inactivity (took {}ms)",
                                            unload_duration.as_millis()
                                        );
                                    }
                                    Err(e) => {
                                        error!("Failed to unload idle model: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }
                debug!("Idle watcher thread shutting down gracefully");
            });
            *manager.watcher_handle.lock().unwrap() = Some(handle);
        }

        Ok(manager)
    }

    /// Whether a model is loaded. A model stays loaded while it is busy, and
    /// while its worker is replaced after a crash, so a run or stream in
    /// progress never reads as "unloaded".
    pub fn is_model_loaded(&self) -> bool {
        self.engine.loaded().is_some()
    }

    /// Whether a speech model load is in progress right now. The recording
    /// overlay reads it to say "Loading model…" rather than "Transcribing…"
    /// when the wait after a dictation is the load (`overlay.rs`).
    pub fn is_model_loading(&self) -> bool {
        self.is_loading
            .lock()
            .map(|loading| *loading)
            .unwrap_or(false)
    }

    /// An explicit speech-processing or GPU change is the user's way of trying
    /// the GPU again: forget every GPU failure so far, so the next load may
    /// use (and list) the GPU.
    pub fn retry_transcribe_gpu(&self, reason: &str) {
        self.engine.retry_gpu(reason);
    }

    /// The engine gives up on a model by itself when its worker can't be
    /// restarted (the file is gone, or it crashes on the CPU too). Keep the
    /// model id and the UI in step, so the next use loads the model again and
    /// reports why it can't.
    fn forget_model_if_engine_dropped(&self, error: &str) {
        if self.engine.loaded().is_some() {
            return;
        }
        let forgotten = self
            .current_model_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let Some(model_id) = forgotten else {
            return;
        };
        warn!(
            "Transcription model '{}' is no longer loaded: {}",
            model_id, error
        );
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "unloaded".to_string(),
                model_id: None,
                model_name: None,
                error: Some(error.to_string()),
            },
        );
    }

    /// Atomically check whether a model load is in progress and, if not, mark
    /// one as starting. Returns a [`LoadingGuard`] whose [`Drop`] impl will
    /// clear the flag and wake waiters. Returns `None` if a load is already in
    /// progress.
    pub fn try_start_loading(&self) -> Option<LoadingGuard> {
        let mut is_loading = self.is_loading.lock().unwrap();
        if *is_loading {
            return None;
        }
        *is_loading = true;
        Some(LoadingGuard {
            is_loading: self.is_loading.clone(),
            loading_condvar: self.loading_condvar.clone(),
        })
    }

    /// Unload the model. Returns once the worker has exited, which frees all
    /// of its CPU and GPU memory and the model file (Windows refuses to delete
    /// a file the worker still has mapped). Waits behind a transcription in
    /// progress, so it belongs on a thread that may block.
    pub fn unload_model(&self) -> Result<()> {
        let unload_start = std::time::Instant::now();
        debug!("Starting to unload model");
        self.begin_unload().wait();
        debug!(
            "Model unloaded (took {}ms)",
            unload_start.elapsed().as_millis()
        );
        Ok(())
    }

    /// Unload the model without waiting for its worker to exit, so the caller
    /// (the main thread, an async command) is never held behind a
    /// transcription in progress. The model reads as unloaded at once.
    pub fn request_unload(&self) {
        drop(self.begin_unload());
    }

    /// Stop the transcription worker before the app exits, waiting at most
    /// `timeout`. The worker would end with the app anyway (its stdin closes,
    /// and on Windows the kill-on-close job ends it), but a Windows in-app
    /// update overwrites `speakoflow.exe`, which the worker is running from,
    /// so it should be gone before the installer gets there. Bounded, because
    /// the unload queues behind a transcription in progress, and quitting
    /// must never wait on a hung one. Returns whether the worker has exited.
    pub fn stop_engine_before_exit(&self, timeout: Duration) -> bool {
        // Unconditionally: a load in progress has a worker but no model yet.
        self.engine.unload().wait_timeout(timeout)
    }

    /// Every state change of an unload, done now on the caller's thread: a
    /// dictation started right after sees no model and loads it again, and its
    /// load is queued behind this unload, so the unload can never affect it.
    /// Only the worker's exit is left to wait for.
    fn begin_unload(&self) -> crate::engine_supervisor::Unloading {
        let unloading = self.engine.unload();
        {
            let mut current_model = self
                .current_model_id
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *current_model = None;
        }

        // Emit unloaded event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "unloaded".to_string(),
                model_id: None,
                model_name: None,
                error: None,
            },
        );
        unloading
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    /// Reset the idle timer to now.
    fn touch_activity(&self) {
        self.last_activity.store(Self::now_ms(), Ordering::Relaxed);
    }

    /// Unloads the model immediately if the setting is enabled and the model
    /// is loaded. Never waits for the worker to exit, so it is safe on any
    /// thread.
    pub fn maybe_unload_immediately(&self, context: &str) {
        // A meeting transcribes a chunk every ~30 s through this same engine.
        // Unloading after each one reloaded the model twice a minute; the chunk
        // that lands after the meeting stops unloads it as usual.
        if meeting_is_recording(&self.app_handle) {
            return;
        }
        let settings = get_settings(&self.app_handle);
        if settings.model_unload_timeout == ModelUnloadTimeout::Immediately
            && self.is_model_loaded()
        {
            info!("Immediately unloading model after {}", context);
            self.request_unload();
        }
    }

    pub fn load_model(&self, model_id: &str) -> Result<()> {
        let load_start = std::time::Instant::now();
        debug!("Starting to load model: {}", model_id);

        // Emit loading started event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "loading_started".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: None,
                error: None,
            },
        );

        let model_info = self
            .model_manager
            .get_model_info(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        // A model of a retired engine stays on disk but is never loaded; say
        // what to do instead of failing on a file the app cannot run. Checked
        // before the download state, which does not matter for such a model.
        if is_retired_engine(&model_info.engine_type) {
            let error_msg = retired_engine_message(&model_info.name);
            warn!(
                "Refusing to load {}: its engine ({:?}) is no longer included",
                model_id, model_info.engine_type
            );
            let _ = self.app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_failed".to_string(),
                    model_id: Some(model_id.to_string()),
                    model_name: Some(model_info.name.clone()),
                    error: Some(error_msg.clone()),
                },
            );
            return Err(anyhow::anyhow!(error_msg));
        }

        if !model_info.is_downloaded {
            let error_msg = "Model not downloaded";
            let _ = self.app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_failed".to_string(),
                    model_id: Some(model_id.to_string()),
                    model_name: Some(model_info.name.clone()),
                    error: Some(error_msg.to_string()),
                },
            );
            return Err(anyhow::anyhow!(error_msg));
        }

        let model_path = self.model_manager.get_model_path(model_id)?;

        let emit_loading_failed = |error_msg: &str| {
            let _ = self.app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_failed".to_string(),
                    model_id: Some(model_id.to_string()),
                    model_name: Some(model_info.name.clone()),
                    error: Some(error_msg.to_string()),
                },
            );
        };

        let loaded = match model_info.engine_type {
            // Every speech model loads through transcribe.cpp, which reads the
            // format from the file header: a GGUF, or a whisper.cpp `.bin` (the
            // legacy Whisper catalog entries, custom `.bin` files dropped into
            // the models folder, and `.bin` files registered from disk). Both
            // get the same run plan, including the initial prompt built from
            // the custom words for whisper-arch models. The load runs in the
            // transcription worker (`engine_supervisor`), which picks the
            // device, opens the session and reports what it bound to.
            EngineType::Whisper | EngineType::TranscribeCpp => {
                // Clear the id first: if this load fails, the status must read
                // "no model loaded", not the model the worker is replacing.
                *self
                    .current_model_id
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None;
                let target = worker_load_target(&get_settings(&self.app_handle));
                self.engine
                    .load(crate::engine_supervisor::LoadSpec {
                        path: model_path.clone(),
                        target,
                    })
                    .map_err(|e| {
                        let error_msg = match e {
                            // 0.4 reports capabilities only for models with the
                            // ASR role; a diarizer or language-ID GGUF has none
                            // and cannot transcribe.
                            EngineError::NotSpeechModel(reason) => format!(
                                "{} is not a speech-to-text model and cannot be used for dictation ({})",
                                model_info.name, reason
                            ),
                            other => format!(
                                "Failed to load transcribe.cpp model {}: {}",
                                model_id, other
                            ),
                        };
                        emit_loading_failed(&error_msg);
                        anyhow::anyhow!(error_msg)
                    })?
            }
            // The ONNX engines are gone. `load_model` refuses them before this
            // match (see `is_retired_engine`); listed here so a new engine type
            // cannot slip through without a decision.
            EngineType::Parakeet
            | EngineType::Moonshine
            | EngineType::MoonshineStreaming
            | EngineType::SenseVoice
            | EngineType::GigaAM
            | EngineType::Canary
            | EngineType::Cohere => {
                let error_msg = retired_engine_message(&model_info.name);
                emit_loading_failed(&error_msg);
                return Err(anyhow::anyhow!(error_msg));
            }
            // Not transcription engines — these are handled by their own
            // subsystems (LocalLlmManager / the assistant webview) and must
            // never be loaded as the active recording model.
            EngineType::LlamaCpp | EngineType::Kokoro | EngineType::NativeTts => {
                let error_msg = format!(
                    "Model {} is not a transcription model and cannot be loaded for recording",
                    model_id
                );
                emit_loading_failed(&error_msg);
                return Err(anyhow::anyhow!(error_msg));
            }
        };

        // Log the device the model actually bound to, so a silent CPU fallback
        // (a requested GPU that wasn't available, or one that crashed) or an
        // integrated GPU chosen over a dedicated one is visible.
        info!(
            "Loaded transcribe.cpp model {} (arch={}, variant={}, backend={}, device={} [{}], cpu_threads={})",
            model_id,
            loaded.arch,
            loaded.variant,
            loaded.backend,
            loaded.device,
            loaded
                .device_type
                .map(device_type_name)
                .unwrap_or("unknown"),
            if loaded.cpu_threads == 0 {
                "all".to_string()
            } else {
                loaded.cpu_threads.to_string()
            }
        );
        // The loaded model's capabilities are ground truth (read from the
        // file), so the model list shows real streaming / translation /
        // language support. The `loading_completed` event below makes the
        // frontend reload the list.
        let caps = &loaded.capabilities;
        self.model_manager.set_runtime_capabilities(
            model_id,
            caps.supports_streaming,
            caps.supports_translate,
            &caps.languages,
        );
        // Also ground truth: what "Auto" means for this model, and what an
        // intent it does not list falls back to.
        self.model_manager
            .set_runtime_language_detection(model_id, caps.supports_language_detect);

        // Update the current model ID
        {
            let mut current_model = self
                .current_model_id
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *current_model = Some(model_id.to_string());
        }

        // Reset idle timer so the watcher doesn't immediately unload a just-loaded model
        self.touch_activity();

        // Emit loading completed event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "loading_completed".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: Some(model_info.name.clone()),
                error: None,
            },
        );

        let load_duration = load_start.elapsed();
        debug!(
            "Successfully loaded transcription model: {} (took {}ms)",
            model_id,
            load_duration.as_millis()
        );
        Ok(())
    }

    /// Kicks off the model loading in a background thread if it's not already loaded
    pub fn initiate_model_load(&self) {
        // All callers share this policy, including history retries. A complete
        // cloud configuration needs no local engine; an incomplete one still
        // preloads the local fallback just as transcribe() expects.
        if crate::stt_cloud::cloud_stt_active(&get_settings(&self.app_handle)) {
            return;
        }
        let mut is_loading = self.is_loading.lock().unwrap();
        // A busy model is still loaded; `transcribe` queues behind its work.
        if *is_loading || self.is_model_loaded() {
            return;
        }

        *is_loading = true;
        let self_clone = self.clone();
        thread::spawn(move || {
            let settings = get_settings(&self_clone.app_handle);
            // The provider may have changed between scheduling and running.
            if !crate::stt_cloud::cloud_stt_active(&settings) {
                if let Err(e) = self_clone.load_model(&settings.selected_model) {
                    error!("Failed to load model: {}", e);
                }
            }
            let mut is_loading = self_clone.is_loading.lock().unwrap();
            *is_loading = false;
            self_clone.loading_condvar.notify_all();
        });
    }

    /// Unload the local model once cloud transcription is fully configured.
    ///
    /// Called when the user switches to cloud, or completes a cloud setup. The
    /// pipeline will not use the local engine again unless a cloud request fails,
    /// and that path (`load_fallback_engine`) loads it on demand, so keeping it
    /// resident only holds RAM and VRAM. Skipped while anything is recording or
    /// loading, so an in-flight dictation or meeting is never pulled out from
    /// under itself.
    pub fn release_local_for_cloud(&self) {
        if !self.is_model_loaded() {
            return;
        }
        // Off the caller's thread: the settings commands that call this run on
        // the main thread, and tearing down a GPU context is not instant.
        let this = self.clone();
        thread::spawn(move || {
            // Every condition is decided inside the loading slot, so a switch
            // back to local (whose load needs the slot) or a recording starting
            // in the meantime is seen rather than raced. A transcription still
            // running is waited for: the unload queues behind it.
            let Some(_loading) = this.try_start_loading() else {
                return;
            };
            let recording = this
                .app_handle
                .try_state::<Arc<AudioRecordingManager>>()
                .is_some_and(|a| a.is_recording())
                || meeting_is_recording(&this.app_handle);
            if recording
                || !this.is_model_loaded()
                || !crate::stt_cloud::cloud_stt_active(&get_settings(&this.app_handle))
            {
                return;
            }
            info!("Cloud transcription is active; unloading the local model");
            if let Err(e) = this.unload_model() {
                warn!("Failed to unload the local model: {}", e);
            }
        });
    }

    /// Load the configured local model so a failed cloud request has somewhere
    /// to go, returning the model id that is now resident.
    ///
    /// Cloud mode never preloads a local engine (`initiate_model_load` returns
    /// early), so this is the one place that pays that cost — deliberately, and
    /// only after a cloud request has already failed. It is a plain synchronous
    /// load rather than a spawn because the caller is mid-`transcribe()` and has
    /// audio in hand: the point is to finish *this* recording, and the model
    /// stays resident afterwards so a second failure costs nothing.
    ///
    /// Mirrors `initiate_model_load`'s handling of `is_loading` so a concurrent
    /// load is awaited rather than duplicated.
    fn load_fallback_engine(&self, settings: &crate::settings::AppSettings) -> Result<String> {
        {
            // Wait out a load already in flight before deciding anything.
            let mut is_loading = self.is_loading.lock().unwrap();
            while *is_loading {
                is_loading = self.loading_condvar.wait(is_loading).unwrap();
            }
            // A model busy with another transcription (a meeting chunk that
            // fell back a moment earlier, during the same outage) is loaded,
            // not missing: this one queues behind it.
            if self.is_model_loaded() {
                return self
                    .get_current_model()
                    .ok_or_else(|| anyhow::anyhow!("a model is loaded but unnamed"));
            }
            *is_loading = true;
        }

        let model_id = settings.selected_model.clone();
        let outcome = if model_id.is_empty() {
            Err(anyhow::anyhow!("no local model is selected"))
        } else {
            self.load_model(&model_id).map(|()| model_id)
        };

        let mut is_loading = self.is_loading.lock().unwrap();
        *is_loading = false;
        self.loading_condvar.notify_all();
        outcome
    }

    pub fn get_current_model(&self) -> Option<String> {
        let current_model = self.current_model_id.lock().unwrap();
        current_model.clone()
    }

    /// Transcribe a whole clip: a meeting chunk, a History retry, a call's
    /// utterance, an ask. Not stopped by Esc; see
    /// [`transcribe_for_dictation`](Self::transcribe_for_dictation).
    pub fn transcribe(&self, audio: Vec<f32>) -> Result<String> {
        self.transcribe_with(audio, None)
    }

    /// Transcribe a dictation's recording. [`cancel_transcription`] (Esc)
    /// stops it promptly, ending in [`TranscriptionCancelled`].
    ///
    /// [`cancel_transcription`]: Self::cancel_transcription
    pub fn transcribe_for_dictation(&self, audio: Vec<f32>) -> Result<String> {
        let cancel = self
            .dictation_cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.transcribe_with(audio, Some(cancel))
    }

    /// Stop the dictation transcription in progress (Esc): a batch run, the
    /// chunks of a long recording, or a live stream's finalize. Through
    /// transcribe.cpp's abort when the model honours it, so the model stays
    /// loaded; otherwise its worker is stopped and the model reloads on next
    /// use. Meetings, History and calls are left alone.
    pub fn cancel_transcription(&self) {
        let mut token = self
            .dictation_cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        token.cancel();
        *token = CancelToken::new();
        drop(token);
        self.stream_router.cancel_latest();
    }

    fn transcribe_with(&self, mut audio: Vec<f32>, cancel: Option<CancelToken>) -> Result<String> {
        CLOUD_FELL_BACK.with(|flag| flag.set(false));
        #[cfg(debug_assertions)]
        if std::env::var("HANDY_FORCE_TRANSCRIPTION_FAILURE").is_ok() {
            return Err(anyhow::anyhow!(
                "Simulated transcription failure (HANDY_FORCE_TRANSCRIPTION_FAILURE)"
            ));
        }

        // The idle timer is touched below, on the local path only: a cloud
        // dictation must not keep an unused fallback model resident.

        let st = std::time::Instant::now();

        debug!("Audio vector length: {}", audio.len());

        if audio.is_empty() {
            debug!("Empty audio vector");
            self.maybe_unload_immediately("empty audio");
            return Ok(String::new());
        }

        // transcribe.cpp refuses audio with a NaN or infinite sample, and a
        // non-finite sample would also skew the peak the gain below is computed
        // from. Silence them first, so what reaches either is always finite.
        let non_finite = sanitize_pcm(&mut audio);
        if non_finite > 0 {
            warn!(
                "Replaced {} non-finite audio sample(s) with silence before transcription",
                non_finite
            );
        }

        // Boost quiet captures up toward a healthy level before inference.
        // Whisper-style models degrade on faint, low-SNR audio (e.g. speaking
        // away from the mic or outdoors), and this is a cheap O(n) pass that
        // only ever boosts — already-loud audio is left untouched. Runs once,
        // off the live capture path, so it adds no recording latency.
        let applied_gain = normalize_peak(&mut audio, 0.95, 8.0);
        if applied_gain > 1.0 {
            debug!("Applied input gain normalization: {:.2}x", applied_gain);
        }

        // Cloud transcription, when the user has selected it and configured it
        // completely. Checked before the model-loaded guard below because the
        // cloud path has no local model to wait for — and must not require one,
        // since a cloud user may have no model downloaded at all.
        //
        // An incomplete cloud configuration (no key, unknown provider) resolves
        // to `None` here and falls through to the local engine rather than
        // failing the dictation. A *complete* configuration that then fails on
        // the wire falls through too, loading the local engine on demand — a
        // dead key is a configuration problem wearing a network error's clothes,
        // and it would otherwise fail every recording forever.
        {
            let settings = get_settings(&self.app_handle);
            if let Ok(cfg) = crate::stt_cloud::resolve_cloud_stt(&settings) {
                // A provider without a translation route transcribes in the
                // language spoken; `resolve_cloud_stt` already said so, once.
                // On a provider that has the route (`cfg.translate`), the
                // request goes to `/audio/translations`, so there is nothing to
                // warn about — this used to claim the opposite on every request.
                // The provider was already given the custom words as keyterm /
                // prompt biasing, so a second fuzzy pass over its output would
                // only risk rewriting words it already got right. With biasing
                // turned off there is nothing upstream, so the local pass earns
                // its place again.
                let biased_upstream = !cfg.keyterms.is_empty();
                let result = crate::stt_cloud::transcribe_cloud_blocking(&cfg, &audio);
                match result {
                    Ok(raw) => {
                        let facts = RunFacts::cloud(&settings, biased_upstream);
                        let finished = self.finish_transcription(&settings, raw, st, &facts);
                        return Ok(finished);
                    }
                    Err(e) => {
                        error!("Cloud transcription failed: {}", e);
                        // A revoked, expired, or mistyped key fails identically on
                        // every single recording, so returning here does not report
                        // a problem so much as break dictation until the user
                        // happens to read the error. The local engine is
                        // deliberately not loaded in cloud mode (see
                        // `initiate_model_load`), which is why this used to be a
                        // dead end — so pay for the one-time load and finish the
                        // recording locally instead of throwing the audio away.
                        //
                        // The cloud error is still the one that surfaces if the
                        // fallback cannot run: for a cloud-only user with nothing
                        // downloaded, "invalid API key" explains the failure and
                        // "no model loaded" only describes a consequence of it.
                        match self.load_fallback_engine(&settings) {
                            Ok(model_id) => {
                                CLOUD_FELL_BACK.with(|flag| flag.set(true));
                                warn!(
                                    "Falling back to the local model '{}' for this \
                                     recording — {} did not transcribe it. If this \
                                     keeps happening, check the key and credit in \
                                     Settings → Models → Cloud transcription.",
                                    model_id, cfg.provider.label
                                );
                            }
                            Err(load_err) => {
                                warn!(
                                    "No local engine to fall back to ({}), so the \
                                     cloud failure stands",
                                    load_err
                                );
                                return Err(anyhow::anyhow!(e));
                            }
                        }
                    }
                }
            }
        }

        // Local inference from here on: reset the idle timer.
        self.touch_activity();

        // Get current settings for configuration
        let settings = get_settings(&self.app_handle);
        // Resolved once: only a clip long enough to cut ever loads a VAD from
        // it, and the VAD runs here in the app, never in the worker.
        let vad_path = crate::managers::audio::silero_vad_path(&self.app_handle).ok();

        let info = self.wait_for_loaded_model()?;
        let plan = transcribe_cpp_run_plan(
            &settings,
            &info.capabilities,
            &info.arch,
            info.supports_vocabulary,
        );
        let traits = ModelTraits::of_loaded(&info);
        // Our own chunking code is the only thing left in this process that
        // could panic mid-transcription; it must not take the dictation task
        // (and its overlay and History row) down with it.
        let decoded = catch_unwind(AssertUnwindSafe(|| {
            let mut decode =
                |chunk: Vec<f32>| self.run_in_worker(chunk, &plan.options, cancel.as_ref());
            match batch_decode(traits, settings.long_form_chunking) {
                BatchDecode::Chunked if Chunker::would_cut(audio.len()) => {
                    chunked::decode_clip(&mut decode, audio, silero_scorer_factory(vad_path))
                        .map(|decoded| decoded.text)
                }
                _ => decode(audio),
            }
        }));
        let text = match decoded {
            Ok(text) => text?,
            Err(payload) => {
                let message = panic_payload_message(payload.as_ref());
                error!("Transcription panicked: {}", message);
                return Err(anyhow::anyhow!("Transcription panicked: {}", message));
            }
        };

        let facts = RunFacts::local(&plan);
        Ok(self.finish_transcription(&settings, text, st, &facts))
    }

    /// The loaded model, once a load in progress has finished.
    fn wait_for_loaded_model(&self) -> Result<LoadedInfo> {
        let mut is_loading = self.is_loading.lock().unwrap_or_else(|e| e.into_inner());
        while *is_loading {
            is_loading = self
                .loading_condvar
                .wait(is_loading)
                .unwrap_or_else(|e| e.into_inner());
        }
        drop(is_loading);
        self.engine
            .loaded()
            .ok_or_else(|| anyhow::anyhow!("Model is not loaded for transcription."))
    }

    /// One run in the transcription worker. A clip too short to decode said
    /// nothing, the same outcome as a recording of silence: an empty
    /// transcript, not an error the user has to dismiss.
    fn run_in_worker(
        &self,
        pcm: Vec<f32>,
        options: &transcribe_cpp::RunOptions,
        cancel: Option<&CancelToken>,
    ) -> Result<String> {
        // Per run, so the idle watcher never unloads the model between the
        // chunks of one long transcription.
        self.touch_activity();
        match self
            .engine
            .transcribe(pcm, options.clone(), cancel.cloned())
        {
            Ok(transcript) => Ok(transcript.text),
            Err(EngineError::InputTooShort(reason)) => {
                debug!("Audio too short to transcribe ({}); empty result", reason);
                Ok(String::new())
            }
            Err(EngineError::Cancelled) => Err(anyhow::Error::new(TranscriptionCancelled)),
            Err(e) => {
                let message = e.to_string();
                self.forget_model_if_engine_dropped(&message);
                Err(anyhow::anyhow!(
                    "transcribe.cpp transcription failed: {}",
                    message
                ))
            }
        }
    }

    /// Shared tail of every transcription: fuzzy custom-word correction, the
    /// filler/hallucination filter, timing log, and the immediate-unload check.
    ///
    /// Factored out so the cloud path gets *identical* treatment to the local
    /// one. The user-visible behaviours here — a custom dictionary being
    /// applied, "um" being dropped — are app features, not engine features, and
    /// having them silently stop working on a cloud provider would read as the
    /// provider being worse than it is.
    fn finish_transcription(
        &self,
        settings: &crate::settings::AppSettings,
        raw: String,
        started: std::time::Instant,
        facts: &RunFacts,
    ) -> String {
        let final_result = post_process_text(settings, raw, facts);

        let et = std::time::Instant::now();
        // Only claim a translation the engine actually performed. The setting
        // being on is not evidence: the cloud path cannot translate at all, and
        // a local model that does not support the translate task has it dropped
        // too, so reading the flag here made the log assert something false.
        let translation_note = if facts.translated {
            " (translated)"
        } else {
            ""
        };
        info!(
            "Transcription completed in {}ms{}",
            (et - started).as_millis(),
            translation_note
        );

        if final_result.is_empty() {
            info!("Transcription result is empty");
        } else {
            info!(
                "Transcription result: {}",
                crate::utils::redact_text(&final_result)
            );
        }

        self.maybe_unload_immediately("transcription");

        final_result
    }

    /// Shared handle to the live-transcription router. Handed to the recorder
    /// so its frame callback can feed audio while a stream is active. Public
    /// accessor kept for external/state access even though the router is also
    /// injected directly at construction.
    #[allow(dead_code)]
    pub fn stream_router(&self) -> Arc<StreamRouter> {
        self.stream_router.clone()
    }

    /// Begin a recording's live decode. Opens the router and spawns a worker
    /// for `mode`, which the caller chose from the model it expects
    /// ([`policy::live_decode`]); the worker decides again with the loaded
    /// model and falls back to batch when that one wants something else:
    ///
    /// - [`LiveDecode::Stream`]: a streaming model, or a cloud provider with a
    ///   realtime endpoint, decodes every frame as it arrives
    ///   (`session.stream`→`feed`→`finalize`, real committed+tentative text).
    /// - [`LiveDecode::Chunked`]: a model not built for long audio gets the
    ///   kept speech cut at pauses, each chunk decoded once its cut is certain,
    ///   so a long recording has only its tail left when it stops.
    ///
    /// No-op for [`LiveDecode::None`], and (with a warning) if a session is
    /// already open. The batch `transcribe()` path remains the fallback (see
    /// [`finalize_stream`](Self::finalize_stream)).
    pub fn start_stream(&self, mode: LiveDecode) {
        let kind = match mode {
            LiveDecode::Stream => FeedKind::Raw,
            LiveDecode::Chunked => FeedKind::Speech,
            LiveDecode::None => return,
        };
        if self.stream_router.is_open() {
            warn!("Live transcription stream already open; ignoring start_stream()");
            return;
        }
        let opened = self.stream_router.open(kind);
        let this = self.clone();
        thread::spawn(move || {
            // Cloud realtime streaming never touches the local engine, so it
            // gets its own worker rather than a branch inside the engine-leasing
            // one. Resolved here (on the worker thread) so a settings read never
            // happens on the caller's path. It ignores `give_up`: its finish is
            // bounded by its own timeouts, not by a backlog.
            let settings = get_settings(&this.app_handle);
            if let Ok(cfg) = crate::stt_cloud::resolve_cloud_stt(&settings) {
                if mode == LiveDecode::Stream && crate::stt_cloud::streams_live(&settings, &cfg) {
                    this.run_cloud_stream_worker(opened.rx, cfg, opened.generation);
                } else {
                    // The cloud transcribes the whole recording when it ends.
                    this.drain_stream_no_model(opened.rx);
                    this.stream_router.clear();
                }
                return;
            }
            match mode {
                LiveDecode::Chunked => this.run_chunked_worker(opened),
                _ => this.run_stream_worker(opened),
            }
        });
    }

    /// Worker body for cloud realtime transcription.
    ///
    /// Structurally the same contract as [`run_stream_worker`](Self::run_stream_worker)
    /// — consume the FIFO command channel, answer `Finalize` with the transcript
    /// or `None` — but with no engine lease, because there is no local model
    /// involved. Every failure answers `None`, which routes
    /// [`finalize_stream`](Self::finalize_stream) to the batch path so the
    /// complete recording is transcribed in one request instead of the dictation
    /// being lost.
    fn run_cloud_stream_worker(
        &self,
        rx: mpsc::Receiver<StreamCmd>,
        cfg: ResolvedCloudStt,
        stream_generation: u64,
    ) {
        // Live text is gated on the stream generation for the same reason the
        // local worker gates its emissions: a late message arriving during the
        // finalize window would otherwise paint one recording's words onto the
        // next recording's overlay — and hand them to Flow's phrase watcher.
        let app = self.app_handle.clone();
        let router = self.stream_router.clone();
        let sink: crate::stt_cloud_stream::LiveTextSink =
            Box::new(move |committed: &str, tentative: &str| {
                if !router.is_generation_current(stream_generation) {
                    return;
                }
                // Flow's activation-phrase watcher reads the live transcript to
                // prewarm the local model early; cloud streaming feeds it the
                // same way local streaming does.
                crate::flow::note_live_transcript(&app, committed);
                let _ = app.emit(
                    "stream-text",
                    StreamTextPayload {
                        committed: committed.to_string(),
                        tentative: tentative.to_string(),
                    },
                );
            });

        let mut session = match crate::stt_cloud_stream::CloudStreamSession::connect(sink, &cfg) {
            Ok(session) => session,
            Err(e) => {
                warn!(
                    "Realtime cloud transcription unavailable ({}); the recording will be \
                     transcribed in one request when it ends",
                    e
                );
                self.drain_stream_no_model(rx);
                self.stream_router.clear();
                return;
            }
        };

        let mut final_text: Option<String> = None;
        let mut finalize_reply: Option<mpsc::Sender<Option<LiveResult>>> = None;
        let mut cancelled = false;

        while let Ok(cmd) = rx.recv() {
            match cmd {
                StreamCmd::Feed(frame) => {
                    self.stream_router.note_feed_consumed();
                    session.feed(&frame);
                }
                StreamCmd::Finalize { reply, .. } => {
                    finalize_reply = Some(reply);
                    break;
                }
                StreamCmd::Cancel => {
                    cancelled = true;
                    break;
                }
            }
        }

        if cancelled {
            session.abort();
        } else {
            final_text = session.finish();
        }

        self.stream_router.clear();

        if let Some(reply) = finalize_reply {
            let settings = get_settings(&self.app_handle);
            let facts = RunFacts::cloud(&settings, !cfg.keyterms.is_empty());
            let _ = reply.send(
                final_text
                    .filter(|t| !t.trim().is_empty())
                    .map(|text| LiveResult { text, facts }),
            );
        }
    }

    /// Worker body for a native stream: wait for any in-progress load, then
    /// drive a stream in the transcription worker from the FIFO command
    /// channel until Finalize or Cancel:
    ///
    /// - **the loaded model streams** → native streaming (begin → feed →
    ///   finalize in the worker process), emitting real committed + tentative
    ///   text to the overlay as each feed is answered.
    /// - **no model / a model that does not stream / the stream cannot begin**
    ///   → drain the channel and reply `None` to Finalize, so
    ///   `finalize_stream()` routes to the batch `transcribe()` path.
    ///
    /// The stream occupies the engine until it ends: a batch run (a meeting
    /// chunk) that arrives meanwhile waits for it, as it waited for the leased
    /// engine before. A worker that crashes mid-stream loses only the stream:
    /// finalize answers `None` and the recording is transcribed in batch, in a
    /// fresh worker.
    ///
    /// `give_up` is raised when the dictation has ended and this worker is
    /// still behind. Only a stream on a GPU honours it, by skipping the rest of
    /// its backlog and answering `None`: there one batch pass over the whole
    /// recording is cheap. Elsewhere starting over could cost more than
    /// finishing, so the worker carries on.
    fn run_stream_worker(&self, opened: OpenedStream) {
        let OpenedStream {
            rx,
            generation,
            give_up,
            cancel,
        } = opened;

        // Wait for any in-progress model load to finish (mirrors transcribe()).
        {
            let mut is_loading = self.is_loading.lock().unwrap_or_else(|e| e.into_inner());
            while *is_loading {
                is_loading = self
                    .loading_condvar
                    .wait(is_loading)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }

        // The loaded model's own capabilities decide, not the model list.
        let Some(info) = self
            .engine
            .loaded()
            .filter(|info| info.capabilities.supports_streaming)
        else {
            if self.is_model_loaded() {
                debug!(
                    "Live transcription: the model does not advertise streaming; \
                     falling back to batch"
                );
            } else {
                warn!("Live transcription: no model loaded; falling back to batch");
            }
            self.drain_stream_no_model(rx);
            self.stream_router.clear();
            return;
        };

        let settings = get_settings(&self.app_handle);
        let may_give_up = info.on_gpu;
        let pads_tail = info.buffered_stream;
        let run_plan = transcribe_cpp_run_plan(
            &settings,
            &info.capabilities,
            &info.arch,
            info.supports_vocabulary,
        );
        let final_facts = RunFacts::local(&run_plan);
        // Live-display correction: a model that was given the custom words
        // (Whisper's initial prompt, a model's vocabulary) already biases its
        // decoding, so only the others get the fuzzy display pass (same rule
        // as the final text).
        let display = Arc::new(Mutex::new(LiveDisplay {
            script: LiveScript::new(chinese_script::script_rule(
                &settings.selected_language,
                &run_plan.effective_language,
                settings.chinese_script,
                &run_plan.languages,
            )),
            hints: if run_plan.words_biased {
                Vec::new()
            } else {
                recognition_words(&settings)
            },
            threshold: settings.word_correction_threshold,
            flow_phrase: settings
                .flow_enabled
                .then(|| settings.flow_phrase.trim().to_string())
                .filter(|p| !p.is_empty()),
        }));
        let stream_opts = transcribe_cpp::StreamOptions {
            commit_policy: transcribe_cpp::CommitPolicy::Auto,
            ..Default::default()
        };

        // Runs on the engine's owner thread after every feed: it only converts
        // and emits, never blocks or calls back into the engine.
        let on_progress = {
            let app = self.app_handle.clone();
            let router = Arc::clone(&self.stream_router);
            let display = Arc::clone(&display);
            move |progress: StreamProgress| {
                let Some(text) = progress.text else {
                    return;
                };
                let (committed, tentative) = lock_display(&display).show(&text);
                // Flow watches the live text for its activation phrase (early
                // local-model prewarm). Gated on the generation, so a late
                // feed of this recording never reaches the next one.
                if router.is_generation_current(generation) {
                    crate::flow::note_live_transcript(&app, &committed);
                }
                let _ = app.emit(
                    "stream-text",
                    StreamTextPayload {
                        committed,
                        tentative,
                    },
                );
            }
        };

        let stream = match self.engine.start_stream(
            run_plan.options.clone(),
            stream_opts,
            cancel.clone(),
            on_progress,
        ) {
            Ok(stream) => stream,
            Err(e) => {
                warn!(
                    "Live transcription: transcribe.cpp stream begin failed ({}); \
                     falling back to batch",
                    e
                );
                self.drain_stream_no_model(rx);
                self.stream_router.clear();
                return;
            }
        };
        self.touch_activity();

        // Accumulate the recorder's ~30 ms frames up to ~80 ms (1280 samples
        // @ 16 kHz) before each feed — the cadence validated as smooth for
        // Parakeet's cache-aware streaming, and few enough messages that the
        // trip to the worker costs nothing measurable.
        const FEED_SAMPLES: usize = 1280;
        let mut acc: Vec<f32> = Vec::with_capacity(FEED_SAMPLES * 2);
        let mut final_text: Option<String> = None;
        let mut finalize_reply: Option<mpsc::Sender<Option<LiveResult>>> = None;
        let mut stream = Some(stream);

        while let Ok(cmd) = rx.recv() {
            match cmd {
                StreamCmd::Feed(mut pcm) => {
                    self.stream_router.note_feed_consumed();
                    if may_give_up && give_up.load(Ordering::Acquire) {
                        // The recording is being transcribed in one pass; the
                        // backlog would be wasted.
                        continue;
                    }
                    let Some(stream) = stream.as_ref() else {
                        continue;
                    };
                    // A non-finite sample would make the engine reject the
                    // whole feed.
                    sanitize_pcm(&mut pcm);
                    acc.extend_from_slice(&pcm);
                    while acc.len() >= FEED_SAMPLES {
                        let chunk: Vec<f32> = acc.drain(..FEED_SAMPLES).collect();
                        // Waits only while the engine has a few feeds queued
                        // already; past that the router's bounded queue takes
                        // the backlog (and drops frames with a warning).
                        stream.feed(chunk);
                    }
                    self.touch_activity();
                }
                StreamCmd::Finalize { reply, .. } => {
                    finalize_reply = Some(reply);
                    let Some(stream) = stream.take() else {
                        break;
                    };
                    if may_give_up && give_up.load(Ordering::Acquire) {
                        debug!(
                            "Live transcription: dropped the backlog; the recording is \
                             transcribed in one pass"
                        );
                        // Dropping the handle resets the stream; `final_text`
                        // stays None, which sends the caller to the batch path.
                        drop(stream);
                        break;
                    }
                    if pads_tail {
                        acc.resize(acc.len() + BUFFERED_STREAM_TAIL_PAD_SAMPLES, 0.0);
                    }
                    // Flush the sub-80 ms tail (and any pad) so the last words
                    // are included before finalizing.
                    if !acc.is_empty() {
                        stream.feed(std::mem::take(&mut acc));
                    }
                    match stream.finalize(false) {
                        Ok(Some(finalized)) => {
                            // committed now holds the full result and tentative
                            // is emptied; reflect it in the overlay, then take
                            // display() (= committed + tentative) as the final
                            // transcript.
                            let (committed, tentative) =
                                lock_display(&display).show(&finalized.text);
                            let _ = self.app_handle.emit(
                                "stream-text",
                                StreamTextPayload {
                                    committed,
                                    tentative,
                                },
                            );
                            final_text = Some(finalized.text.display());
                        }
                        // Lost to a worker crash or a failure: batch has the
                        // whole recording.
                        Ok(None) => {
                            info!(
                                "Live transcription: the stream was lost; the recording is \
                                 transcribed in batch"
                            );
                        }
                        // `finalize_stream` reads the cancel from the token.
                        Err(EngineError::Cancelled) => {}
                        Err(e) => warn!("Live transcription finalize failed: {}", e),
                    }
                    break;
                }
                StreamCmd::Cancel => break,
            }
        }
        // A stream still held here was cancelled or abandoned; dropping the
        // handle resets it in the worker.
        drop(stream);
        self.stream_router.clear();

        if let Some(reply) = finalize_reply {
            let result = final_text
                .filter(|t| !t.trim().is_empty())
                .map(|text| LiveResult {
                    text,
                    facts: final_facts,
                });
            let _ = reply.send(result);
        }
    }

    /// Drain the command channel when there is no stream to feed, replying
    /// `None` to any Finalize so the caller uses the batch path.
    fn drain_stream_no_model(&self, rx: mpsc::Receiver<StreamCmd>) {
        while let Ok(cmd) = rx.recv() {
            match cmd {
                StreamCmd::Feed(_) => self.stream_router.note_feed_consumed(),
                StreamCmd::Finalize { reply, .. } => {
                    let _ = reply.send(None);
                    break;
                }
                StreamCmd::Cancel => break,
            }
        }
    }

    /// Finalize the live-transcription session and return the merged transcript.
    ///
    /// `recorded_samples` is the length of the recording `stop_recording`
    /// returned, which a long-form session checks its own audio against.
    ///
    /// Returns `Ok(None)` when no stream is active (→ caller uses batch), when no
    /// model was available, or when the streamed result is empty; `Err` on
    /// timeout, and [`TranscriptionCancelled`] when Esc stopped it, so a cancel
    /// never turns into a batch run. Applies the SAME post-processing as batch
    /// (custom words for models that were not given them + the
    /// filler/hallucination filter).
    pub fn finalize_stream(&self, recorded_samples: usize) -> Result<Option<String>> {
        let TakenStream {
            tx,
            give_up,
            cancel,
        } = match self.stream_router.take() {
            Some(taken) => taken,
            None => return Ok(None),
        };
        // How many frames the live path had to discard because inference
        // couldn't keep up. take() has already closed the router to new feeds
        // (and the recorder's stop-drain has already run), so this count is
        // final for the session.
        let dropped = self.stream_router.dropped_frames();

        let (reply_tx, reply_rx) = mpsc::channel();
        if tx
            .send(StreamCmd::Finalize {
                recorded_samples,
                reply: reply_tx,
            })
            .is_err()
        {
            // Worker already gone — fall back to batch.
            return Ok(None);
        }
        drop(tx); // no further commands for this session

        // The worker answers Finalize only after it has worked through every frame
        // queued ahead of it, and its stream occupies the engine until then, so a
        // batch fallback started now would only queue behind it. Simply giving up
        // on it is therefore never an option. The first dictation after launch is
        // one way to fall behind (every frame queues while the model loads), a busy
        // machine is another: a full build in the background once held dictations
        // here for 15 to 17 seconds.
        //
        // So past `STREAM_FINALIZE_PATIENCE` the worker is asked to stop instead.
        // On a GPU it drops what is left, hands the engine back and answers None,
        // and the recording is transcribed in one pass, which there takes well
        // under a second where draining the backlog took many. A worker that
        // cannot redo the recording cheaply ignores the request, and is waited
        // for as before. A disconnect means the worker is gone and waiting is
        // pointless.
        let raw = match reply_rx.recv_timeout(STREAM_FINALIZE_PATIENCE) {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                give_up.store(true, Ordering::Release);
                info!(
                    "Live transcription is still behind {:?} after the recording ended; \
                     asking it to stop so the recording can be transcribed in one pass",
                    STREAM_FINALIZE_PATIENCE
                );
                match reply_rx.recv_timeout(Duration::from_secs(30)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        warn!(
                            "Live transcription is still working through its backlog after \
                             30s; waiting for it rather than losing the recording"
                        );
                        reply_rx.recv_timeout(Duration::from_secs(120))
                    }
                    other => other,
                }
            }
            other => other,
        };
        if cancel.is_cancelled() {
            return Err(anyhow::Error::new(TranscriptionCancelled));
        }
        let live = match raw {
            Ok(Some(live)) => {
                if dropped > 0 {
                    // The machine couldn't keep up with real-time streaming and
                    // discarded audio, so the streamed transcript is missing
                    // words. Prefer completeness over speed here: return None so
                    // the caller re-transcribes the full recording in batch (a
                    // short extra wait) rather than pasting a truncated result.
                    // The stream has ended before the worker replied, so the
                    // batch run is not held behind it.
                    warn!(
                        "Live transcription dropped {} frame(s) under load; using the complete \
                         batch transcription of the full recording instead of the truncated \
                         live result (a short extra wait, but nothing is cut off).",
                        dropped
                    );
                    return Ok(None);
                }
                live
            }
            Ok(None) => return Ok(None),
            Err(_) => return Err(anyhow::anyhow!("Live transcription finalize timed out")),
        };

        // Mirror batch post-processing on the final text: the worker reported
        // how its run was configured, so the same rules apply as there.
        let settings = get_settings(&self.app_handle);
        let filtered = post_process_text(&settings, live.text, &live.facts);

        if filtered.trim().is_empty() {
            return Ok(None);
        }

        self.maybe_unload_immediately("live transcription");
        Ok(Some(filtered))
    }

    /// Cancel any active live-transcription session, ending its stream in the
    /// engine. No-op when no stream is active. A feed or long-form chunk the
    /// engine is decoding stops too, so a cancelled recording never waits for
    /// it.
    pub fn cancel_stream(&self) {
        if let Some(taken) = self.stream_router.take() {
            taken.cancel.cancel();
            let _ = taken.tx.send(StreamCmd::Cancel);
        }
    }
}

/// Initialize transcribe.cpp once: route its native + ggml logs into the `log`
/// facade, then register the compute backends. In the `dynamic-backends`
/// Windows/Linux build this loads the ggml backend DLLs/`.so`s that sit next to
/// libtranscribe; on macOS and Windows on ARM (compiled-in backends) it only
/// registers what is built in. A failure here (e.g. missing backend modules) is
/// logged and swallowed. Must run before any model load or device enumeration
/// in this process.
///
/// The app itself never calls this: every transcribe.cpp call it makes runs in
/// the transcription worker (`engine_supervisor`), which initializes its own
/// backends, so a driver that crashes while registering takes only the worker
/// down. What is left here serves the headless `--list-devices` and
/// `--probe-devices` flags, which are their own short-lived process anyway,
/// and the in-process live tests.
pub fn init_transcribe_cpp() {
    // Once, and on first use rather than at launch: registering the ggml
    // Vulkan module creates a Vulkan instance and loads the GPU driver into
    // this process.
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        transcribe_cpp::init_logging();
        let result = if transcribe_gpu_disabled_for_host() {
            // A backend outside the mask never runs any code, so the emulated
            // GPU driver is never loaded at all.
            warn!(
                "Windows x64 build is running under emulation on an ARM64 PC; \
                 speech runs on the processor"
            );
            transcribe_cpp::init_backends_with(
                None::<&std::path::Path>,
                transcribe_cpp::BackendMask::CPU,
            )
        } else {
            transcribe_cpp::init_backends_default()
        };
        match result {
            Ok(()) => {
                let devices = transcribe_cpp::devices();
                info!(
                    "transcribe.cpp {} initialized with {} compute device(s): [{}]",
                    transcribe_cpp::version(),
                    devices.len(),
                    devices
                        .iter()
                        .map(|d| format!(
                            "{} ({}, {})",
                            device_label(d),
                            d.kind,
                            device_type_name(d.device_type)
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Err(e) => warn!(
                "transcribe.cpp backend init failed (local speech models unavailable): {}",
                e
            ),
        }
    });
}

// Where a model load should run, decided from the settings alone. The same
// type the transcription worker resolves, so there is one rule.
use crate::engine_supervisor::LoadTarget;

/// Map the user's speech-processing choice onto a load target.
///
/// `Gpu` names one exact device; without a saved key it means what `Auto`
/// means, the way upstream Handy reads it (settings migration normalizes that
/// pair to `Auto` anyway). A host whose GPU is off limits always gets the CPU.
fn load_target(
    setting: TranscribeAcceleratorSetting,
    gpu_device: Option<&str>,
    gpu_disabled: bool,
) -> LoadTarget {
    if gpu_disabled {
        return LoadTarget::Cpu;
    }
    match (setting, gpu_device) {
        (TranscribeAcceleratorSetting::Cpu, _) => LoadTarget::Cpu,
        (TranscribeAcceleratorSetting::Gpu, Some(key)) => LoadTarget::Device(key.to_string()),
        (TranscribeAcceleratorSetting::Gpu, None) | (TranscribeAcceleratorSetting::Auto, _) => {
            LoadTarget::Auto
        }
    }
}

/// The load target the transcription worker gets for the user's choice.
///
/// `Auto` leaves the device unset: transcribe.cpp then takes the first GPU that
/// initializes in its probe order, every dedicated GPU before any integrated
/// one, and falls back to the processor, so a laptop or desktop with both
/// lands on the dedicated card. A saved device that is gone (a removed eGPU, a
/// driver that stopped reporting it) makes the worker fall back to that policy
/// with a warning rather than fail the load, since an exact device request
/// never falls back on its own.
fn worker_load_target(settings: &crate::settings::AppSettings) -> LoadTarget {
    load_target(
        settings.transcribe_accelerator,
        settings.transcribe_gpu_device.as_deref(),
        transcribe_gpu_disabled_for_host(),
    )
}

/// transcribe.cpp load options for the user's speech-processing choice, for
/// the in-process live tests, which measure the engine itself. The app loads
/// through [`worker_load_target`].
#[cfg(test)]
fn transcribe_cpp_load_options(
    settings: &crate::settings::AppSettings,
) -> transcribe_cpp::ModelOptions {
    use transcribe_cpp::{Backend, ModelOptions};

    init_transcribe_cpp();
    match worker_load_target(settings) {
        LoadTarget::Cpu => ModelOptions {
            backend: Backend::Cpu,
            device: None,
        },
        LoadTarget::Auto => ModelOptions::default(),
        LoadTarget::Device(key) => {
            let device = transcribe_compute_devices()
                .into_iter()
                .find(|d| is_gpu_device(d) && device_key_of(d) == key);
            if device.is_none() {
                warn!(
                    "The GPU chosen for speech is no longer available; using automatic device selection"
                );
            }
            // `Auto` accepts an exact GPU or integrated GPU device.
            ModelOptions {
                backend: Backend::Auto,
                device,
            }
        }
    }
}

/// CPU threads for a transcribe.cpp session; 0 lets the library use every
/// logical CPU.
///
/// The Parakeet family (Parakeet, Nemotron) decodes on the CPU even when its
/// encoder runs on a GPU, one small step per frame and token, each spread
/// across that many threads that wait for one another. On an idle machine that
/// is harmless; with every core busy (a build, a game) each step waits for all
/// of them to be scheduled, and dictations sat on "Transcribing…" for 15 to 17
/// seconds. One thread measured the same as sixteen on an idle machine
/// (Parakeet Unified streaming finished 64 ms after the audio against 105 ms),
/// and under load turned a live stream that could not keep up into one at 0.2x
/// real time, and a 37-second batch from 4–11 s into 0.4–0.6 s. Raising the
/// thread priority instead made it worse, since the boosted thread then spins
/// waiting for its own helpers.
///
/// Every other family, and any model on the CPU, keeps all threads: Canary
/// decodes on the GPU and was slower with one thread under load, and a
/// CPU-only encoder needs them all.
///
/// Decided in the transcription worker, which opens the session.
pub(crate) fn transcribe_cpp_session_threads(arch: &str, on_gpu: bool) -> i32 {
    if on_gpu && arch == "parakeet" {
        1
    } else {
        0
    }
}

/// The user's custom words plus safe temporary correction hints.
///
/// The app name is distinctive enough for fuzzy correction. The Flow activation
/// phrase is deliberately excluded here: correction runs over the whole
/// transcript, so including "Hey Flow" could rewrite ordinary mid-sentence
/// speech such as "hey flaw". Flow's leading-phrase matcher handles recognition
/// near-misses independently.
///
/// Words learned from the user's own corrections (`autolearn`) are folded in here
/// too, and this is the single point at which they take effect — biasing the
/// recogniser, seeding the cloud providers' keyterm lists, and driving the fuzzy
/// correction pass, all from one list. They are stored separately from
/// `custom_words` because a learned word is a *guess* the user must be able to
/// review and remove, but by the time recognition happens the distinction no longer
/// matters and keeping two paths would mean one of them eventually forgot them.
pub fn recognition_words(settings: &crate::settings::AppSettings) -> Vec<String> {
    let mut words = settings.custom_words.clone();
    for learned in &settings.learned_words {
        // The user's own list wins: if they have typed the word in themselves, the
        // learned duplicate contributes nothing and would only be sent twice.
        if !words.iter().any(|word| word.eq_ignore_ascii_case(learned)) {
            words.push(learned.clone());
        }
    }
    if settings.flow_enabled
        && !words
            .iter()
            .any(|word| word.eq_ignore_ascii_case("SpeakoFlow"))
    {
        words.push("SpeakoFlow".to_string());
    }
    words
}

/// Decoder-only prompts can safely include the full activation phrase because
/// they bias recognition without rewriting text after the fact. Used only by
/// Whisper-family engines that support an initial prompt.
fn recognition_prompt_words(settings: &crate::settings::AppSettings) -> Vec<String> {
    let mut words = recognition_words(settings);
    let phrase = settings.flow_phrase.trim();
    if settings.flow_enabled
        && !phrase.is_empty()
        && !words.iter().any(|word| word.eq_ignore_ascii_case(phrase))
    {
        words.push(phrase.to_string());
    }
    words
}

/// The language that was **spoken**, for choosing which filler words are safe
/// to remove. Deliberately not `app_language`: the UI language defaults to the
/// OS locale, so a Portuguese speaker on an English UI had every "um" ("a")
/// deleted. Evidence, strongest first: a translation to English, the language
/// the run used for the user's choice (`effective_language`, which is `auto`
/// when the model detected it), a model that only speaks one language.
/// Otherwise `"auto"`, which makes the filter detect it from the text and, if
/// the text is not conclusive, remove only fillers that are a word nowhere.
fn spoken_language_for_fillers(facts: &RunFacts) -> String {
    if facts.translated {
        return "en".to_string();
    }
    let effective = facts.effective_language.trim();
    if !effective.is_empty() && effective != language::AUTO {
        return effective.to_string();
    }
    if let [only] = facts.languages.as_slice() {
        return language::app_language_code(only);
    }
    language::AUTO.to_string()
}

/// What the text post-processing needs to know about the run that produced
/// a transcript. Built where the run was configured, so the batch path, a
/// live stream and the cloud all describe themselves the same way.
#[derive(Clone, Debug, PartialEq)]
struct RunFacts {
    /// The custom words already reached the recogniser: Whisper's initial
    /// prompt, a model's vocabulary, or a cloud provider's keyterms. A second,
    /// fuzzy pass over its output could then only rewrite words it got right.
    words_biased: bool,
    /// Remove filler words. Local engines always have; on the cloud,
    /// "Remove filler words" decides it (the provider-side flag alone was not
    /// enough, since several providers do not implement it).
    remove_fillers: bool,
    /// The output was translated to English.
    translated: bool,
    /// The language the run used for the user's intent, in the app's
    /// vocabulary; `auto` when the model detected it.
    effective_language: String,
    /// The model's languages; empty when unknown, as on the cloud.
    languages: Vec<String>,
}

impl RunFacts {
    fn local(plan: &RunPlan) -> Self {
        Self {
            words_biased: plan.words_biased,
            remove_fillers: true,
            translated: plan.translated,
            effective_language: plan.effective_language.clone(),
            languages: plan.languages.clone(),
        }
    }

    /// A cloud run: the provider was told the spoken language as it was
    /// chosen, and translates on its own route (`stt_cloud`), never claimed
    /// here.
    fn cloud(settings: &crate::settings::AppSettings, biased_upstream: bool) -> Self {
        Self {
            words_biased: biased_upstream,
            remove_fillers: settings.cloud_stt_no_verbatim,
            translated: false,
            effective_language: settings.selected_language.clone(),
            languages: Vec::new(),
        }
    }
}

/// Custom-word correction and the filler/hallucination filter, as every
/// transcript gets them. Fails open: a bug here never loses the transcript.
fn post_process_text(
    settings: &crate::settings::AppSettings,
    raw: String,
    facts: &RunFacts,
) -> String {
    let recognition_hints = recognition_words(settings);
    let spoken_language = spoken_language_for_fillers(facts);
    fail_open_text_transform(raw, |raw| {
        let corrected = if !recognition_hints.is_empty() && !facts.words_biased {
            apply_custom_words(&raw, &recognition_hints, settings.word_correction_threshold)
        } else {
            raw
        };
        if facts.remove_fillers {
            filter_transcription_output(&corrected, &spoken_language, &settings.custom_filler_words)
        } else {
            corrected
        }
    })
}

/// One in-process run over a whole clip, for the live tests that measure the
/// engine itself; the app runs in the worker ([`TranscriptionManager`]'s
/// `run_in_worker`), with the same empty result for a clip too short to
/// decode.
#[cfg(test)]
fn run_once(
    session: &mut transcribe_cpp::Session,
    pcm: &[f32],
    options: &transcribe_cpp::RunOptions,
) -> Result<String> {
    match session.run(pcm, options) {
        Ok(transcript) => Ok(transcript.text),
        Err(transcribe_cpp::Error::InputTooShort(reason)) => {
            debug!("Audio too short to transcribe ({}); empty result", reason);
            Ok(String::new())
        }
        Err(e) => Err(anyhow::anyhow!(
            "transcribe.cpp transcription failed: {}",
            e
        )),
    }
}

/// A fresh Silero scorer for each clip long-form chunking cuts. Without the
/// model file the chunker cuts at fixed lengths instead.
fn silero_scorer_factory(vad_path: Option<PathBuf>) -> MakeScorer {
    Box::new(move || {
        let path = vad_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the VAD model could not be found"))?;
        // The threshold only matters for `push_frame`; chunking reads the
        // probabilities themselves.
        let vad = SileroVad::new(&path, 0.5)?;
        Ok(Box::new(vad) as Box<dyn crate::audio_toolkit::vad::SpeechScorer>)
    })
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Optional text cleanup (custom-word correction, filler removal) must never
/// discard a successful transcription. The transform is pure and owns its
/// input, so recovering the untouched text is safe even if a bug in it unwinds.
/// Before this, a panic there killed the dictation task: the overlay went away,
/// nothing was pasted, and no history entry was written. Backport of Handy
/// 2203a826.
fn fail_open_text_transform<F>(raw: String, transform: F) -> String
where
    F: FnOnce(String) -> String,
{
    let fallback = raw.clone();
    match catch_unwind(AssertUnwindSafe(|| transform(raw))) {
        Ok(processed) => processed,
        Err(payload) => {
            error!(
                "Optional transcription text post-processing panicked: {}; using the raw transcription",
                panic_payload_message(payload.as_ref())
            );
            fallback
        }
    }
}

/// Cosmetic pass for the LIVE overlay text: the same safe custom-word /
/// recognition-hint correction the final transcript receives, plus — when
/// Flow is on — an exact rewrite of the leading activation phrase to its
/// configured spelling ("Hey Flo," reads as "Hey Flow," while still
/// speaking). Display-only; the final transcript is corrected independently
/// on finalize. The phrase rewrite is surgical (byte-range replacement), so
/// unlike fuzzy n-gram hints it can never absorb neighboring words.
fn correct_live_display(
    text: &str,
    hints: &[String],
    threshold: f64,
    flow_phrase: Option<&str>,
) -> String {
    let mut out = if text.is_empty() || hints.is_empty() {
        text.to_string()
    } else {
        fail_open_text_transform(text.to_string(), |text| {
            apply_custom_words(&text, hints, threshold)
        })
    };
    if let Some(phrase) = flow_phrase {
        if let Some(fixed) = crate::flow::canonicalize_leading_phrase(&out, phrase) {
            out = fixed;
        }
    }
    out
}

/// How one local run is configured, and what its text post-processing needs
/// to know about it.
#[derive(Clone, Debug, PartialEq)]
pub struct RunPlan {
    pub options: transcribe_cpp::RunOptions,
    /// The language the model uses for the user's intent, in the app's
    /// vocabulary (`managers::language::effective_language`); `auto` when it
    /// detects it.
    pub effective_language: String,
    /// The custom words went to the model, as Whisper's initial prompt or as
    /// vocabulary.
    pub words_biased: bool,
    /// The run translates to English.
    pub translated: bool,
    /// What the loaded model lists.
    pub languages: Vec<String>,
}

/// Build the transcribe.cpp run for a batch or streaming run from the user
/// settings and the loaded model's capabilities, architecture, and whether it
/// takes a vocabulary (`Feature::Vocabulary`).
///
/// Shared by the batch `transcribe()` arm, each long-form chunk, and the
/// native streaming worker:
/// - **language**: `selected_language` is an intent. The model uses
///   [`language::effective_language`] for it: the intent when the model lists
///   it, otherwise detection when the model can, otherwise English or its
///   first language. So a model that cannot detect is never left on auto,
///   and an intent it does not list falls back instead of silently becoming
///   auto. The engine gets the model's own spelling of that language
///   ([`language::engine_language_code`]: `zh-Hans` → `zh`, `no` → `nb`,
///   `tl` → `fil`), or `None` to detect.
/// - **translate**: when `translate_to_english` is on, the model advertises
///   translation to English, and the language in use isn't already English,
///   request `Task::Translate` with `target_language = "en"`; otherwise
///   `Transcribe`. A detected language may be English; the model then
///   transcribes, which is what translating English to English means.
/// - **timestamps**: `Auto` — "the richest the model supports", resolved
///   per-family by transcribe.cpp. We never consume timestamps (only `.text`),
///   and hardcoding `Segment` made every model that declares no timestamp
///   support (Canary, Cohere, Qwen3-ASR, Voxtral, …) fail its run with
///   "unsupported timestamp granularity (status 12)". `Auto` transcribes on all
///   families and still yields the same text.
/// - **custom words**: Whisper gets them as its initial prompt (a run-slot
///   extension every other family rejects). A model with `Feature::Vocabulary`
///   gets them as vocabulary (upstream Handy #2284), which biases recognition
///   rather than rewriting text afterwards. Either way the fuzzy
///   `apply_custom_words` pass is skipped; only a model given neither gets it.
pub fn transcribe_cpp_run_plan(
    settings: &crate::settings::AppSettings,
    caps: &transcribe_cpp::Capabilities,
    arch: &str,
    supports_vocabulary: bool,
) -> RunPlan {
    use transcribe_cpp::{RunExtension, RunOptions, Task, TimestampKind, WhisperRunOptions};

    let effective = language::effective_language(
        &settings.selected_language,
        &caps.languages,
        caps.supports_language_detect,
    );
    let engine_language = language::engine_language_code(&effective, &caps.languages);

    // Translate to English only when supported and the source isn't English.
    let src_is_english = language::base_language(&effective) == "en";
    let translates_to_english = caps.supports_translate
        && (caps.translate_target_languages.is_empty()
            || caps
                .translate_target_languages
                .iter()
                .any(|target| language::base_language(target) == "en"));
    let translated = settings.translate_to_english && translates_to_english && !src_is_english;
    let (task, target_language) = if translated {
        (Task::Translate, Some("en".to_string()))
    } else {
        (Task::Transcribe, None)
    };

    // Custom words (+ session recognition hints). Both forms bias decoding
    // rather than rewrite its output, so the Flow phrase is safe in them.
    let recognition_hints = recognition_prompt_words(settings);
    let is_whisper = arch == "whisper";
    let family = if is_whisper && !recognition_hints.is_empty() {
        Some(RunExtension::Whisper(WhisperRunOptions {
            initial_prompt: Some(recognition_hints.join(", ")),
            ..Default::default()
        }))
    } else {
        None
    };
    let vocabulary = if !is_whisper && supports_vocabulary {
        recognition_hints.clone()
    } else {
        Vec::new()
    };
    let words_biased = family.is_some() || !vocabulary.is_empty();

    RunPlan {
        options: RunOptions {
            task,
            timestamps: TimestampKind::Auto,
            language: engine_language,
            target_language,
            family,
            vocabulary,
            ..Default::default()
        },
        effective_language: effective,
        words_biased,
        translated,
        languages: caps.languages.clone(),
    }
}

/// One GPU transcribe.cpp can run a model on.
///
/// `name`, `kind` and `total_vram_mb` keep the meanings the old whisper.cpp
/// device list gave them, because more than the speech engine reads them:
/// llama.cpp device pinning (`local_llm`) matches `name` against its own adapter
/// descriptions and treats `kind == "integrated"` as the iGPU to avoid, and
/// onboarding and the assistant catalog size local models from the VRAM.
/// `Deserialize` keeps the `--probe-devices` JSON readable by a script.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq, Type)]
pub struct GpuDeviceOption {
    /// Stable identity, saved as `transcribe_gpu_device` (see [`device_key`]).
    /// Never a registry index: those are process-local in transcribe.cpp.
    pub id: String,
    /// The adapter description, e.g. "NVIDIA GeForce RTX 4070 Ti SUPER".
    pub name: String,
    /// `"dedicated"`, `"integrated"`, or `"unknown"`.
    pub kind: String,
    /// Reported device memory, 0 when the backend does not report it.
    pub total_vram_mb: usize,
}

/// The saved identity of a device registered in this process (the in-process
/// live tests); the app reads it from the worker's [`DeviceInfo`].
///
/// [`DeviceInfo`]: crate::engine_supervisor::DeviceInfo
#[cfg(test)]
fn device_key_of(device: &transcribe_cpp::Device) -> String {
    device_key(&device.kind, device.device_id.as_deref(), &device.name)
}

/// The human name of a device: the adapter description when there is one.
fn device_label(device: &transcribe_cpp::Device) -> &str {
    if device.description.is_empty() {
        &device.name
    } else {
        &device.description
    }
}

/// Whether a device is a GPU, dedicated or integrated. CPU, host-memory
/// accelerators (BLAS) and devices a newer runtime reports in a class this
/// binding does not know cannot host a model on their own.
fn is_gpu_device(device: &transcribe_cpp::Device) -> bool {
    matches!(
        device.device_type,
        transcribe_cpp::DeviceType::Gpu | transcribe_cpp::DeviceType::Igpu
    )
}

/// The `kind` the device list has always used for a GPU.
fn gpu_kind(device_type: transcribe_cpp::DeviceType) -> &'static str {
    match device_type {
        transcribe_cpp::DeviceType::Gpu => "dedicated",
        transcribe_cpp::DeviceType::Igpu => "integrated",
        _ => "unknown",
    }
}

/// A readable device class for the log.
fn device_type_name(device_type: transcribe_cpp::DeviceType) -> &'static str {
    match device_type {
        transcribe_cpp::DeviceType::Gpu => "dedicated GPU",
        transcribe_cpp::DeviceType::Igpu => "integrated GPU",
        transcribe_cpp::DeviceType::Cpu => "CPU",
        transcribe_cpp::DeviceType::Accel => "accelerator",
        transcribe_cpp::DeviceType::Unknown => "unknown",
    }
}

/// A GPU as the app's device list shows it.
fn gpu_option(device: &crate::engine_supervisor::DeviceInfo) -> GpuDeviceOption {
    GpuDeviceOption {
        id: device.key.clone(),
        name: device.label().to_string(),
        kind: gpu_kind(device.device_type).to_string(),
        total_vram_mb: (device.memory_total / (1024 * 1024)) as usize,
    }
}

/// The compute devices transcribe.cpp registered in THIS process, after the
/// backends are up. Only the headless CLI flags and the in-process tests use
/// it; the app lists devices through a worker ([`gpu_devices`]).
fn transcribe_compute_devices() -> Vec<transcribe_cpp::Device> {
    init_transcribe_cpp();
    transcribe_cpp::devices()
}

/// Whether listing GPUs can work at all on this CPU. ggml's Vulkan backend
/// uses FMA3, which raises an uncatchable SIGILL on a CPU without it. A worker
/// would survive that, but every listing would then crash a worker and mark
/// the GPU unavailable, so such a CPU lists no GPU instead.
fn cpu_can_list_gpus() -> bool {
    #[cfg(target_arch = "x86_64")]
    if !std::arch::is_x86_feature_detected!("fma") {
        return false;
    }
    true
}

/// Enumerate the GPUs in THIS process, for `--probe-devices`.
fn enumerate_gpu_devices() -> Vec<GpuDeviceOption> {
    if !cpu_can_list_gpus() {
        warn!("CPU lacks FMA3 support — skipping transcribe.cpp device enumeration");
        return Vec::new();
    }
    transcribe_compute_devices()
        .iter()
        .filter(|device| is_gpu_device(device))
        .map(|device| gpu_option(&crate::engine_supervisor::DeviceInfo::from_device(device)))
        .collect()
}

/// The device list as produced by one enumeration pass, as `--probe-devices`
/// prints it: one JSON line, for a script or an audit to read.
#[derive(Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct DeviceProbe {
    pub gpu_devices: Vec<GpuDeviceOption>,
}

/// Enumerate the devices in THIS process, for the `--probe-devices` flag. The
/// flag's process is short-lived and does nothing else, so a crash there
/// costs only the listing.
pub fn enumerate_devices_in_process() -> DeviceProbe {
    DeviceProbe {
        gpu_devices: enumerate_gpu_devices(),
    }
}

/// The GPUs, as the transcription worker last listed them.
///
/// Listing happens in a worker process on every platform, so a Vulkan driver
/// that crashes or hangs while enumerating costs a GPU listing, not the app.
/// This replaced the Linux-only `--probe-devices` child the app used to spawn
/// for the same reason, and the in-process listing on Windows and macOS. The
/// first call with no list yet starts a short-lived, model-less worker on the
/// caller's thread; afterwards every model worker that may use the GPU
/// refreshes the list in its hello. Empty when no list could be had (no engine
/// yet, a GPU marked unavailable before it was ever listed, a CPU without
/// FMA3).
fn cached_gpu_devices() -> Vec<GpuDeviceOption> {
    if !cpu_can_list_gpus() {
        return Vec::new();
    }
    let Some(engine) = ENGINE.get() else {
        debug!("GPU list asked for before the transcription engine started");
        return Vec::new();
    };
    engine
        .devices()
        .unwrap_or_default()
        .iter()
        .filter(|device| device.is_gpu())
        .map(gpu_option)
        .collect()
}

fn select_preferred_gpu_device(devices: &[GpuDeviceOption]) -> Option<&GpuDeviceOption> {
    devices.iter().max_by_key(|device| {
        let kind_priority = match device.kind.as_str() {
            "dedicated" => 2u8,
            "unknown" => 1,
            _ => 0,
        };
        (kind_priority, device.total_vram_mb)
    })
}

/// Best compute device for local inference. Dedicated GPUs win over integrated
/// GPUs even when an iGPU reports a larger shared-memory budget.
pub fn preferred_gpu_device() -> Option<GpuDeviceOption> {
    select_preferred_gpu_device(&cached_gpu_devices()).cloned()
}

/// Every GPU this machine reports, in enumeration order.
///
/// [`preferred_gpu_device`] answers "which one is best"; this answers "what else
/// is there", which is what a consumer needs in order to *exclude* a device
/// rather than merely rank it (see `local_llm::pinned_device_arg`).
pub fn gpu_devices() -> Vec<GpuDeviceOption> {
    cached_gpu_devices()
}

/// Whether this process must not use a GPU for speech: the x64 Windows build
/// running under emulation on a Windows on ARM PC (Handy #1731). The emulated
/// process sees the ARM GPU's Vulkan driver through a translation layer that
/// is not reliable for ggml's compute shaders, so the GPU backend is not even
/// registered there and every setting means the processor.
fn transcribe_gpu_disabled_for_host() -> bool {
    is_windows_x64_emulated_on_arm64()
}

#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const IMAGE_FILE_MACHINE_ARM64: u16 = 0xaa64;

#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
fn native_machine_is_arm64(native_machine: Option<u16>) -> bool {
    native_machine == Some(IMAGE_FILE_MACHINE_ARM64)
}

/// Whether this is the x64 Windows build running under emulation on Windows
/// ARM64. Fail-open: a native x64 host, a Windows without `IsWow64Process2`, or
/// any API error leaves the GPU path as it is.
fn is_windows_x64_emulated_on_arm64() -> bool {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        static DETECTED: OnceLock<bool> = OnceLock::new();
        *DETECTED.get_or_init(|| native_machine_is_arm64(native_windows_machine()))
    }

    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        false
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn native_windows_machine() -> Option<u16> {
    use windows::core::{s, w, BOOL};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Threading::GetCurrentProcess;

    type IsWow64Process2 = unsafe extern "system" fn(HANDLE, *mut u16, *mut u16) -> BOOL;

    // Resolved at run time so starting the app never raises the minimum Windows
    // version: every Windows on ARM release has this function, and where it is
    // missing the x64 behaviour is the right one anyway.
    unsafe {
        let kernel32 = GetModuleHandleW(w!("kernel32.dll")).ok()?;
        let address = GetProcAddress(kernel32, s!("IsWow64Process2"))?;
        // SAFETY: GetProcAddress returned the documented IsWow64Process2
        // export; a function pointer has one representation on Windows.
        let is_wow64_process2: IsWow64Process2 = std::mem::transmute(address);
        let mut process_machine = 0u16;
        let mut native_machine = 0u16;
        is_wow64_process2(
            GetCurrentProcess(),
            &mut process_machine,
            &mut native_machine,
        )
        .as_bool()
        .then_some(native_machine)
    }
}

/// The speech-processing choices this process offers.
fn available_transcribe_accelerators(gpu_disabled: bool) -> Vec<String> {
    if gpu_disabled {
        vec!["cpu".to_string()]
    } else {
        vec!["auto".to_string(), "cpu".to_string(), "gpu".to_string()]
    }
}

#[derive(Serialize, Clone, Debug, Type)]
pub struct AvailableAccelerators {
    /// The `transcribe_accelerator` values that mean something here: `auto`,
    /// `cpu`, and `gpu` (one exact device from `gpu_devices`). Only `cpu` on a
    /// host whose GPU is off limits.
    pub transcribe: Vec<String>,
    /// The GPUs transcribe.cpp can run on, each with its stable key in `id`.
    /// Also read by onboarding and the assistant catalog for the VRAM.
    pub gpu_devices: Vec<GpuDeviceOption>,
}

/// The speech-processing choices and GPUs available to this process.
pub fn get_available_accelerators() -> AvailableAccelerators {
    AvailableAccelerators {
        transcribe: available_transcribe_accelerators(transcribe_gpu_disabled_for_host()),
        gpu_devices: gpu_devices(),
    }
}

impl Drop for TranscriptionManager {
    fn drop(&mut self) {
        // Skip shutdown unless this is the very last clone. TranscriptionManager
        // is cloned by initiate_model_load() and the watcher thread — those
        // clones dropping must not kill the watcher. The watcher thread holds
        // its own clone, so the strong count of any shared field is always >= 2
        // while the watcher is alive. When it reaches 1, only this instance
        // remains and we can safely shut down.
        if Arc::strong_count(&self.current_model_id) > 1 {
            return;
        }

        // Signal the watcher thread to shutdown
        self.shutdown_signal.store(true, Ordering::Relaxed);

        // Wait for the thread to finish gracefully. Use match instead of
        // unwrap to avoid panicking if the mutex is poisoned — a panic inside
        // Drop calls abort() (Handy #1354).
        let mut guard = match self.watcher_handle.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if let Some(handle) = guard.take() {
            if let Err(e) = handle.join() {
                warn!("Failed to join idle watcher thread: {:?}", e);
            } else {
                debug!("Idle watcher thread joined successfully");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_text_transform_falls_back_to_raw_text_after_panic() {
        let raw = "原始轉錄。".to_string();
        let result = fail_open_text_transform(raw.clone(), |_| {
            panic!("simulated optional cleanup failure")
        });
        assert_eq!(result, raw);
    }

    #[test]
    fn filler_language_follows_the_spoken_language_not_the_ui() {
        let facts = |effective: &str, languages: &[&str], translated: bool| RunFacts {
            words_biased: false,
            remove_fillers: true,
            translated,
            effective_language: effective.to_string(),
            languages: languages.iter().map(|code| code.to_string()).collect(),
        };
        assert_eq!(spoken_language_for_fillers(&facts("pt", &[], false)), "pt");
        assert_eq!(
            spoken_language_for_fillers(&facts("auto", &[], false)),
            "auto"
        );
        // A one-language model says what was spoken, in the app's spelling.
        assert_eq!(
            spoken_language_for_fillers(&facts("auto", &["en-US"], false)),
            "en"
        );
        assert_eq!(
            spoken_language_for_fillers(&facts("auto", &["en", "de"], false)),
            "auto"
        );
        // The language the run fell back to, not the intent it could not use.
        assert_eq!(
            spoken_language_for_fillers(&facts("en", &["en", "de"], false)),
            "en"
        );
        // Translated output is English whatever was spoken.
        assert_eq!(spoken_language_for_fillers(&facts("ja", &[], true)), "en");
    }

    #[test]
    fn a_cloud_run_keeps_the_intent_and_its_own_switches() {
        let mut settings = crate::settings::get_default_settings();
        settings.selected_language = "de".into();
        settings.cloud_stt_no_verbatim = false;
        let facts = RunFacts::cloud(&settings, true);
        assert!(facts.words_biased);
        assert!(!facts.remove_fillers);
        assert!(!facts.translated);
        assert_eq!(facts.effective_language, "de");
        assert!(facts.languages.is_empty());
    }

    #[test]
    fn fuzzy_correction_is_skipped_exactly_when_the_words_reached_the_model() {
        let mut settings = crate::settings::get_default_settings();
        settings.custom_words = vec!["Kubernetes".into()];
        let facts = |words_biased| RunFacts {
            words_biased,
            remove_fillers: false,
            translated: false,
            effective_language: "en".into(),
            languages: vec!["en".into()],
        };
        assert_eq!(
            post_process_text(&settings, "I deploy on kubernetis".into(), &facts(false)),
            "I deploy on Kubernetes"
        );
        assert_eq!(
            post_process_text(&settings, "I deploy on kubernetis".into(), &facts(true)),
            "I deploy on kubernetis"
        );
    }
    use transcribe_cpp::{Capabilities, Task, TimestampKind};

    /// `--probe-devices` prints one JSON line that reads back as it was
    /// written, so a script can parse it.
    #[test]
    fn device_probe_json_is_one_line_that_round_trips() {
        let probe = DeviceProbe {
            gpu_devices: vec![GpuDeviceOption {
                id: device_key("vulkan", Some("0000:01:00.0"), "Vulkan0"),
                name: "Test GPU".to_string(),
                kind: "dedicated".to_string(),
                total_vram_mb: 8192,
            }],
        };
        let json = serde_json::to_string(&probe).unwrap();
        assert!(!json.contains('\n'));
        let parsed: DeviceProbe = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.gpu_devices, probe.gpu_devices);
    }

    /// The device list the worker reports keeps the shape `local_llm` and the
    /// frontend read: the saved key, the adapter name, the old kind words.
    #[test]
    fn a_worker_device_becomes_the_listed_gpu() {
        let info = crate::engine_supervisor::DeviceInfo {
            name: "Vulkan1".into(),
            description: "AMD Radeon(TM) Graphics".into(),
            kind: "vulkan".into(),
            device_type: transcribe_cpp::DeviceType::Igpu,
            device_id: Some("0000:0e:00.0".into()),
            memory_total: 2 * 1024 * 1024 * 1024,
            index: Some(2),
            key: device_key("vulkan", Some("0000:0e:00.0"), "Vulkan1"),
        };
        let option = gpu_option(&info);
        assert_eq!(option.id, r#"["vulkan","id","0000:0e:00.0"]"#);
        assert_eq!(option.name, "AMD Radeon(TM) Graphics");
        assert_eq!(option.kind, "integrated");
        assert_eq!(option.total_vram_mb, 2048);
    }

    #[test]
    fn a_failure_is_not_a_cancellation() {
        assert!(is_cancellation(&anyhow::Error::new(TranscriptionCancelled)));
        assert!(is_cancellation(
            &anyhow::Error::new(TranscriptionCancelled).context("while dictating")
        ));
        assert!(!is_cancellation(&anyhow::anyhow!("worker crashed")));
    }

    /// A stalled/behind worker (the receiver is held, nothing is consumed) must
    /// never let the live-transcription feed queue grow past the cap: excess
    /// frames are dropped and counted, and a freed slot lets feed() enqueue
    /// again. This is the guard against unbounded memory growth on a long
    /// recording when inference can't keep up with real-time.
    #[test]
    fn stream_router_bounds_backlog_when_worker_stalls() {
        let router = StreamRouter::new();

        // Closed router: feed is a no-op and nothing is queued.
        router.feed(&[0.0f32; 480]);
        assert_eq!(router.queued.load(Ordering::Relaxed), 0);
        assert!(!router.is_open());

        // Open a session and deliberately keep the receiver unread so the
        // "worker" never consumes — the worst case (a hung/overloaded engine).
        let opened = router.open(FeedKind::Raw);
        let generation = opened.generation;
        assert!(router.is_open());
        assert!(router.is_generation_current(generation));

        // Feed well past the cap; the in-flight queue must clamp at the cap.
        let overshoot = 500usize;
        for _ in 0..(MAX_QUEUED_FEED_FRAMES + overshoot) {
            router.feed(&[0.0f32; 480]);
        }
        assert_eq!(
            router.queued.load(Ordering::Relaxed),
            MAX_QUEUED_FEED_FRAMES,
            "queue must be bounded by the cap"
        );
        assert!(
            router.dropped.load(Ordering::Relaxed) >= overshoot as u64,
            "frames past the cap must be counted as dropped"
        );

        // Simulate the worker catching up on a few frames; freed slots must be
        // reusable by subsequent feeds.
        for _ in 0..10 {
            router.note_feed_consumed();
        }
        assert_eq!(
            router.queued.load(Ordering::Relaxed),
            MAX_QUEUED_FEED_FRAMES - 10
        );
        router.feed(&[0.0f32; 480]);
        assert_eq!(
            router.queued.load(Ordering::Relaxed),
            MAX_QUEUED_FEED_FRAMES - 9
        );

        let _tx = router.take();
        assert!(!router.is_generation_current(generation));
        drop(opened);

        // note_feed_consumed must saturate at zero, never underflow.
        let router2 = StreamRouter::new();
        router2.note_feed_consumed();
        assert_eq!(router2.queued.load(Ordering::Relaxed), 0);
    }

    /// Giving up on one recording's stream must never reach the next one's
    /// worker: every session gets its own flag and cancel token, and take()
    /// hands back the ones of the session it closes.
    #[test]
    fn stream_give_up_belongs_to_one_session() {
        let router = StreamRouter::new();
        let first = router.open(FeedKind::Raw);
        let taken = router.take().expect("an open session");
        assert!(Arc::ptr_eq(&first.give_up, &taken.give_up));
        taken.give_up.store(true, Ordering::Release);
        taken.cancel.cancel();
        assert!(first.cancel.is_cancelled());

        let second = router.open(FeedKind::Raw);
        assert!(!second.give_up.load(Ordering::Acquire));
        assert!(!second.cancel.is_cancelled());
        assert!(first.give_up.load(Ordering::Acquire));
        assert!(router
            .take()
            .is_some_and(|taken| Arc::ptr_eq(&taken.give_up, &second.give_up)));
    }

    /// A session takes one of the recorder's two feeds, never both: a stream
    /// gets every frame, long-form chunks only the kept speech.
    #[test]
    fn a_session_takes_one_feed() {
        let router = StreamRouter::new();
        let stream = router.open(FeedKind::Raw);
        router.feed(&[0.1; 480]);
        router.feed_speech(&[0.2; 480]);
        let frames: Vec<f32> = stream
            .rx
            .try_iter()
            .filter_map(|cmd| match cmd {
                StreamCmd::Feed(frame) => Some(frame[0]),
                _ => None,
            })
            .collect();
        assert_eq!(frames, vec![0.1]);
        let _ = router.take();

        let chunks = router.open(FeedKind::Speech);
        router.feed(&[0.1; 480]);
        router.feed_speech(&[0.2; 480]);
        let frames: Vec<f32> = chunks
            .rx
            .try_iter()
            .filter_map(|cmd| match cmd {
                StreamCmd::Feed(frame) => Some(frame[0]),
                _ => None,
            })
            .collect();
        assert_eq!(frames, vec![0.2]);
        let _ = router.take();

        // Closed: neither feed queues anything.
        let queued = router.queued.load(Ordering::Relaxed);
        router.feed(&[0.1; 480]);
        router.feed_speech(&[0.2; 480]);
        assert_eq!(router.queued.load(Ordering::Relaxed), queued);
    }

    #[test]
    fn only_the_parakeet_family_on_a_gpu_decodes_on_one_thread() {
        assert_eq!(transcribe_cpp_session_threads("parakeet", true), 1);
        // On the CPU the encoder needs every thread.
        assert_eq!(transcribe_cpp_session_threads("parakeet", false), 0);
        // These decode on the GPU, so the cap would only slow them down.
        assert_eq!(transcribe_cpp_session_threads("canary", true), 0);
        assert_eq!(transcribe_cpp_session_threads("whisper", true), 0);
    }

    /// Build a minimal `Capabilities` for run-plan tests.
    fn caps(languages: &[&str], supports_translate: bool) -> Capabilities {
        caps_with(languages, supports_translate, false)
    }

    fn caps_with(languages: &[&str], supports_translate: bool, detects: bool) -> Capabilities {
        Capabilities {
            native_sample_rate: 16000,
            languages: languages.iter().map(|s| s.to_string()).collect(),
            translate_target_languages: vec![],
            max_timestamp_kind: TimestampKind::Segment,
            supports_language_detect: detects,
            supports_translate,
            supports_streaming: false,
            supports_spec_decode: false,
            max_audio_ms: 0,
        }
    }

    fn plan_for(intent: &str, caps: &Capabilities, arch: &str) -> RunPlan {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = intent.into();
        transcribe_cpp_run_plan(&s, caps, arch, false)
    }

    /// Every intent against every kind of model: what the run uses (in the
    /// app's vocabulary) and what the engine is handed (in the model's).
    #[test]
    fn run_plan_language_matrix() {
        let whisper = caps_with(&["en", "zh", "de", "no", "yue", "ja"], true, true);
        let nemotron = caps_with(&["en", "de", "nb", "zh", "fil"], false, false);
        let detecting = caps_with(&["en", "de", "fr"], false, true);
        let english = caps_with(&["en"], false, false);
        let chinese = caps_with(&["zh"], false, false);
        let agnostic = caps_with(&[], false, false);
        let locales = caps_with(&["en-US", "de-DE"], false, false);

        for (intent, model, arch, effective, engine) in [
            // Listed: the model's own code goes to the engine.
            ("de", &whisper, "whisper", "de", Some("de")),
            ("zh-Hans", &whisper, "whisper", "zh-Hans", Some("zh")),
            ("zh-Hant", &nemotron, "parakeet", "zh-Hant", Some("zh")),
            ("no", &nemotron, "parakeet", "no", Some("nb")),
            ("tl", &nemotron, "parakeet", "tl", Some("fil")),
            ("de", &locales, "canary", "de", Some("de-DE")),
            // Auto on a model that detects: detection.
            ("auto", &whisper, "whisper", "auto", None),
            ("auto", &detecting, "parakeet", "auto", None),
            // Not listed, model detects: detection, not a wrong hint.
            ("ja", &detecting, "parakeet", "auto", None),
            // Not listed and no detection: never auto. English first...
            ("ja", &nemotron, "parakeet", "en", Some("en")),
            ("auto", &nemotron, "parakeet", "en", Some("en")),
            ("auto", &english, "parakeet", "en", Some("en")),
            ("auto", &locales, "canary", "en", Some("en-US")),
            // ...otherwise the model's first language.
            ("de", &chinese, "parakeet", "zh-Hans", Some("zh")),
            // Cantonese is not Chinese.
            ("yue", &whisper, "whisper", "yue", Some("yue")),
            ("yue", &nemotron, "parakeet", "en", Some("en")),
            // A model that lists nothing takes the intent as it is.
            ("pt", &agnostic, "voxtral", "pt", Some("pt")),
            ("auto", &agnostic, "voxtral", "auto", None),
        ] {
            let plan = plan_for(intent, model, arch);
            assert_eq!(plan.effective_language, effective, "intent {intent}");
            assert_eq!(plan.options.language.as_deref(), engine, "intent {intent}");
            assert_eq!(plan.languages, model.languages);
        }
    }

    #[test]
    fn run_plan_auto_language_is_none() {
        let plan = plan_for("auto", &caps_with(&["en"], false, true), "parakeet");
        assert_eq!(plan.options.language, None);
        assert_eq!(plan.options.task, Task::Transcribe);
        assert_eq!(plan.options.family, None);
        assert_eq!(plan.options.timestamps, TimestampKind::Auto);
        assert!(!plan.translated);
    }

    #[test]
    fn a_model_without_detection_is_never_sent_auto() {
        // Whatever the intent, a model that lists languages and cannot detect
        // one always gets a language.
        for intent in ["auto", "ja", "yue", "zh-Hans", "de"] {
            let plan = plan_for(intent, &caps(&["en", "de"], false), "parakeet");
            assert!(plan.options.language.is_some(), "intent {intent}");
        }
    }

    #[test]
    fn preferred_gpu_prioritizes_dedicated_over_larger_integrated_memory() {
        let devices = vec![
            GpuDeviceOption {
                id: "dgpu".to_string(),
                name: "NVIDIA GeForce RTX 4070 Ti SUPER".to_string(),
                kind: "dedicated".to_string(),
                total_vram_mb: 16_063,
            },
            GpuDeviceOption {
                id: "igpu".to_string(),
                name: "AMD Radeon(TM) Graphics".to_string(),
                kind: "integrated".to_string(),
                total_vram_mb: 16_180,
            },
        ];

        let selected = select_preferred_gpu_device(&devices).unwrap();
        assert_eq!(selected.id, "dgpu");
        assert_eq!(selected.kind, "dedicated");
    }

    fn translate_plan(intent: &str, caps: &Capabilities, arch: &str) -> RunPlan {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = intent.into();
        s.translate_to_english = true;
        transcribe_cpp_run_plan(&s, caps, arch, false)
    }

    #[test]
    fn run_plan_translate_when_supported_and_non_english() {
        let plan = translate_plan("de", &caps(&["de", "en"], true), "whisper");
        assert_eq!(plan.options.task, Task::Translate);
        assert_eq!(plan.options.target_language.as_deref(), Some("en"));
        assert!(plan.translated);
        // A detected language may be anything, so detection translates too.
        let plan = translate_plan("auto", &caps_with(&["de", "en"], true, true), "whisper");
        assert!(plan.translated);
    }

    #[test]
    fn run_plan_no_translate_when_unsupported() {
        let plan = translate_plan("de", &caps(&["de"], false), "parakeet");
        assert_eq!(plan.options.task, Task::Transcribe);
        assert_eq!(plan.options.target_language, None);
        assert!(!plan.translated);
        // A model that translates, but not into English.
        let mut french_only = caps(&["de", "fr"], true);
        french_only.translate_target_languages = vec!["fr".into()];
        assert!(!translate_plan("de", &french_only, "canary").translated);
        // One that names English as a target, in full locale form.
        let mut to_english = caps(&["de", "en"], true);
        to_english.translate_target_languages = vec!["en-US".into()];
        assert!(translate_plan("de", &to_english, "canary").translated);
    }

    #[test]
    fn run_plan_no_translate_when_source_english() {
        let plan = translate_plan("en", &caps(&["en"], true), "whisper");
        assert_eq!(plan.options.task, Task::Transcribe);
        assert_eq!(plan.options.target_language, None);
        // English reached by fallback is English too.
        let plan = translate_plan("ja", &caps(&["en", "de"], true), "canary");
        assert_eq!(plan.effective_language, "en");
        assert!(!plan.translated);
    }

    #[test]
    fn custom_words_reach_whisper_as_a_prompt_and_others_as_vocabulary() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "en".into();
        s.custom_words = vec!["Kubernetes".into(), "kubectl".into()];
        let english = caps(&["en"], false);

        // Whisper keeps its initial prompt, whether or not it also lists
        // vocabulary, and is never sent both.
        for vocabulary in [false, true] {
            let plan = transcribe_cpp_run_plan(&s, &english, "whisper", vocabulary);
            assert!(matches!(
                plan.options.family,
                Some(transcribe_cpp::RunExtension::Whisper(ref w))
                    if w.initial_prompt.as_deref() == Some("Kubernetes, kubectl")
            ));
            assert!(plan.options.vocabulary.is_empty());
            assert!(plan.words_biased);
        }

        // A model that takes vocabulary gets the words in priority order.
        let plan = transcribe_cpp_run_plan(&s, &english, "parakeet", true);
        assert!(plan.options.family.is_none());
        assert_eq!(plan.options.vocabulary, vec!["Kubernetes", "kubectl"]);
        assert!(plan.words_biased);

        // One that takes neither relies on the fuzzy pass afterwards.
        let plan = transcribe_cpp_run_plan(&s, &english, "parakeet", false);
        assert!(plan.options.family.is_none());
        assert!(plan.options.vocabulary.is_empty());
        assert!(!plan.words_biased);

        // No words: nothing is sent and nothing is skipped.
        s.custom_words.clear();
        let plan = transcribe_cpp_run_plan(&s, &english, "parakeet", true);
        assert!(plan.options.vocabulary.is_empty());
        assert!(!plan.words_biased);
        let plan = transcribe_cpp_run_plan(&s, &english, "whisper", false);
        assert!(plan.options.family.is_none());
        assert!(!plan.words_biased);
    }

    #[test]
    fn flow_recognition_hints_do_not_rewrite_ordinary_words() {
        let mut settings = crate::settings::get_default_settings();
        settings.flow_enabled = true;
        settings.flow_phrase = "Hey Flow".to_string();

        let correction_hints = recognition_words(&settings);
        assert!(correction_hints.iter().any(|hint| hint == "SpeakoFlow"));
        assert!(!correction_hints.iter().any(|hint| hint == "Hey Flow"));
        assert!(!correction_hints.iter().any(|hint| hint == "Flow"));
        assert_eq!(
            apply_custom_words(
                "I said hey flaw yesterday.",
                &correction_hints,
                settings.word_correction_threshold
            ),
            "I said hey flaw yesterday."
        );

        let decoder_hints = recognition_prompt_words(&settings);
        assert!(decoder_hints.iter().any(|hint| hint == "Hey Flow"));
    }

    #[test]
    fn speech_processing_choice_maps_to_a_load_target() {
        use TranscribeAcceleratorSetting::{Auto, Cpu, Gpu};
        let key = r#"["vulkan","id","0000:01:00.0"]"#;

        assert_eq!(load_target(Auto, None, false), LoadTarget::Auto);
        assert_eq!(load_target(Cpu, None, false), LoadTarget::Cpu);
        // A stale key next to Cpu is ignored, never a reason to use the GPU.
        assert_eq!(load_target(Cpu, Some(key), false), LoadTarget::Cpu);
        assert_eq!(
            load_target(Gpu, Some(key), false),
            LoadTarget::Device(key.to_string())
        );
        // `Gpu` without a device is upstream's retired generic GPU: Auto.
        assert_eq!(load_target(Gpu, None, false), LoadTarget::Auto);
        assert_eq!(load_target(Auto, Some(key), false), LoadTarget::Auto);
    }

    /// Handy #1731: the x64 build under emulation on Windows on ARM never
    /// touches the GPU, whatever was chosen, and offers only the processor.
    #[test]
    fn emulated_x64_on_arm64_runs_every_choice_on_the_processor() {
        use TranscribeAcceleratorSetting::{Auto, Cpu, Gpu};
        for (setting, device) in [(Auto, None), (Cpu, None), (Gpu, Some("key"))] {
            assert_eq!(load_target(setting, device, true), LoadTarget::Cpu);
        }
        assert_eq!(available_transcribe_accelerators(true), ["cpu"]);
        assert_eq!(
            available_transcribe_accelerators(false),
            ["auto", "cpu", "gpu"]
        );
    }

    #[test]
    fn only_an_arm64_native_machine_counts_as_emulation() {
        assert!(native_machine_is_arm64(Some(IMAGE_FILE_MACHINE_ARM64)));
        assert!(!native_machine_is_arm64(Some(0x8664))); // AMD64
        assert!(!native_machine_is_arm64(Some(0x014c))); // I386
        assert!(!native_machine_is_arm64(None)); // API missing or failed
    }

    /// The saved identity must not change between versions, or a saved GPU
    /// choice silently turns into Auto. Same encoding as upstream Handy.
    #[test]
    fn device_keys_are_stable_and_prefer_the_hardware_id() {
        assert_eq!(
            device_key("vulkan", Some("0000:01:00.0"), "Vulkan0"),
            r#"["vulkan","id","0000:01:00.0"]"#
        );
        assert_eq!(
            device_key("metal", None, "Metal"),
            r#"["metal","name","Metal"]"#
        );
    }

    #[test]
    fn gpu_kinds_keep_the_names_local_llm_reads() {
        use transcribe_cpp::DeviceType;
        assert_eq!(gpu_kind(DeviceType::Gpu), "dedicated");
        assert_eq!(gpu_kind(DeviceType::Igpu), "integrated");
        assert_eq!(gpu_kind(DeviceType::Unknown), "unknown");
    }

    #[test]
    fn non_finite_samples_become_silence() {
        let mut pcm = vec![0.5, f32::NAN, -0.25, f32::INFINITY, f32::NEG_INFINITY];
        assert_eq!(sanitize_pcm(&mut pcm), 3);
        assert_eq!(pcm, vec![0.5, 0.0, -0.25, 0.0, 0.0]);
        // The gain pass after it can then only ever see finite input.
        let applied = normalize_peak(&mut pcm, 0.95, 8.0);
        assert!(applied.is_finite());
        assert!(pcm.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn a_retired_engine_says_what_to_do() {
        assert_eq!(
            retired_engine_message("Parakeet V3"),
            "Parakeet V3 used an older engine that SpeakoFlow no longer includes. \
             Download its new version on the Models page."
        );
    }
}

/// Live checks against the real engine and real models: the legacy whisper.cpp
/// `.bin`, Parakeet Unified, and Nemotron streaming, on the automatic device and
/// on the processor. Ignored by default because they need the downloaded
/// models, a recording, and (for the GPU cases) a graphics card:
///
/// `cargo test --lib transcribe_cpp_live -- --ignored --nocapture --test-threads=1`
///
/// Paths default to the installed app's data folder and can be overridden with
/// `SPEAKOFLOW_MODELS_DIR`, `SPEAKOFLOW_RECORDINGS_DIR` or `SPEAKOFLOW_LIVE_WAV`.
/// Transcript text is never printed, only its length.
#[cfg(test)]
mod transcribe_cpp_live_tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    const SAMPLE_RATE: usize = 16_000;
    /// The slice of the recording each test transcribes.
    const MAX_SECONDS: usize = 20;
    /// A recording shorter than this says too little to check anything, so the
    /// newest recording at least this long is used.
    const MIN_SECONDS: usize = 3;

    fn app_data() -> PathBuf {
        let roaming = std::env::var_os("APPDATA").expect("APPDATA is set on Windows");
        PathBuf::from(roaming).join("com.abhishekbarali.speakoflow")
    }

    fn models_dir() -> PathBuf {
        std::env::var_os("SPEAKOFLOW_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| app_data().join("models"))
    }

    fn read_wav(path: &Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).expect("open the recording");
        let spec = reader.spec();
        assert_eq!(
            spec.sample_rate as usize, SAMPLE_RATE,
            "recordings are 16 kHz"
        );
        let channels = spec.channels.max(1) as usize;
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|s| s.expect("sample") as f32 / (1i64 << (spec.bits_per_sample - 1)) as f32)
                .collect(),
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|s| s.expect("sample"))
                .collect(),
        };
        // Keep the first channel; recordings are mono.
        samples.into_iter().step_by(channels).collect()
    }

    /// The newest recording with at least `MIN_SECONDS` of audio (or the one
    /// `SPEAKOFLOW_LIVE_WAV` names), cut to `MAX_SECONDS`.
    fn recording() -> Vec<f32> {
        if let Some(path) = std::env::var_os("SPEAKOFLOW_LIVE_WAV") {
            let mut pcm = read_wav(Path::new(&path));
            pcm.truncate(MAX_SECONDS * SAMPLE_RATE);
            return pcm;
        }
        let dir = std::env::var_os("SPEAKOFLOW_RECORDINGS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| app_data().join("recordings"));
        let mut wavs: Vec<_> = std::fs::read_dir(&dir)
            .expect("read the recordings folder")
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        wavs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, path) in wavs {
            let mut pcm = read_wav(&path);
            if pcm.len() >= MIN_SECONDS * SAMPLE_RATE {
                pcm.truncate(MAX_SECONDS * SAMPLE_RATE);
                eprintln!(
                    "recording: {} ({:.1} s used)",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    pcm.len() as f64 / SAMPLE_RATE as f64
                );
                return pcm;
            }
        }
        panic!(
            "no recording of at least {MIN_SECONDS} s in {}",
            dir.display()
        );
    }

    fn settings_for(accelerator: TranscribeAcceleratorSetting) -> crate::settings::AppSettings {
        let mut settings = crate::settings::get_default_settings();
        settings.transcribe_accelerator = accelerator;
        settings.transcribe_gpu_device = None;
        settings.selected_language = "auto".into();
        settings.translate_to_english = false;
        settings
    }

    /// Load through the app's own load options, report where it landed, and
    /// check the automatic choice took a dedicated GPU when there is one.
    fn load(file: &str, accelerator: TranscribeAcceleratorSetting) -> transcribe_cpp::Session {
        load_path(&models_dir().join(file), accelerator)
    }

    fn load_path(
        path: &Path,
        accelerator: TranscribeAcceleratorSetting,
    ) -> transcribe_cpp::Session {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        assert!(path.exists(), "missing model {}", path.display());
        let options = transcribe_cpp_load_options(&settings_for(accelerator));
        let started = Instant::now();
        let model = transcribe_cpp::Model::load_with(path, &options).expect("model load");
        let device = model.device().expect("a loaded model has a device");
        eprintln!(
            "{file}: loaded in {} ms on {} [{}] (arch={}, backend={})",
            started.elapsed().as_millis(),
            device_label(&device),
            device_type_name(device.device_type),
            model.arch(),
            model.backend()
        );

        let gpus: Vec<_> = transcribe_compute_devices()
            .into_iter()
            .filter(is_gpu_device)
            .collect();
        match accelerator {
            TranscribeAcceleratorSetting::Cpu => assert_eq!(
                device.device_type,
                transcribe_cpp::DeviceType::Cpu,
                "Processor (CPU) must not use a GPU"
            ),
            _ if gpus
                .iter()
                .any(|g| g.device_type == transcribe_cpp::DeviceType::Gpu) =>
            {
                assert_eq!(
                    device.device_type,
                    transcribe_cpp::DeviceType::Gpu,
                    "Auto must take the dedicated GPU when one is present"
                );
            }
            _ => {}
        }

        let threads = transcribe_cpp_session_threads(&model.arch(), is_gpu_device(&device));
        model
            .session_with(&transcribe_cpp::SessionOptions {
                n_threads: threads,
                ..Default::default()
            })
            .expect("session")
    }

    fn batch(file: &str, accelerator: TranscribeAcceleratorSetting) {
        init_transcribe_cpp();
        let pcm = recording();
        let mut session = load(file, accelerator);
        let model = session.model();
        let caps = model.capabilities().expect("an ASR model has capabilities");
        let plan = transcribe_cpp_run_plan(
            &settings_for(accelerator),
            &caps,
            &model.arch(),
            model.supports(transcribe_cpp::Feature::Vocabulary),
        )
        .options;
        // Twice: the first run on a GPU includes compiling its shaders, the
        // second is what every later dictation costs.
        for pass in ["first", "second"] {
            let started = Instant::now();
            let text = session.run(&pcm, &plan).expect("transcription").text;
            eprintln!(
                "{file}: {pass} batch of {:.1} s took {} ms, {} chars",
                pcm.len() as f64 / SAMPLE_RATE as f64,
                started.elapsed().as_millis(),
                text.chars().count()
            );
            assert!(!text.trim().is_empty(), "{file} heard nothing");
        }
    }

    #[test]
    #[ignore = "needs the downloaded models, a recording and the engine's backends"]
    fn transcribe_cpp_live_legacy_whisper_bin() {
        batch(
            "whisper-medium-q4_1.bin",
            TranscribeAcceleratorSetting::Auto,
        );
    }

    #[test]
    #[ignore = "needs the downloaded models, a recording and the engine's backends"]
    fn transcribe_cpp_live_parakeet_unified() {
        batch(
            "parakeet-unified-en-0.6b-Q8_0.gguf",
            TranscribeAcceleratorSetting::Auto,
        );
    }

    #[test]
    #[ignore = "needs the downloaded models, a recording and the engine's backends"]
    fn transcribe_cpp_live_nemotron_on_the_processor() {
        batch(
            "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf",
            TranscribeAcceleratorSetting::Cpu,
        );
    }

    /// Nemotron through the streaming API, fed the way `run_stream_worker` feeds
    /// it: 30 ms frames gathered into 80 ms chunks, then finalize.
    #[test]
    #[ignore = "needs the downloaded models, a recording and the engine's backends"]
    fn transcribe_cpp_live_nemotron_streaming() {
        init_transcribe_cpp();
        let file = "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf";
        let pcm = recording();
        let mut session = load(file, TranscribeAcceleratorSetting::Auto);
        let model = session.model();
        let caps = model.capabilities().expect("an ASR model has capabilities");
        assert!(caps.supports_streaming, "{file} should stream");
        let settings = settings_for(TranscribeAcceleratorSetting::Auto);
        let plan = transcribe_cpp_run_plan(&settings, &caps, &model.arch(), false).options;
        let stream_opts = transcribe_cpp::StreamOptions {
            commit_policy: transcribe_cpp::CommitPolicy::Auto,
            ..Default::default()
        };

        let started = Instant::now();
        let mut stream = session.stream(&plan, &stream_opts).expect("stream begin");
        const FRAME: usize = 480;
        const FEED_SAMPLES: usize = 1280;
        let mut acc = Vec::with_capacity(FEED_SAMPLES * 2);
        let mut updates = 0usize;
        for frame in pcm.chunks(FRAME) {
            acc.extend_from_slice(frame);
            while acc.len() >= FEED_SAMPLES {
                let chunk: Vec<f32> = acc.drain(..FEED_SAMPLES).collect();
                let update = stream.feed(&chunk).expect("stream feed");
                if update.committed_changed || update.tentative_changed {
                    updates += 1;
                }
            }
        }
        if !acc.is_empty() {
            stream.feed(&acc).expect("tail feed");
        }
        let fed = started.elapsed();
        stream.finalize().expect("finalize");
        let text = stream.text().display();
        eprintln!(
            "{file}: streamed {:.1} s in {} ms (+{} ms finalize), {updates} updates, {} chars",
            pcm.len() as f64 / SAMPLE_RATE as f64,
            fed.as_millis(),
            (started.elapsed() - fed).as_millis(),
            text.chars().count()
        );
        assert!(!text.trim().is_empty(), "{file} streamed nothing");

        // The finished stream must hand the model back: a cancelled stream
        // (the worker's Cancel arm) and then a batch run must both work on the
        // same session, or the next dictation would find the model busy.
        drop(stream);
        {
            let mut cancelled = session
                .stream(&plan, &stream_opts)
                .expect("a second stream after finalize");
            cancelled
                .feed(&pcm[..pcm.len().min(FEED_SAMPLES)])
                .expect("feed");
            cancelled.reset();
        }
        let again = session.run(&pcm, &plan).expect("batch after a cancel");
        assert!(
            !again.text.trim().is_empty(),
            "session unusable after reset"
        );
    }

    /// A saved device key must load on exactly that GPU, including the
    /// integrated one Auto passes over.
    #[test]
    #[ignore = "needs the downloaded models and a GPU"]
    fn transcribe_cpp_live_saved_device_key_selects_that_gpu() {
        init_transcribe_cpp();
        let path = models_dir().join("parakeet-unified-en-0.6b-Q8_0.gguf");
        for gpu in enumerate_gpu_devices() {
            let mut settings = settings_for(TranscribeAcceleratorSetting::Gpu);
            settings.transcribe_gpu_device = Some(gpu.id.clone());
            let options = transcribe_cpp_load_options(&settings);
            assert!(
                options.device.is_some(),
                "{} was not found by its key",
                gpu.name
            );
            let model = transcribe_cpp::Model::load_with(&path, &options).expect("load");
            let device = model.device().expect("device");
            eprintln!(
                "key for {} [{}] loaded on {} [{}]",
                gpu.name,
                gpu.kind,
                device_label(&device),
                device_type_name(device.device_type)
            );
            assert_eq!(device_key_of(&device), gpu.id);
        }
    }

    /// The GGUF Parakeet TDT 0.6B v3 (the catalog's default Q8_0 quant), where
    /// the app downloads it; `SPEAKOFLOW_PARAKEET_TDT` points elsewhere.
    fn parakeet_tdt_path() -> PathBuf {
        std::env::var_os("SPEAKOFLOW_PARAKEET_TDT")
            .map(PathBuf::from)
            .unwrap_or_else(|| models_dir().join("parakeet-tdt-0.6b-v3-Q8_0.gguf"))
    }

    /// A long recording, whole: `SPEAKOFLOW_LONG_WAV`, else the longest one in
    /// the app's recordings folder.
    fn long_recording() -> Vec<f32> {
        let path = std::env::var_os("SPEAKOFLOW_LONG_WAV")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::fs::read_dir(app_data().join("recordings"))
                    .expect("the app's recordings folder")
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
                    .max_by_key(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
                    .map(|e| e.path())
                    .expect("at least one recording")
            });
        let pcm = read_wav(&path);
        eprintln!(
            "long recording: {:.1} s",
            pcm.len() as f64 / SAMPLE_RATE as f64
        );
        pcm
    }

    fn bundled_vad() -> Option<PathBuf> {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/models/silero_vad_v4.onnx");
        path.exists().then_some(path)
    }

    /// (a) The regression target: a nine-minute recording on the GGUF Parakeet
    /// TDT v3, cut at pauses, decoded chunk by chunk as `transcribe()` does.
    #[test]
    #[ignore = "needs the GGUF Parakeet TDT v3, the long recording and the engine's backends"]
    fn transcribe_cpp_live_long_recording_in_chunks() {
        init_transcribe_cpp();
        let mut pcm = long_recording();
        normalize_peak(&mut pcm, 0.95, 8.0);
        let mut session = load_path(&parakeet_tdt_path(), TranscribeAcceleratorSetting::Auto);
        let model = session.model();
        let caps = model.capabilities().expect("capabilities");
        let traits = ModelTraits::of(&model, &caps);
        eprintln!(
            "traits: {:?}, max_audio_ms={}, vocabulary={}, cancellation={}, detects language={}",
            traits,
            caps.max_audio_ms,
            model.supports(transcribe_cpp::Feature::Vocabulary),
            model.supports(transcribe_cpp::Feature::Cancellation),
            caps.supports_language_detect
        );
        assert_eq!(batch_decode(traits, true), BatchDecode::Chunked);
        let plan = transcribe_cpp_run_plan(
            &settings_for(TranscribeAcceleratorSetting::Auto),
            &caps,
            &model.arch(),
            false,
        );

        let started = Instant::now();
        let mut cutter = Chunker::new(silero_scorer_factory(bundled_vad()));
        let mut cut = cutter.push(pcm.clone());
        cut.extend(cutter.finish());
        let longest = cut.iter().map(Vec::len).max().unwrap_or(0);
        let shortest = cut.iter().map(Vec::len).min().unwrap_or(0);
        eprintln!(
            "cut alone: {} chunks in {} ms ({:.1}-{:.1} s each)",
            cut.len(),
            started.elapsed().as_millis(),
            shortest as f64 / SAMPLE_RATE as f64,
            longest as f64 / SAMPLE_RATE as f64
        );
        assert!(longest <= crate::audio_toolkit::chunking::MAX_CHUNK_SAMPLES);
        drop(cut);

        let started = Instant::now();
        let options = plan.options.clone();
        let mut decode = |chunk: Vec<f32>| run_once(&mut session, &chunk, &options);
        let decoded = chunked::decode_clip(&mut decode, pcm, silero_scorer_factory(bundled_vad()))
            .expect("a chunked long recording transcribes");
        eprintln!(
            "chunked: {} chunks in {} ms, {} chars",
            decoded.chunks,
            started.elapsed().as_millis(),
            decoded.text.chars().count()
        );
        assert!(decoded.chunks >= 10, "{} chunks", decoded.chunks);
        assert!(!decoded.text.trim().is_empty());
    }

    /// (a, live) The same recording fed the way the recorder feeds a chunked
    /// session, 30 ms at a time: how much is decoded before the stop, and
    /// how long the stop then waits for the tail.
    #[test]
    #[ignore = "needs the GGUF Parakeet TDT v3, the long recording and the engine's backends"]
    fn transcribe_cpp_live_long_recording_decoded_while_recording() {
        init_transcribe_cpp();
        let pcm = long_recording();
        let mut session = load_path(&parakeet_tdt_path(), TranscribeAcceleratorSetting::Auto);
        let model = session.model();
        let caps = model.capabilities().expect("capabilities");
        let options = transcribe_cpp_run_plan(
            &settings_for(TranscribeAcceleratorSetting::Auto),
            &caps,
            &model.arch(),
            false,
        )
        .options;
        let mut decode = |mut chunk: Vec<f32>| {
            normalize_peak(&mut chunk, 0.95, 8.0);
            run_once(&mut session, &chunk, &options)
        };
        let mut live = chunked::ChunkSession::new(silero_scorer_factory(bundled_vad()));
        let started = Instant::now();
        for frame in pcm.chunks(480) {
            live.push(frame.to_vec(), &mut decode);
        }
        let fed = started.elapsed();
        let while_recording = live.decoded();
        let stopped = Instant::now();
        let text = live
            .finish(pcm.len(), &mut decode)
            .expect("a live long-form result");
        eprintln!(
            "live: {while_recording} chunks decoded while feeding ({} ms), tail in {} ms after the stop, {} chars",
            fed.as_millis(),
            stopped.elapsed().as_millis(),
            text.chars().count()
        );
        assert!(while_recording >= 10);
    }

    /// (b) The same recording in one run, as with "Split long recordings" off.
    /// Reports what happens rather than asserting it.
    #[test]
    #[ignore = "needs the GGUF Parakeet TDT v3, the long recording and the engine's backends"]
    fn transcribe_cpp_live_long_recording_in_one_run() {
        init_transcribe_cpp();
        let mut pcm = long_recording();
        normalize_peak(&mut pcm, 0.95, 8.0);
        let mut session = load_path(&parakeet_tdt_path(), TranscribeAcceleratorSetting::Auto);
        let model = session.model();
        let caps = model.capabilities().expect("capabilities");
        let plan = transcribe_cpp_run_plan(
            &settings_for(TranscribeAcceleratorSetting::Auto),
            &caps,
            &model.arch(),
            false,
        );
        let started = Instant::now();
        match session.run(&pcm, &plan.options) {
            Ok(transcript) => eprintln!(
                "one run: ok in {} ms, {} chars, truncated={}",
                started.elapsed().as_millis(),
                transcript.text.chars().count(),
                session.was_truncated()
            ),
            Err(e) => eprintln!(
                "one run: failed after {} ms: {}",
                started.elapsed().as_millis(),
                e
            ),
        }
    }

    /// (c) Nemotron streaming with a language it does not list and cannot
    /// detect, and one it lists: the code the engine is handed.
    #[test]
    #[ignore = "needs the downloaded models, a recording and the engine's backends"]
    fn transcribe_cpp_live_nemotron_streams_the_effective_language() {
        init_transcribe_cpp();
        let file = "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf";
        let pcm = recording();
        let mut session = load(file, TranscribeAcceleratorSetting::Auto);
        let model = session.model();
        let caps = model.capabilities().expect("capabilities");
        eprintln!(
            "{file}: {} languages, detects={}, streams={}, vocabulary={}",
            caps.languages.len(),
            caps.supports_language_detect,
            caps.supports_streaming,
            model.supports(transcribe_cpp::Feature::Vocabulary)
        );
        for intent in ["de", "no", "zh-Hant", "ko", "auto"] {
            let mut settings = settings_for(TranscribeAcceleratorSetting::Auto);
            settings.selected_language = intent.into();
            let plan = transcribe_cpp_run_plan(&settings, &caps, &model.arch(), false);
            eprintln!(
                "intent {intent}: effective {}, engine {:?}",
                plan.effective_language, plan.options.language
            );
            if !caps.supports_language_detect && !caps.languages.is_empty() {
                assert!(
                    plan.options.language.is_some(),
                    "never auto without detection"
                );
            }
            if let Some(code) = &plan.options.language {
                assert!(
                    caps.languages.contains(code),
                    "{code} is the model's own code"
                );
            }
            let mut stream = session
                .stream(&plan.options, &transcribe_cpp::StreamOptions::default())
                .expect("stream begin with the effective language");
            for chunk in pcm.chunks(1280) {
                stream.feed(chunk).expect("feed");
            }
            stream.finalize().expect("finalize");
            eprintln!(
                "intent {intent}: streamed {} chars",
                stream.text().display().chars().count()
            );
        }
    }

    /// (d) A Whisper `.bin` with custom words and no vocabulary: the words go
    /// in as the initial prompt and the run still works.
    #[test]
    #[ignore = "needs the downloaded models, a recording and the engine's backends"]
    fn transcribe_cpp_live_whisper_bin_keeps_its_initial_prompt() {
        init_transcribe_cpp();
        let file = "whisper-medium-q4_1.bin";
        let pcm = recording();
        let mut session = load(file, TranscribeAcceleratorSetting::Auto);
        let model = session.model();
        let caps = model.capabilities().expect("capabilities");
        let mut settings = settings_for(TranscribeAcceleratorSetting::Auto);
        settings.custom_words = vec!["SpeakoFlow".into(), "Kubernetes".into()];
        let vocabulary = model.supports(transcribe_cpp::Feature::Vocabulary);
        let plan = transcribe_cpp_run_plan(&settings, &caps, &model.arch(), vocabulary);
        eprintln!(
            "{file}: arch={}, supports vocabulary={}, initial prompt={}",
            model.arch(),
            vocabulary,
            model.supports(transcribe_cpp::Feature::InitialPrompt)
        );
        assert!(plan.options.vocabulary.is_empty());
        assert!(matches!(
            plan.options.family,
            Some(transcribe_cpp::RunExtension::Whisper(ref w)) if w.initial_prompt.is_some()
        ));
        assert!(plan.words_biased);
        let started = Instant::now();
        let text = session
            .run(&pcm, &plan.options)
            .expect("run with a prompt")
            .text;
        eprintln!(
            "{file}: prompted run of {:.1} s in {} ms, {} chars",
            pcm.len() as f64 / SAMPLE_RATE as f64,
            started.elapsed().as_millis(),
            text.chars().count()
        );
        assert!(!text.trim().is_empty());
    }
}
