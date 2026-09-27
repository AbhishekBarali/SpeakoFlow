//! Noticing that a call is happening, so the app can *offer* to record it.
//!
//! # Why this is not a list of app names
//!
//! The obvious implementation is a list: Zoom, Teams, Meet, Discord, Slack,
//! WhatsApp, Messenger. That list is wrong the moment it is written. It cannot
//! know about the meeting tool a company built in-house, it cannot see a call
//! taken inside a browser tab (where the process is `chrome.exe` whether the
//! user is in a standup or reading the news), and it turns "does SpeakoFlow
//! support my call app" into a support question with a different answer every
//! week.
//!
//! So this module never asks *which* app. It asks the OS a structural question:
//! **is some other process holding a microphone open right now?** That is what a
//! call is, mechanically, and it is true of every app in the list above plus
//! every app that is not. A browser in a Meet call holds a capture session; the
//! same browser playing YouTube does not.
//!
//! Process names *are* collected, but only to put a recognisable word in the
//! prompt and the log ("Zoom seems to be in a call"). No decision anywhere in
//! this file reads a name. If [`AudioProcess::name`] were always `None` the
//! detection would behave identically.
//!
//! # Why a capture session and not audio in general
//!
//! Playback alone is a bad signal: music, video, a game, a notification chime.
//! Capture is nearly unambiguous — outside of a call, desktop software has
//! almost no reason to hold the microphone open, and the handful of exceptions
//! (a voice recorder, another dictation tool) produce a prompt the user
//! dismisses once. Playback is still recorded in
//! [`CallObservation::other_render_active`] because it corroborates and is worth
//! having in a log line, but it is deliberately not required: a call taken with
//! dead speakers, or one where the user is listening on a device we are not
//! looking at, is still a call.
//!
//! Notification sounds are excluded twice over. Structurally, a chime is a
//! *render* session and can never satisfy the capture test. Explicitly,
//! `IsSystemSoundsSession` drops the Windows system-sounds session before it is
//! considered at all.
//!
//! # This never starts recording
//!
//! [`CallWatcher`] fires a callback meaning "offer to record", and nothing else.
//! It has no access to [`super::session::MeetingRecorder`] and cannot start a
//! meeting even by accident. That is a hard rule for two reasons:
//!
//! * **Trust.** Software that begins recording a private conversation because it
//!   inferred one was happening is software nobody should leave installed. The
//!   inference does not have to be wrong to be unacceptable.
//! * **Law.** Recording consent rules vary by jurisdiction, and in all-party
//!   consent regimes the recording has to be a decision someone made. An
//!   automatic one cannot be consented to because nobody knew it started.
//!
//! The detector's whole output is therefore a suggestion the user accepts or
//! dismisses. If it is wrong, the cost is one dismissed prompt.
//!
//! # Platform support
//!
//! | Platform | Mechanism | Status |
//! |---|---|---|
//! | Windows | WASAPI audio session enumeration (`IAudioSessionManager2`) | native |
//! | macOS | none — see below | unsupported |
//! | Linux | none — see below | unsupported |
//!
//! macOS and Linux return [`None`] from [`observe`] rather than a guess.
//! Core Audio can report `kAudioDevicePropertyDeviceIsRunningSomewhere`, and
//! PipeWire can list which nodes are linked to a source, but neither is written
//! here and neither can be verified on this machine — and a detector that
//! silently never fires is indistinguishable from a broken one. `None` means
//! "this platform cannot answer the question", which the UI can say out loud;
//! it never means "no call".

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

/// How often the watcher asks the OS.
///
/// The query is a handful of COM calls over a few endpoints — cheap, but it runs
/// for as long as the app does, so it is not free either. Three seconds is the
/// balance: polling faster buys latency that [`SUSTAINED_FOR`] immediately
/// spends anyway, and polling slower means the offer arrives after the meeting's
/// opening sentences, which are the ones worth having.
pub const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// How long call-shaped audio must persist before it counts as a call.
///
/// Plenty of things hold the microphone for a moment: a device test, a voice
/// message, a hotword check, another app's push-to-talk. Requiring the signal to
/// survive two or three polls costs a few seconds of the meeting and removes
/// most of the false positives, which matters more than it sounds — a prompt
/// that is usually wrong gets dismissed reflexively, and then it is also
/// dismissed on the one occasion it was right.
pub const SUSTAINED_FOR: Duration = Duration::from_secs(6);

/// How long a dismissal silences the offer for *subsequent* calls.
///
/// The current call is already handled without a clock (see
/// [`CallDetector::dismiss`]). This covers the next one, because the common
/// reason a call ends and restarts within the hour is a dropped connection or a
/// rejoin — the same conversation the user just declined to record.
pub const DISMISS_COOLDOWN: Duration = Duration::from_secs(30 * 60);

/// How long the microphone must be *quiet* before the call is considered over.
///
/// Session state flickers: a participant switches devices, an app renegotiates
/// its format, a poll lands in the gap. Without a grace period a single missed
/// observation would end the call as far as this module is concerned, re-arm the
/// prompt, and offer again three seconds later for a call that never stopped.
pub const CALL_ENDED_GRACE: Duration = Duration::from_secs(20);

/// A process that is using audio, identified for the message shown to the user.
///
/// The `name` is presentation only. Detection is based on
/// [`CallObservation::other_capture_active`], which is a fact about a device
/// handle rather than about an executable, and nothing in this module branches
/// on the name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AudioProcess {
    pub pid: u32,
    /// The executable's file name (`"Zoom.exe"`), when it could be read. `None`
    /// is normal and harmless: an elevated or protected process will not open
    /// for inspection, and the detection does not need it.
    pub name: Option<String>,
}

/// What the OS said at one instant.
///
/// Deliberately a value with no methods that touch the OS, so the decision logic
/// can be driven entirely from unit tests.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CallObservation {
    /// Another process holds an **active capture** session — it has the
    /// microphone open right now. This is the whole signal.
    pub other_capture_active: bool,
    /// Another process is actively playing audio that is not a system sound.
    /// Corroborating detail for logs and UI copy; never required.
    pub other_render_active: bool,
    /// Which process the capture was attributed to, for the prompt's wording.
    pub capturing_process: Option<AudioProcess>,
}

impl CallObservation {
    /// Whether this observation looks like a call in progress.
    ///
    /// One condition, on purpose. Adding "and something is playing" would raise
    /// precision slightly and lose the muted-speaker and push-to-talk cases,
    /// which are real; a false positive costs a dismissed prompt, while a false
    /// negative costs the recording the feature exists for.
    pub fn indicates_call(&self) -> bool {
        self.other_capture_active
    }

    /// A name for the prompt, or `None` when only the structural signal is
    /// available. Callers must be able to phrase the offer without this.
    pub fn app_label(&self) -> Option<&str> {
        self.capturing_process
            .as_ref()
            .and_then(|p| p.name.as_deref())
    }
}

/// Everything outside the observation that the decision depends on.
///
/// Bundled into a struct rather than passed as five positional arguments so a
/// caller cannot silently swap two booleans — and so adding a condition later
/// does not break every call site.
#[derive(Clone, Copy, Debug, Default)]
pub struct PromptContext {
    /// A meeting is already being recorded. Offering to record a call that is
    /// being recorded is pure noise, and the microphone session we can see may
    /// well be our own doing.
    pub already_recording: bool,
    /// When the user last dismissed an offer, if ever.
    pub dismissed_at: Option<Instant>,
    /// When the current unbroken run of call-shaped audio was first observed.
    /// `None` means no call is running.
    pub call_since: Option<Instant>,
    /// Whether an offer was already raised for this same run.
    pub already_prompted: bool,
}

/// Whether to offer to record, given an observation and what came before it.
///
/// Pure, and the clock is a parameter. Every bug this kind of code has is in the
/// decision rather than in the OS query, and a decision that reads `Instant::now`
/// internally can only be tested by sleeping — which is why
/// `reminders::resolve_due_at` and `history::resolve_retention_plan` are shaped
/// the same way.
///
/// Answering `true` is a request to *ask* the user. It is never a request to
/// record; see the module docs.
pub fn should_prompt(observation: &CallObservation, ctx: &PromptContext, now: Instant) -> bool {
    // Ordered cheapest-and-most-decisive first; each of these alone is enough.
    if ctx.already_recording {
        return false;
    }
    if !observation.indicates_call() {
        return false;
    }
    if ctx.already_prompted {
        return false;
    }
    if dismissed_recently(ctx.dismissed_at, now) {
        return false;
    }

    match ctx.call_since {
        // The run has to have lasted long enough to be a conversation rather
        // than a device touching the microphone.
        Some(since) => now.saturating_duration_since(since) >= SUSTAINED_FOR,
        None => false,
    }
}

/// Whether a dismissal is still in force.
///
/// `saturating_duration_since` rather than subtraction: a caller that passes a
/// `now` earlier than the stored instant (a rounding artefact, a timestamp kept
/// across a suspend) should get "not recently" rather than a panic.
pub fn dismissed_recently(dismissed_at: Option<Instant>, now: Instant) -> bool {
    match dismissed_at {
        Some(at) => now.saturating_duration_since(at) < DISMISS_COOLDOWN,
        None => false,
    }
}

/// The watcher's memory, separated from the thread that drives it.
///
/// Splitting the state machine out of [`CallWatcher`] is what makes the
/// interesting behaviour — debounce, the sustain window, end-of-call detection —
/// testable at full speed with a synthetic clock, instead of only observable by
/// making real phone calls.
#[derive(Debug, Default)]
pub struct CallDetector {
    /// Start of the current unbroken run of call-shaped audio.
    call_since: Option<Instant>,
    /// Last time call-shaped audio was seen, for [`CALL_ENDED_GRACE`].
    last_seen: Option<Instant>,
    /// An offer has already been raised for the current run.
    prompted: bool,
    /// Last dismissal, for [`DISMISS_COOLDOWN`].
    dismissed_at: Option<Instant>,
}

/// What one observation changed.
///
/// `step` used to answer `bool`, which could only express "offer now". The call
/// *ending* was therefore unreportable: the branch below reset the detector's own
/// memory and told nobody, so an offer the user had neither accepted nor dismissed
/// stayed on screen for the rest of the session. An enum makes the second edge
/// sayable, and leaves room for a third without changing the signature again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallEvent {
    /// Nothing the caller needs to act on.
    Quiet,
    /// Ask the user whether to record. Never an instruction to record.
    Prompt,
    /// A run of call-shaped audio has ended, `CALL_ENDED_GRACE` ago.
    Ended,
}

impl CallDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one observation; returns what, if anything, the caller should do.
    ///
    /// [`CallEvent::Prompt`] marks the current run as prompted, so a call produces at
    /// most one offer no matter how long it lasts or how often this is polled.
    /// [`CallEvent::Ended`] is reported once per run, on the poll where the quiet
    /// finally outlasts [`CALL_ENDED_GRACE`], because that is the same moment the
    /// detector re-arms itself — so the two can never disagree about whether a run
    /// is over.
    pub fn step(
        &mut self,
        observation: &CallObservation,
        already_recording: bool,
        now: Instant,
    ) -> CallEvent {
        let mut ended = false;
        if observation.indicates_call() {
            if self.call_since.is_none() {
                self.call_since = Some(now);
            }
            self.last_seen = Some(now);
        } else if let Some(last) = self.last_seen {
            // Quiet for long enough that this is the call ending rather than a
            // gap in what we can see. Re-arm for whatever comes next.
            if now.saturating_duration_since(last) >= CALL_ENDED_GRACE {
                self.call_since = None;
                self.last_seen = None;
                self.prompted = false;
                ended = true;
            }
        }

        let ctx = PromptContext {
            already_recording,
            dismissed_at: self.dismissed_at,
            call_since: self.call_since,
            already_prompted: self.prompted,
        };

        if should_prompt(observation, &ctx, now) {
            self.prompted = true;
            return CallEvent::Prompt;
        }
        if ended {
            return CallEvent::Ended;
        }
        CallEvent::Quiet
    }

    /// The user said no.
    ///
    /// Two separate effects, and both are needed. `prompted` staying true means
    /// **this** call is never raised again, with no dependence on a clock — the
    /// answer to "record this call?" does not expire while the call is still
    /// running. `dismissed_at` starts [`DISMISS_COOLDOWN`], which covers the
    /// *next* call, because a rejoin after a dropped connection is a new run of
    /// the same conversation the user already declined.
    pub fn dismiss(&mut self, now: Instant) {
        self.prompted = true;
        self.dismissed_at = Some(now);
    }

    /// Treat the current call as answered without arming the cooldown — for when
    /// the user accepted and a meeting is now recording.
    pub fn accept(&mut self) {
        self.prompted = true;
    }
}

/// Whether this build can observe other processes' audio use at all.
///
/// Cheap and constant, so the UI can hide the whole setting rather than offer a
/// switch that does nothing on this platform.
pub fn detection_supported() -> bool {
    cfg!(target_os = "windows")
}

/// Ask the OS what is using audio right now.
///
/// `None` means **this platform cannot tell** — not "no call". On Windows this
/// never returns `None`: a failed or partial COM query resolves to an
/// observation with nothing set, logged at debug, because one transient failure
/// (an endpoint disappearing mid-enumeration is routine) must not be mistaken
/// for the feature being unavailable.
pub fn observe() -> Option<CallObservation> {
    #[cfg(target_os = "windows")]
    {
        Some(windows_impl::observe())
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// A background poller that says "offer to record" and nothing more.
///
/// Its own thread parked on a channel `recv_timeout`, following
/// `reminders::scheduler`, rather than a tokio task: this thread spends
/// essentially all of its life asleep, and a tokio worker occupied by a
/// multi-second sleep is a worker the rest of the app cannot use. The channel is
/// what lets a dismissal or a shutdown take effect immediately instead of at the
/// end of the current [`POLL_INTERVAL`].
pub struct CallWatcher {
    tx: Sender<Msg>,
    handle: Option<std::thread::JoinHandle<()>>,
}

enum Msg {
    /// The user dismissed the offer.
    Dismissed,
    /// The user accepted; a meeting is recording now.
    Accepted,
    /// Check immediately rather than at the next tick.
    CheckNow,
    Stop,
}

impl CallWatcher {
    /// Start watching. Returns `None` on a platform that cannot observe audio
    /// sessions, so the caller can say so rather than wonder why nothing fires.
    ///
    /// `is_recording` is a closure rather than a handle to the recorder so this
    /// module keeps no dependency on Tauri state — and, more to the point, so it
    /// has no route by which it *could* start a recording. `on_event` runs on the
    /// watcher thread and must not block; it is expected to emit an event to the
    /// UI and return.
    pub fn start<R, F>(is_recording: R, mut on_event: F) -> Option<Self>
    where
        R: Fn() -> bool + Send + 'static,
        F: FnMut(CallEvent, &CallObservation) + Send + 'static,
    {
        if !detection_supported() {
            log::info!(
                "Call auto-detection is not available on this platform; meetings must be started by hand"
            );
            return None;
        }

        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("call-detect".into())
            .spawn(move || run(rx, is_recording, &mut on_event))
            .map_err(|e| log::error!("Could not start the call watcher: {e}"))
            .ok()?;

        Some(Self {
            tx,
            handle: Some(handle),
        })
    }

    /// The user dismissed the offer. Silences this call permanently and the next
    /// one for [`DISMISS_COOLDOWN`].
    pub fn dismiss(&self) {
        let _ = self.tx.send(Msg::Dismissed);
    }

    /// The user accepted. Stops further offers for this call without arming the
    /// cooldown, since the answer was yes.
    pub fn accepted(&self) {
        let _ = self.tx.send(Msg::Accepted);
    }

    /// Poll now instead of waiting for the next tick — useful right after a
    /// meeting stops, when the state that suppressed offers has just changed.
    pub fn check_now(&self) {
        let _ = self.tx.send(Msg::CheckNow);
    }

    /// Stop watching and wait for the thread to finish.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(handle) = self.handle.take() {
            if let Err(e) = handle.join() {
                log::warn!("Call watcher thread panicked on shutdown: {e:?}");
            }
        }
    }
}

impl Drop for CallWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run<R, F>(rx: Receiver<Msg>, is_recording: R, on_event: &mut F)
where
    R: Fn() -> bool,
    F: FnMut(CallEvent, &CallObservation),
{
    log::debug!("Call auto-detection started");
    let mut detector = CallDetector::new();

    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                log::debug!("Call auto-detection stopped");
                return;
            }
            Ok(Msg::Dismissed) => {
                detector.dismiss(Instant::now());
                continue;
            }
            Ok(Msg::Accepted) => {
                detector.accept();
                continue;
            }
            // An explicit check falls through to the same poll as a timeout.
            Ok(Msg::CheckNow) => {}
            Err(RecvTimeoutError::Timeout) => {}
        }

        let observation = match observe() {
            Some(o) => o,
            // Unreachable in practice: `start` refuses to run on a platform
            // where `observe` cannot answer. Handled rather than asserted so a
            // future platform arm cannot turn into a busy loop.
            None => return,
        };

        match detector.step(&observation, is_recording(), Instant::now()) {
            CallEvent::Prompt => {
                match observation.app_label() {
                    Some(app) => log::info!("{app} appears to be in a call; offering to record"),
                    None => log::info!("A call appears to be in progress; offering to record"),
                }
                on_event(CallEvent::Prompt, &observation);
            }
            // The call is over. Whether an offer is still on screen is the pill's
            // business, not ours — this module's whole contract is that it reports
            // and never acts.
            CallEvent::Ended => {
                log::debug!("The detected call has ended");
                on_event(CallEvent::Ended, &observation);
            }
            CallEvent::Quiet => {}
        }
    }
}

/// Every process descended from `root`, including `root` itself.
///
/// `pairs` is `(pid, parent_pid)` for every process on the machine. Pure so the
/// walk can be tested without a process table.
///
/// # Why this exists
///
/// Excluding our own pid was not enough to keep the detector from seeing
/// ourselves. The assistant's voice call listens through the webview's
/// `getUserMedia`, and WebView2 runs that capture in a `msedgewebview2.exe`
/// **child** process — a different pid. So a hands-free call with the assistant
/// looked exactly like a call in another app, and six seconds in the meeting
/// card appeared on top of the call asking to record "msedgewebview2". Matching
/// on that name would be wrong (the new Microsoft Teams is itself a WebView2
/// app); asking whether the process is one of ours is the question that is
/// actually meant.
///
/// A parent pid can outlive its process and be reused, so a stale entry could in
/// principle adopt an unrelated process into our tree. The consequence is one
/// missed offer, and the `visited` set keeps a reuse cycle from looping.
pub fn process_family(root: u32, pairs: &[(u32, u32)]) -> std::collections::HashSet<u32> {
    let mut family = std::collections::HashSet::from([root]);
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for &(pid, parent_pid) in pairs {
            if parent_pid == parent && pid != parent && family.insert(pid) {
                frontier.push(pid);
            }
        }
    }
    family
}

/* ───────────────────────────── Windows ───────────────────────────── */

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{process_family, AudioProcess, CallObservation};
    use windows::core::Interface;
    use windows::Win32::Foundation::S_OK;
    use windows::Win32::Media::Audio::{
        eCapture, eRender, AudioSessionStateActive, EDataFlow, IAudioSessionControl2,
        IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Threading::GetCurrentProcessId;

    use std::collections::HashSet;

    pub fn observe() -> CallObservation {
        unsafe {
            // Same pattern as `managers::audio::set_mute` and
            // `audio_toolkit::audio::loopback`. `S_OK` means we were the ones who
            // initialised COM on this thread and therefore owe a matching
            // `CoUninitialize`; `S_FALSE` means somebody else already did, and
            // unbalancing their count would break them. A hard error (an STA
            // thread, say) is fine to proceed on — the MMDevice APIs work in
            // either apartment.
            let init = CoInitializeEx(None, COINIT_MULTITHREADED);
            let we_initialised_com = init == S_OK;

            let result = collect();

            if we_initialised_com {
                CoUninitialize();
            }

            match result {
                Ok(observation) => observation,
                Err(e) => {
                    // Endpoints appear and disappear while being enumerated, and
                    // a session can die between `GetSession` and `GetState`. That
                    // is ordinary, so it is a debug line and an empty
                    // observation — never a claim that detection is unsupported.
                    log::debug!("Audio session enumeration failed: {e}");
                    CallObservation::default()
                }
            }
        }
    }

    unsafe fn collect() -> windows::core::Result<CallObservation> {
        let own_pid = GetCurrentProcessId();
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

        // Taken lazily and at most once per poll: the process table is only
        // needed when some other pid is actually holding an active session.
        let mut family: Option<HashSet<u32>> = None;
        let mut is_ours = |pid: u32| -> bool {
            pid == own_pid
                || family
                    .get_or_insert_with(|| own_family(own_pid))
                    .contains(&pid)
        };

        // Capture first: it is the only thing the decision depends on, so if it
        // is absent the render scan is work nobody reads.
        let capturing = first_foreign_active_session(&enumerator, eCapture, &mut is_ours);
        let rendering = if capturing.is_some() {
            first_foreign_active_session(&enumerator, eRender, &mut is_ours).is_some()
        } else {
            false
        };

        Ok(CallObservation {
            other_capture_active: capturing.is_some(),
            other_render_active: rendering,
            capturing_process: capturing,
        })
    }

    /// First active session on any endpoint of `flow` that belongs to some other
    /// process.
    ///
    /// Everything here degrades to "keep looking" rather than propagating an
    /// error: on a machine with several endpoints, one that refuses to activate a
    /// session manager (a disconnected Bluetooth headset mid-teardown is the
    /// usual culprit) must not hide a live call on the endpoint after it.
    unsafe fn first_foreign_active_session(
        enumerator: &IMMDeviceEnumerator,
        flow: EDataFlow,
        is_ours: &mut dyn FnMut(u32) -> bool,
    ) -> Option<AudioProcess> {
        // `DEVICE_STATE_ACTIVE` only: an unplugged or disabled endpoint cannot
        // be carrying a call, and enumerating it costs an activation that fails.
        let devices = match enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) {
            Ok(d) => d,
            Err(e) => {
                log::debug!("Could not enumerate audio endpoints: {e}");
                return None;
            }
        };
        let device_count = devices.GetCount().unwrap_or(0);

        for i in 0..device_count {
            let device = match devices.Item(i) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
                Ok(m) => m,
                Err(e) => {
                    log::debug!("Could not open the session manager for an endpoint: {e}");
                    continue;
                }
            };
            let sessions = match manager.GetSessionEnumerator() {
                Ok(s) => s,
                Err(e) => {
                    log::debug!("Could not enumerate audio sessions: {e}");
                    continue;
                }
            };
            let session_count = sessions.GetCount().unwrap_or(0);

            for index in 0..session_count {
                let control = match sessions.GetSession(index) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let control: IAudioSessionControl2 = match control.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                // The system-sounds session is where notification chimes,
                // the low-battery beep and UI feedback live. It is not an app
                // doing anything, so it is dropped before the state is even
                // read. `IsSystemSoundsSession` returns a raw `HRESULT` here
                // precisely because `S_FALSE` is meaningful, so compare rather
                // than treat it as a fallible call.
                if control.IsSystemSoundsSession() == S_OK {
                    continue;
                }

                let pid = control.GetProcessId().unwrap_or(0);
                // pid 0 is the audio engine rather than an application. Anything
                // in our own process tree is us: dictation holds a capture
                // session in this process, meeting loopback a render one, and the
                // assistant's voice call captures from a WebView2 child process.
                // Offering to record because we can see ourselves listening
                // would be a loop — and during a call with the assistant it put
                // a "record this call?" card on top of the conversation.
                if pid == 0 {
                    continue;
                }

                match control.GetState() {
                    Ok(state) if state == AudioSessionStateActive => {}
                    // Inactive means the app owns a session but has stopped
                    // streaming; expired means it is gone. Neither is a call.
                    _ => continue,
                }

                // After the state check, so the process table is only read for
                // a session that would otherwise count.
                if is_ours(pid) {
                    continue;
                }

                return Some(AudioProcess {
                    pid,
                    name: process_file_name(pid),
                });
            }
        }
        None
    }

    /// Our own process and every process it started, directly or not.
    ///
    /// An empty snapshot degrades to "just us", which is the behaviour this had
    /// before the tree existed — never to excluding nothing.
    unsafe fn own_family(own_pid: u32) -> HashSet<u32> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };

        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(handle) => handle,
            Err(e) => {
                log::debug!("Could not snapshot the process table: {e}");
                return HashSet::from([own_pid]);
            }
        };
        let mut pairs = Vec::with_capacity(256);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        process_family(own_pid, &pairs)
    }

    /// The executable's file name, for the wording of the prompt only.
    ///
    /// `PROCESS_QUERY_LIMITED_INFORMATION` is the weakest right that answers
    /// this, and it is the one that works without elevation across integrity
    /// levels. Failure is expected and unremarkable — a protected process will
    /// not open — so the caller treats `None` as normal rather than as an error.
    unsafe fn process_file_name(pid: u32) -> Option<String> {
        use windows::core::PWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        // MAX_PATH is not a real limit for process paths, but a truncated name
        // for a log line is a better trade than a heap allocation on every poll.
        let mut buffer = [0u16; 260];
        let mut length = buffer.len() as u32;
        let queried = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(handle);
        queried.ok()?;

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        std::path::Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call in progress, as the OS would report it.
    fn call() -> CallObservation {
        CallObservation {
            other_capture_active: true,
            other_render_active: true,
            capturing_process: Some(AudioProcess {
                pid: 4242,
                name: Some("SomeCallApp.exe".into()),
            }),
        }
    }

    /// Audio playing with nothing capturing — music, a video, a notification.
    fn playback_only() -> CallObservation {
        CallObservation {
            other_capture_active: false,
            other_render_active: true,
            capturing_process: None,
        }
    }

    /// Tests move `now` **forward** from a base instant rather than subtracting
    /// from `Instant::now()`. Subtraction is what makes this kind of test flaky:
    /// on a machine that booted two minutes ago, `Instant::now() - 30 minutes`
    /// overflows and panics.
    fn base() -> Instant {
        Instant::now()
    }

    fn ctx_with_call(since: Instant) -> PromptContext {
        PromptContext {
            call_since: Some(since),
            ..Default::default()
        }
    }

    /// `step` answers an enum, and most of these cases only care about the offer.
    /// Named predicates keep the assertions reading as sentences rather than as
    /// comparisons against a variant.
    fn prompted(event: CallEvent) -> bool {
        event == CallEvent::Prompt
    }

    fn ended(event: CallEvent) -> bool {
        event == CallEvent::Ended
    }

    #[test]
    fn playback_alone_is_not_a_call() {
        assert!(!playback_only().indicates_call());
        assert!(call().indicates_call());
    }

    #[test]
    fn no_call_never_prompts() {
        let t0 = base();
        let ctx = PromptContext::default();
        assert!(!should_prompt(
            &playback_only(),
            &ctx,
            t0 + SUSTAINED_FOR * 10
        ));
    }

    /// The condition that matters most: whatever else is true, a meeting already
    /// being recorded means silence. The capture session we can see during a
    /// meeting may even be our own.
    #[test]
    fn already_recording_never_prompts() {
        let t0 = base();
        let ctx = PromptContext {
            already_recording: true,
            ..ctx_with_call(t0)
        };
        assert!(!should_prompt(&call(), &ctx, t0 + SUSTAINED_FOR * 3));
    }

    #[test]
    fn a_momentary_capture_does_not_prompt() {
        let t0 = base();
        let ctx = ctx_with_call(t0);
        assert!(!should_prompt(&call(), &ctx, t0));
        assert!(!should_prompt(
            &call(),
            &ctx,
            t0 + SUSTAINED_FOR - Duration::from_millis(1)
        ));
        assert!(should_prompt(&call(), &ctx, t0 + SUSTAINED_FOR));
    }

    #[test]
    fn a_run_already_prompted_does_not_prompt_again() {
        let t0 = base();
        let ctx = PromptContext {
            already_prompted: true,
            ..ctx_with_call(t0)
        };
        assert!(!should_prompt(&call(), &ctx, t0 + SUSTAINED_FOR * 5));
    }

    #[test]
    fn a_recent_dismissal_suppresses_a_new_call() {
        let t0 = base();
        let ctx = PromptContext {
            dismissed_at: Some(t0),
            ..ctx_with_call(t0)
        };
        // A fresh call one minute after the user said no: still no.
        assert!(!should_prompt(&call(), &ctx, t0 + Duration::from_secs(60)));
    }

    #[test]
    fn the_cooldown_expires() {
        let t0 = base();
        let ctx = PromptContext {
            dismissed_at: Some(t0),
            ..ctx_with_call(t0)
        };
        let just_inside = t0 + DISMISS_COOLDOWN - Duration::from_secs(1);
        let just_outside = t0 + DISMISS_COOLDOWN;
        assert!(!should_prompt(&call(), &ctx, just_inside));
        assert!(should_prompt(&call(), &ctx, just_outside));
    }

    #[test]
    fn dismissed_recently_handles_never_dismissed() {
        assert!(!dismissed_recently(None, base()));
    }

    /// A `now` that appears to precede the dismissal must not panic on a
    /// negative duration. It resolves to "zero elapsed", i.e. still in force,
    /// which errs toward silence — the safe direction for a prompt.
    #[test]
    fn dismissed_recently_tolerates_a_backwards_clock() {
        let t0 = base();
        let later = t0 + Duration::from_secs(10);
        assert!(dismissed_recently(Some(t0), later));
        // Reversed: the "dismissal" is in the future.
        assert!(dismissed_recently(Some(later), t0));
    }

    #[test]
    fn detector_prompts_once_per_call() {
        let t0 = base();
        let mut detector = CallDetector::new();

        // First sighting starts the run but is too early to be believed.
        assert!(!prompted(detector.step(&call(), false, t0)));
        assert!(!prompted(detector.step(&call(), false, t0 + POLL_INTERVAL)));
        // Sustained: offer.
        assert!(prompted(detector.step(&call(), false, t0 + SUSTAINED_FOR)));
        // And never again for the same call, however long it runs.
        assert!(!prompted(detector.step(
            &call(),
            false,
            t0 + SUSTAINED_FOR * 2
        )));
        assert!(!prompted(detector.step(
            &call(),
            false,
            t0 + Duration::from_secs(3600)
        )));
    }

    #[test]
    fn detector_stays_silent_while_a_meeting_records() {
        let t0 = base();
        let mut detector = CallDetector::new();
        for tick in 0..20 {
            let now = t0 + POLL_INTERVAL * tick;
            assert!(!prompted(detector.step(&call(), true, now)));
        }
    }

    /// One missed observation is not the call ending. Without the grace period
    /// this would re-arm and offer again seconds later, mid-conversation.
    #[test]
    fn a_flicker_does_not_re_arm_the_prompt() {
        let t0 = base();
        let mut detector = CallDetector::new();
        // Establish the run and consume its single prompt.
        assert!(!prompted(detector.step(&call(), false, t0)));
        assert!(prompted(detector.step(&call(), false, t0 + SUSTAINED_FOR)));

        // A single quiet poll, well inside the grace window, then audio again.
        let blip = t0 + SUSTAINED_FOR + POLL_INTERVAL;
        assert!(!prompted(detector.step(&playback_only(), false, blip)));
        assert!(!prompted(detector.step(
            &call(),
            false,
            blip + POLL_INTERVAL
        )));
        assert!(!prompted(detector.step(
            &call(),
            false,
            blip + POLL_INTERVAL + SUSTAINED_FOR * 2
        )));
    }

    /// A call that genuinely ends re-arms, so the *next* meeting is offered.
    #[test]
    fn a_finished_call_arms_the_next_one() {
        let t0 = base();
        let mut detector = CallDetector::new();
        assert!(!prompted(detector.step(&call(), false, t0)));
        assert!(prompted(detector.step(&call(), false, t0 + SUSTAINED_FOR)));

        // Quiet for longer than the grace period.
        let quiet = t0 + SUSTAINED_FOR + CALL_ENDED_GRACE;
        assert!(!prompted(detector.step(&playback_only(), false, quiet)));

        // A new call, sustained, is offered again — no dismissal was involved,
        // so no cooldown applies.
        let second = quiet + Duration::from_secs(30);
        assert!(!prompted(detector.step(&call(), false, second)));
        assert!(prompted(detector.step(
            &call(),
            false,
            second + SUSTAINED_FOR
        )));
    }

    /// The call ending has to be *reported*, not merely noticed.
    ///
    /// This is the regression guard for an offer that outlived its call. The
    /// end-of-call branch reset the detector's own memory and then returned the same
    /// `false` as an ordinary quiet poll, so the only things that could ever remove an
    /// offer were the user's X, a meeting starting, and the setting being switched
    /// off — an ignored card therefore stayed always-on-top for the rest of the
    /// session, long after the conversation it was asking about had finished.
    #[test]
    fn the_end_of_a_call_is_announced_exactly_once() {
        let t0 = base();
        let mut detector = CallDetector::new();
        assert!(!prompted(detector.step(&call(), false, t0)));
        assert!(prompted(detector.step(&call(), false, t0 + SUSTAINED_FOR)));

        // Still inside the grace window: a flicker, not an ending, so nothing to say.
        let blip = t0 + SUSTAINED_FOR + POLL_INTERVAL;
        assert_eq!(
            detector.step(&playback_only(), false, blip),
            CallEvent::Quiet
        );

        // Past the grace window: over, and said so.
        let quiet = t0 + SUSTAINED_FOR + CALL_ENDED_GRACE;
        assert!(ended(detector.step(&playback_only(), false, quiet)));

        // Once only. Every later poll must be silent, or the app would keep trying to
        // withdraw an offer that is already gone.
        assert_eq!(
            detector.step(&playback_only(), false, quiet + POLL_INTERVAL),
            CallEvent::Quiet
        );
        assert_eq!(
            detector.step(&playback_only(), false, quiet + Duration::from_secs(3600)),
            CallEvent::Quiet
        );
    }

    /// A fresh sighting is neither an offer nor an ending: it has to sustain first.
    #[test]
    fn a_new_call_starts_from_silence() {
        let t0 = base();
        let mut detector = CallDetector::new();
        assert!(!prompted(detector.step(&call(), false, t0)));
        assert!(prompted(detector.step(&call(), false, t0 + SUSTAINED_FOR)));
        let quiet = t0 + SUSTAINED_FOR + CALL_ENDED_GRACE;
        assert!(ended(detector.step(&playback_only(), false, quiet)));

        let second = quiet + Duration::from_secs(30);
        assert_eq!(detector.step(&call(), false, second), CallEvent::Quiet);
    }

    /// Dismissing covers the current call without a clock, and the next one with
    /// one.
    #[test]
    fn dismissing_survives_the_call_ending() {
        let t0 = base();
        let mut detector = CallDetector::new();
        assert!(!prompted(detector.step(&call(), false, t0)));
        assert!(prompted(detector.step(&call(), false, t0 + SUSTAINED_FOR)));
        detector.dismiss(t0 + SUSTAINED_FOR);

        // Call drops and is rejoined a minute later: the user already said no.
        let quiet = t0 + SUSTAINED_FOR + CALL_ENDED_GRACE;
        assert!(!prompted(detector.step(&playback_only(), false, quiet)));
        let rejoin = quiet + Duration::from_secs(60);
        assert!(!prompted(detector.step(&call(), false, rejoin)));
        assert!(!prompted(detector.step(
            &call(),
            false,
            rejoin + SUSTAINED_FOR
        )));

        // A different call, after the cooldown, is offered.
        let much_later = t0 + DISMISS_COOLDOWN + Duration::from_secs(60);
        assert!(!prompted(detector.step(
            &playback_only(),
            false,
            much_later
        )));
        assert!(!prompted(detector.step(
            &call(),
            false,
            much_later + Duration::from_secs(1)
        )));
        assert!(prompted(detector.step(
            &call(),
            false,
            much_later + Duration::from_secs(1) + SUSTAINED_FOR
        )));
    }

    /// Accepting stops the offers without pretending the user refused.
    #[test]
    fn accepting_does_not_arm_the_cooldown() {
        let t0 = base();
        let mut detector = CallDetector::new();
        assert!(!prompted(detector.step(&call(), false, t0)));
        assert!(prompted(detector.step(&call(), false, t0 + SUSTAINED_FOR)));
        detector.accept();

        let quiet = t0 + SUSTAINED_FOR + CALL_ENDED_GRACE;
        assert!(!prompted(detector.step(&playback_only(), false, quiet)));
        let next = quiet + Duration::from_secs(30);
        assert!(!prompted(detector.step(&call(), false, next)));
        assert!(prompted(detector.step(
            &call(),
            false,
            next + SUSTAINED_FOR
        )));
    }

    /// The prompt must be phrasable with no process name at all, since reading
    /// one is allowed to fail.
    #[test]
    fn a_nameless_call_is_still_a_call() {
        let observation = CallObservation {
            other_capture_active: true,
            other_render_active: false,
            capturing_process: Some(AudioProcess { pid: 7, name: None }),
        };
        assert!(observation.indicates_call());
        assert_eq!(observation.app_label(), None);
    }

    #[test]
    fn app_label_reads_the_process_name() {
        assert_eq!(call().app_label(), Some("SomeCallApp.exe"));
    }

    /// `None` from [`observe`] means "cannot tell", and only unsupported
    /// platforms may say it.
    #[test]
    fn unsupported_platforms_report_none() {
        if detection_supported() {
            assert!(observe().is_some());
        } else {
            assert!(observe().is_none());
        }
        assert_eq!(detection_supported(), cfg!(target_os = "windows"));
    }

    /// The assistant's voice call captures from a WebView2 renderer, which is a
    /// grandchild of the app: browser process under us, renderer under that.
    #[test]
    fn a_webview_grandchild_is_part_of_our_family() {
        let pairs = [
            (100, 4),   // us, started by explorer
            (200, 100), // msedgewebview2 browser process
            (300, 200), // its renderer / audio utility process
            (900, 4),   // Zoom, started by explorer
        ];
        let family = process_family(100, &pairs);
        assert!(family.contains(&100));
        assert!(family.contains(&200));
        assert!(family.contains(&300), "the capturing grandchild is ours");
        assert!(!family.contains(&900), "an unrelated call app is not");
        assert!(!family.contains(&4), "our parent is not our child");
    }

    /// A reused parent pid can form a cycle; the walk must still end.
    #[test]
    fn a_parent_cycle_does_not_loop() {
        let pairs = [(100, 200), (200, 100)];
        let family = process_family(100, &pairs);
        assert_eq!(family.len(), 2);
    }

    #[test]
    fn an_empty_process_table_is_just_us() {
        assert_eq!(process_family(7, &[]), std::collections::HashSet::from([7]));
    }
}
