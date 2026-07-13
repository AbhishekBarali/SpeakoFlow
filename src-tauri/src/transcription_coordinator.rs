use crate::actions::ACTION_MAP;
use crate::lock_watch::LockWatch;
use crate::managers::audio::AudioRecordingManager;
use log::{debug, error, warn};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);

/// Hard safety cap on how long a single recording may run before it is
/// automatically finalized (stopped → saved → transcribed), even if the user
/// never releases the key or taps stop.
///
/// Why this exists: a recording's audio is only ever persisted *after* it
/// stops, and — for a streaming-capable model (the default) — the ASR engine
/// runs continuous GPU inference for the entire recording. An unbounded
/// recording (a stuck/held key, a forgotten hands-free session, or a genuine
/// runaway) therefore risks both losing every captured second of audio if the
/// app or system falls over mid-session, and pinning the GPU under sustained
/// load indefinitely. Auto-finalizing at the cap guarantees the audio is
/// written to disk and bounds worst-case resource use. Deliberately generous so
/// ordinary long-form dictation is never cut short.
const MAX_RECORDING_DURATION: Duration = Duration::from_secs(30 * 60);

/// Whether a fired max-duration timer should actually finalize the recording.
/// Only when its generation still matches the currently-active recording (no
/// newer recording has started since the timer was armed) *and* something is
/// still recording — so a stale timer can never stop a later, unrelated
/// recording or fire while idle/processing.
fn max_duration_should_stop(
    timer_generation: u64,
    current_generation: u64,
    is_recording: bool,
) -> bool {
    is_recording && timer_generation == current_generation
}

/// Suffix marking the auto-derived "Shift" variant of a recording binding — the
/// hands-free counterpart of a hold shortcut (e.g. `transcribe` → `transcribe.lock`).
pub const LOCK_SUFFIX: &str = ".lock";

/// How a recording is driven.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordingMode {
    /// Push-to-talk: hold the shortcut to record, release to stop & transcribe.
    Hold,
    /// Hands-free: press to start, press again (or commit/cancel) to stop.
    /// Releases are ignored.
    Lock,
}

/// Resolve the recording mode for a shortcut event. The base shortcut uses the
/// user's default mode; its Shift variant uses the opposite. With push-to-talk
/// on (default) the base shortcut holds and Shift locks hands-free; with it off
/// the base shortcut locks and Shift holds.
pub fn recording_mode(push_to_talk_default: bool, is_lock_variant: bool) -> RecordingMode {
    let base_holds = push_to_talk_default;
    let holds = if is_lock_variant {
        !base_holds
    } else {
        base_holds
    };
    if holds {
        RecordingMode::Hold
    } else {
        RecordingMode::Lock
    }
}

/// Commands processed sequentially by the coordinator thread.
enum Command {
    Input {
        binding_id: String,
        hotkey_string: String,
        is_pressed: bool,
        mode: RecordingMode,
    },
    /// Finish the active recording and transcribe it (overlay "done" tick /
    /// assistant panel finish button). No-op unless something is recording.
    Commit,
    /// Convert the active push-to-talk (hold) recording to hands-free (lock)
    /// mode without stopping it — the runtime "tap Shift to lock" gesture. No-op
    /// unless a hold recording is active.
    Lock,
    Cancel {
        recording_was_active: bool,
    },
    /// Fired by a per-recording timer when [`MAX_RECORDING_DURATION`] elapses.
    /// Auto-finalizes the recording iff `generation` still matches the active
    /// recording (see [`max_duration_should_stop`]). No-op otherwise.
    MaxDuration {
        generation: u64,
    },
    ProcessingFinished,
}

/// Pipeline lifecycle, owned exclusively by the coordinator thread.
enum Stage {
    Idle,
    Recording {
        binding_id: String,
        mode: RecordingMode,
    },
    Processing,
}

/// Serialises all transcription lifecycle events through a single thread
/// to eliminate race conditions between keyboard shortcuts, signals, and
/// the async transcribe-paste pipeline.
pub struct TranscriptionCoordinator {
    tx: Sender<Command>,
}

pub fn is_transcribe_binding(id: &str) -> bool {
    id == "transcribe" || id == "transcribe_with_post_process" || id == "assistant"
}

impl TranscriptionCoordinator {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();

        // A clone of the command sender handed to per-recording max-duration
        // timers so they can post a `MaxDuration` command back to this same
        // single-threaded loop (keeping all lifecycle transitions serialized).
        let timer_tx = tx.clone();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                let mut last_press: Option<Instant> = None;
                // Monotonic id for the active recording, bumped on every start.
                // A max-duration timer captures the value at arm time and only
                // fires if it still matches — so it can never stop a newer
                // recording (see `max_duration_should_stop`).
                let mut generation: u64 = 0;

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Command::Input {
                            binding_id,
                            hotkey_string,
                            is_pressed,
                            mode,
                        } => {
                            if is_pressed {
                                // Debounce rapid-fire / auto-repeat press events.
                                let now = Instant::now();
                                if last_press.map_or(false, |t| now.duration_since(t) < DEBOUNCE) {
                                    debug!("Debounced press for '{binding_id}'");
                                    continue;
                                }
                                last_press = Some(now);

                                match &stage {
                                    Stage::Idle => {
                                        start(&app, &mut stage, &binding_id, &hotkey_string, mode);
                                        if matches!(stage, Stage::Recording { .. }) {
                                            // Arm the max-duration safety timer
                                            // for this recording. Tagged with a
                                            // fresh generation so it only ever
                                            // finalizes THIS recording.
                                            generation = generation.wrapping_add(1);
                                            let g = generation;
                                            let ttx = timer_tx.clone();
                                            thread::spawn(move || {
                                                thread::sleep(MAX_RECORDING_DURATION);
                                                let _ = ttx
                                                    .send(Command::MaxDuration { generation: g });
                                            });
                                            match mode {
                                                // Hands-free (Push-to-talk OFF): a
                                                // tap-to-toggle recording. Tell the
                                                // overlay/pill so it shows the quiet
                                                // "locked / tap again to stop" cue.
                                                RecordingMode::Lock => {
                                                    use tauri::Emitter;
                                                    let _ = app.emit("recording-locked", true);
                                                }
                                                // Push-to-talk (hold): nothing extra
                                                // to arm — tap-to-lock was removed in
                                                // favour of the simple hold/tap model.
                                                RecordingMode::Hold => {}
                                            }
                                        }
                                    }
                                    // A second press only stops a hands-free
                                    // (locked) recording. A held one stops on
                                    // release, so ignore extra presses (incl.
                                    // key auto-repeat) while holding.
                                    Stage::Recording {
                                        binding_id: id,
                                        mode: RecordingMode::Lock,
                                    } if id == &binding_id => {
                                        stop(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                    _ => {
                                        debug!("Ignoring press for '{binding_id}': held or busy")
                                    }
                                }
                            } else {
                                // Release only stops a push-to-talk (hold)
                                // recording. Hands-free ignores releases, which
                                // are unreliable for global shortcuts anyway.
                                let should_stop = matches!(
                                    &stage,
                                    Stage::Recording { binding_id: id, mode: RecordingMode::Hold }
                                        if id == &binding_id
                                );
                                if should_stop {
                                    stop(&app, &mut stage, &binding_id, &hotkey_string);
                                }
                            }
                        }
                        Command::Commit => {
                            // Finish + transcribe whatever is recording. Used by
                            // the overlay tick and the assistant finish button so
                            // a hands-free recording can end without the keyboard.
                            if let Stage::Recording { binding_id, .. } = &stage {
                                let id = binding_id.clone();
                                stop(&app, &mut stage, &id, "commit");
                            }
                        }
                        Command::Lock => {
                            // Tap-to-lock: flip an active push-to-talk hold to
                            // hands-free so the user can release the keys and keep
                            // talking. Ignored unless a hold recording is active.
                            // Works for both dictation and the assistant (each has
                            // its own configured lock shortcut).
                            if let Stage::Recording { mode, .. } = &mut stage {
                                if *mode == RecordingMode::Hold {
                                    *mode = RecordingMode::Lock;
                                    if let Some(lw) = app.try_state::<LockWatch>() {
                                        lw.disarm();
                                    }
                                    use tauri::Emitter;
                                    let _ = app.emit("recording-locked", true);
                                    // Audible confirmation that the hold is now
                                    // hands-free. Respects the audio-feedback
                                    // toggle/volume and is a no-op when off.
                                    crate::audio_feedback::play_feedback_sound(
                                        &app,
                                        crate::audio_feedback::SoundType::Lock,
                                    );
                                }
                            }
                        }
                        Command::Cancel {
                            recording_was_active,
                        } => {
                            if let Some(lw) = app.try_state::<LockWatch>() {
                                lw.disarm();
                            }
                            // Don't reset during processing — wait for the pipeline to finish.
                            if !matches!(stage, Stage::Processing)
                                && (recording_was_active
                                    || matches!(stage, Stage::Recording { .. }))
                            {
                                stage = Stage::Idle;
                            }
                        }
                        Command::MaxDuration { generation: g } => {
                            let is_recording = matches!(stage, Stage::Recording { .. });
                            if max_duration_should_stop(g, generation, is_recording) {
                                if let Stage::Recording { binding_id, .. } = &stage {
                                    let minutes = MAX_RECORDING_DURATION.as_secs() / 60;
                                    warn!(
                                        "Recording reached the {minutes}-minute safety cap; \
                                         auto-finalizing to save the audio and stop runaway \
                                         streaming/GPU load"
                                    );
                                    let id = binding_id.clone();
                                    // Let the UI explain why recording stopped
                                    // on its own (the transcript is still saved).
                                    {
                                        use tauri::Emitter;
                                        let _ = app.emit("recording-auto-stopped", minutes);
                                    }
                                    stop(&app, &mut stage, &id, "max-duration");
                                }
                            } else {
                                debug!(
                                    "Ignoring stale max-duration timer (gen {g} vs {generation}, \
                                     recording={is_recording})"
                                );
                            }
                        }
                        Command::ProcessingFinished => {
                            stage = Stage::Idle;
                        }
                    }
                }
                debug!("Transcription coordinator exited");
            }));
            if let Err(e) = result {
                error!("Transcription coordinator panicked: {e:?}");
            }
        });

        Self { tx }
    }

    /// Send a keyboard/signal input event for a transcribe binding. `mode`
    /// selects push-to-talk (hold) vs hands-free (lock). Programmatic triggers
    /// (pill mic, signals/CLI) pass `is_pressed: true` with `RecordingMode::Lock`.
    pub fn send_input(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        mode: RecordingMode,
    ) {
        if self
            .tx
            .send(Command::Input {
                binding_id: binding_id.to_string(),
                hotkey_string: hotkey_string.to_string(),
                is_pressed,
                mode,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_cancel(&self, recording_was_active: bool) {
        if self
            .tx
            .send(Command::Cancel {
                recording_was_active,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    /// Finish + transcribe the active recording, if any. Drives the overlay's
    /// "done" tick and the assistant panel's finish button so a hands-free
    /// recording can be ended without touching the keyboard.
    pub fn notify_commit(&self) {
        if self.tx.send(Command::Commit).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }

    /// Convert the active push-to-talk hold recording to hands-free, if any.
    /// Driven by the tap-to-lock watcher when the user taps Shift mid-recording.
    pub fn notify_lock(&self) {
        if self.tx.send(Command::Lock).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_processing_finished(&self) {
        if self.tx.send(Command::ProcessingFinished).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }
}

fn start(
    app: &AppHandle,
    stage: &mut Stage,
    binding_id: &str,
    hotkey_string: &str,
    mode: RecordingMode,
) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.start(app, binding_id, hotkey_string);
    if app
        .try_state::<Arc<AudioRecordingManager>>()
        .map_or(false, |a| a.is_recording())
    {
        *stage = Stage::Recording {
            binding_id: binding_id.to_string(),
            mode,
        };
    } else {
        debug!("Start for '{binding_id}' did not begin recording; staying idle");
    }
}

fn stop(app: &AppHandle, stage: &mut Stage, binding_id: &str, hotkey_string: &str) {
    // Recording is ending — make sure tap-to-lock isn't left armed.
    if let Some(lw) = app.try_state::<LockWatch>() {
        lw.disarm();
    }
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.stop(app, binding_id, hotkey_string);
    *stage = Stage::Processing;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_duration_stops_only_matching_active_recording() {
        // Same generation, still recording → finalize.
        assert!(max_duration_should_stop(1, 1, true));
        // Same generation but no longer recording (already stopped / idle /
        // processing) → no-op.
        assert!(!max_duration_should_stop(1, 1, false));
        // A newer recording has started since the timer was armed → the stale
        // timer must never stop it.
        assert!(!max_duration_should_stop(1, 2, true));
        // Stale timer while idle → no-op.
        assert!(!max_duration_should_stop(1, 2, false));
    }

    #[test]
    fn max_duration_cap_is_generous_but_bounded() {
        // Long enough that ordinary long-form dictation is never cut short,
        // but finite so a runaway recording can't run forever.
        let secs = MAX_RECORDING_DURATION.as_secs();
        assert!(secs >= 10 * 60, "cap should not cut short normal dictation");
        assert!(secs <= 60 * 60, "cap must bound a runaway recording");
    }
}
