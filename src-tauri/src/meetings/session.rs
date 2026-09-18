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
//! * **Transcription runs on one worker thread, behind a channel.** Capture
//!   callbacks must never block — they run on the audio device's thread and
//!   stalling one drops samples — and Whisper on a 30-second chunk is not fast.
//!   If transcription falls behind, the queue absorbs it.
//! * **The two streams never merge.** They are chunked, transcribed and stored
//!   separately, tagged with the [`SpeakerSource`] they came from. That tag is
//!   the speaker attribution, and it costs nothing.

use anyhow::{anyhow, Result};
use hound::{WavSpec, WavWriter};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
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
    /// Milliseconds since [`Self::started`] at the last publish.
    last_emit_ms: AtomicU64,
    started: std::time::Instant,
}

impl LevelMeter {
    fn new() -> Self {
        Self {
            mic: AtomicU32::new(0),
            system: AtomicU32::new(0),
            last_emit_ms: AtomicU64::new(0),
            started: std::time::Instant::now(),
        }
    }

    /// Record one frame's level for `source` and publish both if it is time.
    fn observe(&self, app: &AppHandle, source: SpeakerSource, frame: &[f32]) {
        let slot = match source {
            SpeakerSource::Mic => &self.mic,
            SpeakerSource::System => &self.system,
        };
        let previous = f32::from_bits(slot.load(Ordering::Relaxed));
        slot.store(
            smooth_level(previous, frame_level(frame)).to_bits(),
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

/// Perceptual 0–1 level for one frame.
///
/// RMS scaled by [`LEVEL_FULL_SCALE_RMS`] rather than by peak: peak jumps to full
/// on a single click and reads as speech, while RMS tracks how loud something
/// actually sounds. The scale is well under 1.0 because normal speech sits far
/// below digital full scale, and a meter that only moves when someone shouts
/// looks broken.
fn frame_level(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    let rms = (sum / frame.len() as f32).sqrt();
    (rms / LEVEL_FULL_SCALE_RMS).clamp(0.0, 1.0)
}

/// RMS that counts as a full meter.
const LEVEL_FULL_SCALE_RMS: f32 = 0.15;

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
    job_tx: Option<mpsc::Sender<Job>>,
    worker: Option<std::thread::JoinHandle<()>>,
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
}

impl MeetingRecorder {
    pub fn new(app: AppHandle, store: Arc<MeetingStore>) -> Self {
        Self {
            app,
            store,
            active: Mutex::new(None),
        }
    }

    pub fn store(&self) -> &Arc<MeetingStore> {
        &self.store
    }

    /// Whether a meeting is recording right now.
    pub fn is_recording(&self) -> bool {
        self.active.lock().map(|a| a.is_some()).unwrap_or(false)
    }

    /// Current state for the UI.
    pub fn state(&self) -> MeetingState {
        let guard = match self.active.lock() {
            Ok(g) => g,
            Err(_) => return MeetingState::idle(),
        };
        match guard.as_ref() {
            None => MeetingState::idle(),
            Some(session) => MeetingState {
                meeting_id: Some(session.meeting_id),
                status: MeetingStatus::Recording,
                paused: session.paused.load(Ordering::Relaxed),
                system_audio: session.system_stream.is_some(),
                system_audio_error: session.system_error.clone(),
                elapsed_ms: session.mic_acc.lock().map(|a| a.position_ms()).unwrap_or(0),
            },
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

        let (job_tx, job_rx) = mpsc::channel::<Job>();
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
                SpeakerSource::Mic,
                Arc::clone(&mic_acc),
                Arc::clone(&mic_wav),
                Arc::clone(&paused),
                Arc::clone(&levels),
                job_tx.clone(),
            ),
        ) {
            Ok(stream) => stream,
            Err(e) => {
                // Roll the meeting row back; a meeting that never captured a
                // sample should not appear in the list as an empty entry.
                let _ = self.store.delete_meeting(meeting_id);
                let _ = job_tx.send(Job::Finish);
                return Err(anyhow!("Could not start the microphone: {e}"));
            }
        };

        // System audio. Failure here is reported, not fatal.
        let (system_stream, system_error) = match start_loopback(capture_sink(
            self.app.clone(),
            SpeakerSource::System,
            Arc::clone(&system_acc),
            Arc::clone(&system_wav),
            Arc::clone(&paused),
            Arc::clone(&levels),
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
        });

        drop(guard);
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
    /// 2. flush both chunkers, so the last thing said is not lost;
    /// 3. tell the worker to finish and **wait for it**, so every queued chunk is
    ///    transcribed and written;
    /// 4. finalise the WAV headers;
    /// 5. only then mark the meeting as no longer recording.
    pub fn stop(&self) -> Result<i64> {
        let mut session = {
            let mut guard = self
                .active
                .lock()
                .map_err(|_| anyhow!("Meeting recorder lock poisoned"))?;
            guard
                .take()
                .ok_or_else(|| anyhow!("No meeting is recording"))?
        };

        // 1. Streams down first.
        if let Some(stream) = session.mic_stream.take() {
            stream.stop();
        }
        if let Some(stream) = session.system_stream.take() {
            stream.stop();
        }

        // 2. Trailing audio.
        let job_tx = session.job_tx.take();
        if let Some(tx) = job_tx.as_ref() {
            flush_accumulator(&session.mic_acc, SpeakerSource::Mic, tx);
            flush_accumulator(&session.system_acc, SpeakerSource::System, tx);
            let _ = tx.send(Job::Finish);
        }
        drop(job_tx);

        // 3. Let the queue drain.
        if let Some(worker) = session.worker.take() {
            if worker.join().is_err() {
                error!("Meeting transcription worker panicked");
            }
        }

        // 4. WAV headers need finalising or the files are unreadable.
        let mic_file = finalize_wav(&session.mic_wav)
            .then_some(session.mic_file.clone())
            .flatten();
        let system_file = finalize_wav(&session.system_wav)
            .then_some(session.system_file.clone())
            .flatten();

        // 5. Record the outcome.
        let ended_at = chrono::Utc::now().timestamp();
        self.store.finish_capture(
            session.meeting_id,
            ended_at,
            mic_file.as_deref(),
            system_file.as_deref(),
        )?;

        info!(
            "Meeting {} captured {}s",
            session.meeting_id,
            ended_at - session.started_at
        );

        self.emit_state();
        Ok(session.meeting_id)
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
fn capture_sink(
    app: AppHandle,
    source: SpeakerSource,
    accumulator: Arc<Mutex<ChunkAccumulator>>,
    wav: SharedWav,
    paused: Arc<AtomicBool>,
    levels: Arc<LevelMeter>,
    job_tx: mpsc::Sender<Job>,
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
            // A closed channel means the session is stopping; dropping the chunk
            // is correct at that point.
            let _ = job_tx.send(Job::Chunk { source, chunk });
        }
    }
}

fn flush_accumulator(
    accumulator: &Arc<Mutex<ChunkAccumulator>>,
    source: SpeakerSource,
    job_tx: &mpsc::Sender<Job>,
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
fn transcribe_chunk(app: &AppHandle, samples: Vec<f32>) -> Result<String> {
    let tm = app
        .try_state::<Arc<TranscriptionManager>>()
        .ok_or_else(|| anyhow!("Transcription manager is not initialised"))?
        .inner()
        .clone();
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
}
