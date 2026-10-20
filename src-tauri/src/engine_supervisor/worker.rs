//! The worker process: the only place transcribe.cpp native code runs. Owns
//! the model, session and stream, and serves [`Request`]s until the parent
//! closes its stdin.
//!
//! A dedicated thread reads stdin, so the parent's writes never block, a
//! [`Request::Cancel`] reaches a run that is still decoding, and the worker
//! exits the moment the parent goes away, even if it is hung in native code.

use super::protocol::{
    read_message, write_message, DeviceInfo, LoadTarget, LoadedInfo, Request, Response,
};
use super::{CPU_ONLY_FLAG, LOG_LEVEL_ENV};
use log::{debug, error, info, warn, LevelFilter, Log, Metadata, Record};
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use transcribe_cpp::{
    Backend, BackendMask, CancelToken, ErrorKind, ExtSlot, Feature, Model, ModelOptions, Session,
    SessionOptions, Stream,
};

/// Prefix on worker log lines so the parent can tell them apart from raw
/// native output (e.g. a `GGML_ASSERT` message right before an abort).
pub(super) const LOG_LINE_PREFIX: &str = "\u{1}";

/// A request as handed from the stdin reader to the main loop.
struct Incoming {
    request: Request,
    pcm: Vec<f32>,
}

/// The loaded model's session and what the parent was told about it.
struct Loaded {
    session: Session,
    info: LoadedInfo,
}

pub fn run() -> i32 {
    #[cfg(unix)]
    ignore_app_signals();
    #[cfg(target_os = "linux")]
    set_process_name();
    // Take the real stdout for the protocol before any native code runs, and
    // point fd 1 at stderr: ggml writes to stdout in places, and any stray
    // byte there would corrupt the protocol stream.
    let protocol_out = match take_stdout_for_protocol() {
        Ok(file) => file,
        Err(e) => {
            eprintln!("transcribe worker: failed to claim stdout: {e}");
            return 2;
        }
    };
    init_logger();

    let cancels = Arc::new(Cancels::default());
    let (requests_tx, requests) = mpsc::channel();
    {
        let cancels = Arc::clone(&cancels);
        if let Err(e) = thread::Builder::new()
            .name("transcribe-worker-in".into())
            .spawn(move || read_requests(requests_tx, &cancels))
        {
            error!("Failed to start the request reader: {}", e);
            return 2;
        }
    }

    transcribe_cpp::init_logging();
    // Registering a GPU backend runs its driver code, so this is where a
    // broken driver crashes or hangs, before the hello is answered.
    let cpu_only = std::env::args_os().any(|arg| arg == CPU_ONLY_FLAG);
    inject_fault("init", !cpu_only);
    let init = if cpu_only {
        transcribe_cpp::init_backends_with(None::<&Path>, BackendMask::CPU)
    } else {
        transcribe_cpp::init_backends_default()
    };
    // Init only fails when no compute device registered at all, which no
    // other worker could fix either: the hello reports it.
    let init_error = match init {
        Ok(()) => {
            log_registered_devices(cpu_only);
            None
        }
        Err(e) => Some(format!("Failed to initialize transcribe.cpp backends: {e}")),
    };

    let mut output = protocol_out;
    let mut loaded: Option<Loaded> = None;

    while let Ok(Incoming { request, pcm }) = requests.recv() {
        let finished_run = matches!(request, Request::Run { .. });
        let response = match request {
            Request::Hello { list_devices } => match &init_error {
                Some(message) => Response::Error(message.clone()),
                None => Response::Hello {
                    devices: list_devices.then(|| {
                        inject_fault("list", !cpu_only);
                        transcribe_cpp::devices()
                            .iter()
                            .map(DeviceInfo::from_device)
                            .collect()
                    }),
                },
            },
            Request::Load { path, target } => {
                // Free any previous model first to avoid holding two at once.
                loaded = None;
                match load(&path, target, cpu_only) {
                    Ok(model) => {
                        let info = model.info.clone();
                        loaded = Some(model);
                        Response::Loaded(info)
                    }
                    Err(response) => response,
                }
            }
            Request::Run { id, options } => match loaded.as_mut() {
                Some(Loaded { session, info }) => {
                    inject_fault("run", info.on_gpu);
                    let token = cancels.begin(id);
                    let response = if token.is_cancelled() {
                        aborted("cancelled before it started")
                    } else {
                        session.set_cancel_token(&token);
                        let result = session.run(&pcm, &options);
                        session.clear_cancel_token();
                        match result {
                            Ok(transcript) => Response::Transcript(transcript),
                            Err(e) => Response::failed(&e),
                        }
                    };
                    cancels.end();
                    response
                }
                None => not_loaded(),
            },
            Request::StreamBegin { id, run, stream } => match loaded.as_mut() {
                Some(Loaded { session, info }) => {
                    let on_gpu = info.on_gpu;
                    let token = cancels.begin(id);
                    session.set_cancel_token(&token);
                    let outcome = match session.stream(&run, &stream) {
                        Ok(stream) => {
                            if write_message(&mut output, &Response::Ok, None).is_err() {
                                return 1;
                            }
                            serve_stream(stream, &requests, &mut output, on_gpu)
                                .map(|()| None)
                                .map_err(|e| {
                                    error!("Protocol write failed during stream: {}", e);
                                })
                        }
                        Err(e) => Ok(Some(Response::failed(&e))),
                    };
                    session.clear_cancel_token();
                    cancels.end();
                    match outcome {
                        // The stream answered every request itself.
                        Ok(None) => {
                            trim_freed_memory();
                            continue;
                        }
                        Ok(Some(response)) => response,
                        Err(()) => return 1,
                    }
                }
                None => not_loaded(),
            },
            Request::Feed | Request::Finalize { .. } | Request::StreamReset => {
                Response::Error("no active stream".to_string())
            }
            // Handled by the reader; never forwarded.
            Request::Cancel { .. } => continue,
        };
        if let Err(e) = write_message(&mut output, &response, None) {
            error!("Protocol write failed: {}", e);
            return 1;
        }
        // After the reply, so it never delays the transcript.
        if finished_run {
            drop(pcm);
            trim_freed_memory();
        }
    }
    // Unreachable in practice: the reader exits the process on EOF.
    0
}

/// Which run or stream a [`Request::Cancel`] stops.
///
/// The cancel travels behind the request it cancels on the same pipe, and the
/// reader may see it before the main loop has started that request, so an
/// early cancel is remembered and applied when the request begins. Only the
/// request in flight is ever cancelled (the parent cancels only what it is
/// waiting on), so one remembered id is enough, and ids are never reused.
#[derive(Default)]
struct Cancels {
    state: Mutex<CancelState>,
}

#[derive(Default)]
struct CancelState {
    current: Option<(u64, CancelToken)>,
    early: Option<u64>,
}

impl Cancels {
    fn lock(&self) -> std::sync::MutexGuard<'_, CancelState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cancel(&self, id: u64) {
        let mut state = self.lock();
        match &state.current {
            Some((current, token)) if *current == id => token.cancel(),
            _ => state.early = Some(id),
        }
    }

    /// A fresh token for request `id`, already cancelled if its cancel came
    /// first.
    fn begin(&self, id: u64) -> CancelToken {
        let mut state = self.lock();
        let token = CancelToken::new();
        if state.early == Some(id) {
            token.cancel();
            state.early = None;
        }
        state.current = Some((id, token.clone()));
        token
    }

    fn end(&self) {
        self.lock().current = None;
    }
}

/// The stdin reader. Applies cancels at once, forwards everything else to the
/// main loop, and ends the process when the parent closes stdin (unload,
/// quit, or the parent died). `_exit` skips C++ static destructors, so a
/// model still alive at that point can't trip ggml-metal's teardown asserts,
/// and the OS reclaims its CPU and GPU memory.
fn read_requests(requests: mpsc::Sender<Incoming>, cancels: &Cancels) -> ! {
    let mut input = BufReader::new(io::stdin().lock());
    loop {
        match read_message::<Request>(&mut input) {
            Ok(Some((Request::Cancel { id }, _))) => {
                debug!("Cancel requested for request {}", id);
                cancels.cancel(id);
            }
            Ok(Some((request, pcm))) => {
                if requests.send(Incoming { request, pcm }).is_err() {
                    exit_now(1);
                }
            }
            Ok(None) => exit_now(0),
            Err(e) => {
                error!("Protocol read failed: {}", e);
                exit_now(1);
            }
        }
    }
}

fn exit_now(code: i32) -> ! {
    let _ = io::stderr().flush();
    // SAFETY: `_exit` only ends the process; nothing runs afterwards.
    unsafe { libc::_exit(code) }
}

/// Serve requests against an active stream until it is finalized or reset.
fn serve_stream(
    mut stream: Stream<'_>,
    requests: &mpsc::Receiver<Incoming>,
    output: &mut impl Write,
    on_gpu: bool,
) -> io::Result<()> {
    while let Ok(Incoming { request, pcm }) = requests.recv() {
        let (response, done) = match request {
            Request::Feed => {
                inject_fault("feed", on_gpu);
                match stream.feed(&pcm) {
                    Ok(update) => {
                        let text = (update.committed_changed || update.tentative_changed)
                            .then(|| stream.text());
                        (Response::Fed { update, text }, false)
                    }
                    Err(e) => (Response::failed(&e), false),
                }
            }
            Request::Finalize { want_language } => {
                inject_fault("finalize", on_gpu);
                let (update, error) = match stream.finalize() {
                    Ok(update) => (Some(update), None),
                    // Finalize ends the stream even when it fails; what it
                    // committed until then is still the user's words.
                    Err(e) => (None, Some(e.to_string())),
                };
                let language = if want_language {
                    stream.snapshot().language
                } else {
                    None
                };
                (
                    Response::Finalized {
                        update,
                        text: stream.text(),
                        language,
                        error,
                    },
                    true,
                )
            }
            Request::StreamReset => {
                stream.reset();
                (Response::Ok, true)
            }
            Request::Cancel { .. } => continue,
            _ => (
                Response::Error("a stream is active; finalize or reset it first".to_string()),
                false,
            ),
        };
        write_message(output, &response, None)?;
        if done {
            break;
        }
    }
    Ok(())
}

fn not_loaded() -> Response {
    Response::Error("no model loaded".to_string())
}

fn aborted(message: &str) -> Response {
    Response::Failed {
        kind: ErrorKind::Aborted,
        message: message.to_string(),
    }
}

/// The error is the worker's answer to the load, sent as is; it is built once
/// per model load, so its size does not matter.
#[allow(clippy::result_large_err)]
fn load(path: &Path, target: LoadTarget, cpu_only: bool) -> Result<Loaded, Response> {
    let (backend, device) = match target {
        LoadTarget::Cpu => (Backend::Cpu, None),
        LoadTarget::Auto => (Backend::Auto, None),
        LoadTarget::Device(key) => {
            let found = transcribe_cpp::devices().into_iter().find(|d| {
                let info = DeviceInfo::from_device(d);
                info.is_gpu() && info.key == key
            });
            if found.is_none() {
                warn!(
                    "The GPU chosen for speech is no longer available; using automatic device selection"
                );
            }
            // `Auto` accepts an exact GPU or integrated GPU device.
            (Backend::Auto, found)
        }
    };

    inject_fault("load", backend != Backend::Cpu && !cpu_only);
    let model = Model::load_with(path, &ModelOptions { backend, device })
        .map_err(|e| Response::failed(&e))?;
    // 0.4 reports capabilities only for models with the ASR role; a diarizer
    // or language-ID GGUF has none and cannot transcribe.
    let capabilities = model
        .capabilities()
        .map_err(|e| Response::NotSpeechModel(e.to_string()))?;
    let bound = model.device().ok().map(|d| DeviceInfo::from_device(&d));
    let backend_name = model.backend();
    let on_gpu = match &bound {
        Some(device) => device.is_gpu(),
        None => !backend_name.is_empty() && !backend_name.eq_ignore_ascii_case("cpu"),
    };
    let arch = model.arch();
    let cpu_threads = crate::managers::transcription::transcribe_cpp_session_threads(&arch, on_gpu);
    let session = model
        .session_with(&SessionOptions {
            n_threads: cpu_threads,
            ..Default::default()
        })
        .map_err(|e| Response::failed(&e))?;
    let info = LoadedInfo {
        variant: model.variant(),
        backend: backend_name,
        device: bound
            .as_ref()
            .map(|d| d.label().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        device_type: bound.as_ref().map(|d| d.device_type),
        on_gpu,
        capabilities,
        supports_initial_prompt: model.supports(Feature::InitialPrompt),
        supports_long_form: model.supports(Feature::LongForm),
        supports_vocabulary: model.supports(Feature::Vocabulary),
        supports_cancellation: model.supports(Feature::Cancellation),
        buffered_stream: model.accepts_ext(
            ExtSlot::Stream,
            transcribe_cpp::sys::TRANSCRIBE_EXT_KIND_PARAKEET_BUFFERED_STREAM,
        ),
        cpu_threads,
        arch,
    };
    Ok(Loaded { session, info })
}

/// The init line the app has always logged, now from the process that
/// actually registered the devices.
fn log_registered_devices(cpu_only: bool) {
    let devices = transcribe_cpp::devices();
    info!(
        "transcribe.cpp {} initialized{} with {} compute device(s): [{}]",
        transcribe_cpp::version(),
        if cpu_only { " (CPU only)" } else { "" },
        devices.len(),
        devices
            .iter()
            .map(|d| {
                let info = DeviceInfo::from_device(d);
                format!("{} ({}, {:?})", info.label(), info.kind, info.device_type)
            })
            .collect::<Vec<_>>()
            .join(", ")
    );
    debug!(
        "transcribe.cpp allowed backends: {:#x}",
        transcribe_cpp::allowed_backends().bits()
    );
}

/// Debug-build fault injection for exercising crash/hang recovery:
/// `SPEAKOFLOW_WORKER_FAULT=<abort|segv|hang>@<init|list|load|run|feed|finalize>`
/// (`abort` is what `GGML_ASSERT` ends in, `segv` a real invalid memory write;
/// `init` is backend registration, before the hello). With
/// `SPEAKOFLOW_WORKER_FAULT_ONCE=<marker path>` it fires only in the first
/// worker to reach that stage (the marker file records that it fired). With
/// `SPEAKOFLOW_WORKER_FAULT_GPU_ONLY=1` it fires only on a GPU (backend init
/// and device listing count as GPU work unless the worker is CPU-only), so
/// recovery on CPU can succeed. Compiled out of release builds entirely.
#[cfg(debug_assertions)]
fn inject_fault(stage: &str, on_gpu: bool) {
    let Ok(spec) = std::env::var("SPEAKOFLOW_WORKER_FAULT") else {
        return;
    };
    let Some((kind, at)) = spec.split_once('@') else {
        return;
    };
    if at != stage {
        return;
    }
    if !on_gpu && std::env::var_os("SPEAKOFLOW_WORKER_FAULT_GPU_ONLY").is_some() {
        return;
    }
    if let Some(marker) = std::env::var_os("SPEAKOFLOW_WORKER_FAULT_ONCE") {
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .is_err()
        {
            return;
        }
    }
    eprintln!("injected fault: {kind} at {stage}");
    match kind {
        "abort" => std::process::abort(),
        // SAFETY: deliberately crash the worker with a real invalid memory
        // write, as a faulting driver does: SIGSEGV on Unix, an access
        // violation on Windows (where raising SIGSEGV only exits with code
        // 3). On Unix, restore the default disposition first so Rust's
        // stack-overflow handler doesn't get involved.
        "segv" => unsafe {
            #[cfg(unix)]
            libc::signal(libc::SIGSEGV, libc::SIG_DFL);
            std::ptr::write_volatile(std::ptr::null_mut::<u32>(), 1);
        },
        "hang" => loop {
            thread::sleep(std::time::Duration::from_secs(3600));
        },
        _ => {}
    }
}

#[cfg(not(debug_assertions))]
fn inject_fault(_stage: &str, _on_gpu: bool) {}

/// Hand the run's freed buffers back to the OS (Linux, glibc), as the app
/// does after every dictation (`actions::trim_freed_memory`): the worker holds
/// the per-run engine scratch and the audio, which glibc would otherwise keep.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim_freed_memory() {
    // SAFETY: no memory arguments; malloc_trim only releases free pages via
    // madvise and is thread-safe.
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim_freed_memory() {}

/// Logs go to stderr as `\x01LEVEL\ttarget\tmessage` lines; the parent
/// re-logs them at the same level.
struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        let mut stderr = io::stderr().lock();
        for line in message.lines() {
            let _ = writeln!(
                stderr,
                "{LOG_LINE_PREFIX}{}\t{}\t{}",
                record.level(),
                record.target(),
                line
            );
        }
    }

    fn flush(&self) {
        let _ = io::stderr().flush();
    }
}

fn init_logger() {
    static LOGGER: StderrLogger = StderrLogger;
    let level = std::env::var(LOG_LEVEL_ENV)
        .ok()
        .and_then(|v| v.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info);
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

/// SIGUSR2 toggles transcription in the app (`signal_handle.rs`). Aimed at
/// SpeakoFlow by name (`pkill -USR2 -n speakoflow`), it can reach this newer
/// process instead, and its default action would kill it.
///
/// On Linux the worker has a name of its own (`set_process_name`), so a
/// signal aimed at the app by name never lands here, and the signals are
/// ignored. macOS cannot rename a process: the worker is `speakoflow` too, and
/// the newest one whenever a model is loaded, so `pkill -n` picks it every
/// time and the toggle went nowhere. There the worker passes the signal on to
/// the app, which drops the second copy when a `pkill` without `-n` reached
/// both (`signal_handle::setup_signal_handler`).
#[cfg(unix)]
fn ignore_app_signals() {
    #[cfg(target_os = "macos")]
    let handler = forward_to_app as extern "C" fn(libc::c_int) as libc::sighandler_t;
    #[cfg(not(target_os = "macos"))]
    let handler = libc::SIG_IGN;
    // SAFETY: SIG_IGN has no preconditions, and `forward_to_app` only makes
    // async-signal-safe calls.
    unsafe {
        libc::signal(libc::SIGUSR1, handler);
        libc::signal(libc::SIGUSR2, handler);
    }
}

/// macOS: pass an app signal on to the app (see [`ignore_app_signals`]).
#[cfg(target_os = "macos")]
extern "C" fn forward_to_app(signal: libc::c_int) {
    // SAFETY: getppid and kill are async-signal-safe.
    unsafe {
        let parent = libc::getppid();
        // Orphaned (the app is gone): reparented to launchd, pid 1.
        if parent > 1 {
            libc::kill(parent, signal);
        }
    }
}

/// Started from /proc/self/exe, the kernel names this process "exe". Name it
/// as SpeakoFlow's, but not `speakoflow`: `pkill`/`killall` match names
/// case-sensitively, so a `pkill -USR2 -n speakoflow` keeps reaching the app.
#[cfg(target_os = "linux")]
fn set_process_name() {
    // SAFETY: PR_SET_NAME copies a NUL-terminated name of up to 16 bytes.
    unsafe {
        libc::prctl(libc::PR_SET_NAME, c"SpeakoFlow-wkr".as_ptr());
    }
}

#[cfg(unix)]
fn take_stdout_for_protocol() -> io::Result<File> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain fd duplication; the new fd is owned by the returned File.
    unsafe {
        let fd = libc::dup(libc::STDOUT_FILENO);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_fd(fd))
    }
}

#[cfg(windows)]
fn take_stdout_for_protocol() -> io::Result<File> {
    use std::os::windows::io::FromRawHandle;
    // Native code writes through the CRT's fd 1, so redirect that one. The
    // protocol uses the OS handle behind a CRT duplicate of the original.
    // SAFETY: CRT fd duplication; the duplicate's handle is owned by the File
    // for the rest of the process.
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::dup2(2, 1) < 0 {
            return Err(io::Error::last_os_error());
        }
        let handle = libc::get_osfhandle(fd);
        if handle == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_handle(handle as _))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_that_arrives_first_still_cancels_its_request() {
        let cancels = Cancels::default();
        cancels.cancel(4);
        assert!(cancels.begin(4).is_cancelled());
        cancels.end();
        // Remembered once, for that id only.
        assert!(!cancels.begin(5).is_cancelled());
        cancels.end();
    }

    #[test]
    fn a_cancel_reaches_the_request_in_flight_and_no_other() {
        let cancels = Cancels::default();
        let token = cancels.begin(1);
        cancels.cancel(1);
        assert!(token.is_cancelled());
        cancels.end();

        let token = cancels.begin(2);
        // A late cancel for a request that already ended stops nothing.
        cancels.cancel(1);
        assert!(!token.is_cancelled());
        cancels.end();
        assert!(!cancels.begin(3).is_cancelled());
    }
}
