//! Quieting the computer's other audio while SpeakoFlow records: lowering it
//! ("ducking") or muting it, then bringing it back.
//!
//! It all runs on one worker thread, in the order it was asked for. The volume
//! tools are slow on some systems (`osascript` on macOS, `wpctl` and `pactl` on
//! Linux, tens of milliseconds a call), a fade adds a few hundred milliseconds,
//! and a recording that starts or stops must never wait for either.
//!
//! The worker keeps four rules:
//!
//! - **It puts back what it changed, where it changed it.** The device is
//!   pinned when it is lowered (except on macOS, whose tool only reaches the
//!   current default), so headphones plugged in mid-recording are not turned
//!   up to the speakers' volume.
//! - **A volume the user changed meanwhile is theirs.** If the level is no
//!   longer the one the worker left it at, it stays where the user put it.
//! - **Output that was already muted or silent is left alone**, and is not
//!   unmuted afterwards.
//! - **A preview never fights a recording.** The settings slider lets the user
//!   hear an amount on whatever is playing. A recording that starts takes the
//!   lowered audio over, and a preview asked for during a recording is ignored.

use log::{debug, warn};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

// Linux and macOS only shell out, so both also build in tests on any platform,
// where their parsers are tested.
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// How a recording quiets other audio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Quieting {
    /// Bring the volume down to this share of what it was: 0.3 keeps 30%.
    Lower { keep: f32 },
    /// Mute the output.
    Mute,
}

impl Quieting {
    /// Lower by `percent`, the way the settings store it ("lower by 70%").
    pub fn lower_by(percent: u8) -> Self {
        Quieting::Lower {
            keep: 1.0 - f32::from(percent.min(100)) / 100.0,
        }
    }
}

/// The volume control of one output device.
trait OutputVolume {
    /// The volume and the mute switch, either `None` when it cannot be read.
    fn read(&self) -> Reading;
    /// Set the volume, 0.0 to 1.0. Whether the device took it.
    fn set_level(&self, level: f32) -> bool;
    /// Mute or unmute. Whether the device took it.
    fn set_muted(&self, muted: bool) -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Reading {
    /// 0.0 to 1.0. PipeWire and PulseAudio allow a little more.
    level: Option<f32>,
    muted: Option<bool>,
}

/// The system's default output, pinned to the device it is now.
fn default_output() -> Option<Box<dyn OutputVolume>> {
    #[cfg(target_os = "windows")]
    return windows::default_output();
    #[cfg(target_os = "linux")]
    return linux::default_output();
    #[cfg(target_os = "macos")]
    return macos::default_output();
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    return None;
}

/// Whatever the platform needs on the worker thread before it touches a
/// device (Windows joins COM's multithreaded apartment).
fn prepare_worker_thread() {
    #[cfg(target_os = "windows")]
    windows::prepare_thread();
}

/// Below this the output is already silent, and there is nothing to lower.
const SILENT: f32 = 0.005;
/// How far a level may be from the one the worker left before it counts as
/// the user's own change. Wider than the rounding of every tool (1% steps on
/// Linux and macOS), narrower than the smallest volume key step (2% on
/// Windows).
const MOVED: f32 = 0.015;
/// The longest the exit waits for other audio to come back.
const EXIT_WAIT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
struct Timing {
    /// Going down: soft enough not to be a jolt, and it runs while the user
    /// starts speaking, so nothing waits for it.
    fade_down: Duration,
    /// Coming back: shorter, because the stop cue waits for it.
    fade_up: Duration,
    /// Between volume changes in a fade. A slow tool takes longer than this
    /// per change and simply makes fewer of them in the same time.
    step: Duration,
    /// How long a preview holds the lowered volume after the last change.
    preview_hold: Duration,
}

const TIMING: Timing = Timing {
    fade_down: Duration::from_millis(300),
    fade_up: Duration::from_millis(200),
    step: Duration::from_millis(15),
    preview_hold: Duration::from_millis(1500),
};

/// Who quieted the audio. Only that owner brings it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Recording,
    Preview,
}

enum Command {
    Quiet {
        owner: Owner,
        how: Quieting,
    },
    /// Undo what `owner` (anyone, for `None`) quieted. `done` is dropped once
    /// the audio is back, which is what [`Restored::wait`] waits for.
    Release {
        owner: Option<Owner>,
        done: Sender<()>,
    },
}

/// Lowers or mutes the computer's other audio, and brings it back.
///
/// Cheap to create: the worker thread starts with the first quieting, so an
/// install that never turns ducking on never starts it.
pub struct Ducker {
    commands: OnceLock<Sender<Command>>,
}

impl Default for Ducker {
    fn default() -> Self {
        Self::new()
    }
}

impl Ducker {
    pub fn new() -> Self {
        Self {
            commands: OnceLock::new(),
        }
    }

    /// Quiet other audio for a recording. Returns at once; the fade runs on
    /// the worker.
    pub fn quiet(&self, how: Quieting) {
        self.start(Command::Quiet {
            owner: Owner::Recording,
            how,
        });
    }

    /// Let the user hear `how` on whatever is playing: the audio dips, and
    /// comes back by itself shortly after the last call. Ignored while a
    /// recording has other audio quieted.
    pub fn preview(&self, how: Quieting) {
        self.start(Command::Quiet {
            owner: Owner::Preview,
            how,
        });
    }

    /// Bring back what [`Self::quiet`] changed. The result settles once the
    /// audio is back up.
    pub fn restore(&self) -> Restored {
        self.release(Some(Owner::Recording))
    }

    /// Bring back anything still quieted, by a recording or a preview, waiting
    /// a moment for it. For quitting.
    pub fn restore_before_exit(&self) {
        self.release(None).wait(EXIT_WAIT);
    }

    fn start(&self, command: Command) {
        let commands = self
            .commands
            .get_or_init(|| spawn_worker(Box::new(default_output), TIMING));
        if commands.send(command).is_err() {
            debug!("The audio ducking worker is not running; other audio is left as it is");
        }
    }

    fn release(&self, owner: Option<Owner>) -> Restored {
        // A worker that never started never quieted anything.
        let Some(commands) = self.commands.get() else {
            return Restored::settled();
        };
        let (done, settled) = mpsc::channel();
        if commands.send(Command::Release { owner, done }).is_err() {
            return Restored::settled();
        }
        Restored(Some(settled))
    }
}

/// Settles once other audio is back where the recording found it.
#[must_use = "wait on it before playing anything that should be heard at full volume"]
pub struct Restored(Option<Receiver<()>>);

impl Restored {
    /// Nothing to bring back.
    pub fn settled() -> Self {
        Restored(None)
    }

    /// Whether there was nothing to bring back, so nothing to wait for.
    pub fn is_settled(&self) -> bool {
        self.0.is_none()
    }

    /// Block until the audio is back, or `limit` has passed.
    pub fn wait(self, limit: Duration) {
        if let Some(settled) = self.0 {
            // Either outcome is the answer: the worker drops its end when done.
            let _ = settled.recv_timeout(limit);
        }
    }
}

type OpenOutput = Box<dyn Fn() -> Option<Box<dyn OutputVolume>> + Send>;

fn spawn_worker(open: OpenOutput, timing: Timing) -> Sender<Command> {
    let (commands, received) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("audio-ducking".into())
        .spawn(move || run_worker(received, Engine::new(open, timing)));
    if let Err(err) = spawned {
        // The receiver went down with the closure, so every send fails and is
        // treated as nothing quieted.
        warn!("Could not start the audio ducking worker: {err}");
    }
    commands
}

fn run_worker(commands: Receiver<Command>, mut engine: Engine) {
    prepare_worker_thread();
    let mut next: Option<Command> = None;
    loop {
        let command = match next.take() {
            Some(command) => command,
            None => match engine.preview_ends {
                Some(at) => {
                    match commands.recv_timeout(at.saturating_duration_since(Instant::now())) {
                        Ok(command) => command,
                        Err(RecvTimeoutError::Timeout) => {
                            engine.release(Some(Owner::Preview));
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match commands.recv() {
                    Ok(command) => command,
                    Err(_) => break,
                },
            },
        };
        // A fade down listens for the next command between its steps, so a
        // recording that ends mid-fade turns straight back up.
        next = engine.handle(command, &mut |step| commands.recv_timeout(step).ok());
    }
    engine.release(None);
}

/// What a fade waits with between steps. A command that arrives meanwhile
/// ends the fade and is handed back to be handled next.
type Wait<'a> = &'a mut dyn FnMut(Duration) -> Option<Command>;

/// What the worker changed, and on which device.
struct Applied {
    output: Box<dyn OutputVolume>,
    owner: Owner,
    change: Change,
}

enum Change {
    /// Lowered from `original`. `left_at` is the level read back afterwards,
    /// which tells a level the user changed since apart from the worker's.
    Lowered {
        original: f32,
        left_at: f32,
    },
    Muted,
}

/// The worker's state, kept apart from the thread so it can be driven
/// directly in tests.
struct Engine {
    open: OpenOutput,
    timing: Timing,
    applied: Option<Applied>,
    /// When a preview with no call after it gives the audio back.
    preview_ends: Option<Instant>,
}

impl Engine {
    fn new(open: OpenOutput, timing: Timing) -> Self {
        Self {
            open,
            timing,
            applied: None,
            preview_ends: None,
        }
    }

    fn handle(&mut self, command: Command, wait: Wait) -> Option<Command> {
        match command {
            Command::Quiet { owner, how } => self.quiet(owner, how, wait),
            Command::Release { owner, done } => {
                self.release(owner);
                drop(done);
                None
            }
        }
    }

    fn quiet(&mut self, owner: Owner, how: Quieting, wait: Wait) -> Option<Command> {
        // A recording's quiet audio is nobody else's to change, and it is
        // already quiet for a second call.
        if self
            .applied
            .as_ref()
            .is_some_and(|applied| applied.owner == Owner::Recording)
        {
            return None;
        }
        self.preview_ends =
            (owner == Owner::Preview).then(|| Instant::now() + self.timing.preview_hold);

        match (self.applied.take(), how) {
            (None, _) => self.apply(owner, how, wait),
            // A preview is up: move it to the new amount, or hand it over to
            // the recording that just started. Either way the amount is taken
            // from the real volume, not from the lowered one.
            (
                Some(Applied {
                    output,
                    change: Change::Lowered { original, left_at },
                    ..
                }),
                Quieting::Lower { keep },
            ) => {
                let from = output.read().level.unwrap_or(left_at);
                let (reached, next) = fade(
                    &*output,
                    from,
                    original * keep.clamp(0.0, 1.0),
                    self.timing.fade_down,
                    self.timing.step,
                    wait,
                );
                let left_at = output.read().level.or(reached).unwrap_or(from);
                self.applied = Some(Applied {
                    output,
                    owner,
                    change: Change::Lowered { original, left_at },
                });
                next
            }
            // A lowered preview, and a recording that mutes: put the preview
            // back first, so the mute is undone to the real volume.
            (Some(preview), _) => {
                restore(preview, &self.timing);
                self.apply(owner, how, wait)
            }
        }
    }

    fn apply(&mut self, owner: Owner, how: Quieting, wait: Wait) -> Option<Command> {
        let Some(output) = (self.open)() else {
            debug!("No output volume control found; other audio is left as it is");
            return None;
        };
        let reading = output.read();
        if reading.muted == Some(true) {
            debug!("The output is already muted; leaving it");
            return None;
        }
        match how {
            Quieting::Mute => {
                if output.set_muted(true) {
                    debug!("Muted other audio");
                    self.applied = Some(Applied {
                        output,
                        owner,
                        change: Change::Muted,
                    });
                }
                None
            }
            Quieting::Lower { keep } => {
                let Some(original) = reading.level.filter(|level| *level > SILENT) else {
                    debug!("The output volume is silent or unreadable; nothing to lower");
                    return None;
                };
                let (reached, next) = fade(
                    &*output,
                    original,
                    original * keep.clamp(0.0, 1.0),
                    self.timing.fade_down,
                    self.timing.step,
                    wait,
                );
                // Refused before it changed anything: nothing to bring back.
                if let Some(reached) = reached {
                    let left_at = output.read().level.unwrap_or(reached);
                    debug!("Lowered other audio from {original:.2} to {left_at:.2}");
                    self.applied = Some(Applied {
                        output,
                        owner,
                        change: Change::Lowered { original, left_at },
                    });
                }
                next
            }
        }
    }

    /// Undo what `owner` (anyone, for `None`) quieted.
    fn release(&mut self, owner: Option<Owner>) {
        // A recording that never quieted anything leaves a running preview,
        // and its timer, alone.
        if owner != Some(Owner::Recording) {
            self.preview_ends = None;
        }
        if let Some(applied) = self
            .applied
            .take_if(|applied| owner.is_none_or(|owner| owner == applied.owner))
        {
            restore(applied, &self.timing);
        }
    }
}

fn restore(applied: Applied, timing: &Timing) {
    let Applied { output, change, .. } = applied;
    let reading = output.read();
    match change {
        Change::Muted => {
            // Unmuted already, by the user: nothing to undo.
            if reading.muted != Some(false) {
                output.set_muted(false);
                debug!("Unmuted other audio");
            }
        }
        Change::Lowered { original, left_at } => match reading.level {
            Some(now) if (now - left_at).abs() > MOVED => {
                debug!("The volume was changed during the recording; leaving it at {now:.2}");
            }
            Some(now) => {
                fade(
                    &*output,
                    now,
                    original,
                    timing.fade_up,
                    timing.step,
                    &mut |step| {
                        thread::sleep(step);
                        None
                    },
                );
                debug!("Brought other audio back to {original:.2}");
            }
            // Unreadable now, though it was readable before: put it back anyway.
            None => {
                output.set_level(original);
            }
        },
    }
}

/// Move the volume from `from` to `to` over `duration`, easing in and out.
///
/// Returns the last level set, `None` if the device refused the first one, and
/// a command that arrived mid-fade and cut it short.
fn fade(
    output: &dyn OutputVolume,
    from: f32,
    to: f32,
    duration: Duration,
    step: Duration,
    wait: Wait,
) -> (Option<f32>, Option<Command>) {
    let start = Instant::now();
    let mut reached = None;
    loop {
        if let Some(command) = wait(step) {
            return (reached, Some(command));
        }
        let progress = if duration.is_zero() {
            1.0
        } else {
            (start.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0)
        };
        let level = from + (to - from) * ease(progress);
        if !output.set_level(level) {
            return (reached, None);
        }
        reached = Some(level);
        if progress >= 1.0 {
            return (reached, None);
        }
    }
}

/// Smoothstep: slow at both ends, so neither end of a fade is a jump.
fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// One output device, shared with the test so it can look and meddle.
    #[derive(Debug)]
    struct Device {
        level: Option<f32>,
        muted: Option<bool>,
        /// Every level set, in order.
        set: Vec<f32>,
        refuses: bool,
    }

    #[derive(Clone)]
    struct Fake(Arc<Mutex<Device>>);

    impl Fake {
        fn new(level: f32) -> Self {
            Fake(Arc::new(Mutex::new(Device {
                level: Some(level),
                muted: Some(false),
                set: Vec::new(),
                refuses: false,
            })))
        }
        fn level(&self) -> f32 {
            self.0.lock().unwrap().level.unwrap()
        }
        fn muted(&self) -> Option<bool> {
            self.0.lock().unwrap().muted
        }
        fn with(&self, change: impl FnOnce(&mut Device)) {
            change(&mut self.0.lock().unwrap());
        }
        fn sets(&self) -> usize {
            self.0.lock().unwrap().set.len()
        }
        fn opener(&self) -> OpenOutput {
            let fake = self.clone();
            Box::new(move || Some(Box::new(fake.clone()) as Box<dyn OutputVolume>))
        }
    }

    impl OutputVolume for Fake {
        fn read(&self) -> Reading {
            let device = self.0.lock().unwrap();
            Reading {
                level: device.level,
                muted: device.muted,
            }
        }
        fn set_level(&self, level: f32) -> bool {
            let mut device = self.0.lock().unwrap();
            if device.refuses {
                return false;
            }
            // Round like a tool that takes whole percent.
            let level = (level * 100.0).round() / 100.0;
            device.level = Some(level);
            device.set.push(level);
            true
        }
        fn set_muted(&self, muted: bool) -> bool {
            let mut device = self.0.lock().unwrap();
            if device.refuses {
                return false;
            }
            device.muted = Some(muted);
            true
        }
    }

    const INSTANT: Timing = Timing {
        fade_down: Duration::ZERO,
        fade_up: Duration::ZERO,
        step: Duration::ZERO,
        preview_hold: Duration::from_millis(40),
    };

    fn engine(fake: &Fake) -> Engine {
        Engine::new(fake.opener(), INSTANT)
    }

    fn no_wait() -> impl FnMut(Duration) -> Option<Command> {
        |_| None
    }

    fn quiet(engine: &mut Engine, owner: Owner, how: Quieting) -> Option<Command> {
        engine.quiet(owner, how, &mut no_wait())
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.011
    }

    #[test]
    fn lower_by_reads_the_setting_as_what_is_taken_away() {
        for (percent, kept) in [(70, 0.3), (10, 0.9), (250, 0.0)] {
            let Quieting::Lower { keep } = Quieting::lower_by(percent) else {
                panic!("lowering is not muting");
            };
            assert!(close(keep, kept), "{percent}% keeps {keep}");
        }
    }

    #[test]
    fn a_recording_lowers_the_volume_and_puts_it_back() {
        let fake = Fake::new(0.8);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(75));
        assert!(close(fake.level(), 0.2), "lowered to {}", fake.level());

        engine.release(Some(Owner::Recording));
        assert!(close(fake.level(), 0.8), "back at {}", fake.level());
        assert!(engine.applied.is_none());
    }

    #[test]
    fn a_fade_eases_through_levels_in_between() {
        let fake = Fake::new(1.0);
        let timing = Timing {
            fade_down: Duration::from_millis(60),
            step: Duration::from_millis(5),
            ..INSTANT
        };
        let mut engine = Engine::new(fake.opener(), timing);
        engine.quiet(
            Owner::Recording,
            Quieting::lower_by(50),
            &mut |step: Duration| {
                thread::sleep(step);
                None
            },
        );
        let set = fake.0.lock().unwrap().set.clone();
        assert!(set.len() > 2, "a fade, not a jump: {set:?}");
        assert!(set.windows(2).all(|pair| pair[1] <= pair[0]), "{set:?}");
        assert!(close(*set.last().unwrap(), 0.5), "{set:?}");
    }

    #[test]
    fn a_recording_mutes_and_unmutes() {
        let fake = Fake::new(0.5);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::Mute);
        assert_eq!(fake.muted(), Some(true));
        assert!(close(fake.level(), 0.5), "the volume is not touched");

        engine.release(Some(Owner::Recording));
        assert_eq!(fake.muted(), Some(false));
    }

    #[test]
    fn output_muted_before_the_recording_stays_muted_after_it() {
        for how in [Quieting::Mute, Quieting::lower_by(50)] {
            let fake = Fake::new(0.5);
            fake.with(|device| device.muted = Some(true));
            let mut engine = engine(&fake);
            quiet(&mut engine, Owner::Recording, how);
            engine.release(Some(Owner::Recording));
            assert_eq!(fake.muted(), Some(true), "{how:?}");
            assert_eq!(fake.sets(), 0, "{how:?}");
        }
    }

    #[test]
    fn silent_or_unreadable_output_is_left_alone() {
        let fake = Fake::new(0.0);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        assert!(engine.applied.is_none());

        fake.with(|device| device.level = None);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        assert!(engine.applied.is_none());
        assert_eq!(fake.sets(), 0);
    }

    #[test]
    fn no_volume_control_means_nothing_to_do() {
        let mut engine = Engine::new(Box::new(|| None), INSTANT);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        assert!(engine.applied.is_none());
        engine.release(None);
    }

    #[test]
    fn a_device_that_refuses_has_nothing_to_restore() {
        let fake = Fake::new(0.6);
        fake.with(|device| device.refuses = true);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        assert!(engine.applied.is_none());
        quiet(&mut engine, Owner::Recording, Quieting::Mute);
        assert!(engine.applied.is_none());
    }

    #[test]
    fn a_volume_the_user_changed_during_the_recording_stays_theirs() {
        let fake = Fake::new(0.8);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        // Volume key: up two steps.
        fake.with(|device| device.level = Some(0.44));
        engine.release(Some(Owner::Recording));
        assert!(close(fake.level(), 0.44), "kept at {}", fake.level());
    }

    #[test]
    fn an_unmute_by_the_user_is_not_undone() {
        let fake = Fake::new(0.5);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::Mute);
        fake.with(|device| device.muted = Some(false));
        let sets = fake.sets();
        engine.release(Some(Owner::Recording));
        assert_eq!(fake.muted(), Some(false));
        assert_eq!(fake.sets(), sets);
    }

    #[test]
    fn the_device_that_was_lowered_is_the_one_restored() {
        let speakers = Fake::new(0.9);
        let headphones = Fake::new(0.2);
        let current = Arc::new(Mutex::new(speakers.clone()));
        let open = {
            let current = current.clone();
            Box::new(move || {
                Some(Box::new(current.lock().unwrap().clone()) as Box<dyn OutputVolume>)
            })
        };
        let mut engine = Engine::new(open, INSTANT);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        // Headphones plugged in mid-recording become the default.
        *current.lock().unwrap() = headphones.clone();
        engine.release(Some(Owner::Recording));
        assert!(close(speakers.level(), 0.9));
        assert!(close(headphones.level(), 0.2), "not turned up");
        assert_eq!(headphones.sets(), 0);
    }

    #[test]
    fn a_second_quiet_for_the_same_recording_changes_nothing() {
        let fake = Fake::new(0.8);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        let sets = fake.sets();
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        assert_eq!(fake.sets(), sets);
        engine.release(Some(Owner::Recording));
        assert!(close(fake.level(), 0.8));
    }

    #[test]
    fn a_release_mid_fade_ends_the_fade_and_turns_back_up() {
        let fake = Fake::new(1.0);
        let timing = Timing {
            fade_down: Duration::from_secs(10),
            ..INSTANT
        };
        let mut engine = Engine::new(fake.opener(), timing);
        let (done, _settled) = mpsc::channel();
        let mut release = Some(Command::Release {
            owner: Some(Owner::Recording),
            done,
        });
        let mut steps = 0;
        let next = engine.quiet(Owner::Recording, Quieting::lower_by(80), &mut |_| {
            steps += 1;
            if steps == 3 {
                release.take()
            } else {
                None
            }
        });
        assert!(
            matches!(next, Some(Command::Release { .. })),
            "the fade hands the release back"
        );
        assert!(fake.level() > 0.9, "cut short at {}", fake.level());
        engine.handle(next.unwrap(), &mut no_wait());
        assert!(close(fake.level(), 1.0));
        assert!(engine.applied.is_none());
    }

    #[test]
    fn a_release_before_the_first_step_leaves_nothing_behind() {
        let fake = Fake::new(1.0);
        let mut engine = engine(&fake);
        let (done, _settled) = mpsc::channel();
        let mut release = Some(Command::Release {
            owner: Some(Owner::Recording),
            done,
        });
        let next = engine.quiet(Owner::Recording, Quieting::lower_by(80), &mut |_| {
            release.take()
        });
        assert!(next.is_some());
        assert!(engine.applied.is_none());
        assert_eq!(fake.sets(), 0);
    }

    #[test]
    fn a_preview_follows_the_slider_from_the_real_volume() {
        let fake = Fake::new(0.8);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Preview, Quieting::lower_by(50));
        assert!(close(fake.level(), 0.4));
        quiet(&mut engine, Owner::Preview, Quieting::lower_by(25));
        assert!(close(fake.level(), 0.6), "from the original, not 0.4");
        assert!(engine.preview_ends.is_some());

        engine.release(Some(Owner::Preview));
        assert!(close(fake.level(), 0.8));
        assert!(engine.preview_ends.is_none());
    }

    #[test]
    fn a_preview_is_ignored_while_a_recording_has_the_audio() {
        let fake = Fake::new(0.8);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(50));
        quiet(&mut engine, Owner::Preview, Quieting::lower_by(10));
        assert!(close(fake.level(), 0.4));
        assert!(engine.preview_ends.is_none());
        // Nor can its end bring the recording's audio back.
        engine.release(Some(Owner::Preview));
        assert!(close(fake.level(), 0.4));
        engine.release(Some(Owner::Recording));
        assert!(close(fake.level(), 0.8));
    }

    #[test]
    fn a_recording_takes_over_a_preview_and_restores_the_real_volume() {
        let fake = Fake::new(0.8);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Preview, Quieting::lower_by(50));
        quiet(&mut engine, Owner::Recording, Quieting::lower_by(75));
        assert!(close(fake.level(), 0.2));
        assert!(engine.preview_ends.is_none(), "the recording owns it now");
        engine.release(Some(Owner::Preview));
        assert!(close(fake.level(), 0.2));
        engine.release(Some(Owner::Recording));
        assert!(close(fake.level(), 0.8));
    }

    #[test]
    fn a_muting_recording_after_a_preview_unmutes_to_the_real_volume() {
        let fake = Fake::new(0.8);
        let mut engine = engine(&fake);
        quiet(&mut engine, Owner::Preview, Quieting::lower_by(50));
        quiet(&mut engine, Owner::Recording, Quieting::Mute);
        assert_eq!(fake.muted(), Some(true));
        assert!(close(fake.level(), 0.8), "the preview was put back first");
        engine.release(Some(Owner::Recording));
        assert_eq!(fake.muted(), Some(false));
    }

    #[test]
    fn a_release_for_nobody_in_particular_restores_whatever_is_quiet() {
        for owner in [Owner::Recording, Owner::Preview] {
            let fake = Fake::new(0.8);
            let mut engine = engine(&fake);
            quiet(&mut engine, owner, Quieting::lower_by(50));
            engine.release(None);
            assert!(close(fake.level(), 0.8), "{owner:?}");
        }
    }

    #[test]
    fn the_worker_restores_a_preview_by_itself() {
        let fake = Fake::new(0.8);
        // Held long enough for a busy machine to see it lowered.
        let timing = Timing {
            preview_hold: Duration::from_millis(250),
            ..INSTANT
        };
        let commands = spawn_worker(fake.opener(), timing);
        let ducker = Ducker {
            commands: OnceLock::from(commands),
        };
        ducker.preview(Quieting::lower_by(50));
        let lowered = Instant::now() + Duration::from_secs(2);
        while !close(fake.level(), 0.4) && Instant::now() < lowered {
            thread::sleep(Duration::from_millis(2));
        }
        assert!(close(fake.level(), 0.4), "previewed at {}", fake.level());
        let restored = Instant::now() + Duration::from_secs(2);
        while !close(fake.level(), 0.8) && Instant::now() < restored {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(close(fake.level(), 0.8), "back at {}", fake.level());
    }

    #[test]
    fn restored_settles_once_the_audio_is_back() {
        let fake = Fake::new(0.8);
        let commands = spawn_worker(fake.opener(), INSTANT);
        let ducker = Ducker {
            commands: OnceLock::from(commands),
        };
        ducker.quiet(Quieting::lower_by(50));
        let restored = ducker.restore();
        assert!(!restored.is_settled());
        restored.wait(Duration::from_secs(2));
        assert!(close(fake.level(), 0.8), "back at {}", fake.level());
    }

    #[test]
    fn a_ducker_that_never_started_has_nothing_to_restore() {
        let ducker = Ducker::new();
        assert!(ducker.restore().is_settled());
        ducker.restore_before_exit();
        assert!(ducker.commands.get().is_none(), "no worker started for it");
    }
}
