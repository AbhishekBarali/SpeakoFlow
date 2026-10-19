//! Process isolation for transcribe.cpp.
//!
//! All transcribe.cpp native code (backend init, device enumeration, model
//! load, inference, streaming) runs in a child process: the SpeakoFlow
//! executable re-launched with [`WORKER_FLAG`]. A GPU driver fault, a
//! `GGML_ASSERT` abort or a hang then ends only the worker, where it used to
//! take the whole app down or leave dictation stuck on "Transcribing…" for
//! good (issue #82).
//!
//! [`EngineSupervisor`] is the single owner of that worker. It loads and
//! unloads models, runs and streams transcriptions, cancels them, and recovers
//! from crashes and hangs (retrying in a CPU-only worker when the GPU was at
//! fault), so the rest of the app never sees processes, restarts or fallback.
//! At most one worker holds a model at a time, and it lives exactly as long as
//! that model, so unloading returns all of its CPU and GPU memory to the OS.
//! Listing compute devices may briefly run a second, model-less worker.
//!
//! A CPU-only worker ([`CPU_ONLY_FLAG`]) restricts transcribe.cpp to its CPU
//! backends before any are registered, so no GPU driver code runs in it at
//! all: a driver that crashes or hangs while initializing can't take the CPU
//! fallback down with it.
//!
//! Ported from upstream Handy #2208 (`engine_supervisor/`), adapted to
//! SpeakoFlow: cancellation goes through transcribe.cpp's own abort callback
//! first, so Esc stops a dictation without throwing the loaded model away,
//! and a batch run waits for a live stream instead of ending it, because a
//! meeting may transcribe beside a dictation.
//!
//! - `protocol`: the framed request/response protocol between the two.
//! - `supervisor`: the parent side ([`EngineSupervisor`]).
//! - `worker`: the child process.

mod protocol;
mod supervisor;
mod worker;

pub use protocol::{device_key, DeviceInfo, LoadTarget, LoadedInfo};
pub use supervisor::{
    EngineError, EngineSupervisor, Finalized, LoadSpec, StreamHandle, StreamProgress, Unloading,
};

/// Hidden first argument that turns the executable into a worker.
pub const WORKER_FLAG: &str = "--transcribe-worker";
/// Worker argument: register only the CPU backends.
const CPU_ONLY_FLAG: &str = "--cpu-only";
/// The parent's log level, so the worker does not format lines nobody keeps.
const LOG_LEVEL_ENV: &str = "SPEAKOFLOW_TRANSCRIBE_WORKER_LOG";
/// Debug and test builds only: run this executable as the worker instead of
/// the current one. Under `cargo test` the current executable is the test
/// harness, which cannot be a worker.
#[cfg(any(test, debug_assertions))]
const WORKER_EXE_ENV: &str = "SPEAKOFLOW_TRANSCRIBE_WORKER_EXE";

/// Whether this process was launched as a transcription worker. Checked in
/// `main` before CLI parsing, Tauri, single-instance, portable-mode or
/// logging setup, none of which a worker may touch.
pub fn is_worker_invocation() -> bool {
    std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == WORKER_FLAG)
}

/// Run the worker until the parent closes its stdin. Returns the exit code.
pub fn run_worker() -> i32 {
    worker::run()
}
