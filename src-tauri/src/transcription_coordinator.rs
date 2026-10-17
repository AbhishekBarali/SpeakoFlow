use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use log::{debug, error, info, warn};
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

/// How long before an automatic silence stop the recording pill starts
/// counting down. Never more than half the timeout ([`silence_warning`]), so a
/// short timeout does not spend most of every pause warning about itself.
const SILENCE_WARNING: Duration = Duration::from_secs(2);

/// How often a dictation that is counting down looks for speech again, so
/// speaking clears the countdown at once rather than a whole warning later.
const SILENCE_POLL: Duration = Duration::from_millis(100);

/// The VAD judges 30 ms frames, so looking more often learns nothing new. Also
/// the floor that keeps a microphone which has stopped delivering audio (whose
/// silence then stops growing) from turning the countdown into a spin.
const SILENCE_MIN_POLL: Duration = Duration::from_millis(30);

/// The two shortcuts that dictate text (one with AI cleanup). The assistant's
/// ask records too, but it is not dictation.
fn is_dictation_binding(id: &str) -> bool {
    id == "transcribe" || id == crate::settings::CLEANUP_BINDING_ID
}

/// The silence after which a recording ends by itself, if it gets one at all
/// (`dictation_silence_stop_secs`).
///
/// Only a hands-free dictation (`hands_free`): one started with a tap, one a
/// held dictation turned into with the keys plus Space or a quick tap, and one
/// started by `--toggle-transcription` and the like. A held recording already
/// ends when the key is let go, so stopping it under the user's finger could
/// only surprise them. The assistant's ask, calls and meetings each have their
/// own idea of when someone has finished, and are left to it.
pub(crate) fn silence_timeout_for(
    binding_id: &str,
    mode: RecordingMode,
    secs: u32,
) -> Option<Duration> {
    (is_dictation_binding(binding_id) && mode == RecordingMode::Lock && secs > 0)
        .then(|| Duration::from_secs(u64::from(secs)))
}

/// Where a hands-free dictation stands after `quiet_for` without speech.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SilenceVerdict {
    /// Speech was recent: nothing to show.
    Listening,
    /// Quiet long enough that the pill counts down: the recording stops in
    /// `remaining` unless speech resumes.
    Ending { remaining: Duration },
    /// Quiet for the whole timeout: stop, as a second tap would.
    Stop,
}

/// The countdown before an automatic stop.
fn silence_warning(timeout: Duration) -> Duration {
    SILENCE_WARNING.min(timeout / 2)
}

pub(crate) fn silence_verdict(quiet_for: Duration, timeout: Duration) -> SilenceVerdict {
    let remaining = timeout.saturating_sub(quiet_for);
    if remaining.is_zero() {
        SilenceVerdict::Stop
    } else if remaining <= silence_warning(timeout) {
        SilenceVerdict::Ending { remaining }
    } else {
        SilenceVerdict::Listening
    }
}

/// How long to wait before looking at a dictation's silence again.
///
/// Until the countdown would begin nothing can bring the stop closer, since
/// speech only pushes it further away, so the wait runs straight to that point.
/// During the countdown it looks every [`SILENCE_POLL`], so speaking clears the
/// cue promptly, and the last wait ends exactly when the timeout does instead
/// of up to a poll after it.
pub(crate) fn next_silence_check(quiet_for: Duration, timeout: Duration) -> Duration {
    let until_stop = timeout.saturating_sub(quiet_for);
    let until_countdown = until_stop.saturating_sub(silence_warning(timeout));
    let wait = if until_countdown.is_zero() {
        until_stop.min(SILENCE_POLL)
    } else {
        until_countdown
    };
    wait.max(SILENCE_MIN_POLL)
}

/// Whether a silence check may act. Mirrors [`max_duration_should_stop`]: only
/// for the recording it was armed for, and only while that is still recording,
/// so a check left over from an earlier dictation can never stop a later one.
fn silence_check_applies(
    check_generation: u64,
    current_generation: u64,
    is_recording: bool,
) -> bool {
    is_recording && check_generation == current_generation
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
    SilenceCheck { generation: u64 },
}

impl Deadline {
    fn command(self) -> Command {
        match self {
            Deadline::MaxRecording { generation } => Command::MaxDuration { generation },
            Deadline::ProcessingStalled { generation } => Command::ProcessingStalled { generation },
            Deadline::SilenceCheck { generation } => Command::SilenceCheck { generation },
        }
    }
}

/// The sooner of two deadlines. A tie goes to `first`, so a stage's cap is never
/// held up behind a silence check due at the same instant.
fn earliest(
    first: Option<(Instant, Deadline)>,
    second: Option<(Instant, Deadline)>,
) -> Option<(Instant, Deadline)> {
    match (first, second) {
        (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
        (a, None) => a,
        (None, b) => b,
    }
}

/// A hands-free dictation being watched for silence. Armed when such a
/// recording starts and dropped the moment the coordinator leaves the recording
/// stage, so no check can outlive the recording it was for. Kept apart from the
/// stage's own deadline because it re-arms itself on every check, and the
/// 30-minute cap must never be pushed out by that.
struct SilenceWatch {
    /// The recording it belongs to.
    generation: u64,
    timeout: Duration,
    /// When to look again ([`next_silence_check`]).
    next_check: Instant,
    /// The pill is counting down to the stop.
    counting_down: bool,
}

impl SilenceWatch {
    fn deadline(&self) -> (Instant, Deadline) {
        (
            self.next_check,
            Deadline::SilenceCheck {
                generation: self.generation,
            },
        )
    }
}

/// The silence watch for a recording that has just started or just turned
/// hands-free, if it gets one. Settings are read here, once, so a change applies
/// to the next recording.
fn arm_silence_watch(
    app: &AppHandle,
    binding_id: &str,
    mode: RecordingMode,
    generation: u64,
) -> Option<SilenceWatch> {
    let settings = crate::settings::get_settings(app);
    let timeout = silence_timeout_for(binding_id, mode, settings.dictation_silence_stop_secs)?;
    debug!(
        "'{binding_id}' stops after {}s without speech",
        timeout.as_secs()
    );
    Some(SilenceWatch {
        generation,
        timeout,
        next_check: Instant::now() + next_silence_check(Duration::ZERO, timeout),
        counting_down: false,
    })
}

/// Tell the recording pill how long it has before an automatic stop (`Some`),
/// or that speech resumed and it no longer does (`None`).
fn emit_silence_countdown(app: &AppHandle, remaining: Option<Duration>) {
    use tauri::Emitter;
    let millis = remaining.map(|r| u64::try_from(r.as_millis()).unwrap_or(u64::MAX));
    let _ = app.emit("recording-silence-countdown", millis);
}

/// How a recording is driven.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordingMode {
    /// Push-to-talk: hold the shortcut to record, release to stop & transcribe.
    Hold,
    /// Hands-free: press to start, press again (or commit/cancel) to stop.
    /// Releases are ignored. A held dictation can become one (`hands_free`).
    Lock,
}

/// The recording mode a shortcut press asks for. There is one setting for
/// every recording shortcut: Push-to-talk on means hold to record, off means
/// tap to start and tap to stop.
pub fn recording_mode(push_to_talk: bool) -> RecordingMode {
    if push_to_talk {
        RecordingMode::Hold
    } else {
        RecordingMode::Lock
    }
}

/// Whether the hands-free key has a dictation to keep listening: one held to
/// talk. With Tap to toggle a dictation already is hands-free, and anything
/// else (an ask, a paste, nothing at all) is not the key's business.
fn keeps_listening(stage: &Stage) -> bool {
    matches!(
        stage,
        Stage::Recording {
            binding_id,
            mode: RecordingMode::Hold,
        } if crate::hands_free::is_dictation_binding(binding_id)
    )
}

/// Commands processed sequentially by the coordinator thread.
enum Command {
    Input {
        binding_id: String,
        hotkey_string: String,
        is_pressed: bool,
        mode: RecordingMode,
    },
    /// The hands-free key went down on top of dictation shortcut `binding_id`
    /// (see `hands_free`). Never debounced: pressed right after the dictation
    /// keys, it lands within the press debounce of their own press.
    HandsFree {
        binding_id: String,
    },
    /// Finish the active recording and transcribe it (overlay "done" tick /
    /// assistant panel finish button). No-op unless something is recording.
    Commit,
    Cancel {
        recording_was_active: bool,
    },
    /// The press that started `binding_id`'s recording turned out to be the
    /// start of another application's shortcut (see handy-keys'
    /// `HotkeyState::Cancelled`). Throws that recording away without a trace.
    Abort {
        binding_id: String,
        done: Sender<()>,
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
    /// Fired by the coordinator's receive deadline while a hands-free dictation
    /// is watched for silence ([`SilenceWatch`]): starts or clears the pill's
    /// countdown, or, once the whole timeout has passed without speech, ends the
    /// recording exactly as a second tap would.
    SilenceCheck {
        generation: u64,
    },
    ProcessingFinished,
    /// Take the pipeline stage for work that did not start with a recording: a
    /// dismissed or failed dictation being recovered. Granted only while idle,
    /// and released like any pipeline, by [`Command::ProcessingFinished`].
    /// `audio` is how much speech it has to transcribe, which sizes its cap.
    Claim {
        audio: Duration,
        reply: Sender<bool>,
    },
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

        thread::spawn(move || loop {
            // Restarted after a panic rather than left dead: a panic used to
            // end this thread, after which every shortcut was dropped with only
            // a "channel closed" warning and dictation stayed broken until the
            // app was restarted. Each run starts from a fresh idle state.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                let mut last_press: Option<Instant> = None;
                // Monotonic id for the active recording, bumped on every start.
                // A max-duration timer captures the value at arm time and only
                // fires if it still matches — so it can never stop a newer
                // recording (see `max_duration_should_stop`).
                let mut generation: u64 = 0;
                let mut deadline = None;
                // The recording's silence watch, when it has one. Woken by its
                // own deadline, alongside the stage's.
                let mut silence: Option<SilenceWatch> = None;
                // When the active recording started, so the processing cap can
                // scale with how much audio the pipeline actually has to chew on.
                let mut recording_since = Instant::now();

                loop {
                    let wake = earliest(deadline, silence.as_ref().map(SilenceWatch::deadline));
                    let Ok(cmd) = receive_command(&rx, wake) else {
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
                        Command::HandsFree { binding_id } => {
                            if keeps_listening(&stage) {
                                debug!("Hands-free key: the held dictation keeps listening");
                                keep_listening(&app, &mut stage);
                                // Hands-free from here, so it may stop on
                                // silence. The stage did not change, so the
                                // arming on entry never sees it.
                                silence = match &stage {
                                    Stage::Recording { binding_id, .. } => arm_silence_watch(
                                        &app,
                                        binding_id,
                                        RecordingMode::Lock,
                                        generation,
                                    ),
                                    _ => None,
                                };
                            } else {
                                debug!(
                                    "Ignoring the hands-free key of '{binding_id}': nothing held"
                                );
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
                        Command::Abort { binding_id, done } => {
                            // Only the recording this press started. Anything
                            // else (an earlier hands-free recording, a pipeline
                            // already processing) belongs to someone else.
                            let ours = matches!(
                                &stage,
                                Stage::Recording { binding_id: id, .. } if id == &binding_id
                            );
                            if ours {
                                debug!("Discarding '{binding_id}': its press was another shortcut");
                                crate::utils::discard_aborted_recording(&app);
                                stage = Stage::Idle;
                                // The shortcut it gave way to may be pressed
                                // within the debounce window.
                                last_press = None;
                            }
                            let _ = done.send(());
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
                        Command::SilenceCheck { generation: g } => {
                            let recording = match &stage {
                                Stage::Recording { binding_id, .. } => Some(binding_id.clone()),
                                _ => None,
                            };
                            let applies = silence_check_applies(g, generation, recording.is_some());
                            match (recording.filter(|_| applies), silence.as_mut()) {
                                (Some(id), Some(watch)) => {
                                    let audio = app.try_state::<Arc<AudioRecordingManager>>();
                                    // No recorder to ask means no evidence of
                                    // silence, and that never ends a recording.
                                    let quiet_for = audio.as_ref().map_or(Duration::ZERO, |a| {
                                        a.speech_activity().quiet_for()
                                    });
                                    match silence_verdict(quiet_for, watch.timeout) {
                                        SilenceVerdict::Stop => {
                                            // Re-armed before acting: a check
                                            // whose time has passed is returned
                                            // ahead of anything queued, so one
                                            // that neither moved on nor left the
                                            // recording stage would be handed
                                            // back forever and starve the queue.
                                            watch.next_check = Instant::now() + SILENCE_POLL;
                                            // An Esc that landed a moment ago has
                                            // already taken the recording
                                            // (`cancel_recording`) and only told
                                            // us in a message still queued behind
                                            // this check. Stopping now would put
                                            // "transcribing" over its Undo pill;
                                            // that message settles it instead.
                                            let still_recording =
                                                audio.as_ref().is_some_and(|a| a.is_recording());
                                            if still_recording {
                                                // A recording in which nothing was
                                                // said needs nothing special: its
                                                // VAD-filtered audio is empty, and
                                                // the pipeline already ends an
                                                // empty recording quietly, without
                                                // transcribing silence, pasting or
                                                // keeping a History row.
                                                let said_something =
                                                    audio.as_ref().is_some_and(|a| {
                                                        a.speech_activity().heard_speech()
                                                    });
                                                info!(
                                                    "No speech for {:.1}s; ending the dictation as \
                                                     a second tap would{}",
                                                    quiet_for.as_secs_f32(),
                                                    if said_something {
                                                        ""
                                                    } else {
                                                        " (nothing was said, so nothing is pasted)"
                                                    }
                                                );
                                                stop(&app, &mut stage, &id, "silence");
                                            } else {
                                                debug!(
                                                    "Silence ran out on a recording that was \
                                                     already cancelled; leaving it to the cancel"
                                                );
                                            }
                                        }
                                        verdict => {
                                            let remaining = match verdict {
                                                SilenceVerdict::Ending { remaining } => {
                                                    Some(remaining)
                                                }
                                                _ => None,
                                            };
                                            // Only changes are sent: the pill
                                            // runs the countdown on its own clock.
                                            if remaining.is_some() != watch.counting_down {
                                                watch.counting_down = remaining.is_some();
                                                emit_silence_countdown(&app, remaining);
                                            }
                                            watch.next_check = Instant::now()
                                                + next_silence_check(quiet_for, watch.timeout);
                                        }
                                    }
                                }
                                // Unreachable while the watch is dropped with its
                                // recording, but a watch that may not act must
                                // not stay armed either: it would fire again at
                                // once, ahead of the queue, for as long as it
                                // lived.
                                _ => {
                                    debug!(
                                        "Ignoring stale silence check (gen {g} vs {generation})"
                                    );
                                    silence = None;
                                }
                            }
                        }
                        Command::ProcessingFinished => {
                            stage = Stage::Idle;
                        }
                        Command::Claim { audio, reply } => {
                            let granted = claim_granted(&stage);
                            if granted {
                                stage = Stage::Processing;
                                // A fresh generation, so a stall cap armed for
                                // an earlier pipeline can never release this one.
                                generation = generation.wrapping_add(1);
                                recording_since = Instant::now()
                                    .checked_sub(audio)
                                    .unwrap_or_else(Instant::now);
                            } else {
                                debug!("Recovery claim refused: the pipeline is busy");
                            }
                            let _ = reply.send(granted);
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
                        // A silence watch lives and dies with its recording,
                        // like the cap. Whatever ended it has already moved the
                        // pill on; taking the countdown down here as well is for
                        // the path that one day forgets to.
                        if silence.as_ref().is_some_and(|w| w.counting_down) {
                            emit_silence_countdown(&app, None);
                        }
                        silence = match &stage {
                            Stage::Recording {
                                binding_id, mode, ..
                            } => arm_silence_watch(&app, binding_id, *mode, generation),
                            _ => None,
                        };
                    }
                }
                debug!("Transcription coordinator exited");
            }));
            match result {
                // The channel closed: the app is shutting down.
                Ok(()) => break,
                Err(e) => {
                    let message = e
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| e.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    error!("Transcription coordinator panicked: {message}; restarting it");
                    // Take down whatever the panic left half-done (recording,
                    // overlay, tray icon) so the fresh idle state is true.
                    let reset = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::utils::cancel_current_operation(&app);
                    }));
                    if reset.is_err() {
                        error!("Resetting after the coordinator panic also panicked");
                    }
                }
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

    /// The hands-free key went down on top of dictation shortcut `binding_id`
    /// (see `hands_free`).
    pub fn send_hands_free(&self, binding_id: &str) {
        if self
            .tx
            .send(Command::HandsFree {
                binding_id: binding_id.to_string(),
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

    /// Throw away the recording `binding_id`'s press started, if it is still
    /// the one recording: the press was the start of another shortcut.
    ///
    /// Waits (briefly) until it is done, because the shortcut the press gave
    /// way to is handled right after this returns. Ctrl+Alt+C pressed slowly
    /// first starts an ask and then the call; if the ask's teardown ran
    /// afterwards, it would hide the panel the call had just opened.
    pub fn notify_abort(&self, binding_id: &str) {
        let (done, finished) = mpsc::channel();
        if self
            .tx
            .send(Command::Abort {
                binding_id: binding_id.to_string(),
                done,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
            return;
        }
        if finished.recv_timeout(Duration::from_secs(2)).is_err() {
            warn!("Discarding '{binding_id}' did not finish in time");
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

    /// Hold the pipeline stage for a dictation being recovered, exactly as a
    /// recording's own pipeline holds it: a hotkey pressed meanwhile is "busy"
    /// instead of a second dictation racing this one for the overlay and the
    /// paste. Returns false when something else already has the stage. A
    /// granted claim must be released with [`Self::notify_processing_finished`]
    /// (the pipeline's `FinishGuard` does this).
    ///
    /// Blocks briefly for the coordinator's answer, so call it off the async
    /// runtime and never from the main thread.
    pub fn claim_processing(&self, audio: Duration) -> bool {
        let (reply, answer) = mpsc::channel();
        if self.tx.send(Command::Claim { audio, reply }).is_err() {
            warn!("Transcription coordinator channel closed");
            return false;
        }
        answer.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
    }
}

/// Whether a recovery may take the stage. Only an idle coordinator gives it up:
/// a recording owns the microphone and the overlay, and a pipeline owns the
/// paste.
fn claim_granted(stage: &Stage) -> bool {
    matches!(stage, Stage::Idle)
}

/// What the pill can say about a dictation's keys: its own keys, which finish
/// it once it is hands-free, and, while it is held, the hands-free key that
/// would turn it. Nothing for anything that is not a dictation.
fn recording_keys(
    app: &AppHandle,
    binding_id: &str,
    mode: RecordingMode,
) -> crate::overlay::RecordingKeys {
    if !crate::hands_free::is_dictation_binding(binding_id) {
        return crate::overlay::RecordingKeys::default();
    }
    let finish = crate::settings::get_settings(app)
        .bindings
        .get(binding_id)
        .map(|binding| binding.current_binding.trim().to_string())
        .filter(|keys| !keys.is_empty());
    let hands_free_key = if mode == RecordingMode::Hold {
        crate::shortcut::handy_keys::hands_free_key(app, binding_id)
    } else {
        None
    };
    crate::overlay::RecordingKeys {
        finish,
        hands_free_key,
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
    // Read by the recording's own show, which `action.start` puts up (or, held
    // back by `shortcut::chord`, schedules).
    crate::overlay::set_recording_hands_free(
        mode == RecordingMode::Lock && crate::hands_free::is_dictation_binding(binding_id),
    );
    crate::overlay::set_recording_keys(recording_keys(app, binding_id, mode));
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

/// A held dictation keeps listening after its keys are let go: from here a
/// release does nothing and the next press finishes it.
fn keep_listening(app: &AppHandle, stage: &mut Stage) {
    if let Stage::Recording { mode, .. } = stage {
        *mode = RecordingMode::Lock;
    }
    crate::overlay::enter_hands_free(app);
    // The pill may still be waiting to see whether the press was the start of
    // a longer shortcut (`shortcut::chord`). It was not.
    crate::shortcut::chord::reveal_now(app);
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
    fn only_an_idle_coordinator_lends_its_stage_to_a_recovery() {
        assert!(claim_granted(&Stage::Idle));
        assert!(!claim_granted(&Stage::Processing));
        assert!(!claim_granted(&Stage::Recording {
            binding_id: "transcribe".to_string(),
            mode: RecordingMode::Lock,
        }));
    }

    fn recording(binding_id: &str, mode: RecordingMode) -> Stage {
        Stage::Recording {
            binding_id: binding_id.to_string(),
            mode,
        }
    }

    #[test]
    fn the_hands_free_key_keeps_a_held_dictation_listening() {
        assert!(keeps_listening(&recording(
            "transcribe",
            RecordingMode::Hold
        )));
        assert!(keeps_listening(&recording(
            crate::settings::CLEANUP_BINDING_ID,
            RecordingMode::Hold
        )));
    }

    #[test]
    fn the_hands_free_key_leaves_everything_else_alone() {
        // Already hands-free.
        assert!(!keeps_listening(&recording(
            "transcribe",
            RecordingMode::Lock
        )));
        // An ask is the assistant's, held or tapped.
        assert!(!keeps_listening(&recording(
            "assistant",
            RecordingMode::Hold
        )));
        assert!(!keeps_listening(&recording(
            "assistant",
            RecordingMode::Lock
        )));
        // Nothing to keep, and nothing may interrupt a paste.
        assert!(!keeps_listening(&Stage::Idle));
        assert!(!keeps_listening(&Stage::Processing));
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

    const LOCK: RecordingMode = RecordingMode::Lock;

    #[test]
    fn only_a_hands_free_dictation_stops_on_silence() {
        let five = Some(Duration::from_secs(5));
        assert_eq!(silence_timeout_for("transcribe", LOCK, 5), five);
        assert_eq!(
            silence_timeout_for("transcribe_with_post_process", LOCK, 5),
            five
        );
        // Off.
        assert_eq!(silence_timeout_for("transcribe", LOCK, 0), None);
        // The assistant's ask decides for itself when a question is over.
        assert_eq!(silence_timeout_for("assistant", LOCK, 5), None);
        // A held recording ends when the key is let go. Turned hands-free
        // (the keys plus Space, a quick tap) it is `Lock`, and covered above.
        assert_eq!(
            silence_timeout_for("transcribe", RecordingMode::Hold, 5),
            None
        );
    }

    #[test]
    fn the_stop_lands_exactly_on_the_timeout() {
        let timeout = Duration::from_secs(5);
        assert_eq!(
            silence_verdict(timeout - Duration::from_millis(1), timeout),
            SilenceVerdict::Ending {
                remaining: Duration::from_millis(1)
            }
        );
        assert_eq!(silence_verdict(timeout, timeout), SilenceVerdict::Stop);
        assert_eq!(
            silence_verdict(timeout + Duration::from_secs(1), timeout),
            SilenceVerdict::Stop
        );
    }

    #[test]
    fn the_pill_counts_down_only_at_the_end() {
        let timeout = Duration::from_secs(5);
        assert_eq!(
            silence_verdict(Duration::ZERO, timeout),
            SilenceVerdict::Listening
        );
        assert_eq!(
            silence_verdict(Duration::from_millis(2_999), timeout),
            SilenceVerdict::Listening
        );
        assert_eq!(
            silence_verdict(Duration::from_secs(3), timeout),
            SilenceVerdict::Ending {
                remaining: Duration::from_secs(2)
            }
        );
        // A short timeout warns for half of it, not most of every pause.
        let short = Duration::from_secs(3);
        assert_eq!(
            silence_verdict(Duration::from_millis(1_499), short),
            SilenceVerdict::Listening
        );
        assert_eq!(
            silence_verdict(Duration::from_millis(1_500), short),
            SilenceVerdict::Ending {
                remaining: Duration::from_millis(1_500)
            }
        );
    }

    #[test]
    fn speech_during_the_countdown_calls_it_off() {
        let timeout = Duration::from_secs(10);
        // Counting down…
        assert!(matches!(
            silence_verdict(Duration::from_millis(9_000), timeout),
            SilenceVerdict::Ending { .. }
        ));
        // …then a word: the silence starts over, and so does the whole timeout.
        assert_eq!(
            silence_verdict(Duration::from_millis(200), timeout),
            SilenceVerdict::Listening
        );
        assert_eq!(
            next_silence_check(Duration::from_millis(200), timeout),
            Duration::from_millis(7_800)
        );
    }

    #[test]
    fn checks_sleep_until_the_countdown_then_land_on_the_stop() {
        let timeout = Duration::from_secs(10);
        // Nothing can bring the stop closer before the countdown starts.
        assert_eq!(
            next_silence_check(Duration::ZERO, timeout),
            Duration::from_secs(8)
        );
        // During it: often enough to notice speech at once…
        assert_eq!(
            next_silence_check(Duration::from_secs(8), timeout),
            SILENCE_POLL
        );
        // …and the last wait ends on the timeout, not a poll after it.
        assert_eq!(
            next_silence_check(Duration::from_millis(9_950), timeout),
            Duration::from_millis(50)
        );
        // Never more often than the VAD can say anything new.
        assert_eq!(
            next_silence_check(Duration::from_millis(9_999), timeout),
            SILENCE_MIN_POLL
        );
        assert_eq!(
            next_silence_check(Duration::from_millis(7_995), timeout),
            SILENCE_MIN_POLL
        );
    }

    #[test]
    fn a_stale_silence_check_never_stops_a_newer_recording() {
        // The recording it was armed for, still recording → act.
        assert!(silence_check_applies(6, 6, true));
        // A newer recording has started since → leave it alone.
        assert!(!silence_check_applies(6, 7, true));
        // Already stopped, cancelled or processing → nothing to stop.
        assert!(!silence_check_applies(6, 6, false));
    }

    #[test]
    fn a_silence_check_fires_its_own_command() {
        let (_tx, rx) = mpsc::channel();
        assert!(matches!(
            receive_command(
                &rx,
                Some((Instant::now(), Deadline::SilenceCheck { generation: 9 }))
            )
            .unwrap(),
            Command::SilenceCheck { generation: 9 }
        ));
    }

    #[test]
    fn a_silence_check_never_delays_the_recording_cap() {
        let now = Instant::now();
        let cap = Some((now, Deadline::MaxRecording { generation: 1 }));
        let check = Some((now, Deadline::SilenceCheck { generation: 1 }));
        // Due together: the cap goes first.
        assert_eq!(earliest(cap, check), cap);
        // Otherwise the sooner one wakes the coordinator.
        let later_cap = Some((
            now + Duration::from_secs(1),
            Deadline::MaxRecording { generation: 1 },
        ));
        assert_eq!(earliest(later_cap, check), check);
        assert_eq!(earliest(None, check), check);
        assert_eq!(earliest(cap, None), cap);
        assert_eq!(earliest(None, None), None);
    }
}
