//! Parent side: [`EngineSupervisor`], the single owner of the transcribe.cpp
//! worker process.
//!
//! One owner thread works through a command queue and holds the only worker
//! slot, the loaded model's spec and any active stream, so "at most one model
//! worker alive" holds by construction. Callers' commands wait their turn;
//! waiting in the queue (behind a model load, say) is never a timeout. Device
//! probes bypass the queue: a probe runs its own short-lived, model-less
//! worker on the caller's thread, so a hung GPU driver can't hold dictation up
//! behind it.
//!
//! A batch run that arrives while a live stream is open waits for the stream
//! to end instead of ending it (upstream Handy ends it). SpeakoFlow can
//! transcribe a meeting chunk while the user dictates, and ending the
//! dictation's stream for it would throw the live transcript away. The old
//! in-process engine made the same choice by leasing the engine to the stream.
//!
//! A worker that will only use the CPU is started CPU-only, so it never
//! registers a GPU backend. One rule decides: the spec it loads asks for the
//! processor (the user's choice, or a fallback after a GPU failure), or the
//! host must not use the GPU at all.

use super::protocol::{
    encode_message, read_message, DeviceInfo, LoadTarget, LoadedInfo, Request, Response,
};
use super::worker::LOG_LINE_PREFIX;
use super::{CPU_ONLY_FLAG, LOG_LEVEL_ENV, WORKER_FLAG};
use log::{debug, error, info, warn, Level};
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command as ProcessCommand, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use transcribe_cpp::{
    CancelToken, ErrorKind, RunOptions, StreamOptions, StreamText, StreamUpdate, Transcript,
};

/// Spawn, backend init and device listing. Listing first opens the GPU, which
/// on macOS compiles ggml's Metal library when the system shader cache does
/// not hold it yet (~15 s after an install or update).
const HELLO_TIMEOUT: Duration = Duration::from_secs(60);
/// Model loads stay generous: a slow load is slow, not hung (Handy #1841).
const LOAD_TIMEOUT: Duration = Duration::from_secs(180);
/// Floor for stream calls that do no decoding (begin, reset).
const CALL_FLOOR: Duration = Duration::from_secs(30);
/// How long a worker gets to exit after its stdin closes before it's killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// How long a cancelled call waits for transcribe.cpp's abort to take effect
/// before the worker is stopped instead. Abort is checked between decode
/// steps, which on every family measured here is a few tens of milliseconds;
/// this only bounds a model that never checks.
const CANCEL_GRACE: Duration = Duration::from_millis(750);
/// How often a call waiting on the worker looks at its cancel token.
const CANCEL_POLL: Duration = Duration::from_millis(20);
/// How often the owner looks at batch runs waiting for a stream to end, so a
/// cancelled one is answered without waiting for the stream.
const DEFERRED_SWEEP: Duration = Duration::from_millis(100);
/// Feeds of one stream allowed in the queue at once. The app's 80 ms feeds
/// make this about 0.6 s of audio: enough to keep the worker busy, and it
/// leaves any further backlog in the live router, whose bound is what keeps a
/// stream that can't keep up from growing memory for the whole recording.
const FEED_WINDOW: usize = 8;
/// Native stderr lines kept for crash reports (a `GGML_ASSERT` message lands
/// here right before an abort).
const STDERR_TAIL_LINES: usize = 64;
const SAMPLE_RATE: f64 = 16_000.0;

/// How long decoding work may take before its worker counts as hung, by the
/// device the model runs on. On a GPU a hang is recovered from (retried on
/// CPU), so it should be caught soon. On CPU a hang is never retried, so the
/// deadline only decides when to give up: a slow machine must not be mistaken
/// for a hung one.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Deadlines {
    /// Floor for a stream feed or finalize, so sub-second chunks and
    /// first-use shader compiles are not mistaken for hangs.
    call_floor: Duration,
    /// Floor for a batch run. A run has a fixed cost however short the clip:
    /// Whisper always encodes a full 30 s window, which on CPU takes seconds
    /// even for a 1 s clip.
    run_floor: Duration,
    /// Work on N seconds of audio may take up to this many times N.
    audio_factor: f64,
}

const GPU_DEADLINES: Deadlines = Deadlines {
    call_floor: Duration::from_secs(30),
    run_floor: Duration::from_secs(60),
    audio_factor: 10.0,
};

/// A single CPU feed or finalize can take well over 10 s, and a large model on
/// an older CPU runs slower than real time.
const CPU_DEADLINES: Deadlines = Deadlines {
    call_floor: Duration::from_secs(120),
    run_floor: Duration::from_secs(300),
    audio_factor: 20.0,
};

impl Deadlines {
    fn for_device(on_gpu: bool) -> Self {
        if on_gpu {
            GPU_DEADLINES
        } else {
            CPU_DEADLINES
        }
    }

    /// Deadline for stream work on `samples` of 16 kHz audio.
    fn audio(&self, samples: usize) -> Duration {
        self.call_floor.max(Duration::from_secs_f64(
            samples as f64 / SAMPLE_RATE * self.audio_factor,
        ))
    }

    /// Deadline for a batch run on `samples` of 16 kHz audio.
    fn run(&self, samples: usize) -> Duration {
        self.run_floor.max(self.audio(samples))
    }
}

fn ms_to_samples(ms: i64) -> usize {
    (ms.max(0) as usize) * (SAMPLE_RATE as usize / 1000)
}

/// What to do after a worker was lost (crashed or hung) in the middle of a
/// batch run. Pure, so the policy is tested without a process.
///
/// One retry, in a fresh worker. If the lost worker was on a GPU the GPU is
/// marked unavailable first, so the retry runs CPU-only: a driver fault that
/// recurs on every run then costs one slower dictation instead of every one.
/// A hang on the CPU is not retried: the deadline there is already generous,
/// so the same audio would very likely hang again, and the user has waited
/// long enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterLoss {
    Retry,
    GiveUp,
}

fn after_lost_run(hung: bool, on_gpu: bool, already_retried: bool) -> AfterLoss {
    if already_retried || (hung && !on_gpu) {
        AfterLoss::GiveUp
    } else {
        AfterLoss::Retry
    }
}

/// What a call waiting on its worker does next. Pure, for the same reason as
/// [`after_lost_run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitStep {
    /// Keep waiting, at most this long, then decide again.
    Wait(Duration),
    /// Ask transcribe.cpp to abort the request.
    SendCancel,
    /// Stop the worker: the call was cancelled and the model did not abort
    /// in time (or cannot abort at all).
    KillCancelled,
    /// Stop the worker: it missed its deadline.
    KillHung,
}

/// `elapsed`: since the request was sent. `cancel_sent`: how long ago the
/// abort was requested, if it was.
fn wait_step(
    elapsed: Duration,
    deadline: Duration,
    cancelled: bool,
    cancel_sent: Option<Duration>,
    can_abort: bool,
) -> WaitStep {
    if let Some(since) = cancel_sent {
        return if since >= CANCEL_GRACE {
            WaitStep::KillCancelled
        } else {
            WaitStep::Wait((CANCEL_GRACE - since).min(CANCEL_POLL))
        };
    }
    if cancelled {
        return if can_abort {
            WaitStep::SendCancel
        } else {
            WaitStep::KillCancelled
        };
    }
    if elapsed >= deadline {
        return WaitStep::KillHung;
    }
    WaitStep::Wait((deadline - elapsed).min(CANCEL_POLL))
}

/// Whether a worker for `target` registers only the CPU backends.
fn cpu_only_worker(target: &LoadTarget, gpu_allowed: bool) -> bool {
    *target == LoadTarget::Cpu || !gpu_allowed
}

/// The target a fresh worker should load: the processor while the GPU is
/// marked unavailable, otherwise what was asked for.
fn effective_target(target: &LoadTarget, gpu_unavailable: bool) -> LoadTarget {
    if gpu_unavailable {
        LoadTarget::Cpu
    } else {
        target.clone()
    }
}

/// What to load. The supervisor keeps it so a dead worker can be replaced
/// transparently.
#[derive(Debug, Clone)]
pub struct LoadSpec {
    pub path: PathBuf,
    pub target: LoadTarget,
}

impl LoadSpec {
    fn on_cpu(&self) -> Self {
        Self {
            path: self.path.clone(),
            target: LoadTarget::Cpu,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// The caller's cancel token stopped the work.
    Cancelled,
    /// The audio is shorter than the model can decode: nothing was said.
    InputTooShort(String),
    /// The file loaded but cannot transcribe (no ASR role).
    NotSpeechModel(String),
    /// No model, a transcribe.cpp error, or a worker failure that could not
    /// be recovered.
    Failed(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Cancelled => f.write_str("transcription was cancelled"),
            EngineError::InputTooShort(reason) => write!(f, "input too short: {reason}"),
            EngineError::NotSpeechModel(reason) => {
                write!(f, "not a speech-to-text model ({reason})")
            }
            EngineError::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for EngineError {}

/// The result of one stream feed, handed to the stream's callback.
pub struct StreamProgress {
    pub update: StreamUpdate,
    /// Present only when the committed or tentative text changed.
    pub text: Option<StreamText>,
    /// Time from sending the frame to the worker until its answer.
    pub elapsed: Duration,
}

pub struct Finalized {
    /// `None` when transcribe.cpp failed the finalize; `text` is then what the
    /// stream had committed before it.
    pub update: Option<StreamUpdate>,
    pub text: StreamText,
    /// The model's detected language, when asked for.
    pub language: Option<String>,
    pub elapsed: Duration,
}

type Reply<T> = mpsc::Sender<T>;
type OnProgress = Box<dyn FnMut(StreamProgress) + Send>;

enum Command {
    Load(LoadSpec, Reply<Result<LoadedInfo, EngineError>>),
    /// `now` skips the wait for a live stream (see `Owner::handle`); only
    /// quitting the app wants that.
    Unload {
        reply: Reply<()>,
        now: bool,
    },
    Transcribe {
        pcm: Vec<f32>,
        run: RunOptions,
        cancel: Option<CancelToken>,
        reply: Reply<Result<Transcript, EngineError>>,
    },
    StreamBegin {
        active: ActiveStream,
        run: RunOptions,
        stream: StreamOptions,
        reply: Reply<Result<(), EngineError>>,
    },
    Feed {
        id: u64,
        pcm: Vec<f32>,
        window: Arc<FeedWindow>,
    },
    Finalize {
        id: u64,
        want_language: bool,
        reply: Reply<Result<Option<Finalized>, EngineError>>,
    },
    StreamReset {
        id: u64,
    },
}

/// State readable without going through the queue.
#[derive(Default)]
struct Shared {
    /// Whether this host may use a GPU at all. When not, every worker is
    /// CPU-only, device probes included.
    gpu_allowed: bool,
    /// A worker with GPU backends crashed or hung (in backend init, device
    /// listing, model load or inference). Models load in CPU-only workers, and
    /// devices are not listed again, until [`EngineSupervisor::retry_gpu`].
    gpu_unavailable: AtomicBool,
    /// The loaded model (logically: it stays loaded while its worker is
    /// replaced after a crash or a cancel that had to stop it).
    loaded: Mutex<Option<LoadedInfo>>,
    /// The compute devices as last listed by a worker; `None` until one has.
    devices: Mutex<Option<Vec<DeviceInfo>>>,
    /// Held for the length of a device probe, so concurrent callers wait for
    /// the one probe instead of starting their own.
    probing: Mutex<()>,
    /// A model-less device probe failed without crashing (the worker could
    /// not be started, or refused). Not retried on every call: the GPU list is
    /// read several times per llama.cpp start and by every settings screen,
    /// and each call would start (and wait for) another worker that fails the
    /// same way. A model worker's hello still refreshes the list, and
    /// [`EngineSupervisor::retry_gpu`] allows one more probe.
    probe_failed: AtomicBool,
    /// Bumped by every [`EngineSupervisor::unload`] as it is called, so the
    /// model reads as unloaded at once, even while the unload waits its turn.
    unloads_requested: AtomicU64,
    /// Request ids, unique for the supervisor's lifetime, so a late cancel
    /// can never reach a newer request.
    next_id: AtomicU64,
}

impl Shared {
    /// Start a model-less worker just to list devices. If it may use the GPU
    /// and crashes or hangs doing so, the GPU is marked unavailable.
    fn probe_devices(&self) {
        let cpu_only = !self.gpu_allowed;
        match self.hello(cpu_only, true) {
            Ok(worker) => drop(worker),
            Err(failure) => {
                error!(
                    "Could not start a transcription worker to list compute devices: {}",
                    failure
                );
                self.probe_failed.store(true, Ordering::Release);
                if failure.worker_lost() && !cpu_only {
                    self.mark_gpu_unavailable(format!(
                        "a worker with GPU backends {failure} while listing compute devices"
                    ));
                }
            }
        }
    }

    /// Start a worker and say hello, publishing its device list if it lists
    /// one.
    fn hello(&self, cpu_only: bool, list_devices: bool) -> Result<Worker, Failure> {
        let mut worker = Worker::spawn(cpu_only).map_err(Failure::Spawn)?;
        match worker.call(&Request::Hello { list_devices }, None, HELLO_TIMEOUT, None)? {
            Response::Hello { devices } => {
                if let Some(devices) = devices {
                    self.set_devices(devices);
                }
                Ok(worker)
            }
            other => Err(unexpected(&other)),
        }
    }

    fn set_devices(&self, new: Vec<DeviceInfo>) {
        let mut current = lock(&self.devices);
        if let Some(old) = &*current {
            let identity = |d: &DeviceInfo| (d.key.clone(), d.index);
            if !old.iter().map(identity).eq(new.iter().map(identity)) {
                info!(
                    "transcribe.cpp compute devices changed: [{}]",
                    new.iter()
                        .map(|d| format!("{} ({})", d.label(), d.kind))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        *current = Some(new);
    }

    fn gpu_unavailable(&self) -> bool {
        self.gpu_unavailable.load(Ordering::Acquire)
    }

    /// `reason` is logged when this changes the state.
    fn mark_gpu_unavailable(&self, reason: impl fmt::Display) {
        if !self.gpu_unavailable.swap(true, Ordering::AcqRel) {
            warn!("Transcription GPU marked unavailable: {}", reason);
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Owns the one transcribe.cpp worker slot. Cheap to clone; every clone
/// talks to the same owner thread.
#[derive(Clone)]
pub struct EngineSupervisor {
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
}

impl EngineSupervisor {
    /// Start the owner thread. No worker runs until something needs one.
    /// Without `gpu_allowed`, every worker is CPU-only, so no GPU driver is
    /// ever loaded.
    pub fn new(gpu_allowed: bool) -> Self {
        #[cfg(all(unix, not(test)))]
        record_launched_exe();
        let (commands, queue) = mpsc::channel();
        let shared = Arc::new(Shared {
            gpu_allowed,
            ..Shared::default()
        });
        let owner = Owner {
            shared: Arc::clone(&shared),
            worker: None,
            spec: None,
            stream: None,
            unloads_done: 0,
            deferred: VecDeque::new(),
        };
        // Spawning a thread only fails when the OS is out of threads, and then
        // nothing in the app works; a supervisor without its owner thread
        // answers every call with "the transcription engine has stopped",
        // which keeps the app (and cloud transcription) running.
        if let Err(e) = thread::Builder::new()
            .name("transcribe-engine".into())
            .spawn(move || owner.run(queue))
        {
            error!("Failed to start the transcription engine thread: {}", e);
        }
        Self { commands, shared }
    }

    fn ask<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(command(reply)).ok()?;
        answer.recv().ok()
    }

    /// Load a model, replacing the current one. The old worker has fully
    /// exited before the new one starts, so two models are never held at once.
    pub fn load(&self, spec: LoadSpec) -> Result<LoadedInfo, EngineError> {
        self.ask(|reply| Command::Load(spec, reply))
            .unwrap_or_else(|| Err(engine_stopped()))
    }

    /// Unload the model, without waiting. It reads as unloaded at once and the
    /// unload is queued ahead of anything requested later, so a new dictation
    /// queues a fresh load behind it and is never affected by it.
    /// [`Unloading::wait`] returns once the worker has exited.
    ///
    /// A live stream (a dictation still recording) keeps the model until it
    /// ends: the unload waits for it, the way a batch run does.
    pub fn unload(&self) -> Unloading {
        self.send_unload(false)
    }

    /// [`unload`](Self::unload), without waiting for a live stream to end. For
    /// quitting, where the dictation is going away with the app anyway and the
    /// worker must be gone before an update replaces the executable.
    pub fn unload_now(&self) -> Unloading {
        self.send_unload(true)
    }

    fn send_unload(&self, now: bool) -> Unloading {
        let (reply, exited) = mpsc::channel();
        {
            let mut loaded = lock(&self.shared.loaded);
            self.shared.unloads_requested.fetch_add(1, Ordering::SeqCst);
            *loaded = None;
            if self.commands.send(Command::Unload { reply, now }).is_err() {
                return Unloading(None);
            }
        }
        Unloading(Some(exited))
    }

    /// An explicit speech-processing or GPU change: forget the GPU failure, if
    /// any, so the next load may use (and list) the GPU again. A new failure
    /// sets the flag again.
    pub fn retry_gpu(&self, reason: &str) {
        self.shared.probe_failed.store(false, Ordering::Release);
        if self.shared.gpu_unavailable.swap(false, Ordering::AcqRel) {
            info!("Transcription GPU no longer marked unavailable: {}", reason);
        }
    }

    /// The loaded model. A snapshot; never waits on the queue.
    pub fn loaded(&self) -> Option<LoadedInfo> {
        lock(&self.shared.loaded).clone()
    }

    /// Transcribe 16 kHz mono PCM with the loaded model. A worker crash or
    /// hang is retried once in a fresh worker (on CPU if it was on the GPU).
    /// `cancel` stops the run promptly: through transcribe.cpp's abort when
    /// the model honours it, otherwise by stopping the worker (the model then
    /// reloads on next use).
    pub fn transcribe(
        &self,
        pcm: Vec<f32>,
        run: RunOptions,
        cancel: Option<CancelToken>,
    ) -> Result<Transcript, EngineError> {
        self.ask(|reply| Command::Transcribe {
            pcm,
            run,
            cancel,
            reply,
        })
        .unwrap_or_else(|| Err(engine_stopped()))
    }

    /// Begin a live stream on the loaded model. `on_progress` runs on the
    /// owner thread after every feed, so it must not block or call back into
    /// the supervisor. Cancelling `cancel` stops the feed or finalize in
    /// progress and every later one.
    pub fn start_stream(
        &self,
        run: RunOptions,
        stream: StreamOptions,
        cancel: CancelToken,
        on_progress: impl FnMut(StreamProgress) + Send + 'static,
    ) -> Result<StreamHandle, EngineError> {
        let id = self.shared.next_id();
        let closed = Arc::new(AtomicBool::new(false));
        self.ask(|reply| Command::StreamBegin {
            active: ActiveStream {
                id,
                cancel: cancel.clone(),
                closed: Arc::clone(&closed),
                on_progress: Box::new(on_progress),
                pending_ms: 0,
            },
            run,
            stream,
            reply,
        })
        .unwrap_or_else(|| Err(engine_stopped()))?;
        Ok(StreamHandle {
            id,
            cancel,
            closed,
            window: Arc::new(FeedWindow::default()),
            commands: self.commands.clone(),
            finished: false,
        })
    }

    /// The compute devices, as listed by the latest worker. If no worker has
    /// listed them yet, starts a short-lived worker on this thread to list
    /// them. It runs beside any model worker, which it leaves alone. Not while
    /// the GPU is marked unavailable: that probe would crash or hang the same
    /// way. Nor after a probe failed for another reason (see
    /// [`Shared::probe_failed`]).
    pub fn devices(&self) -> Option<Vec<DeviceInfo>> {
        let _probing = lock(&self.shared.probing);
        if lock(&self.shared.devices).is_none()
            && !self.shared.gpu_unavailable()
            && !self.shared.probe_failed.load(Ordering::Acquire)
        {
            self.shared.probe_devices();
        }
        lock(&self.shared.devices).clone()
    }
}

/// An unload in progress; see [`EngineSupervisor::unload`].
pub struct Unloading(Option<mpsc::Receiver<()>>);

impl Unloading {
    /// Wait until the worker has exited (e.g. before deleting the model file,
    /// which Windows would refuse while the worker has it mapped).
    pub fn wait(self) {
        if let Some(exited) = self.0 {
            let _ = exited.recv();
        }
    }

    /// [`wait`](Self::wait), for at most `timeout`. Returns whether the worker
    /// has exited. The unload stays queued either way.
    pub fn wait_timeout(self, timeout: Duration) -> bool {
        match self.0 {
            Some(exited) => !matches!(
                exited.recv_timeout(timeout),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            None => true,
        }
    }
}

/// Feeds of one stream sent but not yet handled by the owner. Bounded at
/// [`FEED_WINDOW`] so a stream the model can't keep up with backs up into the
/// live router (which is bounded and drops frames with a warning) rather than
/// into the unbounded command queue.
#[derive(Default)]
struct FeedWindow {
    queued: Mutex<usize>,
    freed: Condvar,
}

impl FeedWindow {
    /// Take a slot, waiting while the window is full. Never waits for a
    /// cancelled or closed stream: its feeds are skipped anyway.
    fn acquire(&self, cancel: &CancelToken, closed: &AtomicBool) {
        let mut queued = lock(&self.queued);
        while *queued >= FEED_WINDOW && !cancel.is_cancelled() && !closed.load(Ordering::Acquire) {
            queued = self
                .freed
                .wait_timeout(queued, Duration::from_millis(50))
                .map(|(guard, _)| guard)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        *queued += 1;
    }

    fn release(&self) {
        let mut queued = lock(&self.queued);
        *queued = queued.saturating_sub(1);
        self.freed.notify_all();
    }

    #[cfg(test)]
    fn queued(&self) -> usize {
        *lock(&self.queued)
    }
}

/// Frees a feed's window slot however the owner handles the feed.
struct WindowSlot(Arc<FeedWindow>);

impl Drop for WindowSlot {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// A live stream in the worker. Feeds are queued in order with finalize.
/// Dropping the handle without finalizing resets the stream.
pub struct StreamHandle {
    id: u64,
    cancel: CancelToken,
    /// Set when the handle is dropped; shared with the owner.
    closed: Arc<AtomicBool>,
    window: Arc<FeedWindow>,
    commands: mpsc::Sender<Command>,
    finished: bool,
}

impl StreamHandle {
    /// Queue a frame. Waits only while [`FEED_WINDOW`] feeds are already
    /// queued, which is what bounds a stream that falls behind.
    pub fn feed(&self, pcm: Vec<f32>) {
        if self.cancel.is_cancelled() {
            return;
        }
        self.window.acquire(&self.cancel, &self.closed);
        let command = Command::Feed {
            id: self.id,
            pcm,
            window: Arc::clone(&self.window),
        };
        if self.commands.send(command).is_err() {
            self.window.release();
        }
    }

    /// Flush the stream and return its final text. `Ok(None)` if the stream
    /// was lost or failed; fall back to batch transcription then.
    /// `Err(Cancelled)` if its cancel token stopped it.
    pub fn finalize(mut self, want_language: bool) -> Result<Option<Finalized>, EngineError> {
        self.finished = true;
        let (reply, answer) = mpsc::channel();
        let command = Command::Finalize {
            id: self.id,
            want_language,
            reply,
        };
        if self.commands.send(command).is_err() {
            return Ok(None);
        }
        answer.recv().unwrap_or(Ok(None))
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        if !self.finished {
            // Queued feeds for this stream are skipped from here on.
            self.closed.store(true, Ordering::Release);
            let _ = self.commands.send(Command::StreamReset { id: self.id });
        }
    }
}

fn engine_stopped() -> EngineError {
    EngineError::Failed("the transcription engine has stopped".to_string())
}

fn not_loaded() -> EngineError {
    EngineError::Failed("no transcription model is loaded".to_string())
}

/// How a call to the worker failed.
#[derive(Debug)]
enum Failure {
    /// The worker process could not be started.
    Spawn(io::Error),
    /// transcribe.cpp returned an error; the worker is fine.
    Remote {
        kind: ErrorKind,
        message: String,
    },
    /// The worker refused the request (e.g. no model); the worker is fine.
    Refused(String),
    NotSpeechModel(String),
    /// The worker exited or crashed. Carries exit status and its last native
    /// output.
    Crashed(String),
    /// The worker did not answer within the deadline and was killed.
    Hung(Duration),
    /// The caller cancelled. `worker_lost`: the model did not abort in time,
    /// so the worker was stopped.
    Cancelled {
        worker_lost: bool,
    },
}

impl Failure {
    /// Whether the worker is gone.
    fn worker_lost(&self) -> bool {
        matches!(
            self,
            Failure::Crashed(_) | Failure::Hung(_) | Failure::Cancelled { worker_lost: true }
        )
    }

    /// Whether the worker is gone through a fault of its own (not a cancel).
    fn worker_faulted(&self) -> bool {
        matches!(self, Failure::Crashed(_) | Failure::Hung(_))
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Spawn(e) => write!(f, "failed to start transcription worker: {e}"),
            Failure::Remote { message, .. } => f.write_str(message),
            Failure::Refused(message) => f.write_str(message),
            Failure::NotSpeechModel(message) => {
                write!(f, "not a speech-to-text model ({message})")
            }
            Failure::Crashed(detail) => write!(f, "transcription worker crashed ({detail})"),
            Failure::Hung(t) => write!(f, "transcription worker did not respond within {t:?}"),
            Failure::Cancelled { .. } => f.write_str("cancelled"),
        }
    }
}

impl From<Failure> for EngineError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Cancelled { .. } => EngineError::Cancelled,
            Failure::Remote {
                kind: ErrorKind::InputTooShort,
                message,
            } => EngineError::InputTooShort(message),
            Failure::NotSpeechModel(message) => EngineError::NotSpeechModel(message),
            other => EngineError::Failed(other.to_string()),
        }
    }
}

fn unexpected(response: &Response) -> Failure {
    Failure::Refused(format!(
        "unexpected transcription worker response: {response:?}"
    ))
}

/// The request id and token a call can be cancelled with.
struct Cancel<'a> {
    token: &'a CancelToken,
    id: u64,
    /// The model honours an abort request; otherwise only stopping the worker
    /// ends the work.
    can_abort: bool,
}

// --- Owner thread ---------------------------------------------------------

struct ActiveStream {
    id: u64,
    cancel: CancelToken,
    /// Set when the caller drops its handle.
    closed: Arc<AtomicBool>,
    on_progress: OnProgress,
    /// Audio received but not yet committed, which finalize must still decode;
    /// sizes the feed and finalize deadlines.
    pending_ms: i64,
}

struct Owner {
    shared: Arc<Shared>,
    /// The single worker slot.
    worker: Option<Worker>,
    /// The model the app asked for. Outlives its worker: after a crash, hang
    /// or a cancel that had to stop it, the next use starts a fresh worker.
    spec: Option<LoadSpec>,
    stream: Option<ActiveStream>,
    /// Unload commands processed, against [`Shared::unloads_requested`].
    unloads_done: u64,
    /// Batch runs that arrived while a stream was open, in arrival order.
    deferred: VecDeque<Command>,
}

impl Owner {
    fn run(mut self, queue: mpsc::Receiver<Command>) {
        loop {
            if self.stream.is_none() {
                if let Some(command) = self.deferred.pop_front() {
                    self.handle(command);
                    continue;
                }
            }
            let next = if self.deferred.is_empty() {
                queue.recv().map_err(|_| ())
            } else {
                match queue.recv_timeout(DEFERRED_SWEEP) {
                    Ok(command) => Ok(command),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        self.answer_cancelled_deferred();
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
                }
            };
            match next {
                Ok(command) => self.handle(command),
                // Every handle is gone.
                Err(()) => break,
            }
        }
        self.unload();
    }

    fn handle(&mut self, command: Command) {
        match command {
            // A load or an unload stops the worker, and the live stream with
            // it: the dictation still recording lost its live text and, when
            // it ended, found no model for its batch fallback ("Model is not
            // loaded for transcription"). The in-process engine this replaced
            // was out of reach while a stream had it leased, so a model switch,
            // a Speech processing change or an "Immediately" unload after
            // another transcription waited for the dictation. They wait here
            // too, queued with the batch runs in the order they came. Quitting
            // (`now`) does not wait.
            command @ (Command::Load(..) | Command::Unload { now: false, .. })
                if self.stream.is_some() =>
            {
                debug!("A model load or unload is waiting for the live stream to end");
                self.deferred.push_back(command);
            }
            Command::Load(spec, reply) => {
                let _ = reply.send(self.load(spec));
            }
            Command::Unload { reply, now } => {
                if now {
                    self.drop_deferred();
                }
                self.unload();
                self.unloads_done += 1;
                let _ = reply.send(());
            }
            command @ Command::Transcribe { .. } if self.stream.is_some() => {
                debug!("A transcription is waiting for the live stream to end");
                self.deferred.push_back(command);
            }
            Command::Transcribe {
                pcm,
                run,
                cancel,
                reply,
            } => {
                let _ = reply.send(self.transcribe(&pcm, &run, cancel.as_ref()));
            }
            Command::StreamBegin {
                active,
                run,
                stream,
                reply,
            } => {
                let _ = reply.send(self.stream_begin(active, &run, &stream));
            }
            Command::Feed { id, pcm, window } => {
                let _slot = WindowSlot(window);
                self.feed(id, &pcm);
            }
            Command::Finalize {
                id,
                want_language,
                reply,
            } => {
                let _ = reply.send(self.finalize(id, want_language));
            }
            Command::StreamReset { id } => {
                if self.stream.as_ref().is_some_and(|s| s.id == id) {
                    self.end_stream();
                }
            }
        }
    }

    /// Answer deferred runs whose caller cancelled them while they waited.
    /// Answer everything that was waiting for the live stream, without running
    /// it. Quitting stops the worker without waiting for the stream, which
    /// then reads as ended; a model switch queued behind it would otherwise run
    /// next and start a fresh worker after the app had stopped this one, which
    /// during an in-app update is a running copy of the app the installer is
    /// about to replace.
    fn drop_deferred(&mut self) {
        for command in std::mem::take(&mut self.deferred) {
            match command {
                Command::Load(_, reply) => {
                    let _ = reply.send(Err(engine_stopped()));
                }
                Command::Unload { reply, .. } => {
                    self.unloads_done += 1;
                    let _ = reply.send(());
                }
                Command::Transcribe { reply, .. } => {
                    let _ = reply.send(Err(EngineError::Cancelled));
                }
                // Only loads, unloads and batch runs are ever deferred.
                _ => {}
            }
        }
    }

    fn answer_cancelled_deferred(&mut self) {
        self.deferred.retain(|command| match command {
            Command::Transcribe {
                cancel: Some(token),
                reply,
                ..
            } if token.is_cancelled() => {
                let _ = reply.send(Err(EngineError::Cancelled));
                false
            }
            _ => true,
        });
    }

    fn load(&mut self, spec: LoadSpec) -> Result<LoadedInfo, EngineError> {
        self.unload();
        self.spec = Some(spec);
        let result = self.start();
        if result.is_err() {
            self.spec = None;
        }
        result
    }

    fn unload(&mut self) {
        // The worker's exit ends any stream; no need to reset it first.
        self.stream = None;
        if let Some(worker) = self.worker.take() {
            let started = Instant::now();
            let pid = worker.control.pid;
            drop(worker);
            debug!(
                "Transcription worker (pid {}) stopped in {}ms",
                pid,
                started.elapsed().as_millis()
            );
        }
        self.spec = None;
        *lock(&self.shared.loaded) = None;
    }

    /// Start a worker for `spec` and load it there. A worker that may use the
    /// GPU lists the devices in its hello, which keeps the list current (a GPU
    /// that comes back is just noticed). A CPU-only worker would only see the
    /// CPU, so it doesn't list.
    fn start_worker(&self, spec: &LoadSpec) -> Result<(Worker, LoadedInfo), Failure> {
        let cpu_only = cpu_only_worker(&spec.target, self.shared.gpu_allowed);
        let mut worker = self.shared.hello(cpu_only, !cpu_only)?;
        let info = worker.load(spec)?;
        Ok((worker, info))
    }

    /// Start a worker and load the current spec in it. If a worker with GPU
    /// backends is lost on the way (backend init, device listing or the
    /// load), mark the GPU unavailable and retry once in a CPU-only worker.
    fn start(&mut self) -> Result<LoadedInfo, EngineError> {
        let spec = self.spec.clone().ok_or_else(not_loaded)?;
        let started = Instant::now();
        let wanted = LoadSpec {
            path: spec.path.clone(),
            target: effective_target(&spec.target, self.shared.gpu_unavailable()),
        };
        if wanted.target != spec.target {
            info!("Loading the transcription model on the processor: the GPU failed earlier");
        }
        let (worker, info) = match self.start_worker(&wanted) {
            Ok(started) => started,
            Err(failure)
                if failure.worker_faulted()
                    && !cpu_only_worker(&wanted.target, self.shared.gpu_allowed) =>
            {
                self.shared.mark_gpu_unavailable(format!(
                    "a worker with GPU backends {} while loading '{}'",
                    failure,
                    wanted
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                warn!("Retrying the model load in a CPU-only transcription worker");
                self.start_worker(&wanted.on_cpu())?
            }
            Err(failure) => return Err(failure.into()),
        };
        debug!(
            "Transcription worker (pid {}) spawned and loaded the model in {}ms",
            worker.control.pid,
            started.elapsed().as_millis()
        );
        self.publish_loaded(&info);
        self.worker = Some(worker);
        Ok(info)
    }

    /// Report the model loaded, unless an unload was requested since and is
    /// still waiting in the queue.
    fn publish_loaded(&self, info: &LoadedInfo) {
        let mut loaded = lock(&self.shared.loaded);
        if self.shared.unloads_requested.load(Ordering::SeqCst) == self.unloads_done {
            *loaded = Some(info.clone());
        }
    }

    /// Make sure the slot holds a worker with the model loaded, starting a
    /// fresh one if the last one died or was stopped.
    fn ensure_worker(&mut self) -> Result<(), EngineError> {
        if self.worker.is_some() {
            return Ok(());
        }
        if self.spec.is_none() {
            return Err(not_loaded());
        }
        info!("Starting a fresh transcription worker for the loaded model");
        if let Err(e) = self.start() {
            // The model can't be brought back; report it unloaded so the next
            // use loads it from scratch (and reports why it can't).
            self.spec = None;
            *lock(&self.shared.loaded) = None;
            return Err(e);
        }
        Ok(())
    }

    /// Drop a worker that crashed or hung (its handle already logged the
    /// details and native output). If it was on a GPU, mark the GPU
    /// unavailable so the next worker uses the processor. Returns whether it
    /// was on a GPU.
    fn worker_lost(&mut self, during: &str, failure: &Failure) -> bool {
        // Dropping the handle reaps the process.
        let info = self.worker.take().and_then(|worker| worker.info.clone());
        match info {
            Some(info) if info.on_gpu && failure.worker_faulted() => {
                self.shared.mark_gpu_unavailable(format!(
                    "{failure} during {during} on GPU '{}' ({})",
                    info.device, info.backend
                ));
                true
            }
            _ => false,
        }
    }

    fn on_gpu(&self) -> bool {
        self.worker
            .as_ref()
            .and_then(|worker| worker.info.as_ref())
            .is_some_and(|info| info.on_gpu)
    }

    fn can_abort(&self) -> bool {
        self.worker
            .as_ref()
            .and_then(|worker| worker.info.as_ref())
            .is_some_and(|info| info.supports_cancellation)
    }

    /// Run once, retrying once after a crash or hang (see [`after_lost_run`]).
    fn transcribe(
        &mut self,
        pcm: &[f32],
        run: &RunOptions,
        cancel: Option<&CancelToken>,
    ) -> Result<Transcript, EngineError> {
        let mut retried = false;
        loop {
            if cancel.is_some_and(CancelToken::is_cancelled) {
                return Err(EngineError::Cancelled);
            }
            // A respawn here runs under the load deadline; the run's own
            // deadline starts only once the model is loaded.
            self.ensure_worker()?;
            let deadline = Deadlines::for_device(self.on_gpu()).run(pcm.len());
            let id = self.shared.next_id();
            let request = Request::Run {
                id,
                options: run.clone(),
            };
            let can_abort = self.can_abort();
            let cancel = cancel.map(|token| Cancel {
                token,
                id,
                can_abort,
            });
            let Some(worker) = self.worker.as_mut() else {
                return Err(not_loaded());
            };
            match worker.call(&request, Some(pcm), deadline, cancel.as_ref()) {
                Ok(Response::Transcript(transcript)) => return Ok(transcript),
                Ok(other) => return Err(unexpected(&other).into()),
                Err(failure @ Failure::Cancelled { worker_lost }) => {
                    if worker_lost {
                        // The model stays loaded; the next use starts a fresh
                        // worker for it.
                        self.worker = None;
                    }
                    return Err(failure.into());
                }
                Err(failure) if failure.worker_lost() => {
                    let on_gpu = self.worker_lost("transcription", &failure);
                    let hung = matches!(failure, Failure::Hung(_));
                    match after_lost_run(hung, on_gpu, retried) {
                        AfterLoss::GiveUp => return Err(failure.into()),
                        AfterLoss::Retry => {
                            warn!(
                                "Retrying the transcription in a fresh {}worker after: {}",
                                if on_gpu { "CPU-only " } else { "" },
                                failure
                            );
                            retried = true;
                        }
                    }
                }
                Err(failure) => return Err(failure.into()),
            }
        }
    }

    fn stream_begin(
        &mut self,
        active: ActiveStream,
        run: &RunOptions,
        stream: &StreamOptions,
    ) -> Result<(), EngineError> {
        self.end_stream();
        if active.cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        self.ensure_worker()?;
        let request = Request::StreamBegin {
            id: active.id,
            run: run.clone(),
            stream: stream.clone(),
        };
        let Some(worker) = self.worker.as_mut() else {
            return Err(not_loaded());
        };
        match worker.call(&request, None, CALL_FLOOR, None) {
            Ok(Response::Ok) => {
                self.stream = Some(active);
                Ok(())
            }
            Ok(other) => Err(unexpected(&other).into()),
            Err(failure) => {
                if failure.worker_lost() {
                    self.worker_lost("stream start", &failure);
                }
                Err(failure.into())
            }
        }
    }

    /// Feeds never respawn the worker (the stream state would be lost); a lost
    /// stream just drops the rest of its frames and finalizes to `None`, and
    /// the caller falls back to batch.
    fn feed(&mut self, id: u64, pcm: &[f32]) {
        let Some(stream) = self.stream.as_ref().filter(|s| s.id == id) else {
            return;
        };
        // Skipped once the caller dropped its handle.
        if stream.closed.load(Ordering::Acquire) || self.worker.is_none() {
            return;
        }
        if stream.cancel.is_cancelled() {
            self.end_stream();
            return;
        }
        let token = stream.cancel.clone();
        let deadline = Deadlines::for_device(self.on_gpu())
            .audio(pcm.len() + ms_to_samples(stream.pending_ms));
        let can_abort = self.can_abort();
        let cancel = Cancel {
            token: &token,
            id,
            can_abort,
        };
        let started = Instant::now();
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        match worker.call(&Request::Feed, Some(pcm), deadline, Some(&cancel)) {
            Ok(Response::Fed { update, text }) => {
                let Some(stream) = self.stream.as_mut() else {
                    return;
                };
                stream.pending_ms = pending_ms(&update);
                let progress = StreamProgress {
                    update,
                    text,
                    elapsed: started.elapsed(),
                };
                if catch_unwind(AssertUnwindSafe(|| (stream.on_progress)(progress))).is_err() {
                    error!("Live transcription callback panicked");
                }
            }
            Ok(other) => warn!("Stream feed failed: {}", unexpected(&other)),
            Err(Failure::Remote { message, .. }) | Err(Failure::Refused(message)) => {
                warn!("Stream feed failed: {}", message)
            }
            // Later feeds are skipped, and finalize reports the cancel.
            Err(Failure::Cancelled { worker_lost }) => {
                if worker_lost {
                    self.stream = None;
                    self.worker = None;
                } else {
                    self.end_stream();
                }
            }
            Err(failure) => {
                self.stream = None;
                self.worker_lost("stream feed", &failure);
            }
        }
    }

    fn finalize(&mut self, id: u64, want_language: bool) -> Result<Option<Finalized>, EngineError> {
        let Some(stream) = self.stream.as_ref().filter(|s| s.id == id) else {
            // Lost earlier (or replaced): fall back to batch.
            return Ok(None);
        };
        // Checked first: a stream lost to a cancel must report the cancel,
        // not "no result", which would start a batch run.
        if stream.cancel.is_cancelled() {
            self.end_stream();
            return Err(EngineError::Cancelled);
        }
        let Some(stream) = self.stream.take() else {
            return Ok(None);
        };
        if self.worker.is_none() {
            return Ok(None);
        }
        let deadline = Deadlines::for_device(self.on_gpu()).audio(ms_to_samples(stream.pending_ms));
        let can_abort = self.can_abort();
        let cancel = Cancel {
            token: &stream.cancel,
            id,
            can_abort,
        };
        let started = Instant::now();
        let Some(worker) = self.worker.as_mut() else {
            return Ok(None);
        };
        let request = Request::Finalize { want_language };
        match worker.call(&request, None, deadline, Some(&cancel)) {
            Ok(Response::Finalized {
                update,
                text,
                language,
                error,
            }) => {
                if let Some(error) = error {
                    warn!(
                        "Stream finalize failed ({}); keeping what the stream committed",
                        error
                    );
                }
                Ok(Some(Finalized {
                    update,
                    text,
                    language,
                    elapsed: started.elapsed(),
                }))
            }
            Ok(other) => {
                error!("Stream finalize failed: {}", unexpected(&other));
                Ok(None)
            }
            // The finalize was sent, so the worker's stream has ended either
            // way: it answered (aborted), or it was stopped.
            Err(Failure::Cancelled { worker_lost }) => {
                if worker_lost {
                    self.worker = None;
                }
                Err(EngineError::Cancelled)
            }
            Err(Failure::Remote { message, .. }) | Err(Failure::Refused(message)) => {
                error!("Stream finalize failed: {}", message);
                Ok(None)
            }
            Err(failure) => {
                self.worker_lost("stream finalize", &failure);
                Ok(None)
            }
        }
    }

    fn end_stream(&mut self) {
        if self.stream.take().is_some() {
            self.reset_worker_stream();
        }
    }

    fn reset_worker_stream(&mut self) {
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        match worker.call(&Request::StreamReset, None, CALL_FLOOR, None) {
            Ok(_) => {}
            // Expected when the stream already ended in the worker.
            Err(Failure::Refused(message)) => debug!("Stream reset: {}", message),
            Err(failure) => {
                if failure.worker_lost() {
                    self.worker_lost("stream reset", &failure);
                }
            }
        }
    }
}

fn pending_ms(update: &StreamUpdate) -> i64 {
    update
        .buffered_ms
        .max(update.input_received_ms - update.audio_committed_ms)
        .max(0)
}

// --- Worker process handle ------------------------------------------------

/// The parts of a worker that other threads touch: its stdin queue and the
/// process (to kill it). Every write goes through the one writer thread, so a
/// cancel can never interleave with a large frame.
struct Control {
    pid: u32,
    child: Mutex<Child>,
    stdin: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
}

impl Control {
    fn send(&self, frame: Vec<u8>) -> bool {
        lock(&self.stdin)
            .as_ref()
            .is_some_and(|stdin| stdin.send(frame).is_ok())
    }

    /// Closing stdin tells the worker to exit (it ends on EOF).
    fn close_stdin(&self) {
        lock(&self.stdin).take();
    }

    fn kill(&self) {
        let _ = lock(&self.child).kill();
    }
}

/// One worker process, owned by the supervisor's thread. Strict
/// request/response. Dropping it closes the worker's stdin, gives it
/// [`SHUTDOWN_GRACE`] to exit, kills it otherwise, and reaps it: once dropped,
/// the worker is gone.
struct Worker {
    control: Arc<Control>,
    responses: mpsc::Receiver<Response>,
    stderr_done: mpsc::Receiver<()>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    /// Set once a model is loaded.
    info: Option<LoadedInfo>,
}

impl Worker {
    /// Start a worker process. A `cpu_only` worker never registers a GPU
    /// backend.
    fn spawn(cpu_only: bool) -> io::Result<Self> {
        let exe = worker_exe()?;
        let mut command = ProcessCommand::new(exe);
        #[cfg(target_os = "linux")]
        if let Some(arg0) = std::env::args_os().next() {
            // Keep `ps` showing SpeakoFlow's own path rather than /proc/self/exe.
            use std::os::unix::process::CommandExt;
            command.arg0(arg0);
        }
        command.arg(WORKER_FLAG);
        if cpu_only {
            command.arg(CPU_ONLY_FLAG);
        }
        command
            .env(LOG_LEVEL_ENV, crate::file_log_level().as_str())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn()?;
        // Dies with the app even on a crash or a Task Manager kill, which no
        // Drop or exit handler sees. (The worker also exits on its own when
        // its stdin closes, which covers the other platforms.)
        #[cfg(windows)]
        crate::utils::assign_child_to_kill_on_close_job(&child);
        let pid = child.id();
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("the worker's pipes were not created"));
        };

        let (stdin_tx, frames) = mpsc::channel();
        let (response_tx, responses) = mpsc::channel();
        let (stderr_done_tx, stderr_done) = mpsc::channel();
        #[cfg(test)]
        LIVE_WORKERS.fetch_add(1, Ordering::SeqCst);
        // Built before the threads so a failure below still reaps the child.
        let worker = Worker {
            control: Arc::new(Control {
                pid,
                child: Mutex::new(child),
                stdin: Mutex::new(Some(stdin_tx)),
            }),
            responses,
            stderr_done,
            stderr_tail: Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES))),
            info: None,
        };

        thread::Builder::new()
            .name("transcribe-worker-in".into())
            .spawn(move || write_frames(stdin, frames))?;
        thread::Builder::new()
            .name("transcribe-worker-out".into())
            .spawn(move || read_responses(stdout, response_tx))?;
        // Always drain stderr: a full pipe would block the worker.
        let tail = Arc::clone(&worker.stderr_tail);
        thread::Builder::new()
            .name("transcribe-worker-err".into())
            .spawn(move || {
                // Split on raw bytes: native code may write non-UTF-8 (e.g. a
                // path in the Windows ANSI code page), and `lines()` would end
                // the loop there, losing every later line, abort message
                // included.
                for line in BufReader::new(stderr).split(b'\n') {
                    let Ok(line) = line else { break };
                    let line = String::from_utf8_lossy(&line);
                    let line = line.strip_suffix('\r').unwrap_or(&line).to_string();
                    forward_worker_log(&line);
                    let mut tail = lock(&tail);
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
                let _ = stderr_done_tx.send(());
            })?;

        debug!(
            "Started {}transcription worker (pid {})",
            if cpu_only { "CPU-only " } else { "" },
            pid
        );
        Ok(worker)
    }

    fn load(&mut self, spec: &LoadSpec) -> Result<LoadedInfo, Failure> {
        let request = Request::Load {
            path: spec.path.clone(),
            target: spec.target.clone(),
        };
        match self.call(&request, None, LOAD_TIMEOUT, None)? {
            Response::Loaded(info) => {
                self.info = Some(info.clone());
                Ok(info)
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Send one request and wait up to `deadline` for its response. Hitting
    /// the deadline kills the worker; after any failure that
    /// [`Failure::worker_lost`] the handle is dead and should be dropped.
    fn call(
        &mut self,
        request: &Request,
        pcm: Option<&[f32]>,
        deadline: Duration,
        cancel: Option<&Cancel<'_>>,
    ) -> Result<Response, Failure> {
        let frame = encode_message(request, pcm)
            .map_err(|e| Failure::Refused(format!("failed to encode request: {e}")))?;
        if !self.control.send(frame) {
            return Err(self.died());
        }
        self.wait(request, deadline, cancel)
    }

    /// Wait for the response to `request`, already sent.
    fn wait(
        &mut self,
        request: &Request,
        deadline: Duration,
        cancel: Option<&Cancel<'_>>,
    ) -> Result<Response, Failure> {
        let sent = Instant::now();
        let mut cancel_sent: Option<Instant> = None;
        loop {
            let cancelled = cancel.is_some_and(|c| c.token.is_cancelled());
            let step = wait_step(
                sent.elapsed(),
                deadline,
                cancelled,
                cancel_sent.map(|at| at.elapsed()),
                cancel.is_some_and(|c| c.can_abort),
            );
            let slice = match step {
                WaitStep::Wait(slice) => slice,
                WaitStep::SendCancel => {
                    if let Some(cancel) = cancel {
                        debug!(
                            "Asking the transcription worker to abort its {}",
                            request_name(request)
                        );
                        if let Ok(frame) = encode_message(&Request::Cancel { id: cancel.id }, None)
                        {
                            self.control.send(frame);
                        }
                    }
                    cancel_sent = Some(Instant::now());
                    continue;
                }
                WaitStep::KillCancelled => {
                    info!(
                        "Stopping the transcription worker (pid {}) to cancel its {}",
                        self.control.pid,
                        request_name(request)
                    );
                    self.control.kill();
                    return Err(Failure::Cancelled { worker_lost: true });
                }
                WaitStep::KillHung => {
                    self.control.kill();
                    let tail = self.native_tail();
                    error!(
                        "Transcription worker (pid {}) did not answer its {} within {:?}; killed it{}",
                        self.control.pid,
                        request_name(request),
                        deadline,
                        if tail.is_empty() {
                            String::new()
                        } else {
                            format!(". Native output before the hang:\n{}", tail.join("\n"))
                        }
                    );
                    return Err(Failure::Hung(deadline));
                }
            };
            match self.responses.recv_timeout(slice) {
                // Whatever it answered, a cancelled call is cancelled. An
                // answer means the worker is fine and its work has ended.
                Ok(_) if cancel_sent.is_some() || cancelled => {
                    return Err(Failure::Cancelled { worker_lost: false })
                }
                Ok(Response::Failed { kind, message }) => {
                    return Err(Failure::Remote { kind, message })
                }
                Ok(Response::Error(message)) => return Err(Failure::Refused(message)),
                Ok(Response::NotSpeechModel(message)) => {
                    return Err(Failure::NotSpeechModel(message))
                }
                Ok(response) => return Ok(response),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let failure = self.died();
                    // A worker that crashed after the user cancelled is
                    // still a crash: the GPU may be at fault.
                    return Err(failure);
                }
            }
        }
    }

    /// The worker's stdout closed: collect its exit status and last output.
    fn died(&mut self) -> Failure {
        // Let the stderr thread catch the final lines (the abort message).
        let _ = self.stderr_done.recv_timeout(Duration::from_secs(1));
        let status = {
            let mut child = lock(&self.control.child);
            match wait_exit(&mut child, Duration::from_secs(1)) {
                Some(status) => status.to_string(),
                None => {
                    let _ = child.kill();
                    "unresponsive after closing its output; killed".to_string()
                }
            }
        };
        let tail = self.native_tail();
        if tail.is_empty() {
            error!(
                "Transcription worker (pid {}) died: {}",
                self.control.pid, status
            );
        } else {
            error!(
                "Transcription worker (pid {}) died: {}. Native output before exit:\n{}",
                self.control.pid,
                status,
                tail.join("\n")
            );
        }
        Failure::Crashed(match tail.last() {
            Some(last) => format!("{status}: {last}"),
            None => status,
        })
    }

    /// Recent raw native output (worker log lines are excluded; they were
    /// already re-logged).
    fn native_tail(&self) -> Vec<String> {
        lock(&self.stderr_tail)
            .iter()
            .filter(|line| !line.starts_with(LOG_LINE_PREFIX))
            .cloned()
            .collect()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.control.close_stdin();
        let mut child = lock(&self.control.child);
        if wait_exit(&mut child, SHUTDOWN_GRACE).is_none() {
            warn!(
                "Transcription worker (pid {}) did not exit within {:?}; killing it",
                self.control.pid, SHUTDOWN_GRACE
            );
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(test)]
        LIVE_WORKERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Wait up to `timeout` for `child` to exit. `None` if it is still running (or
/// its status can't be read).
fn wait_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => return None,
        }
    }
}

/// Worker processes started and not yet reaped. Every worker is reaped when
/// its handle drops, so this is exactly the worker processes still alive.
#[cfg(test)]
static LIVE_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The file SpeakoFlow was started from, with its identity (device and inode),
/// recorded at startup; see [`worker_exe`]. `None` if either couldn't be read,
/// which skips the check.
#[cfg(all(unix, not(test)))]
static LAUNCHED_EXE: std::sync::OnceLock<Option<(PathBuf, (u64, u64))>> =
    std::sync::OnceLock::new();

#[cfg(all(unix, not(test)))]
fn file_id(path: &std::path::Path) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

/// Must run before an upgrade can replace the file, so at startup.
#[cfg(all(unix, not(test)))]
fn record_launched_exe() {
    LAUNCHED_EXE.get_or_init(|| {
        std::env::current_exe()
            .and_then(|path| {
                let id = file_id(&path)?;
                Ok((path, id))
            })
            .inspect_err(|e| warn!("Could not identify SpeakoFlow's executable: {}", e))
            .ok()
    });
}

/// The executable a worker runs: always this very binary, so the two agree on
/// the protocol and on transcribe.cpp's backend libraries. Upgrading or moving
/// SpeakoFlow while it runs replaces or removes the file it started from, and a
/// worker started from there would be the new version (macOS), or this binary
/// loading the new version's backend libraries (Linux). Only a restart fixes
/// that, so refuse to start one and say so. Windows locks a running executable
/// against replacement. On Linux the worker runs from `/proc/self/exe`, which
/// is this binary even once its file is replaced.
///
/// Debug and test builds honour [`super::WORKER_EXE_ENV`]; under `cargo test`
/// the default is the app binary the same `cargo build` put beside the test
/// harness's `deps` folder.
fn worker_exe() -> io::Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(exe) = std::env::var_os(super::WORKER_EXE_ENV) {
        return Ok(PathBuf::from(exe));
    }
    #[cfg(test)]
    {
        let harness = std::env::current_exe()?;
        let exe = harness
            .parent()
            .and_then(std::path::Path::parent)
            .map(|dir| dir.join(format!("speakoflow{}", std::env::consts::EXE_SUFFIX)))
            .filter(|exe| exe.exists())
            .ok_or_else(|| {
                io::Error::other(format!(
                    "no worker executable: run `cargo build` first or set {}",
                    super::WORKER_EXE_ENV
                ))
            })?;
        Ok(exe)
    }
    #[cfg(not(test))]
    {
        #[cfg(unix)]
        if let Some((path, launched)) = LAUNCHED_EXE.get().and_then(Option::as_ref) {
            if file_id(path).ok().as_ref() != Some(launched) {
                return Err(io::Error::other(
                    "SpeakoFlow was updated or moved while it was running; restart SpeakoFlow",
                ));
            }
        }
        #[cfg(target_os = "linux")]
        return Ok(PathBuf::from("/proc/self/exe"));
        #[cfg(not(target_os = "linux"))]
        std::env::current_exe()
    }
}

fn write_frames(mut stdin: ChildStdin, frames: mpsc::Receiver<Vec<u8>>) {
    // Ends when the handle closes stdin (dropping it here sends EOF) or the
    // worker is gone.
    for frame in frames {
        if stdin
            .write_all(&frame)
            .and_then(|()| stdin.flush())
            .is_err()
        {
            break;
        }
    }
}

fn read_responses(stdout: ChildStdout, responses: mpsc::Sender<Response>) {
    let mut stdout = BufReader::new(stdout);
    loop {
        match read_message::<Response>(&mut stdout) {
            Ok(Some((response, _))) => {
                if responses.send(response).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                error!("Transcription worker protocol error: {}", e);
                break;
            }
        }
    }
}

fn request_name(request: &Request) -> &'static str {
    match request {
        Request::Hello { .. } => "hello",
        Request::Load { .. } => "model load",
        Request::Run { .. } => "transcription",
        Request::StreamBegin { .. } => "stream begin",
        Request::Feed => "stream feed",
        Request::Finalize { .. } => "stream finalize",
        Request::StreamReset => "stream reset",
        Request::Cancel { .. } => "cancel",
    }
}

/// Re-log a worker stderr line in the parent. Structured lines keep their
/// level; anything else is raw native output.
fn forward_worker_log(line: &str) {
    const TARGET: &str = "transcribe_worker";
    if let Some(rest) = line.strip_prefix(LOG_LINE_PREFIX) {
        let mut parts = rest.splitn(3, '\t');
        if let (Some(level), Some(target), Some(message)) =
            (parts.next(), parts.next(), parts.next())
        {
            let level = level.parse::<Level>().unwrap_or(Level::Info);
            log::log!(target: TARGET, level, "[{}] {}", target, message);
            return;
        }
    }
    if !line.trim().is_empty() {
        log::info!(target: TARGET, "{}", line);
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn deadlines_scale_with_the_audio_above_a_floor() {
        let gpu = Deadlines::for_device(true);
        assert_eq!(gpu, GPU_DEADLINES);
        assert_eq!(gpu.audio(0), gpu.call_floor);
        assert_eq!(gpu.audio(16_000 / 2), gpu.call_floor);
        assert_eq!(gpu.audio(16_000 * 5), Duration::from_secs(50));
        assert_eq!(gpu.run(16_000), gpu.run_floor);
        assert_eq!(gpu.run(16_000 * 10), Duration::from_secs(100));
        // A ten-minute dictation gets 100 minutes before it counts as hung.
        assert_eq!(gpu.run(16_000 * 600), Duration::from_secs(6000));

        let cpu = Deadlines::for_device(false);
        assert_eq!(cpu, CPU_DEADLINES);
        assert_eq!(cpu.audio(16_000), cpu.call_floor);
        assert_eq!(cpu.audio(16_000 * 10), Duration::from_secs(200));
        assert_eq!(cpu.run(16_000 * 5), cpu.run_floor);
        assert_eq!(cpu.run(16_000 * 60), Duration::from_secs(1200));
        // CPU is never stricter than GPU.
        for secs in [0, 1, 5, 30, 120, 600] {
            assert!(cpu.audio(secs * 16_000) >= gpu.audio(secs * 16_000));
            assert!(cpu.run(secs * 16_000) >= gpu.run(secs * 16_000));
        }

        assert_eq!(ms_to_samples(1500), 24_000);
        assert_eq!(ms_to_samples(-5), 0);
    }

    #[test]
    fn a_lost_run_is_retried_once_and_a_cpu_hang_never() {
        // A crash on the GPU: retried (on the CPU, since the GPU is marked).
        assert_eq!(after_lost_run(false, true, false), AfterLoss::Retry);
        // A hang on the GPU is the GPU's fault as often as a crash is.
        assert_eq!(after_lost_run(true, true, false), AfterLoss::Retry);
        // A crash on the CPU: one fresh worker, in case it was a one-off.
        assert_eq!(after_lost_run(false, false, false), AfterLoss::Retry);
        // A hang on the CPU already had the generous deadline.
        assert_eq!(after_lost_run(true, false, false), AfterLoss::GiveUp);
        // Never twice.
        for hung in [false, true] {
            for on_gpu in [false, true] {
                assert_eq!(after_lost_run(hung, on_gpu, true), AfterLoss::GiveUp);
            }
        }
    }

    #[test]
    fn a_waiting_call_is_killed_at_its_deadline_and_not_before() {
        let deadline = Duration::from_secs(60);
        assert_eq!(
            wait_step(Duration::ZERO, deadline, false, None, true),
            WaitStep::Wait(CANCEL_POLL)
        );
        // The last slice ends exactly at the deadline.
        assert_eq!(
            wait_step(Duration::from_millis(59_990), deadline, false, None, true),
            WaitStep::Wait(Duration::from_millis(10))
        );
        assert_eq!(
            wait_step(deadline, deadline, false, None, true),
            WaitStep::KillHung
        );
    }

    #[test]
    fn a_cancel_asks_first_and_stops_the_worker_only_when_it_must() {
        let deadline = Duration::from_secs(60);
        // A model that aborts is asked to.
        assert_eq!(
            wait_step(Duration::from_secs(1), deadline, true, None, true),
            WaitStep::SendCancel
        );
        // One that can't is stopped at once, rather than after the grace.
        assert_eq!(
            wait_step(Duration::from_secs(1), deadline, true, None, false),
            WaitStep::KillCancelled
        );
        // Asked: it gets the grace, then the worker is stopped.
        assert_eq!(
            wait_step(deadline * 2, deadline, true, Some(Duration::ZERO), true),
            WaitStep::Wait(CANCEL_POLL),
            "a cancel in flight is not a hang"
        );
        assert_eq!(
            wait_step(Duration::ZERO, deadline, true, Some(CANCEL_GRACE), true),
            WaitStep::KillCancelled
        );
        assert!(
            CANCEL_GRACE < Duration::from_secs(1),
            "Esc must feel prompt"
        );
    }

    #[test]
    fn a_cpu_target_or_a_gpu_free_host_gets_a_cpu_only_worker() {
        assert!(cpu_only_worker(&LoadTarget::Cpu, true));
        assert!(!cpu_only_worker(&LoadTarget::Auto, true));
        assert!(!cpu_only_worker(&LoadTarget::Device("k".into()), true));
        for target in [
            LoadTarget::Cpu,
            LoadTarget::Auto,
            LoadTarget::Device("k".into()),
        ] {
            assert!(cpu_only_worker(&target, false));
        }
    }

    #[test]
    fn after_a_gpu_failure_every_fresh_worker_loads_on_the_processor() {
        let device = LoadTarget::Device("k".into());
        assert_eq!(effective_target(&device, false), device);
        assert_eq!(effective_target(&LoadTarget::Auto, false), LoadTarget::Auto);
        assert_eq!(effective_target(&device, true), LoadTarget::Cpu);
        assert_eq!(effective_target(&LoadTarget::Auto, true), LoadTarget::Cpu);
        assert_eq!(effective_target(&LoadTarget::Cpu, true), LoadTarget::Cpu);
    }

    #[test]
    fn only_a_fault_of_the_worker_counts_against_the_gpu() {
        assert!(Failure::Crashed("x".into()).worker_faulted());
        assert!(Failure::Hung(Duration::from_secs(1)).worker_faulted());
        let stopped = Failure::Cancelled { worker_lost: true };
        assert!(stopped.worker_lost());
        assert!(!stopped.worker_faulted());
        assert!(!Failure::Cancelled { worker_lost: false }.worker_lost());
        assert!(!Failure::Refused("no model".into()).worker_lost());
    }

    #[test]
    fn engine_errors_keep_what_the_app_acts_on() {
        assert_eq!(
            EngineError::from(Failure::Remote {
                kind: ErrorKind::InputTooShort,
                message: "m".into()
            }),
            EngineError::InputTooShort("m".into())
        );
        assert_eq!(
            EngineError::from(Failure::Cancelled { worker_lost: true }),
            EngineError::Cancelled
        );
        assert!(matches!(
            EngineError::from(Failure::Remote {
                kind: ErrorKind::Backend,
                message: "m".into()
            }),
            EngineError::Failed(_)
        ));
    }

    #[test]
    fn the_feed_window_bounds_a_stream_and_never_blocks_a_cancelled_one() {
        let window = Arc::new(FeedWindow::default());
        let cancel = CancelToken::new();
        let closed = AtomicBool::new(false);
        for _ in 0..FEED_WINDOW {
            window.acquire(&cancel, &closed);
        }
        assert_eq!(window.queued(), FEED_WINDOW);

        // Full: the next feed waits until the owner frees a slot.
        let waiter = {
            let window = Arc::clone(&window);
            thread::spawn(move || {
                let started = Instant::now();
                window.acquire(&CancelToken::new(), &AtomicBool::new(false));
                started.elapsed()
            })
        };
        thread::sleep(Duration::from_millis(100));
        drop(WindowSlot(Arc::clone(&window)));
        let waited = waiter.join().unwrap();
        assert!(waited >= Duration::from_millis(90), "{waited:?}");
        assert_eq!(window.queued(), FEED_WINDOW);

        // A cancelled stream's feed never waits.
        cancel.cancel();
        let started = Instant::now();
        window.acquire(&cancel, &closed);
        assert!(started.elapsed() < Duration::from_millis(50));

        // Releasing more than was taken cannot underflow.
        for _ in 0..FEED_WINDOW * 3 {
            window.release();
        }
        assert_eq!(window.queued(), 0);
    }

    #[test]
    fn worker_log_lines_are_told_from_native_output() {
        // Structured lines are re-logged; this only checks they parse without
        // panicking on odd input (a missing field, an unknown level).
        forward_worker_log(&format!("{LOG_LINE_PREFIX}DEBUG\tworker\tloaded"));
        forward_worker_log(&format!("{LOG_LINE_PREFIX}NOPE\tworker\tx"));
        forward_worker_log(&format!("{LOG_LINE_PREFIX}INFO"));
        forward_worker_log("GGML_ASSERT(n_inputs < GGML_SCHED_MAX_SPLIT_INPUTS) failed");
        forward_worker_log("");
    }
}

/// End-to-end checks against a real worker and real models. The crash and
/// hang tests need a debug worker executable (fault injection is compiled out
/// of release builds). Ignored by default; run with the app built first:
///
/// `cargo build && cargo test --lib engine_supervisor -- --ignored --nocapture --test-threads=1`
///
/// The worker defaults to the `speakoflow` binary beside the test harness
/// (`SPEAKOFLOW_TRANSCRIBE_WORKER_EXE` overrides it). Models come from the
/// installed app's folder (`SPEAKOFLOW_MODELS_DIR`), the GGUF Parakeet TDT v3
/// from `SPEAKOFLOW_PARAKEET_TDT`, the recording from the newest one in the
/// app's recordings folder (`SPEAKOFLOW_LIVE_WAV`). Transcript text is never
/// printed, only its length.
#[cfg(test)]
mod live_tests {
    use super::*;
    use std::path::Path;

    const RATE: usize = 16_000;

    fn app_data() -> PathBuf {
        let roaming = std::env::var_os("APPDATA").expect("APPDATA is set on Windows");
        PathBuf::from(roaming).join("com.abhishekbarali.speakoflow")
    }

    fn models_dir() -> PathBuf {
        std::env::var_os("SPEAKOFLOW_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| app_data().join("models"))
    }

    fn parakeet_tdt() -> PathBuf {
        std::env::var_os("SPEAKOFLOW_PARAKEET_TDT")
            .map(PathBuf::from)
            .unwrap_or_else(|| models_dir().join("parakeet-tdt-0.6b-v3-Q8_0.gguf"))
    }

    fn model(file: &str) -> PathBuf {
        let path = models_dir().join(file);
        assert!(path.exists(), "missing model {}", path.display());
        path
    }

    fn read_wav(path: &Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).expect("open the recording");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate as usize, RATE, "recordings are 16 kHz");
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
        samples.into_iter().step_by(channels).collect()
    }

    /// The newest recording of 3 to 20 seconds of audio, cut to 20 s.
    fn recording() -> Vec<f32> {
        if let Some(path) = std::env::var_os("SPEAKOFLOW_LIVE_WAV") {
            let mut pcm = read_wav(Path::new(&path));
            pcm.truncate(20 * RATE);
            return pcm;
        }
        let dir = app_data().join("recordings");
        let mut wavs: Vec<_> = std::fs::read_dir(&dir)
            .expect("read the recordings folder")
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        wavs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, path) in wavs {
            let mut pcm = read_wav(&path);
            if pcm.len() >= 3 * RATE {
                pcm.truncate(20 * RATE);
                return pcm;
            }
        }
        panic!("no recording of at least 3 s in {}", dir.display());
    }

    /// The recording repeated to at least `secs` seconds.
    fn long_pcm(secs: usize) -> Vec<f32> {
        let clip = recording();
        clip.repeat((secs * RATE).div_ceil(clip.len()))
    }

    fn spec(path: PathBuf) -> LoadSpec {
        LoadSpec {
            path,
            target: LoadTarget::Auto,
        }
    }

    fn secs(pcm: &[f32]) -> f64 {
        pcm.len() as f64 / RATE as f64
    }

    /// Faults are read by each worker at spawn, so set them before loading.
    fn set_fault(fault: Option<&str>, gpu_only: bool) {
        let marker = std::env::temp_dir().join("speakoflow-worker-fault-once");
        let _ = std::fs::remove_file(&marker);
        std::env::remove_var("SPEAKOFLOW_WORKER_FAULT_ONCE");
        std::env::remove_var("SPEAKOFLOW_WORKER_FAULT_GPU_ONLY");
        match fault {
            Some(fault) => {
                std::env::set_var("SPEAKOFLOW_WORKER_FAULT", fault);
                if gpu_only {
                    std::env::set_var("SPEAKOFLOW_WORKER_FAULT_GPU_ONLY", "1");
                } else {
                    std::env::set_var("SPEAKOFLOW_WORKER_FAULT_ONCE", marker);
                }
            }
            None => std::env::remove_var("SPEAKOFLOW_WORKER_FAULT"),
        }
    }

    fn worker_count() -> usize {
        LIVE_WORKERS.load(Ordering::SeqCst)
    }

    /// Feed 80 ms chunks, as the app's stream worker does.
    fn stream_all(
        engine: &EngineSupervisor,
        pcm: &[f32],
        pad_tail: bool,
    ) -> Result<Option<Finalized>, EngineError> {
        let updates = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&updates);
        let stream = engine.start_stream(
            RunOptions::default(),
            StreamOptions {
                commit_policy: transcribe_cpp::CommitPolicy::Auto,
                ..Default::default()
            },
            CancelToken::new(),
            move |progress| {
                if progress.text.is_some() {
                    counted.fetch_add(1, Ordering::Relaxed);
                }
            },
        )?;
        for chunk in pcm.chunks(1280) {
            stream.feed(chunk.to_vec());
        }
        if pad_tail {
            stream.feed(vec![0.0; RATE * 560 / 1000]);
        }
        let result = stream.finalize(false);
        eprintln!(
            "stream: {} live text updates",
            updates.load(Ordering::Relaxed)
        );
        result
    }

    /// (a) Batch through the worker on the automatic device (the dedicated
    /// GPU here), timed against the same model run in this process, and the
    /// cost of moving the audio measured on its own.
    #[test]
    #[ignore = "needs the built app, a model, a recording and a GPU"]
    fn live_batch_through_the_worker_against_in_process() {
        set_fault(None, false);
        let path = parakeet_tdt();
        let pcm = recording();

        let engine = EngineSupervisor::new(true);
        let started = Instant::now();
        let info = engine.load(spec(path.clone())).expect("load");
        eprintln!(
            "worker: spawn + load {} ms on {} ({:?}, on_gpu={}, threads={})",
            started.elapsed().as_millis(),
            info.device,
            info.device_type,
            info.on_gpu,
            info.cpu_threads
        );
        let mut worker_ms = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            let text = engine
                .transcribe(pcm.clone(), RunOptions::default(), None)
                .expect("transcription")
                .text;
            worker_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            assert!(!text.trim().is_empty());
        }
        engine.unload().wait();
        assert_eq!(worker_count(), 0);

        crate::managers::transcription::init_transcribe_cpp();
        let model = transcribe_cpp::Model::load(&path).expect("in-process load");
        let on_gpu = model
            .device()
            .is_ok_and(|d| DeviceInfo::from_device(&d).is_gpu());
        let threads =
            crate::managers::transcription::transcribe_cpp_session_threads(&model.arch(), on_gpu);
        let mut session = model
            .session_with(&transcribe_cpp::SessionOptions {
                n_threads: threads,
                ..Default::default()
            })
            .expect("session");
        let mut in_process_ms = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            session
                .run(&pcm, &RunOptions::default())
                .expect("in-process run");
            in_process_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        // The first run of each includes shader warm-up; compare the rest.
        let median = |v: &[f64]| {
            let mut rest = v[1..].to_vec();
            rest.sort_by(f64::total_cmp);
            rest[rest.len() / 2]
        };
        eprintln!(
            "{:.1} s of audio: worker first {:.0} ms, median {:.0} ms; in-process first {:.0} ms, median {:.0} ms",
            secs(&pcm),
            worker_ms[0],
            median(&worker_ms),
            in_process_ms[0],
            median(&in_process_ms)
        );
    }

    /// The transport alone: a ten-minute recording encoded, sent through a
    /// real pipe and decoded, which is what a long dictation adds.
    #[test]
    #[ignore = "measures IPC cost; needs nothing but time"]
    fn live_ipc_cost_of_a_ten_minute_recording() {
        let pcm: Vec<f32> = (0..10 * 60 * RATE)
            .map(|i| (i % 97) as f32 / 97.0)
            .collect();
        let request = Request::Run {
            id: 1,
            options: RunOptions::default(),
        };
        let started = Instant::now();
        let frame = encode_message(&request, Some(&pcm)).unwrap();
        let encoded = started.elapsed();
        let (mut reader, mut writer) = std::io::pipe().expect("pipe");
        let bytes = frame.len();
        let writer_thread = thread::spawn(move || {
            writer.write_all(&frame).unwrap();
        });
        let started = Instant::now();
        let (_, back): (Request, Vec<f32>) = read_message(&mut BufReader::new(&mut reader))
            .unwrap()
            .unwrap();
        let transferred = started.elapsed();
        writer_thread.join().unwrap();
        assert_eq!(back.len(), pcm.len());
        eprintln!(
            "10 min of PCM = {:.1} MB: encode {} ms, pipe + decode {} ms",
            bytes as f64 / 1e6,
            encoded.as_millis(),
            transferred.as_millis()
        );
    }

    /// (b) A native stream (Nemotron) and the Parakeet buffered stream
    /// (Parakeet Unified, with the tail pad) through the worker.
    #[test]
    #[ignore = "needs the built app, the streaming models and a recording"]
    fn live_streams_through_the_worker() {
        set_fault(None, false);
        let pcm = recording();
        let engine = EngineSupervisor::new(true);
        for (file, pads) in [
            ("nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf", false),
            ("parakeet-unified-en-0.6b-Q8_0.gguf", true),
        ] {
            let info = engine.load(spec(model(file))).expect("load");
            assert!(info.capabilities.supports_streaming, "{file} streams");
            assert_eq!(info.buffered_stream, pads, "{file} buffered stream");
            let started = Instant::now();
            let finalized = stream_all(&engine, &pcm, pads)
                .expect("not cancelled")
                .expect("a stream result");
            eprintln!(
                "{file}: streamed {:.1} s in {} ms (finalize {} ms), {} chars, on {}",
                secs(&pcm),
                started.elapsed().as_millis(),
                finalized.elapsed.as_millis(),
                finalized.text.full.chars().count(),
                info.device
            );
            assert!(!finalized.text.full.trim().is_empty());
        }
        drop(engine);
        thread::sleep(Duration::from_millis(500));
        assert_eq!(worker_count(), 0);
    }

    /// (c) The worker aborts during inference on the GPU: the run comes back
    /// with text from a CPU-only worker, and later loads stay on the CPU until
    /// the user changes the device.
    #[test]
    #[ignore = "needs the built debug app, a model, a recording and a GPU"]
    fn live_gpu_crash_during_inference_recovers_on_cpu() {
        let pcm = recording();
        set_fault(Some("abort@run"), true);
        let engine = EngineSupervisor::new(true);
        let info = engine.load(spec(parakeet_tdt())).expect("load");
        assert!(info.on_gpu, "needs a GPU to test GPU-only faults");
        let started = Instant::now();
        let transcript = engine
            .transcribe(pcm.clone(), RunOptions::default(), None)
            .expect("recovered on CPU");
        let loaded = engine.loaded().expect("still loaded");
        eprintln!(
            "crash at run: recovered in {} ms on {} (on_gpu={}), {} chars",
            started.elapsed().as_millis(),
            loaded.device,
            loaded.on_gpu,
            transcript.text.chars().count()
        );
        assert!(!transcript.text.trim().is_empty());
        assert!(engine.shared.gpu_unavailable());
        assert!(!loaded.on_gpu);
        assert!(!engine.load(spec(parakeet_tdt())).expect("load").on_gpu);
        set_fault(None, false);
        engine.retry_gpu("test");
        assert!(engine.load(spec(parakeet_tdt())).expect("load").on_gpu);
    }

    /// (c) A crash that also happens on the CPU fails the run with a plain
    /// error, and the supervisor keeps working afterwards.
    #[test]
    #[ignore = "needs the built debug app, a model and a recording"]
    fn live_crash_everywhere_fails_plainly() {
        let pcm = recording();
        set_fault(Some("segv@run"), false);
        std::env::remove_var("SPEAKOFLOW_WORKER_FAULT_ONCE");
        let engine = EngineSupervisor::new(true);
        engine.load(spec(parakeet_tdt())).expect("load");
        let result = engine.transcribe(pcm.clone(), RunOptions::default(), None);
        eprintln!(
            "crash everywhere: {:?}",
            result.as_ref().map(|t| t.text.len())
        );
        assert!(matches!(result, Err(EngineError::Failed(_))));
        set_fault(None, false);
        engine.retry_gpu("test");
        engine.load(spec(parakeet_tdt())).expect("load again");
        assert!(engine
            .transcribe(pcm, RunOptions::default(), None)
            .is_ok_and(|t| !t.text.trim().is_empty()));
    }

    /// (c) A worker that aborts while loading the model onto the GPU: the
    /// load is retried in a CPU-only worker and succeeds there.
    #[test]
    #[ignore = "needs the built debug app, a model, a recording and a GPU"]
    fn live_gpu_crash_during_load_loads_on_cpu() {
        set_fault(Some("abort@load"), true);
        let engine = EngineSupervisor::new(true);
        let started = Instant::now();
        let info = engine.load(spec(parakeet_tdt())).expect("load on CPU");
        eprintln!(
            "crash at load: loaded in {} ms on {} (on_gpu={})",
            started.elapsed().as_millis(),
            info.device,
            info.on_gpu
        );
        assert!(!info.on_gpu);
        assert!(engine.shared.gpu_unavailable());
        assert!(engine
            .transcribe(recording(), RunOptions::default(), None)
            .is_ok_and(|t| !t.text.trim().is_empty()));
        set_fault(None, false);
    }

    /// A GPU driver that crashes while its backend registers, before the
    /// hello: only a CPU-only worker survives it, so that is where the model
    /// loads, and the device list is not probed again.
    #[test]
    #[ignore = "needs the built debug app, a model and a recording"]
    fn live_gpu_backend_init_crash_uses_a_cpu_only_worker() {
        set_fault(Some("abort@init"), true);
        let engine = EngineSupervisor::new(true);
        let info = engine.load(spec(parakeet_tdt())).expect("load");
        assert!(!info.on_gpu, "loaded on '{}'", info.backend);
        assert!(engine.shared.gpu_unavailable());
        assert!(engine.devices().is_none());
        assert!(engine
            .transcribe(recording(), RunOptions::default(), None)
            .is_ok());
        set_fault(None, false);
    }

    /// (d) A run that hangs on the GPU is killed at its deadline (shortened
    /// here by a short clip: the GPU floor is 60 s) and retried on the CPU.
    #[test]
    #[ignore = "needs the built debug app, a model, a recording and a GPU; takes a minute"]
    fn live_gpu_hang_is_killed_and_recovered() {
        let pcm = recording();
        set_fault(Some("hang@run"), true);
        let engine = EngineSupervisor::new(true);
        assert!(engine.load(spec(parakeet_tdt())).expect("load").on_gpu);
        let started = Instant::now();
        let transcript = engine
            .transcribe(pcm, RunOptions::default(), None)
            .expect("recovered after the hang");
        let took = started.elapsed();
        eprintln!(
            "hang at run: recovered after {:.1} s, {} chars",
            took.as_secs_f64(),
            transcript.text.chars().count()
        );
        assert!(took >= GPU_DEADLINES.run_floor);
        assert!(!engine.loaded().expect("loaded").on_gpu);
        set_fault(None, false);
    }

    /// (d) A finalize that hangs is killed at its deadline, the stream is
    /// reported lost (so the app falls back to batch), and the batch run then
    /// works in a fresh worker.
    #[test]
    #[ignore = "needs the built debug app, a streaming model and a recording; takes 30 s"]
    fn live_hung_finalize_is_killed_and_batch_recovers() {
        let pcm = recording();
        set_fault(Some("hang@finalize"), false);
        let engine = EngineSupervisor::new(true);
        engine
            .load(spec(model("nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf")))
            .expect("load");
        let started = Instant::now();
        assert!(stream_all(&engine, &pcm, false)
            .expect("not cancelled")
            .is_none());
        eprintln!(
            "hung finalize gave up after {:.1} s",
            started.elapsed().as_secs_f64()
        );
        set_fault(None, false);
        let started = Instant::now();
        let transcript = engine
            .transcribe(pcm, RunOptions::default(), None)
            .expect("batch after the hang");
        eprintln!(
            "batch after the hang: {} ms, {} chars",
            started.elapsed().as_millis(),
            transcript.text.chars().count()
        );
    }

    /// (e) Cancel during a long batch run returns promptly, through the
    /// model's own abort, with the worker and its model kept.
    #[test]
    #[ignore = "needs the built app, a model and a recording"]
    fn live_cancel_stops_a_long_run_promptly() {
        set_fault(None, false);
        let engine = EngineSupervisor::new(true);
        let info = engine
            .load(spec(model("whisper-medium-q4_1.bin")))
            .expect("load");
        eprintln!(
            "cancel test on {} (abort supported: {})",
            info.device, info.supports_cancellation
        );
        // Ten minutes of audio, so the run is still decoding when the cancel
        // lands even on a warm GPU (two minutes once finished inside 2 s).
        let pcm = long_pcm(600);
        let cancel = CancelToken::new();
        let ran = Instant::now();
        let run = {
            let engine = engine.clone();
            let cancel = cancel.clone();
            thread::spawn(move || engine.transcribe(pcm, RunOptions::default(), Some(cancel)))
        };
        thread::sleep(Duration::from_millis(1500));
        assert!(
            !run.is_finished(),
            "the run ended before it could be cancelled"
        );
        let started = Instant::now();
        cancel.cancel();
        let result = run.join().unwrap();
        let took = started.elapsed();
        eprintln!(
            "cancelled {} ms after cancel() ({} ms into the run); workers alive: {}",
            took.as_millis(),
            ran.elapsed().as_millis(),
            worker_count()
        );
        assert_eq!(result.err(), Some(EngineError::Cancelled));
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert!(engine.loaded().is_some());
        assert!(!engine.shared.gpu_unavailable());
        let started = Instant::now();
        engine
            .transcribe(recording(), RunOptions::default(), None)
            .expect("next run");
        eprintln!("next run took {} ms", started.elapsed().as_millis());
    }

    /// (e) Cancel a run that hangs (a model that never checks its abort):
    /// stopped after the grace, no retry, no GPU flag, the next run works.
    #[test]
    #[ignore = "needs the built debug app, a model and a recording"]
    fn live_cancel_stops_a_hung_run() {
        let pcm = recording();
        set_fault(Some("hang@run"), false);
        let engine = EngineSupervisor::new(true);
        engine.load(spec(parakeet_tdt())).expect("load");
        let cancel = CancelToken::new();
        let run = {
            let engine = engine.clone();
            let cancel = cancel.clone();
            let pcm = pcm.clone();
            thread::spawn(move || engine.transcribe(pcm, RunOptions::default(), Some(cancel)))
        };
        let marker = std::env::temp_dir().join("speakoflow-worker-fault-once");
        let waited = Instant::now();
        while !marker.exists() {
            assert!(waited.elapsed() < Duration::from_secs(60), "never hung");
            thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        cancel.cancel();
        let result = run.join().unwrap();
        let took = started.elapsed();
        eprintln!("hung run cancelled {} ms after cancel()", took.as_millis());
        assert_eq!(result.err(), Some(EngineError::Cancelled));
        assert!(took < Duration::from_secs(2));
        assert!(!engine.shared.gpu_unavailable());
        engine
            .transcribe(pcm, RunOptions::default(), None)
            .expect("the next run starts a fresh worker");
        set_fault(None, false);
    }

    /// A batch run that arrives during a live stream waits for it instead of
    /// ending it, and both succeed.
    #[test]
    #[ignore = "needs the built app, a streaming model and a recording"]
    fn live_a_batch_run_waits_for_a_stream() {
        set_fault(None, false);
        let pcm = recording();
        let engine = EngineSupervisor::new(true);
        engine
            .load(spec(model("nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf")))
            .expect("load");
        let stream = engine
            .start_stream(
                RunOptions::default(),
                StreamOptions::default(),
                CancelToken::new(),
                |_| {},
            )
            .expect("stream");
        let batch = {
            let engine = engine.clone();
            let pcm = pcm.clone();
            thread::spawn(move || engine.transcribe(pcm, RunOptions::default(), None))
        };
        for chunk in pcm.chunks(1280) {
            stream.feed(chunk.to_vec());
        }
        let finalized = stream.finalize(false).expect("finalize").expect("result");
        assert!(!finalized.text.full.trim().is_empty());
        assert!(batch
            .join()
            .unwrap()
            .is_ok_and(|t| !t.text.trim().is_empty()));
    }

    /// An unload requested while a dictation streams (a model switch, a
    /// Speech processing change, an "Immediately" unload after another
    /// transcription) waits for the stream to end instead of killing it.
    #[test]
    #[ignore = "needs the built app, a streaming model and a recording"]
    fn live_an_unload_waits_for_a_stream() {
        set_fault(None, false);
        let pcm = recording();
        let engine = EngineSupervisor::new(true);
        engine
            .load(spec(model("nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf")))
            .expect("load");
        let stream = engine
            .start_stream(
                RunOptions::default(),
                StreamOptions::default(),
                CancelToken::new(),
                |_| {},
            )
            .expect("stream");
        let (half, rest) = pcm.split_at(pcm.len() / 2);
        for chunk in half.chunks(1280) {
            stream.feed(chunk.to_vec());
        }
        let unloading = engine.unload();
        assert!(engine.loaded().is_none(), "reads as unloaded at once");
        for chunk in rest.chunks(1280) {
            stream.feed(chunk.to_vec());
        }
        let finalized = stream.finalize(false).expect("finalize").expect("result");
        assert!(!finalized.text.full.trim().is_empty());
        assert!(unloading.wait_timeout(Duration::from_secs(10)));
    }

    /// Quitting does not wait for a live stream, and a model switch that was
    /// waiting for that stream is answered instead of starting a new worker
    /// once the app has stopped this one.
    #[test]
    #[ignore = "needs the built app, a streaming model and a recording"]
    fn live_quitting_drops_a_load_waiting_for_a_stream() {
        set_fault(None, false);
        let pcm = recording();
        let engine = EngineSupervisor::new(true);
        let model_path = model("nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf");
        engine.load(spec(model_path.clone())).expect("load");
        let stream = engine
            .start_stream(
                RunOptions::default(),
                StreamOptions::default(),
                CancelToken::new(),
                |_| {},
            )
            .expect("stream");
        for chunk in pcm[..pcm.len() / 2].chunks(1280) {
            stream.feed(chunk.to_vec());
        }
        let switching = {
            let engine = engine.clone();
            std::thread::spawn(move || engine.load(spec(model_path)))
        };
        // Let the load reach the queue and be deferred behind the stream.
        std::thread::sleep(Duration::from_millis(300));
        assert!(engine.unload_now().wait_timeout(Duration::from_secs(10)));
        let switched = switching.join().expect("load thread");
        assert!(
            switched.is_err(),
            "the waiting load is refused: {switched:?}"
        );
        drop(stream);
        std::thread::sleep(Duration::from_millis(500));
        assert!(engine.loaded().is_none());
        assert_eq!(worker_count(), 0, "no worker started after quitting");
    }

    /// (f) The worker exits when the supervisor is dropped (its stdin closes),
    /// and none is left behind.
    #[test]
    #[ignore = "needs the built app and a model"]
    fn live_dropping_the_supervisor_ends_its_worker() {
        set_fault(None, false);
        let engine = EngineSupervisor::new(true);
        engine.load(spec(parakeet_tdt())).expect("load");
        assert_eq!(worker_count(), 1);
        let started = Instant::now();
        drop(engine);
        while worker_count() > 0 {
            assert!(
                started.elapsed() < SHUTDOWN_GRACE + Duration::from_secs(1),
                "the worker outlived its supervisor"
            );
            thread::sleep(Duration::from_millis(10));
        }
        eprintln!(
            "worker gone {} ms after the drop",
            started.elapsed().as_millis()
        );
    }

    /// (f) A worker hung in native code still exits when its stdin closes,
    /// without being killed, so a parent that dies never leaves one behind.
    #[test]
    #[ignore = "needs the built debug app, a model and a recording"]
    fn live_hung_worker_exits_when_stdin_closes() {
        set_fault(Some("hang@run"), false);
        let mut worker = Worker::spawn(false).expect("spawn");
        worker
            .call(
                &Request::Hello {
                    list_devices: false,
                },
                None,
                HELLO_TIMEOUT,
                None,
            )
            .expect("hello");
        worker.load(&spec(parakeet_tdt())).expect("load");
        let run = Request::Run {
            id: 1,
            options: RunOptions::default(),
        };
        assert!(worker
            .control
            .send(encode_message(&run, Some(&recording())).unwrap()));
        thread::sleep(Duration::from_secs(1));
        worker.control.close_stdin();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = lock(&worker.control.child).try_wait().unwrap() {
                break status;
            }
            assert!(
                started.elapsed() < SHUTDOWN_GRACE,
                "worker outlived its stdin"
            );
            thread::sleep(Duration::from_millis(10));
        };
        eprintln!(
            "hung worker exited {} ms after stdin closed: {status}",
            started.elapsed().as_millis()
        );
        assert!(status.success());
        set_fault(None, false);
    }

    /// Device listing through a short-lived worker leaves nothing running.
    #[test]
    #[ignore = "needs the built app"]
    fn live_device_probe_leaves_no_worker() {
        set_fault(None, false);
        let engine = EngineSupervisor::new(true);
        let started = Instant::now();
        let devices = engine.devices().expect("devices listed");
        eprintln!(
            "probe: {} devices in {} ms: {:?}",
            devices.len(),
            started.elapsed().as_millis(),
            devices
                .iter()
                .map(|d| (d.label().to_string(), d.device_type))
                .collect::<Vec<_>>()
        );
        assert_eq!(worker_count(), 0);
    }
}
