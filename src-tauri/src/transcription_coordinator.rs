use crate::actions::ACTION_MAP;
use crate::lock_watch::LockWatch;
use crate::managers::audio::AudioRecordingManager;
use crate::settings::get_settings;
use log::{debug, error, warn};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);

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

/// Whether the tap-to-lock `lock_key` is entirely contained in the active record
/// `shortcut`. When it is, the lock key is already held to record, so arming
/// tap-to-lock would fire immediately off that held key's auto-repeat and wrongly
/// lock the recording — callers skip arming in that case. Tokens are lowercased
/// and modifier aliases unified so "alt"/"option" and "cmd"/"super" still match.
fn tap_lock_within_shortcut(lock_key: &str, shortcut: &str) -> bool {
    fn norm(token: &str) -> String {
        match token.trim().to_ascii_lowercase().as_str() {
            "control" => "ctrl".to_string(),
            "option" | "opt" => "alt".to_string(),
            "cmd" | "command" | "meta" | "win" | "windows" => "super".to_string(),
            other => other.to_string(),
        }
    }
    fn tokens(s: &str) -> std::collections::HashSet<String> {
        s.split('+').map(norm).filter(|t| !t.is_empty()).collect()
    }
    let lock = tokens(lock_key);
    if lock.is_empty() {
        return false;
    }
    lock.is_subset(&tokens(shortcut))
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

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                let mut last_press: Option<Instant> = None;

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
                                            match mode {
                                                // Hands-free from the start shows
                                                // the "press again / click done"
                                                // controls right away.
                                                RecordingMode::Lock => {
                                                    use tauri::Emitter;
                                                    let _ = app.emit("recording-locked", true);
                                                }
                                                // Push-to-talk: arm tap-to-lock so a
                                                // tap of the lock shortcut can convert
                                                // this hold to hands-free mid-recording.
                                                // Both dictation and the assistant
                                                // support this, each with its own
                                                // configured shortcut (the assistant's
                                                // defaults to Shift) so they can differ.
                                                // An empty shortcut means off.
                                                RecordingMode::Hold => {
                                                    let settings = get_settings(&app);
                                                    let shortcut = if binding_id == "assistant" {
                                                        settings.assistant_tap_to_lock_key.trim()
                                                    } else {
                                                        settings.tap_to_lock_key.trim()
                                                    };
                                                    // Skip arming when the lock key is itself part
                                                    // of the record shortcut being held (e.g. lock
                                                    // "space" while holding "ctrl+alt+space"): the
                                                    // held key auto-repeats key-down events, which
                                                    // would fire the watcher and instantly (and
                                                    // wrongly) lock the recording. The lock key must
                                                    // live outside the record shortcut to work.
                                                    if !shortcut.is_empty()
                                                        && !tap_lock_within_shortcut(
                                                            shortcut,
                                                            &hotkey_string,
                                                        )
                                                    {
                                                        if let Some(lw) =
                                                            app.try_state::<LockWatch>()
                                                        {
                                                            lw.arm(shortcut);
                                                        }
                                                    }
                                                }
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
