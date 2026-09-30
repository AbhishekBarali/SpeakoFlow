//! Running a meeting: capture both streams, transcribe continuously, persist.
//!
//! ```text
//!   microphone ──▶ ChunkAccumulator ──┐
//!                                     ├──▶ transcription worker ──▶ meetings.db
//!   system audio ─▶ ChunkAccumulator ──┘            │
//!         │                                          └──▶ "meeting-segment" event
//!         └──▶ .wav on disk (for later re-transcription and diarization)
//! ```
//!
//! Three properties are worth stating because they are what make this survive a
//! two-hour call rather than a two-minute demo:
//!
//! * **Nothing unbounded lives in memory.** Audio goes to disk as it arrives and
//!   to the transcriber in chunks; transcript rows go to SQLite in batches. The
//!   only resident audio is the chunk currently being accumulated, which the
//!   chunker caps.
//! * **Transcription runs on one worker thread, behind a bounded channel.**
//!   Capture callbacks must never block — they run on the audio device's thread
//!   and stalling one drops samples — and Whisper on a 30-second chunk is not
//!   fast. The queue absorbs a short stall; an engine that is slower than real
//!   time for the whole call gets chunks dropped from its transcript instead of
//!   growing memory for hours (see [`JOB_QUEUE_CAPACITY`]).
//! * **The two streams never merge.** They are chunked, transcribed and stored
//!   separately, tagged with the [`SpeakerSource`] they came from. That tag is
//!   the speaker attribution, and it costs nothing.

use anyhow::{anyhow, Result};
use hound::{WavSpec, WavWriter};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use super::chunker::{AudioChunk, ChunkAccumulator};
use super::store::MeetingStore;
use super::{MeetingSegment, MeetingStatus, NewSegment, SpeakerSource};
use crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE;
use crate::audio_toolkit::{start_loopback, start_mic_capture, LoopbackError, LoopbackStream};
use crate::managers::transcription::TranscriptionManager;

/// Event name for a newly transcribed segment.
///
/// A plain string event rather than a `tauri_specta::Event`: the payload is
/// append-only live data the transcript view listens for, and keeping it out of
/// the generated bindings avoids coupling the capture path to the specta event
/// registry.
pub const SEGMENT_EVENT: &str = "meeting-segment";

/// Event name for a change in recording state.
pub const STATE_EVENT: &str = "meeting-state";

/// Event name for the live input levels of both streams.
///
/// A meeting-specific event rather than reusing the dictation overlay's
/// `mic-level`, because the pill needs to show **two** levels and the second one
/// is the point: a meeting that captures the user but not the other participants
/// is the single most common way this feature fails, and it is silent. A level
/// meter that stays flat on the system side says so during the call rather than
/// after it.
pub const LEVEL_EVENT: &str = "meeting-level";

/// How often levels are published, in milliseconds.
///
/// Not cosmetic. Every Tauri `emit` becomes an `evaluate_script` per listening
/// webview, and wry leaks memory per call (tauri-apps/wry#1489) — unthrottled
/// per-frame emits over a two-hour meeting is the "memory climbs, disk 100%, PC
/// freezes" failure the dictation overlay already had to fix. Frames arrive every
/// 30 ms, so this coalesces roughly one emit per two frames.
const LEVEL_EMIT_INTERVAL_MS: u128 = 66;

/// Live input levels, 0.0–1.0 per stream.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Type)]
pub struct MeetingLevels {
    pub mic: f32,
    pub system: f32,
}

/// How many segments to accumulate before writing.
///
/// Batching trades a few seconds of durability for far fewer transactions over a
/// long meeting. Four is small enough that a crash loses almost nothing and
/// large enough to matter across thousands of segments.
const WRITE_BATCH: usize = 4;

/// Finished chunks allowed to wait for the transcriber.
///
/// A chunk is up to 30 s of `f32` audio, about 2 MB, so this caps the backlog
/// near 30 MB and eight minutes of speech. It used to be unbounded: an engine
/// slower than real time grew memory for the whole call, and `stop` then sat
/// through the entire backlog. A chunk that finds the queue full is dropped from
/// the live transcript only; its audio is already in the WAV.
const JOB_QUEUE_CAPACITY: usize = 16;

/// A newly transcribed segment, pushed to the UI as it happens.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct SegmentEvent {
    pub meeting_id: i64,
    pub source: SpeakerSource,
    pub speaker_key: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Live recording state, mirrored to the UI.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingState {
    pub meeting_id: Option<i64>,
    pub status: MeetingStatus,
    pub paused: bool,
    /// Whether system audio is actually being captured. False means the
    /// transcript will contain only the user's side — worth surfacing loudly.
    pub system_audio: bool,
    /// Why system audio is unavailable, if it is.
    pub system_audio_error: Option<String>,
    pub elapsed_ms: i64,
    /// Chunks left out of the transcript because transcription fell too far
    /// behind. Their audio is still in the recording.
    pub dropped_chunks: u32,
}

impl MeetingState {
    pub fn idle() -> Self {
        Self {
            meeting_id: None,
            status: MeetingStatus::Complete,
            paused: false,
            system_audio: false,
            system_audio_error: None,
            elapsed_ms: 0,
            dropped_chunks: 0,
        }
    }

    /// A meeting whose capture has stopped but whose queued chunks are still
    /// being transcribed.
    fn stopping(meeting_id: i64) -> Self {
        Self {
            meeting_id: Some(meeting_id),
            status: MeetingStatus::Processing,
            ..Self::idle()
        }
    }
}

/// Shared, lock-free level state for one meeting's two streams.
///
/// Written from two audio threads and read by whichever of them next crosses the
/// publish interval. Atomics rather than a mutex because these writes happen on
/// the capture callbacks, which must never block — stalling one drops samples,
/// and the whole point of this meter is to prove that samples are arriving.
struct LevelMeter {
    /// `f32` bits, per stream.
    mic: AtomicU32,
    system: AtomicU32,
    /// `f32` bits of each stream's own loudness reference — see [`next_reference`].
    /// Separate per stream, because sharing one is what made this meter unreadable.
    mic_reference: AtomicU32,
    system_reference: AtomicU32,
    /// Milliseconds since [`Self::started`] at the last publish.
    last_emit_ms: AtomicU64,
    started: std::time::Instant,
}

impl LevelMeter {
    fn new() -> Self {
        Self {
            mic: AtomicU32::new(0),
            system: AtomicU32::new(0),
            mic_reference: AtomicU32::new(REFERENCE_FLOOR.to_bits()),
            system_reference: AtomicU32::new(REFERENCE_FLOOR.to_bits()),
            last_emit_ms: AtomicU64::new(0),
            started: std::time::Instant::now(),
        }
    }

    /// Record one frame's level for `source` and publish both if it is time.
    fn observe(&self, app: &AppHandle, source: SpeakerSource, frame: &[f32]) {
        let (slot, reference_slot) = match source {
            SpeakerSource::Mic => (&self.mic, &self.mic_reference),
            SpeakerSource::System => (&self.system, &self.system_reference),
        };
        let rms = frame_rms(frame);
        // The reference is advanced before the level is taken from it, so a sudden
        // loud frame reads as full scale rather than as a ratio against the quiet
        // that preceded it.
        let reference = next_reference(rms, f32::from_bits(reference_slot.load(Ordering::Relaxed)));
        reference_slot.store(reference.to_bits(), Ordering::Relaxed);

        let previous = f32::from_bits(slot.load(Ordering::Relaxed));
        slot.store(
            smooth_level(previous, stream_level(rms, reference)).to_bits(),
            Ordering::Relaxed,
        );

        let elapsed = self.started.elapsed().as_millis() as u64;
        let last = self.last_emit_ms.load(Ordering::Relaxed);
        if u128::from(elapsed.saturating_sub(last)) < LEVEL_EMIT_INTERVAL_MS {
            return;
        }
        // A lost race here means the other stream published a moment ago, which
        // is exactly as good — both levels go out together either way.
        if self
            .last_emit_ms
            .compare_exchange(last, elapsed, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        let levels = MeetingLevels {
            mic: f32::from_bits(self.mic.load(Ordering::Relaxed)),
            system: f32::from_bits(self.system.load(Ordering::Relaxed)),
        };
        let _ = app.emit(LEVEL_EVENT, levels);
    }
}

/// One frame's RMS, 0.0 upward. Not yet a meter reading.
///
/// RMS rather than peak: peak jumps to full scale on a single click and reads as
/// speech, while RMS tracks how loud something actually sounds.
fn frame_rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

/* -----------------------------------------------------------------------------
 * Why each stream is measured against itself
 *
 * This meter used to divide both streams by one constant (`LEVEL_FULL_SCALE_RMS
 * = 0.15`). The two streams are not remotely comparable in absolute terms, and
 * that single number is the whole reason the two rows looked like different
 * instruments:
 *
 *   * The microphone is a raw `cpal` capture with no automatic gain — normal
 *     speech lands around 0.02 RMS, i.e. about 13% of "full", and every bar
 *     outside the middle of the row then bottoms out on the frontend's
 *     `Math.max(0.08, …)` floor. That is the row of tiny dots users reported.
 *   * System audio is a WASAPI loopback of the endpoint mix, which a conference
 *     app has already normalised and which is unaffected by the listening
 *     volume. It sits around 0.2 RMS, *above* the constant, so it clamped to 1.0
 *     and stayed there — a full row that could not show dynamics at all.
 *
 * So one stream could not move and the other could not stop. No amount of
 * choosing a better constant fixes that, because it is one number for two
 * unrelated scales: raising it flattens the mic further, lowering it pins system
 * audio harder.
 *
 * Each stream therefore carries its own slowly-decaying loudness reference and is
 * drawn as a fraction of that. The meter's job is "is this side being heard, and
 * is it talking right now" — see the `LevelMeter` doc comment in `MeetingPill.tsx`
 * — and a per-stream scale answers exactly that question on both rows at once.
 * The dictation waveform reaches the same conclusion by a different route
 * (`audio_toolkit::audio::visualizer`, which measures dB above an adaptive noise
 * floor); both are auto-ranging because a fixed range cannot serve an unknown
 * input gain.
 *
 * An absolute silence gate is what keeps this honest. Without it, auto-ranging
 * would amplify an idle stream's own noise to full scale, and "the far side is
 * not being captured" — the single most useful thing this meter says — would stop
 * being visible.
 * -------------------------------------------------------------------------- */

/// RMS at or below which a stream reads as silent regardless of its reference.
///
/// Above a quiet room on an open microphone and far below speech. This is what
/// makes a flat row mean "nothing is arriving" rather than "nothing loud is
/// arriving", which is the diagnostic the two-row split exists for.
const SILENCE_RMS: f32 = 0.004;

/// Smallest reference a stream may normalise against.
///
/// Stops a stream that has only ever been quiet from dividing by its own noise and
/// reporting full scale. Sits just above [`SILENCE_RMS`], so the first real speech
/// after silence reads high immediately instead of ramping in.
const REFERENCE_FLOOR: f32 = 0.01;

/// How much of the reference survives each frame once the signal drops below it.
///
/// Frames arrive every 30 ms, so 0.997 halves the reference over roughly 7
/// seconds: long enough to span the pauses inside a sentence (otherwise the
/// reference would collapse into each gap and the next syllable would read as full
/// scale), short enough to follow someone moving closer to their microphone or a
/// call getting louder.
const REFERENCE_DECAY: f32 = 0.997;

/// Shapes the ratio into a meter reading. Below 1 so ordinary speech uses the
/// upper half of the row rather than hugging the bottom, which is the same
/// reasoning as `visualizer::CURVE_POWER`.
const LEVEL_CURVE: f32 = 0.7;

/// Advance a stream's loudness reference by one frame.
///
/// Instant attack, slow release — the standard meter shape, and here it also means
/// the reference is a recent *loud* level rather than an average, so the row reads
/// as a fraction of how loud this stream gets rather than of how loud it is.
fn next_reference(rms: f32, reference: f32) -> f32 {
    if rms > reference {
        rms
    } else {
        (reference * REFERENCE_DECAY).max(REFERENCE_FLOOR)
    }
}

/// A stream's 0–1 meter reading: how loud it is against its own recent loudest.
fn stream_level(rms: f32, reference: f32) -> f32 {
    if rms <= SILENCE_RMS {
        return 0.0;
    }
    let reference = reference.max(REFERENCE_FLOOR);
    (rms / reference).clamp(0.0, 1.0).powf(LEVEL_CURVE)
}

/// How much of the previous level survives when the new one is quieter.
///
/// Attack is instant and release is slow, which is what every audio meter does
/// and for the same reason: a meter that follows the signal down as fast as it
/// goes up spends most of speech at zero, because speech is mostly gaps between
/// syllables. Without this the bars strobe instead of moving.
const LEVEL_RELEASE: f32 = 0.72;

fn smooth_level(previous: f32, current: f32) -> f32 {
    if current >= previous {
        current
    } else {
        previous * LEVEL_RELEASE
    }
}

/// Work handed to the transcription thread.
enum Job {
    Chunk {
        source: SpeakerSource,
        chunk: AudioChunk,
    },
    /// Drain and exit.
    Finish,
}

type SharedWav = Arc<Mutex<Option<WavWriter<BufWriter<std::fs::File>>>>>;

/// One in-flight meeting.
struct ActiveSession {
    meeting_id: i64,
    started_at: i64,
    paused: Arc<AtomicBool>,
    mic_stream: Option<LoopbackStream>,
    system_stream: Option<LoopbackStream>,
    system_error: Option<String>,
    mic_acc: Arc<Mutex<ChunkAccumulator>>,
    system_acc: Arc<Mutex<ChunkAccumulator>>,
    mic_wav: SharedWav,
    system_wav: SharedWav,
    mic_file: Option<String>,
    system_file: Option<String>,
    job_tx: Option<mpsc::SyncSender<Job>>,
    worker: Option<std::thread::JoinHandle<()>>,
    dropped_chunks: Arc<AtomicU32>,
}

/// Tauri-managed owner of the meeting recording lifecycle.
///
/// Only one meeting records at a time. That is a product decision as much as a
/// technical one: there is one microphone and one output mix, and two concurrent
/// meetings would compete for both while producing transcripts neither of which
/// is complete.
pub struct MeetingRecorder {
    app: AppHandle,
    store: Arc<MeetingStore>,
    active: Mutex<Option<ActiveSession>>,
    /// True from the moment a meeting starts until its transcription queue has
    /// fully drained after stop. Lock-free, so the speech engine's idle checks
    /// never wait on `active` (which `start` holds while opening the mic).
    transcribing: AtomicBool,
    /// The meeting `stop` has taken out of `active` and is still draining.
    ///
    /// Without it `state()` read idle for the whole drain, so a new meeting, a
    /// delete of this one, or a diarization of its half-written WAV were all
    /// accepted while the worker was still writing its rows. Lock order is
    /// `active` then `stopping`, everywhere.
    stopping: Mutex<Option<i64>>,
}

/// Clears [`MeetingRecorder::stopping`] however `stop` leaves, panics included,
/// so one failed stop cannot refuse every later meeting.
struct StoppingGuard<'a>(&'a Mutex<Option<i64>>);

impl Drop for StoppingGuard<'_> {
    fn drop(&mut self) {
        let mut slot = match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = None;
    }
}

impl MeetingRecorder {
    pub fn new(app: AppHandle, store: Arc<MeetingStore>) -> Self {
        Self {
            app,
            store,
            active: Mutex::new(None),
            transcribing: AtomicBool::new(false),
            stopping: Mutex::new(None),
        }
    }

    pub fn store(&self) -> &Arc<MeetingStore> {
        &self.store
    }

    /// Whether a meeting still needs the speech engine: recording, or draining
    /// chunks queued before it stopped.
    pub fn uses_speech_engine(&self) -> bool {
        self.transcribing.load(Ordering::SeqCst)
    }

    /// Whether a meeting is recording right now.
    pub fn is_recording(&self) -> bool {
        self.active.lock().map(|a| a.is_some()).unwrap_or(false)
    }

    /// Current state for the UI.
    ///
    /// A meeting that has stopped but is still draining reports itself as
    /// [`MeetingStatus::Processing`] with its id, which is what the delete and
    /// diarize commands check before touching it.
    pub fn state(&self) -> MeetingState {
        let guard = match self.active.lock() {
            Ok(g) => g,
            Err(_) => return MeetingState::idle(),
        };
        match guard.as_ref() {
            None => self
                .stopping_id()
                .map_or_else(MeetingState::idle, MeetingState::stopping),
            Some(session) => MeetingState {
                meeting_id: Some(session.meeting_id),
                status: MeetingStatus::Recording,
                paused: session.paused.load(Ordering::Relaxed),
                system_audio: session.system_stream.is_some(),
                system_audio_error: session.system_error.clone(),
                elapsed_ms: session.mic_acc.lock().map(|a| a.position_ms()).unwrap_or(0),
                dropped_chunks: session.dropped_chunks.load(Ordering::Relaxed),
            },
        }
    }

    fn stopping_id(&self) -> Option<i64> {
        match self.stopping.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Start recording a meeting.
    ///
    /// Microphone capture failing aborts the whole thing — a meeting with no
    /// microphone is not a meeting. System audio failing does **not**: it is
    /// recorded on the session, reported in [`MeetingState::system_audio_error`]
    /// and surfaced by the UI, because a user on Linux without PulseAudio or on
    /// macOS without a virtual device should still be able to record their own
    /// side rather than be refused outright.
    pub fn start(&self, title: &str, language: Option<&str>) -> Result<i64> {
        let mut guard = self
            .active
            .lock()
            .map_err(|_| anyhow!("Meeting recorder lock poisoned"))?;
        if guard.is_some() {
            return Err(anyhow!("A meeting is already recording"));
        }
        if self.stopping_id().is_some() {
            return Err(anyhow!(
                "The last meeting is still being transcribed. Try again in a moment."
            ));
        }

        let started_at = chrono::Utc::now().timestamp();
        let meeting_id = self.store.create_meeting(title, started_at, language)?;

        let mic_file = format!("meeting_{meeting_id}_mic.wav");
        let system_file = format!("meeting_{meeting_id}_system.wav");
        let mic_wav = open_wav(self.store.audio_path(&mic_file));
        let system_wav = open_wav(self.store.audio_path(&system_file));

        let paused = Arc::new(AtomicBool::new(false));
        let mic_acc = Arc::new(Mutex::new(ChunkAccumulator::default()));
        let system_acc = Arc::new(Mutex::new(ChunkAccumulator::default()));
        let levels = Arc::new(LevelMeter::new());
        let dropped_chunks = Arc::new(AtomicU32::new(0));

        let (job_tx, job_rx) = mpsc::sync_channel::<Job>(JOB_QUEUE_CAPACITY);
        let worker = spawn_worker(
            self.app.clone(),
            Arc::clone(&self.store),
            meeting_id,
            job_rx,
        );

        // Microphone. Failure here is fatal to the session.
        let mic_stream = match start_mic_capture(
            None,
            capture_sink(
                self.app.clone(),
                meeting_id,
                SpeakerSource::Mic,
                Arc::clone(&mic_acc),
                Arc::clone(&mic_wav),
                Arc::clone(&paused),
                Arc::clone(&levels),
                Arc::clone(&dropped_chunks),
                job_tx.clone(),
            ),
        ) {
            Ok(stream) => stream,
            Err(e) => {
                // Roll the meeting row back; a meeting that never captured a
                // sample should not appear in the list as an empty entry.
                let _ = self.store.delete_meeting(meeting_id);
                let _ = job_tx.send(Job::Finish);
                // Both files already exist, and with the row gone nothing would
                // ever delete them.
                discard_wav(&mic_wav, &self.store.audio_path(&mic_file));
                discard_wav(&system_wav, &self.store.audio_path(&system_file));
                return Err(anyhow!("Could not start the microphone: {e}"));
            }
        };

        // System audio. Failure here is reported, not fatal.
        let (system_stream, system_error) = match start_loopback(capture_sink(
            self.app.clone(),
            meeting_id,
            SpeakerSource::System,
            Arc::clone(&system_acc),
            Arc::clone(&system_wav),
            Arc::clone(&paused),
            Arc::clone(&levels),
            Arc::clone(&dropped_chunks),
            job_tx.clone(),
        )) {
            Ok(stream) => {
                info!(
                    "Meeting {meeting_id} capturing system audio via '{}'",
                    stream.source_name()
                );
                (Some(stream), None)
            }
            Err(e) => {
                warn!("Meeting {meeting_id} has no system audio: {e}");
                (None, Some(describe_loopback_error(&e)))
            }
        };

        *guard = Some(ActiveSession {
            meeting_id,
            started_at,
            paused,
            mic_stream: Some(mic_stream),
            system_stream,
            system_error,
            mic_acc,
            system_acc,
            mic_wav,
            system_wav,
            mic_file: Some(mic_file),
            system_file: Some(system_file),
            job_tx: Some(job_tx),
            worker: Some(worker),
            dropped_chunks,
        });

        // Under the lock, so a `stop` (which takes it first) can never clear the
        // flag before it is set.
        self.transcribing.store(true, Ordering::SeqCst);
        drop(guard);
        // Start loading the speech model now so the first chunk, 30 s in, does
        // not wait on it.
        if let Some(tm) = self.app.try_state::<Arc<TranscriptionManager>>() {
            tm.initiate_model_load();
        }
        self.emit_state();
        Ok(meeting_id)
    }

    /// Pause or resume. Paused frames are dropped rather than buffered, so a
    /// pause genuinely stops recording rather than deferring it.
    pub fn set_paused(&self, paused: bool) -> Result<()> {
        {
            let guard = self
                .active
                .lock()
                .map_err(|_| anyhow!("Meeting recorder lock poisoned"))?;
            let session = guard
                .as_ref()
                .ok_or_else(|| anyhow!("No meeting is recording"))?;
            session.paused.store(paused, Ordering::SeqCst);
        }
        self.emit_state();
        Ok(())
    }

    /// Stop recording and return the meeting id.
    ///
    /// Ordering matters throughout and is the difference between a clean stop and
    /// a truncated transcript:
    ///
    /// 1. stop the capture streams, so no new frames arrive;
    /// 2. finalise the WAV headers and record the capture on the row, before the
    ///    drain — the drain can take minutes behind a slow engine, and a quit
    ///    during it must not leave unreadable files that no row references;
    /// 3. flush both chunkers, so the last thing said is not lost;
    /// 4. tell the worker to finish and **wait for it**, so every queued chunk is
    ///    transcribed and written.
    ///
    /// From the moment the session leaves `active` until the worker is joined the
    /// meeting is reported as stopping (see [`Self::stopping`]).
    pub fn stop(&self) -> Result<i64> {
        let (session, guard) = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| anyhow!("Meeting recorder lock poisoned"))?;
            let session = active
                .take()
                .ok_or_else(|| anyhow!("No meeting is recording"))?;
            // Under the `active` lock, so there is no instant at which the
            // meeting is in neither slot and a `start` could slip in.
            match self.stopping.lock() {
                Ok(mut slot) => *slot = Some(session.meeting_id),
                Err(poisoned) => *poisoned.into_inner() = Some(session.meeting_id),
            }
            (session, StoppingGuard(&self.stopping))
        };

        // Tell every window now, not after the drain: the transcription queue
        // can take minutes, and until this event the Settings window kept its
        // last "recording" state and offered a Stop that answered "No meeting is
        // recording". The frontend reads this `processing` state as not live
        // (`liveMeetingId`).
        self.emit_state();
        let outcome = self.finish_session(session);
        drop(guard);
        self.emit_state();
        outcome
    }

    fn finish_session(&self, mut session: ActiveSession) -> Result<i64> {
        // 1. Streams down first. `stop` joins each capture thread, so no callback
        //    runs after this and nothing else is writing the WAVs.
        if let Some(stream) = session.mic_stream.take() {
            stream.stop();
        }
        if let Some(stream) = session.system_stream.take() {
            stream.stop();
        }

        // 2. WAV headers need finalising or the files are unreadable.
        let mic_file = finalize_wav(&session.mic_wav)
            .then_some(session.mic_file.clone())
            .flatten();
        let system_file = finalize_wav(&session.system_wav)
            .then_some(session.system_file.clone())
            .flatten();
        let ended_at = chrono::Utc::now().timestamp();
        // Held until the worker is joined: failing to record the outcome must not
        // skip the drain or leave the speech engine marked busy.
        let recorded = self.store.finish_capture(
            session.meeting_id,
            ended_at,
            mic_file.as_deref(),
            system_file.as_deref(),
        );

        // 3. Trailing audio. Blocking sends: this is not an audio thread, and
        //    the worker is draining the queue they wait on.
        let job_tx = session.job_tx.take();
        if let Some(tx) = job_tx.as_ref() {
            flush_accumulator(&session.mic_acc, SpeakerSource::Mic, tx);
            flush_accumulator(&session.system_acc, SpeakerSource::System, tx);
            let _ = tx.send(Job::Finish);
        }
        drop(job_tx);

        // 4. Let the queue drain.
        if let Some(worker) = session.worker.take() {
            if worker.join().is_err() {
                error!("Meeting transcription worker panicked");
            }
        }
        // The speech engine is free again. With "Immediately" nothing else
        // would unload it: per-chunk unloads were held off for the meeting, and
        // a meeting that ended on silence sends no final chunk.
        self.transcribing.store(false, Ordering::SeqCst);
        if let Some(tm) = self.app.try_state::<Arc<TranscriptionManager>>() {
            tm.maybe_unload_immediately("meeting");
        }

        recorded?;

        let dropped = session.dropped_chunks.load(Ordering::Relaxed);
        if dropped > 0 {
            warn!(
                "Meeting {}: {dropped} chunk(s) left out of the transcript because \
                 transcription fell behind; the audio is in the recording",
                session.meeting_id
            );
        }
        info!(
            "Meeting {} captured {}s",
            session.meeting_id,
            ended_at - session.started_at
        );
        Ok(session.meeting_id)
    }

    /// Stop a meeting that is still recording when the app quits.
    ///
    /// `app.exit()` ends the process straight after `RunEvent::Exit`, so without
    /// this the tail was lost, the WAV headers were never written and the row
    /// never learned its file names, orphaning the recording on disk. The drain
    /// behind a slow engine can take minutes and quitting must not, so this waits
    /// at most `timeout`; the WAVs and the row are written before the drain
    /// starts, so a timeout costs only the transcript of the queued chunks. The
    /// row stays `processing` and the next launch reconciles it to `interrupted`,
    /// which is what a meeting cut short by a quit is.
    pub fn stop_before_exit(self: &Arc<Self>, timeout: Duration) {
        if !self.is_recording() {
            return;
        }
        let recorder = Arc::clone(self);
        let (done_tx, done_rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("meeting-exit-stop".into())
            .spawn(move || {
                let _ = done_tx.send(recorder.stop());
            });
        if let Err(e) = spawned {
            error!("Could not stop the meeting at quit: {e}");
            return;
        }
        match done_rx.recv_timeout(timeout) {
            Ok(Ok(meeting_id)) => info!("Meeting {meeting_id} stopped at quit"),
            Ok(Err(e)) => warn!("Could not stop the meeting at quit: {e}"),
            Err(_) => warn!(
                "Meeting still transcribing after {}s at quit; its recording is saved",
                timeout.as_secs()
            ),
        }
    }

    fn emit_state(&self) {
        let state = self.state();
        if let Err(e) = self.app.emit(STATE_EVENT, &state) {
            debug!("Could not emit meeting state: {e}");
        }
    }
}

/// Build the per-frame callback for one stream.
///
/// The returned closure runs on an audio thread. It does exactly four cheap
/// things — write to disk, update the level meter, accumulate, hand off a
/// finished chunk — and never transcribes, allocates unboundedly, or touches the
/// database.
#[allow(clippy::too_many_arguments)]
fn capture_sink(
    app: AppHandle,
    meeting_id: i64,
    source: SpeakerSource,
    accumulator: Arc<Mutex<ChunkAccumulator>>,
    wav: SharedWav,
    paused: Arc<AtomicBool>,
    levels: Arc<LevelMeter>,
    dropped_chunks: Arc<AtomicU32>,
    job_tx: mpsc::SyncSender<Job>,
) -> impl FnMut(&[f32]) + Send + 'static {
    move |frame: &[f32]| {
        if paused.load(Ordering::Relaxed) {
            // Paused still publishes, so the meter falls to zero rather than
            // freezing at whatever it read when the user hit pause — a frozen
            // meter is indistinguishable from a stalled capture.
            levels.observe(&app, source, &[]);
            return;
        }

        if let Ok(mut guard) = wav.lock() {
            if let Some(writer) = guard.as_mut() {
                for sample in frame {
                    // A failed sample write is logged once by the caller's error
                    // path rather than per sample; ignoring it here keeps the
                    // audio thread free of formatting work.
                    let _ = writer.write_sample(*sample);
                }
            }
        }

        levels.observe(&app, source, frame);

        let finished = match accumulator.lock() {
            Ok(mut acc) => acc.push(frame),
            Err(_) => None,
        };

        if let Some(chunk) = finished {
            if let Some((dropped, start_ms)) = offer_chunk(&job_tx, &dropped_chunks, source, chunk)
            {
                warn!(
                    "Meeting {meeting_id}: transcription is behind, left the {} chunk at {start_ms}ms \
                     out of the transcript ({dropped} so far); it is still in the recording",
                    source.as_db_str(),
                );
            }
        }
    }
}

/// Queue a finished chunk without ever waiting, and count it if there is no room.
///
/// Never a blocking send: this runs on the capture callback, and waiting on the
/// transcriber would stall the device, so the WAV would lose audio too. Returns
/// the running drop count and the dropped chunk's start when the queue was full.
/// A closed channel means the session is stopping, and dropping the chunk is
/// correct at that point without counting it.
fn offer_chunk(
    job_tx: &mpsc::SyncSender<Job>,
    dropped_chunks: &AtomicU32,
    source: SpeakerSource,
    chunk: AudioChunk,
) -> Option<(u32, i64)> {
    let start_ms = chunk.start_ms;
    match job_tx.try_send(Job::Chunk { source, chunk }) {
        Err(mpsc::TrySendError::Full(_)) => {
            Some((dropped_chunks.fetch_add(1, Ordering::Relaxed) + 1, start_ms))
        }
        Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => None,
    }
}

fn flush_accumulator(
    accumulator: &Arc<Mutex<ChunkAccumulator>>,
    source: SpeakerSource,
    job_tx: &mpsc::SyncSender<Job>,
) {
    if let Ok(mut acc) = accumulator.lock() {
        if let Some(chunk) = acc.flush() {
            let _ = job_tx.send(Job::Chunk { source, chunk });
        }
    }
}

/// The transcription thread.
///
/// Runs until it sees [`Job::Finish`] *and* the queue is empty. Chunks are
/// transcribed in arrival order across both sources, which is also roughly
/// chronological, so the live transcript grows in the order a reader expects.
fn spawn_worker(
    app: AppHandle,
    store: Arc<MeetingStore>,
    meeting_id: i64,
    job_rx: mpsc::Receiver<Job>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("meeting-{meeting_id}-stt"))
        .spawn(move || {
            let mut pending: Vec<NewSegment> = Vec::with_capacity(WRITE_BATCH);

            while let Ok(job) = job_rx.recv() {
                match job {
                    Job::Finish => break,
                    Job::Chunk { source, chunk } => {
                        match transcribe_chunk(&app, chunk.samples) {
                            Ok(text) if !text.trim().is_empty() => {
                                let speaker_key = source.default_speaker_key().to_string();
                                let event = SegmentEvent {
                                    meeting_id,
                                    source,
                                    speaker_key: speaker_key.clone(),
                                    start_ms: chunk.start_ms,
                                    end_ms: chunk.end_ms,
                                    text: text.clone(),
                                };
                                // Emit before the write so the transcript appears
                                // as fast as possible; the row follows in the
                                // next batch and the UI does not depend on it.
                                if let Err(e) = app.emit(SEGMENT_EVENT, &event) {
                                    debug!("Could not emit meeting segment: {e}");
                                }

                                pending.push(NewSegment {
                                    source,
                                    speaker_key: Some(speaker_key),
                                    start_ms: chunk.start_ms,
                                    end_ms: chunk.end_ms,
                                    text,
                                    confidence: None,
                                });
                            }
                            Ok(_) => {
                                // Empty transcript: silence or noise the model
                                // correctly declined to invent words for.
                            }
                            Err(e) => {
                                // One failed chunk must not end the meeting. The
                                // gap is logged and recording continues.
                                warn!(
                                    "Meeting {meeting_id}: chunk at {}ms failed to transcribe: {e}",
                                    chunk.start_ms
                                );
                            }
                        }

                        if pending.len() >= WRITE_BATCH {
                            write_batch(&store, meeting_id, &mut pending);
                        }
                    }
                }
            }

            // Drain anything already queued behind Finish.
            while let Ok(Job::Chunk { source, chunk }) = job_rx.try_recv() {
                if let Ok(text) = transcribe_chunk(&app, chunk.samples) {
                    if !text.trim().is_empty() {
                        pending.push(NewSegment {
                            source,
                            speaker_key: Some(source.default_speaker_key().to_string()),
                            start_ms: chunk.start_ms,
                            end_ms: chunk.end_ms,
                            text,
                            confidence: None,
                        });
                    }
                }
            }

            write_batch(&store, meeting_id, &mut pending);
            debug!("Meeting {meeting_id} transcription worker finished");
        })
        .expect("spawn meeting transcription worker")
}

fn write_batch(store: &MeetingStore, meeting_id: i64, pending: &mut Vec<NewSegment>) {
    if pending.is_empty() {
        return;
    }
    if let Err(e) = store.append_segments(meeting_id, pending) {
        // Losing a batch is bad but not worth ending the meeting over — the
        // remaining audio is still being captured to disk and can be
        // re-transcribed.
        error!(
            "Meeting {meeting_id}: failed to persist {} segment(s): {e}",
            pending.len()
        );
    }
    pending.clear();
}

/// Transcribe one chunk with the app's configured engine.
///
/// `transcribe` only *waits* for a load in flight; it never starts one, so a
/// model that idled out, or that "Immediately" unloaded, made every later chunk
/// fail with "Model is not loaded". Dictation asks for the load when recording
/// starts; a meeting has to ask for each chunk. A no-op when the model is
/// resident or cloud transcription is active.
fn transcribe_chunk(app: &AppHandle, samples: Vec<f32>) -> Result<String> {
    let tm = app
        .try_state::<Arc<TranscriptionManager>>()
        .ok_or_else(|| anyhow!("Transcription manager is not initialised"))?
        .inner()
        .clone();
    tm.initiate_model_load();
    tm.transcribe(samples)
}

/// Create a 16 kHz mono float WAV for streaming writes.
///
/// Audio is kept, not discarded after transcription, and that is a deliberate
/// cost: diarization runs over the whole recording after the meeting, and
/// re-transcribing with a better model later is one of the most useful things a
/// user can do with a bad transcript. Meetily removed audio saving to save disk
/// and then had to add "import and enhance" back.
///
/// A failure to open returns `None` rather than failing the meeting — losing the
/// archive is survivable, losing the meeting is not.
fn open_wav(path: PathBuf) -> SharedWav {
    let spec = WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    match WavWriter::create(&path, spec) {
        Ok(writer) => Arc::new(Mutex::new(Some(writer))),
        Err(e) => {
            warn!("Could not open meeting audio file {path:?}: {e}");
            Arc::new(Mutex::new(None))
        }
    }
}

/// Finalise a WAV, returning whether a usable file exists.
///
/// `hound` writes the RIFF length fields on finalise. Skipping this leaves a file
/// that most players and every decoder we would later feed it to will reject, so
/// a meeting stopped without finalising has effectively no archive.
fn finalize_wav(wav: &SharedWav) -> bool {
    let Ok(mut guard) = wav.lock() else {
        return false;
    };
    match guard.take() {
        Some(writer) => match writer.finalize() {
            Ok(()) => true,
            Err(e) => {
                warn!("Could not finalise meeting audio: {e}");
                false
            }
        },
        None => false,
    }
}

/// Close and delete a recording that no row will ever reference.
///
/// Closed first because Windows refuses to delete a file that is still open.
fn discard_wav(wav: &SharedWav, path: &Path) {
    if let Ok(mut guard) = wav.lock() {
        drop(guard.take());
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => warn!("Could not remove meeting audio {path:?}: {e}"),
    }
}

/// Turn a capture failure into something worth showing a user.
fn describe_loopback_error(error: &LoopbackError) -> String {
    error.to_string()
}

/// Attach a display name to each segment, for rendering and for the summarizer.
///
/// Kept here rather than in the store because it is a presentation concern: the
/// database records a stable `speaker_key`, and what that key is *called* can
/// change whenever the user renames it.
pub fn label_segments(
    segments: &[MeetingSegment],
    speakers: &[super::MeetingSpeaker],
) -> Vec<(String, String)> {
    segments
        .iter()
        .map(|segment| {
            let key = segment
                .speaker_key
                .as_deref()
                .unwrap_or_else(|| segment.source.default_speaker_key());
            let name = speakers
                .iter()
                .find(|s| s.speaker_key == key)
                .map(|s| s.display_name.clone())
                // An unnamed key is shown as the key itself rather than
                // "Unknown": a raw `spk_2` at least tells the user which voice
                // it is and that it is renameable.
                .unwrap_or_else(|| key.to_string());
            (name, segment.text.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::MeetingSpeaker;

    fn segment(source: SpeakerSource, speaker_key: Option<&str>, text: &str) -> MeetingSegment {
        MeetingSegment {
            id: 1,
            meeting_id: 1,
            source,
            speaker_key: speaker_key.map(str::to_string),
            start_ms: 0,
            end_ms: 1_000,
            text: text.to_string(),
            confidence: None,
        }
    }

    fn speaker(key: &str, name: &str, is_me: bool) -> MeetingSpeaker {
        MeetingSpeaker {
            speaker_key: key.to_string(),
            display_name: name.to_string(),
            is_me,
        }
    }

    /* ---------------------------------------------------------------------- *
     * The two-stream level meter
     *
     * These exist because the meter's bug was entirely in this arithmetic and
     * entirely invisible in the CSS: both rows are one component at one height,
     * so "my microphone is tiny and the computer's is huge" could only come from
     * the numbers.
     * -------------------------------------------------------------------- */

    /// Settle a stream's reference by feeding it the same level repeatedly, the way
    /// a real stream does at ~33 frames a second.
    fn settled_reference(rms: f32, frames: usize) -> f32 {
        let mut reference = REFERENCE_FLOOR;
        for _ in 0..frames {
            reference = next_reference(rms, reference);
        }
        reference
    }

    /// The reported defect, as arithmetic.
    ///
    /// A raw microphone at ~0.02 RMS and a WASAPI loopback of an already-normalised
    /// call at ~0.2 RMS are both people talking, and both rows have to say so. Under
    /// the old single `LEVEL_FULL_SCALE_RMS = 0.15` these came out at 0.13 and 1.0 —
    /// a 7.5x difference in bar height for the same event, with the loud one clamped
    /// so it could not show dynamics either.
    #[test]
    fn a_quiet_mic_and_a_loud_system_stream_read_alike() {
        let mic_rms = 0.02;
        let system_rms = 0.2;

        let mic = stream_level(mic_rms, settled_reference(mic_rms, 200));
        let system = stream_level(system_rms, settled_reference(system_rms, 200));

        assert!(
            mic > 0.6,
            "a normal speaking voice on a low-gain mic must fill most of the row, got {mic}"
        );
        assert!(
            (mic - system).abs() < 0.1,
            "both rows are people talking and must read alike: mic {mic}, system {system}"
        );

        // And the old behaviour is genuinely gone: a 10x difference in absolute level
        // used to be a 7.5x difference on screen.
        let ratio = (mic / system).max(system / mic);
        assert!(ratio < 1.25, "rows still differ by {ratio}x");
    }

    /// The whole point of splitting the meter in two: a flat row means that side is
    /// not being captured. Auto-ranging must not amplify an idle stream's noise into
    /// looking like speech.
    #[test]
    fn a_silent_stream_stays_flat_however_long_it_is_silent() {
        // Digital silence, and the low-level hiss of a connected but unused input.
        for quiet in [0.0, 0.0005, 0.002, SILENCE_RMS] {
            let mut reference = settled_reference(0.25, 50); // it was loud earlier
            for _ in 0..2_000 {
                reference = next_reference(quiet, reference);
                assert_eq!(
                    stream_level(quiet, reference),
                    0.0,
                    "rms {quiet} must read as silence, not as {reference} worth of reference"
                );
            }
        }
    }

    /// A loud stream keeps its dynamics instead of pinning to the top.
    #[test]
    fn a_loud_stream_is_not_clamped_to_full_scale() {
        let reference = settled_reference(0.2, 200);
        let loud = stream_level(0.2, reference);
        let mid = stream_level(0.06, reference);
        let soft = stream_level(0.02, reference);
        assert!(
            loud > mid && mid > soft,
            "system audio must still show shape: {soft} < {mid} < {loud}"
        );
        assert!(soft > 0.0, "quiet speech is not silence");
    }

    /// The reference must span the gaps inside a sentence. If it collapsed into
    /// every pause, the next syllable would read as full scale and the row would
    /// strobe rather than move.
    #[test]
    fn the_reference_outlasts_a_pause_between_words() {
        let speech = 0.05;
        let mut reference = settled_reference(speech, 100);
        // ~10 frames of 30 ms is a 300 ms gap between words.
        for _ in 0..10 {
            reference = next_reference(0.0, reference);
        }
        assert!(
            reference > speech * 0.9,
            "a 300ms pause dropped the reference from {speech} to {reference}"
        );
    }

    /// It must still adapt, or someone who leans into their microphone is stuck at
    /// full scale for the rest of the call.
    #[test]
    fn the_reference_follows_a_stream_that_gets_quieter() {
        let mut reference = settled_reference(0.3, 100);
        // A minute of much quieter speech.
        for _ in 0..2_000 {
            reference = next_reference(0.03, reference);
        }
        assert!(
            reference < 0.05,
            "the reference stayed at {reference} after a minute at 0.03"
        );
        assert!(
            reference >= REFERENCE_FLOOR,
            "the reference must never fall below its floor, got {reference}"
        );
    }

    /// A stream that has only ever been quiet must not divide by its own noise.
    #[test]
    fn a_stream_that_was_never_loud_cannot_amplify_itself_to_full() {
        let mut reference = REFERENCE_FLOOR;
        for _ in 0..5_000 {
            reference = next_reference(0.001, reference);
        }
        assert_eq!(reference, REFERENCE_FLOOR);
        assert_eq!(stream_level(0.001, reference), 0.0);
    }

    #[test]
    fn an_empty_frame_is_silent_rather_than_a_division_by_zero() {
        assert_eq!(frame_rms(&[]), 0.0);
        assert_eq!(stream_level(frame_rms(&[]), REFERENCE_FLOOR), 0.0);
    }

    #[test]
    fn frame_rms_is_rms_and_not_peak() {
        // One loud sample in an otherwise quiet frame is a click, not speech, and
        // must not read as a full frame.
        let mut frame = vec![0.0f32; 480];
        frame[0] = 1.0;
        let rms = frame_rms(&frame);
        assert!(rms < 0.05, "a single click measured {rms}");
    }

    #[test]
    fn idle_state_has_no_meeting() {
        let state = MeetingState::idle();
        assert!(state.meeting_id.is_none());
        assert!(!state.paused);
        assert!(!state.system_audio);
    }

    #[test]
    fn segments_take_their_speaker_display_name() {
        let segments = vec![
            segment(SpeakerSource::Mic, Some("me"), "my line"),
            segment(SpeakerSource::System, Some("spk_0"), "their line"),
        ];
        let speakers = vec![
            speaker("me", "Abhishek", true),
            speaker("spk_0", "Priya", false),
        ];

        let labelled = label_segments(&segments, &speakers);
        assert_eq!(labelled[0].0, "Abhishek");
        assert_eq!(labelled[1].0, "Priya");
    }

    /// A segment whose key has no speaker row still renders something the user
    /// can act on, rather than collapsing every unknown voice into "Unknown".
    #[test]
    fn an_unnamed_speaker_key_is_shown_verbatim() {
        let segments = vec![segment(SpeakerSource::System, Some("spk_7"), "hello")];
        let labelled = label_segments(&segments, &[]);
        assert_eq!(labelled[0].0, "spk_7");
    }

    /// A null key falls back to the channel, which is always known.
    #[test]
    fn a_missing_speaker_key_falls_back_to_the_channel() {
        let segments = vec![
            segment(SpeakerSource::Mic, None, "mine"),
            segment(SpeakerSource::System, None, "theirs"),
        ];
        let speakers = vec![speaker("me", "You", true), speaker("them", "Others", false)];

        let labelled = label_segments(&segments, &speakers);
        assert_eq!(labelled[0].0, "You");
        assert_eq!(labelled[1].0, "Others");
    }

    #[test]
    fn labelling_preserves_order_and_text() {
        let segments = vec![
            segment(SpeakerSource::Mic, Some("me"), "first"),
            segment(SpeakerSource::System, Some("them"), "second"),
            segment(SpeakerSource::Mic, Some("me"), "third"),
        ];
        let labelled = label_segments(&segments, &[]);
        let texts: Vec<_> = labelled.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn labelling_an_empty_transcript_is_empty() {
        assert!(label_segments(&[], &[]).is_empty());
    }

    #[test]
    fn loopback_errors_describe_themselves_for_the_ui() {
        let error = LoopbackError::Unsupported("install BlackHole".into());
        assert_eq!(describe_loopback_error(&error), "install BlackHole");
    }

    fn chunk(start_ms: i64) -> AudioChunk {
        AudioChunk {
            start_ms,
            end_ms: start_ms + 30_000,
            samples: vec![0.0; 16],
        }
    }

    /// The capture callback must never wait on the transcriber. A full queue
    /// drops the new chunk and counts it rather than blocking the audio thread.
    #[test]
    fn a_full_queue_drops_and_counts_instead_of_blocking() {
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        let dropped = AtomicU32::new(0);

        assert_eq!(
            offer_chunk(&tx, &dropped, SpeakerSource::Mic, chunk(0)),
            None
        );
        assert_eq!(
            offer_chunk(&tx, &dropped, SpeakerSource::System, chunk(30_000)),
            Some((1, 30_000))
        );
        assert_eq!(
            offer_chunk(&tx, &dropped, SpeakerSource::Mic, chunk(60_000)),
            Some((2, 60_000))
        );
        assert_eq!(dropped.load(Ordering::Relaxed), 2);

        // The chunk that fitted is the one the worker sees.
        match rx.try_recv() {
            Ok(Job::Chunk { chunk, .. }) => assert_eq!(chunk.start_ms, 0),
            _ => panic!("the first chunk must be queued"),
        }
    }

    /// Once the worker is gone the session is stopping; that is not a transcript
    /// gap worth reporting.
    #[test]
    fn a_closed_queue_is_not_counted_as_a_drop() {
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        drop(rx);
        let dropped = AtomicU32::new(0);
        assert_eq!(
            offer_chunk(&tx, &dropped, SpeakerSource::Mic, chunk(0)),
            None
        );
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    /// While the queue drains after stop, the meeting is still busy: the delete
    /// and diarize commands key on this id.
    #[test]
    fn a_stopping_meeting_reports_processing_with_its_id() {
        let state = MeetingState::stopping(7);
        assert_eq!(state.meeting_id, Some(7));
        assert_eq!(state.status, MeetingStatus::Processing);
        assert!(!state.paused);
    }

    /// A stop that fails or panics mid-drain must not leave the recorder refusing
    /// every later meeting.
    #[test]
    fn the_stopping_slot_clears_on_every_exit() {
        let slot = Mutex::new(Some(3));
        drop(StoppingGuard(&slot));
        assert_eq!(*slot.lock().unwrap(), None);

        let slot = Arc::new(Mutex::new(Some(4)));
        let inner = Arc::clone(&slot);
        let _ = std::thread::spawn(move || {
            let _guard = StoppingGuard(&inner);
            panic!("drain failed");
        })
        .join();
        assert_eq!(*slot.lock().unwrap_or_else(|p| p.into_inner()), None);
    }

    #[test]
    fn discarding_a_recording_closes_and_deletes_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("meeting_1_mic.wav");
        let wav = open_wav(path.clone());
        assert!(path.exists());
        discard_wav(&wav, &path);
        assert!(!path.exists());
        assert!(wav.lock().unwrap().is_none());
    }
}
