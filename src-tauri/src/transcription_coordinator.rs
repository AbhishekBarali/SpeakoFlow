use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use log::{debug, error, warn};
use std::sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError, Sender};
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

/// Backstop for the post-recording pipeline: the coordinator sits in
/// [`Stage::Processing`] until the pipeline's `FinishGuard` reports back, and
/// while it is there **no shortcut does anything** — a press is "busy", and even
/// cancel is deliberately ignored so it cannot reset state mid-paste. So an await
/// in the pipeline that never resolves does not just lose one dictation, it takes
/// dictation out until the app is restarted, with the overlay stuck on screen.
/// That is not a state any single fix can be trusted to make unreachable, so the
/// stage has a cap of its own.
///
/// The budget has to clear the slowest *legitimate* pipeline, which is dominated
/// by transcription of the recording itself: a large local model on CPU can run
/// several times slower than real time, and cleanup can add a cold engine start
/// on top. Hence a flat allowance plus a generous multiple of the recording's own
/// length ([`processing_budget`]) — comfortably longer than any real run, and
/// still finite. Recovery is silent: the pipeline may yet finish and paste, so
/// this only takes back the coordinator and the overlay.
const PROCESSING_BASE_BUDGET: Duration = Duration::from_secs(5 * 60);
const PROCESSING_LENGTH_FACTOR: u32 = 6;

/// Deadline for a recording of `recorded` length that has just stopped.
fn processing_budget(recorded: Duration) -> Duration {
    PROCESSING_BASE_BUDGET.saturating_add(recorded.saturating_mul(PROCESSING_LENGTH_FACTOR))
}

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

/// Whether an expired processing cap should force the coordinator back to idle.
/// Mirrors [`max_duration_should_stop`]: only for the pipeline it was armed for,
/// and only while that pipeline still holds the stage.
fn processing_stall_should_reset(
    timer_generation: u64,
    current_generation: u64,
    is_processing: bool,
) -> bool {
    is_processing && timer_generation == current_generation
}

/// What a stage's deadline means once it expires. Each stage arms its own on
/// entry and drops it on exit, so nothing sleeps on behalf of a stage that has
/// already been left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Deadline {
    MaxRecording { generation: u64 },
    ProcessingStalled { generation: u64 },
}

impl Deadline {
    fn command(self) -> Command {
        match self {
            Deadline::MaxRecording { generation } => Command::MaxDuration { generation },
            Deadline::ProcessingStalled { generation } => Command::ProcessingStalled { generation },
        }
    }
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
    Cancel {
        recording_was_active: bool,
    },
    /// Fired by the coordinator's receive deadline when [`MAX_RECORDING_DURATION`] elapses.
    /// Auto-finalizes the recording iff `generation` still matches the active
    /// recording (see [`max_duration_should_stop`]). No-op otherwise.
    MaxDuration {
        generation: u64,
    },
    /// Fired by the coordinator's receive deadline when the post-recording
    /// pipeline has held [`Stage::Processing`] past its budget without ever
    /// sending [`Command::ProcessingFinished`]. Forces the stage back to idle so
    /// shortcuts work again (see [`processing_stall_should_reset`]).
    ProcessingStalled {
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

/// Which stage the coordinator is in, without its payload. Comparing this before
/// and after a command is how every transition arms or drops the stage's deadline
/// in one place, instead of each handler remembering to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StageKind {
    Idle,
    Recording,
    Processing,
}

fn stage_kind(stage: &Stage) -> StageKind {
    match stage {
        Stage::Idle => StageKind::Idle,
        Stage::Recording { .. } => StageKind::Recording,
        Stage::Processing => StageKind::Processing,
    }
}

/// Wait for input or the current stage's deadline on the coordinator's own
/// thread. A stage that has been left leaves no sleeping timer thread behind.
fn receive_command(
    rx: &Receiver<Command>,
    deadline: Option<(Instant, Deadline)>,
) -> Result<Command, RecvError> {
    let Some((at, expiry)) = deadline else {
        return rx.recv();
    };
    let remaining = at.saturating_duration_since(Instant::now());
    // A busy command queue must not starve the safety cap.
    if remaining.is_zero() {
        return Ok(expiry.command());
    }
    match rx.recv_timeout(remaining) {
        Ok(cmd) => Ok(cmd),
        Err(RecvTimeoutError::Timeout) => Ok(expiry.command()),
        Err(RecvTimeoutError::Disconnected) => Err(RecvError),
    }
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
                // Monotonic id for the active recording, bumped on every start.
                // A max-duration timer captures the value at arm time and only
                // fires if it still matches — so it can never stop a newer
                // recording (see `max_duration_should_stop`).
                let mut generation: u64 = 0;
                let mut deadline = None;
                // When the active recording started, so the processing cap can
                // scale with how much audio the pipeline actually has to chew on.
                let mut recording_since = Instant::now();

                loop {
                    let Ok(cmd) = receive_command(&rx, deadline) else {
                        break;
                    };
                    let was = stage_kind(&stage);
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
                                            // Tag this recording so neither of
                                            // its caps can ever fire against a
                                            // later, unrelated one.
                                            generation = generation.wrapping_add(1);
                                            recording_since = Instant::now();
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
                        Command::Cancel {
                            recording_was_active,
                        } => {
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
                        Command::ProcessingStalled { generation: g } => {
                            let is_processing = matches!(stage, Stage::Processing);
                            if processing_stall_should_reset(g, generation, is_processing) {
                                // Loud on purpose: reaching this means some await
                                // in the pipeline never resolved, which is a bug
                                // worth a log line even though the app recovers.
                                error!(
                                    "Transcription pipeline never reported back; releasing the \
                                     coordinator so shortcuts work again"
                                );
                                stage = Stage::Idle;
                                crate::utils::hide_recording_overlay(&app);
                                crate::tray::change_tray_icon(
                                    &app,
                                    crate::tray::TrayIconState::Idle,
                                );
                            } else {
                                debug!(
                                    "Ignoring stale processing cap (gen {g} vs {generation}, \
                                     processing={is_processing})"
                                );
                            }
                        }
                        Command::ProcessingFinished => {
                            stage = Stage::Idle;
                        }
                    }
                    // One place arms and drops every stage's deadline. Entering a
                    // stage starts its cap; leaving one throws it away; a command
                    // that changes nothing (a debounced press, an ignored cancel)
                    // must not push the current cap further out.
                    let now = stage_kind(&stage);
                    if was != now {
                        deadline = match now {
                            StageKind::Idle => None,
                            StageKind::Recording => Some((
                                Instant::now() + MAX_RECORDING_DURATION,
                                Deadline::MaxRecording { generation },
                            )),
                            StageKind::Processing => Some((
                                Instant::now() + processing_budget(recording_since.elapsed()),
                                Deadline::ProcessingStalled { generation },
                            )),
                        };
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
    fn recording_deadline_fires_without_input() {
        let (_tx, rx) = mpsc::channel();
        let deadline = Instant::now() + Duration::from_millis(20);
        assert!(matches!(
            receive_command(
                &rx,
                Some((deadline, Deadline::MaxRecording { generation: 7 }))
            )
            .unwrap(),
            Command::MaxDuration { generation: 7 }
        ));
        assert!(Instant::now() >= deadline);
    }

    #[test]
    fn expired_recording_deadline_takes_priority_over_queued_input() {
        let (tx, rx) = mpsc::channel();
        tx.send(Command::Commit).unwrap();
        assert!(matches!(
            receive_command(
                &rx,
                Some((Instant::now(), Deadline::MaxRecording { generation: 8 }))
            )
            .unwrap(),
            Command::MaxDuration { generation: 8 }
        ));
        // Once recording ends, clearing the deadline restores ordinary input.
        assert!(matches!(
            receive_command(&rx, None).unwrap(),
            Command::Commit
        ));
    }

    #[test]
    fn recording_wait_accepts_cancel_and_new_deadline_without_stale_timeout() {
        let (tx, rx) = mpsc::channel();
        tx.send(Command::Cancel {
            recording_was_active: true,
        })
        .unwrap();
        assert!(matches!(
            receive_command(
                &rx,
                Some((
                    Instant::now() + MAX_RECORDING_DURATION,
                    Deadline::MaxRecording { generation: 1 }
                ))
            )
            .unwrap(),
            Command::Cancel { .. }
        ));
        // A subsequent recording owns its own deadline; there is no timer left
        // to send a message for the cancelled recording.
        assert!(matches!(
            receive_command(
                &rx,
                Some((Instant::now(), Deadline::MaxRecording { generation: 2 }))
            )
            .unwrap(),
            Command::MaxDuration { generation: 2 }
        ));
        assert!(rx.try_recv().is_err());
        drop(tx);
        assert!(receive_command(&rx, None).is_err());
    }

    #[test]
    fn a_stalled_pipeline_deadline_fires_its_own_command() {
        // The regression this guards: an await in the pipeline that never
        // resolves leaves Stage::Processing forever, and while the coordinator is
        // there every shortcut — cancel included — is ignored, so dictation is
        // dead until the app restarts.
        let (_tx, rx) = mpsc::channel();
        assert!(matches!(
            receive_command(
                &rx,
                Some((
                    Instant::now(),
                    Deadline::ProcessingStalled { generation: 4 }
                ))
            )
            .unwrap(),
            Command::ProcessingStalled { generation: 4 }
        ));
    }

    #[test]
    fn a_pipeline_that_reports_back_beats_its_cap() {
        let (tx, rx) = mpsc::channel();
        tx.send(Command::ProcessingFinished).unwrap();
        assert!(matches!(
            receive_command(
                &rx,
                Some((
                    Instant::now() + PROCESSING_BASE_BUDGET,
                    Deadline::ProcessingStalled { generation: 5 }
                ))
            )
            .unwrap(),
            Command::ProcessingFinished
        ));
    }

    #[test]
    fn processing_stall_resets_only_the_pipeline_it_was_armed_for() {
        // Same generation, still processing → release the coordinator.
        assert!(processing_stall_should_reset(3, 3, true));
        // A newer recording already owns the stage → the old cap must not touch it.
        assert!(!processing_stall_should_reset(3, 4, true));
        // Already idle (the pipeline reported back) → nothing to release.
        assert!(!processing_stall_should_reset(3, 3, false));
    }

    #[test]
    fn the_processing_budget_clears_a_slow_local_transcription() {
        // A short dictation still gets minutes of slack: a cold cleanup engine
        // alone can spend ~3 of them starting up.
        assert!(processing_budget(Duration::from_secs(4)) >= Duration::from_secs(5 * 60));
        // A long recording scales, because transcription time tracks its length —
        // a large local model on CPU runs slower than real time.
        let long = processing_budget(MAX_RECORDING_DURATION);
        assert!(long > MAX_RECORDING_DURATION * 5);
        // Still finite, which is the whole point.
        assert!(long < Duration::from_secs(6 * 60 * 60));
    }

    #[test]
    fn every_stage_maps_to_its_own_deadline_kind() {
        assert!(stage_kind(&Stage::Idle) == StageKind::Idle);
        assert!(stage_kind(&Stage::Processing) == StageKind::Processing);
        assert!(
            stage_kind(&Stage::Recording {
                binding_id: "transcribe".to_string(),
                mode: RecordingMode::Hold,
            }) == StageKind::Recording
        );
    }

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
