use crate::audio_toolkit::{
    ducking::{Ducker, Quieting, Restored},
    list_input_devices,
    vad::{SmoothedVad, VadTail},
    AudioRecorder, SileroVad, SpeechActivity,
};
use crate::helpers::clamshell;
use crate::managers::transcription::StreamRouter;
use crate::settings::{clamp_audio_ducking_amount, get_settings, AppSettings, AudioDucking};
use crate::utils;
use log::{debug, error, info, trace, warn};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

const WHISPER_SAMPLE_RATE: usize = 16000;

/// How `audio_ducking` asks a recording to quiet other audio, if at all.
fn quieting(settings: &AppSettings) -> Option<Quieting> {
    match settings.audio_ducking {
        AudioDucking::Off => None,
        AudioDucking::Lower => Some(Quieting::lower_by(clamp_audio_ducking_amount(
            settings.audio_ducking_amount,
        ))),
        AudioDucking::Mute => Some(Quieting::Mute),
    }
}

/// Whether the recording under way may still quiet other audio, and whether
/// it has. See [`AudioRecordingManager::quiet_other_audio`].
#[derive(Default)]
struct DuckingGate {
    /// Opened when a recording starts, closed the moment it ends.
    armed: bool,
    quieted: bool,
}

impl DuckingGate {
    /// A recording has started: it may quiet other audio from now on.
    fn arm(&mut self) {
        self.armed = true;
    }

    /// How to quiet other audio now, if the recording under way may and has
    /// not yet. `how` is asked only then, and a `None` from it (ducking off)
    /// leaves the gate as it was.
    fn quiet(&mut self, how: impl FnOnce() -> Option<Quieting>) -> Option<Quieting> {
        if !self.armed || self.quieted {
            return None;
        }
        let how = how()?;
        self.quieted = true;
        Some(how)
    }

    /// The recording has ended: close the gate. Whether other audio was
    /// quieted, and so has to be brought back. True at most once per quieting.
    fn restore(&mut self) -> bool {
        self.armed = false;
        std::mem::take(&mut self.quieted)
    }
}

/* ──────────────────────────────────────────────────────────────── */

#[derive(Clone, Debug)]
pub enum RecordingState {
    Idle,
    Recording { binding_id: String },
}

#[derive(Clone, Debug)]
pub enum MicrophoneMode {
    AlwaysOn,
    OnDemand,
}

/* ──────────────────────────────────────────────────────────────── */

fn create_audio_recorder(
    vad_path: &Path,
    app_handle: &tauri::AppHandle,
    stream_router: Arc<StreamRouter>,
    speech: Arc<SpeechActivity>,
) -> Result<AudioRecorder, anyhow::Error> {
    let silero = SileroVad::new(vad_path, 0.3)
        .map_err(|e| anyhow::anyhow!("Failed to create SileroVad: {}", e))?;
    // 450 ms of pre-roll, the offline tail until a recording asks for another
    // (`AudioRecorder::set_vad_tail`), and 60 ms of voice before speech starts.
    let smoothed_vad =
        SmoothedVad::new(Box::new(silero), 15, VadTail::Offline.hangover_frames(), 2);

    // Recorder with VAD plus a spectrum-level callback that forwards updates to
    // the frontend, and two callbacks that feed the live-transcription router
    // (cheap no-ops unless a session that takes that kind of audio is open):
    // every frame for a stream, the kept speech for long-form chunks.
    let recorder = AudioRecorder::new()
        .map_err(|e| anyhow::anyhow!("Failed to create AudioRecorder: {}", e))?
        .with_vad(Box::new(smoothed_vad))
        .with_speech_activity(speech)
        .with_level_callback({
            let app_handle = app_handle.clone();
            move |levels| {
                utils::emit_levels(&app_handle, &levels);
            }
        })
        .with_frame_callback({
            let stream_router = stream_router.clone();
            move |frame| stream_router.feed(frame)
        })
        .with_speech_callback({
            let stream_router = stream_router.clone();
            move |kept| stream_router.feed_speech(kept)
        });

    Ok(recorder)
}

/// The bundled Silero VAD model, which the recorder and long-form chunking
/// both run.
pub fn silero_vad_path(app: &tauri::AppHandle) -> Result<PathBuf, anyhow::Error> {
    app.path()
        .resolve(
            "resources/models/silero_vad_v4.onnx",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| anyhow::anyhow!("Failed to resolve VAD path: {}", e))
}

/* ──────────────────────────────────────────────────────────────── */

#[derive(Clone)]
pub struct AudioRecordingManager {
    state: Arc<Mutex<RecordingState>>,
    mode: Arc<Mutex<MicrophoneMode>>,
    app_handle: tauri::AppHandle,

    recorder: Arc<Mutex<Option<AudioRecorder>>>,
    is_open: Arc<Mutex<bool>>,
    is_recording: Arc<Mutex<bool>>,
    /// Lowers or mutes the computer's other audio while a recording is under
    /// way (`audio_ducking`).
    ducker: Arc<Ducker>,
    ducking: Arc<Mutex<DuckingGate>>,
    /// How many recordings have started, so a stop cue still waiting for
    /// other audio can tell that a newer recording began meanwhile.
    recordings_started: Arc<AtomicU64>,
    close_generation: Arc<AtomicU64>,
    /// Shared with the `TranscriptionManager`; the recorder's frame callback
    /// feeds raw frames into it while a live-transcription stream is active.
    stream_router: Arc<StreamRouter>,
    /// What the VAD has heard in the recording under way. Held here rather
    /// than read through `recorder`, whose lock an opening or draining
    /// microphone holds for seconds (see [`Self::speech_activity`]).
    speech: Arc<SpeechActivity>,
}

impl AudioRecordingManager {
    /* ---------- construction ------------------------------------------------ */

    pub fn new(
        app: &tauri::AppHandle,
        stream_router: Arc<StreamRouter>,
    ) -> Result<Self, anyhow::Error> {
        let settings = get_settings(app);
        let mode = if settings.always_on_microphone {
            MicrophoneMode::AlwaysOn
        } else {
            MicrophoneMode::OnDemand
        };

        let manager = Self {
            state: Arc::new(Mutex::new(RecordingState::Idle)),
            mode: Arc::new(Mutex::new(mode.clone())),
            app_handle: app.clone(),

            recorder: Arc::new(Mutex::new(None)),
            is_open: Arc::new(Mutex::new(false)),
            is_recording: Arc::new(Mutex::new(false)),
            ducker: Arc::new(Ducker::new()),
            ducking: Arc::new(Mutex::new(DuckingGate::default())),
            recordings_started: Arc::new(AtomicU64::new(0)),
            close_generation: Arc::new(AtomicU64::new(0)),
            stream_router,
            speech: Arc::new(SpeechActivity::default()),
        };

        // Always-on?  Open immediately.
        if matches!(mode, MicrophoneMode::AlwaysOn) {
            manager.start_microphone_stream()?;
        }

        Ok(manager)
    }

    /* ---------- helper methods --------------------------------------------- */

    fn get_effective_microphone_device(&self, settings: &AppSettings) -> Option<cpal::Device> {
        // Check if we're in clamshell mode and have a clamshell microphone configured.
        // The setting is checked first: the clamshell probe spawns `ioreg`, and
        // this runs on every recording start.
        let use_clamshell_mic =
            settings.clamshell_microphone.is_some() && clamshell::is_clamshell().unwrap_or(false);

        let device_name = if use_clamshell_mic {
            settings.clamshell_microphone.as_ref().unwrap()
        } else {
            settings.selected_microphone.as_ref()?
        };

        // Find the device by name
        match list_input_devices() {
            Ok(devices) => devices
                .into_iter()
                .find(|d| d.name == *device_name)
                .map(|d| d.device),
            Err(e) => {
                debug!("Failed to list devices, using default: {}", e);
                None
            }
        }
    }

    fn schedule_lazy_close(&self) {
        let gen = self.close_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let app = self.app_handle.clone();
        std::thread::spawn(move || {
            std::thread::sleep(STREAM_IDLE_TIMEOUT);
            let rm = app.state::<Arc<AudioRecordingManager>>();
            // Hold state lock across the check AND close to serialize against
            // try_start_recording, preventing a race where the stream is closed
            // under an active recording.
            let state = rm.state.lock().unwrap();
            if rm.close_generation.load(Ordering::SeqCst) == gen
                && matches!(*state, RecordingState::Idle)
            {
                // stop_microphone_stream does not acquire the state lock,
                // so holding it here is safe (no deadlock).
                info!(
                    "Closing idle microphone stream after {:?}",
                    STREAM_IDLE_TIMEOUT
                );
                rm.stop_microphone_stream();
            }
        });
    }

    /* ---------- other audio ------------------------------------------------- */

    /// Lower or mute the computer's other audio, as `audio_ducking` asks, for
    /// the recording under way.
    ///
    /// Called once the start cue has played, so the cue is heard at full
    /// volume, which means it can arrive after a quick tap has already ended
    /// the recording. The gate stops that: it opens when a recording starts and
    /// [`Self::restore_other_audio`] closes it, so a late call does nothing
    /// instead of leaving the computer quiet with nothing to bring it back.
    pub fn quiet_other_audio(&self) {
        let mut gate = self.ducking.lock().unwrap();
        let Some(how) = gate.quiet(|| quieting(&get_settings(&self.app_handle))) else {
            return;
        };
        debug!("Quieting other audio while recording: {how:?}");
        self.ducker.quiet(how);
    }

    /// Bring other audio back from [`Self::quiet_other_audio`]. Whatever ends
    /// a recording calls this; calling it again is harmless. The result
    /// settles once the audio is back up, for a cue that should be heard at
    /// full volume.
    pub fn restore_other_audio(&self) -> Restored {
        if !self.ducking.lock().unwrap().restore() {
            return Restored::settled();
        }
        self.ducker.restore()
    }

    /// How many recordings have started since the app did. A stop cue
    /// compares it before and after waiting, to stay out of a newer
    /// recording.
    pub fn recordings_started(&self) -> u64 {
        self.recordings_started.load(Ordering::SeqCst)
    }

    /// Let the user hear "Lower by `percent`%" on whatever is playing: it dips
    /// for a moment and comes back by itself. Not during a recording, which
    /// has the audio to itself.
    pub fn preview_ducking(&self, percent: u8) {
        // Never waits: this is called from the settings window, and a held
        // state lock is a recording starting or stopping, which is no time for
        // a preview either.
        let idle = self
            .state
            .try_lock()
            .is_ok_and(|state| matches!(*state, RecordingState::Idle));
        if idle {
            self.ducker.preview(Quieting::lower_by(percent));
        }
    }

    /// Put back anything a recording or a preview left quiet, before the app
    /// quits.
    pub fn restore_other_audio_before_exit(&self) {
        // Closes the gate first, so a start cue still playing cannot quiet the
        // audio again on the way out.
        let _ = self.restore_other_audio();
        self.ducker.restore_before_exit();
    }

    /* ---------- microphone life-cycle -------------------------------------- */

    pub fn preload_vad(&self) -> Result<(), anyhow::Error> {
        let mut recorder_opt = self.recorder.lock().unwrap();
        if recorder_opt.is_none() {
            let vad_path = silero_vad_path(&self.app_handle)?;
            *recorder_opt = Some(create_audio_recorder(
                &vad_path,
                &self.app_handle,
                self.stream_router.clone(),
                self.speech.clone(),
            )?);
        }
        Ok(())
    }

    pub fn start_microphone_stream(&self) -> Result<(), anyhow::Error> {
        let mut open_flag = self.is_open.lock().unwrap();
        if *open_flag {
            // `is_open` only records that a stream was opened at some point,
            // not that it still captures. If cpal has since reported a stream
            // error (mic unplugged, USB/Bluetooth dropout), rebuild it before
            // the next recording instead of handing the caller a stalled
            // recorder that returns silent, empty dictations. Backport of
            // Handy PRs #1838 / #1874.
            let needs_reopen = self
                .recorder
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|rec| rec.needs_reopen());
            if !needs_reopen {
                trace!("Microphone stream already active");
                return Ok(());
            }
            warn!("Microphone stream failed; reopening it");
            if let Some(rec) = self.recorder.lock().unwrap().as_mut() {
                let _ = rec.close();
            }
            *self.is_recording.lock().unwrap() = false;
            *open_flag = false;
            // Fall through: re-resolve the device (the selected one may be gone,
            // in which case the system default is used) and open fresh.
        }

        let start_time = Instant::now();

        // Get the selected device from settings, considering clamshell mode
        let settings = get_settings(&self.app_handle);
        let selected_device = self.get_effective_microphone_device(&settings);

        // The "is there any input device at all" check runs only after an open
        // has failed (below), not on every press: listing every input is slow,
        // and on Linux ALSA opens each PCM to probe it.

        // Ensure VAD is loaded if it wasn't for whatever reason
        self.preload_vad()?;

        let mut recorder_opt = self.recorder.lock().unwrap();
        if let Some(rec) = recorder_opt.as_mut() {
            if let Err(first_err) = rec.open(selected_device) {
                // The device or its cached config may have gone stale (unplugged,
                // Bluetooth profile switch, sample rate changed in the OS). The
                // recorder drops its config cache on a failed open, so
                // re-resolve the device and retry once before surfacing the
                // error. Backport of Handy PR #1582's cache-clear-and-retry.
                warn!("Recorder open failed ({first_err}); re-resolving device and retrying once");
                let fresh_device = self.get_effective_microphone_device(&settings);
                let using_default = fresh_device.is_none();
                if let Err(e) = rec.open(fresh_device) {
                    // With no usable device configured, a machine that has no
                    // input at all gets a clear message instead of whatever
                    // backend-specific error cpal produced for the default.
                    if using_default
                        && !list_input_devices()
                            .map(|devices| !devices.is_empty())
                            .unwrap_or(false)
                    {
                        return Err(anyhow::anyhow!("No input device found"));
                    }
                    return Err(anyhow::anyhow!("Failed to open recorder: {}", e));
                }
            }
        }

        *open_flag = true;
        // This timing covers through cpal's stream.play() returning — i.e. the
        // point cpal surfaces as "stream running." It does NOT guarantee the
        // host audio device is producing samples yet; the first input callback
        // fires asynchronously one buffer period later (hardware dependent,
        // typically ~10–200ms on macOS, longer on Bluetooth/USB).
        info!(
            "Microphone stream initialized in {:?}",
            start_time.elapsed()
        );
        Ok(())
    }

    pub fn stop_microphone_stream(&self) {
        let mut open_flag = self.is_open.lock().unwrap();
        if !*open_flag {
            return;
        }

        // A closed microphone is recording nothing, so nothing should still be
        // quiet for it. Normally already done when the recording ended.
        let _ = self.restore_other_audio();

        if let Some(rec) = self.recorder.lock().unwrap().as_mut() {
            // If still recording, stop first.
            if *self.is_recording.lock().unwrap() {
                let _ = rec.stop();
                *self.is_recording.lock().unwrap() = false;
            }
            let _ = rec.close();
        }

        *open_flag = false;
        debug!("Microphone stream stopped");
    }

    /* ---------- mode switching --------------------------------------------- */

    pub fn update_mode(&self, new_mode: MicrophoneMode) -> Result<(), anyhow::Error> {
        let cur_mode = self.mode.lock().unwrap().clone();

        match (cur_mode, &new_mode) {
            (MicrophoneMode::AlwaysOn, MicrophoneMode::OnDemand) => {
                if matches!(*self.state.lock().unwrap(), RecordingState::Idle) {
                    self.close_generation.fetch_add(1, Ordering::SeqCst);
                    self.stop_microphone_stream();
                }
            }
            (MicrophoneMode::OnDemand, MicrophoneMode::AlwaysOn) => {
                self.close_generation.fetch_add(1, Ordering::SeqCst);
                self.start_microphone_stream()?;
            }
            _ => {}
        }

        *self.mode.lock().unwrap() = new_mode;
        Ok(())
    }

    /* ---------- recording --------------------------------------------------- */

    /// Start recording for `binding_id`, keeping `tail` of trailing silence
    /// after each stretch of speech (see [`VadTail`]): the tail belongs to the
    /// model that will transcribe this recording, so every recording sets it.
    pub fn try_start_recording(&self, binding_id: &str, tail: VadTail) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();

        if let RecordingState::Idle = *state {
            // Ensure the microphone is open (on-demand mode) and still healthy
            // (both modes: an always-on stream can die when the device is
            // unplugged, and start_microphone_stream rebuilds it). A healthy
            // open stream returns immediately.
            if matches!(*self.mode.lock().unwrap(), MicrophoneMode::OnDemand) {
                // Cancel any pending lazy close
                self.close_generation.fetch_add(1, Ordering::SeqCst);
            }
            if let Err(e) = self.start_microphone_stream() {
                let msg = format!("{e}");
                error!("Failed to open microphone stream: {msg}");
                return Err(msg);
            }

            if let Some(rec) = self.recorder.lock().unwrap().as_ref() {
                rec.set_vad_tail(tail);
                if rec.start().is_ok() {
                    *self.is_recording.lock().unwrap() = true;
                    // From here the caller may quiet other audio, once its
                    // start cue has played (`quiet_other_audio`).
                    self.ducking.lock().unwrap().arm();
                    self.recordings_started.fetch_add(1, Ordering::SeqCst);
                    *state = RecordingState::Recording {
                        binding_id: binding_id.to_string(),
                    };
                    debug!("Recording started for binding {binding_id} ({tail:?} VAD tail)");
                    return Ok(());
                }
            }
            Err("Recorder not available".to_string())
        } else {
            Err("Already recording".to_string())
        }
    }

    /// Block until the microphone has started delivering audio for the current
    /// recording, or `timeout` elapses. Returns `true` if the stream went live
    /// in time. Used to delay the "start speaking" cue until capture is real,
    /// so the first words aren't lost during a cold-start/wake-up on slower
    /// devices. The recorder lock is released before waiting so this never
    /// blocks other recorder operations.
    pub fn wait_for_capture_ready(&self, timeout: Duration) -> bool {
        let handle = {
            let guard = self.recorder.lock().unwrap();
            match guard.as_ref() {
                Some(rec) => rec.capture_ready_handle(),
                None => return false,
            }
        };

        let (lock, cvar) = &*handle;
        let mut ready = lock.lock().unwrap();
        if *ready {
            return true;
        }
        let deadline = Instant::now() + timeout;
        while !*ready {
            let remaining = match deadline.checked_duration_since(Instant::now()) {
                Some(r) => r,
                None => break,
            };
            let (guard, res) = cvar.wait_timeout(ready, remaining).unwrap();
            ready = guard;
            if res.timed_out() {
                break;
            }
        }
        *ready
    }

    pub fn update_selected_device(&self) -> Result<(), anyhow::Error> {
        // If currently open, restart the microphone stream to use the new device
        if *self.is_open.lock().unwrap() {
            self.close_generation.fetch_add(1, Ordering::SeqCst);
            self.stop_microphone_stream();
            self.start_microphone_stream()?;
        }
        Ok(())
    }

    pub fn stop_recording(&self, binding_id: &str) -> Option<Vec<f32>> {
        let mut state = self.state.lock().unwrap();

        match *state {
            RecordingState::Recording {
                binding_id: ref active,
            } if active == binding_id => {
                *state = RecordingState::Idle;
                drop(state);

                // The stop paths bring other audio back before their stop cue;
                // this is for any path that ends a recording without one.
                let _ = self.restore_other_audio();

                // Optionally keep recording for a bit longer to capture trailing audio
                let settings = get_settings(&self.app_handle);
                if settings.extra_recording_buffer_ms > 0 {
                    debug!(
                        "Extra recording buffer: sleeping {}ms before stopping",
                        settings.extra_recording_buffer_ms
                    );
                    std::thread::sleep(Duration::from_millis(settings.extra_recording_buffer_ms));
                }

                let samples = if let Some(rec) = self.recorder.lock().unwrap().as_ref() {
                    match rec.stop() {
                        Ok(buf) => buf,
                        Err(e) => {
                            error!("stop() failed: {e}");
                            Vec::new()
                        }
                    }
                } else {
                    error!("Recorder not available");
                    Vec::new()
                };

                *self.is_recording.lock().unwrap() = false;

                // In on-demand mode, close the mic (lazily if the setting is enabled)
                if matches!(*self.mode.lock().unwrap(), MicrophoneMode::OnDemand) {
                    if get_settings(&self.app_handle).lazy_stream_close {
                        self.schedule_lazy_close();
                    } else {
                        self.stop_microphone_stream();
                    }
                }

                // Pad if very short
                Some(pad_short_recording(samples))
            }
            _ => None,
        }
    }
    pub fn is_recording(&self) -> bool {
        matches!(
            *self.state.lock().unwrap(),
            RecordingState::Recording { .. }
        )
    }

    /// What the VAD has heard in the recording under way, for telling when a
    /// hands-free dictation has gone quiet. Takes no lock, so the coordinator
    /// can look as often as it likes without ever waiting on a microphone that
    /// is opening or draining. Meaningless between recordings.
    pub fn speech_activity(&self) -> &SpeechActivity {
        &self.speech
    }

    /// Cancel any ongoing recording and hand back what it captured.
    ///
    /// The audio used to be discarded here, which made a cancel irreversible: an
    /// Esc pressed by mistake threw away everything the user had said. Returning
    /// it lets the caller keep a dismissed dictation for recovery; callers that
    /// do not want it simply drop it. The samples are *not* padded (see
    /// [`pad_short_recording`]), so the caller can still tell a real utterance
    /// from a stray key press by its length.
    pub fn cancel_recording(&self) -> Option<CancelledRecording> {
        let mut state = self.state.lock().unwrap();

        if let RecordingState::Recording { ref binding_id } = *state {
            let binding_id = binding_id.clone();
            *state = RecordingState::Idle;
            drop(state);

            // A cancelled recording gives other audio back too. The old mute
            // was only undone here when the microphone closed straight away,
            // so with an always-on or lazily closed microphone, Esc left the
            // computer muted until the next dictation ended.
            let _ = self.restore_other_audio();

            let samples = match self.recorder.lock().unwrap().as_ref() {
                Some(rec) => rec.stop().unwrap_or_else(|e| {
                    error!("stop() failed while cancelling: {e}");
                    Vec::new()
                }),
                None => Vec::new(),
            };

            *self.is_recording.lock().unwrap() = false;

            // In on-demand mode, close the mic (lazily if the setting is enabled)
            if matches!(*self.mode.lock().unwrap(), MicrophoneMode::OnDemand) {
                if get_settings(&self.app_handle).lazy_stream_close {
                    self.schedule_lazy_close();
                } else {
                    self.stop_microphone_stream();
                }
            }

            return Some(CancelledRecording {
                binding_id,
                samples,
            });
        }
        None
    }
}

/// What a cancelled recording had captured when it was cancelled.
pub struct CancelledRecording {
    pub binding_id: String,
    /// 16 kHz mono, unpadded.
    pub samples: Vec<f32>,
}

/// Pad a recording shorter than a second to 1.25 s of audio. Whisper-style
/// models misbehave on very short input, and every path that transcribes a
/// recording — a normal stop, or recovering a cancelled one — has to agree.
pub fn pad_short_recording(mut samples: Vec<f32>) -> Vec<f32> {
    let len = samples.len();
    if len < WHISPER_SAMPLE_RATE && len > 0 {
        samples.resize(WHISPER_SAMPLE_RATE * 5 / 4, 0.0);
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOWER: Quieting = Quieting::Lower { keep: 0.3 };

    #[test]
    fn the_gate_quiets_only_a_recording_that_has_started() {
        let mut gate = DuckingGate::default();
        assert_eq!(gate.quiet(|| Some(LOWER)), None, "nothing is recording");
        gate.arm();
        assert_eq!(gate.quiet(|| Some(LOWER)), Some(LOWER));
        assert_eq!(gate.quiet(|| Some(LOWER)), None, "already quiet");
        assert!(gate.restore());
    }

    #[test]
    fn a_quiet_that_lands_after_the_restore_does_nothing() {
        // A quick tap: the recording ends before its start cue has played.
        let mut gate = DuckingGate::default();
        gate.arm();
        assert!(!gate.restore(), "nothing was quieted yet");
        assert_eq!(gate.quiet(|| Some(LOWER)), None);
        assert!(!gate.restore());
    }

    #[test]
    fn restoring_twice_brings_the_audio_back_once() {
        let mut gate = DuckingGate::default();
        gate.arm();
        assert_eq!(gate.quiet(|| Some(Quieting::Mute)), Some(Quieting::Mute));
        assert!(gate.restore());
        assert!(!gate.restore());
    }

    #[test]
    fn ducking_off_leaves_the_gate_open_and_asks_nothing_when_closed() {
        let mut gate = DuckingGate::default();
        assert_eq!(
            gate.quiet(|| panic!("a closed gate does not read the settings")),
            None
        );
        gate.arm();
        assert_eq!(gate.quiet(|| None), None);
        assert!(!gate.restore(), "nothing to bring back");
    }
}
