//! Reading the field that was just dictated into, so a correction can be noticed.
//!
//! # What this actually does, stated plainly
//!
//! It reads the text of **one** control: the one that had keyboard focus at the moment
//! the app pasted a transcript into it. It reads it a handful of times over the next
//! minute and then stops. It never enumerates windows, never reads a control the app
//! did not just write to, and never sends anything anywhere — the text is compared
//! against our own transcript in memory and discarded.
//!
//! That narrowness is the design, not a limitation. The feature needs to answer one
//! question — "did the user change a word I produced?" — and anything broader would be
//! surveillance for no additional benefit.
//!
//! # Why polling rather than accessibility events
//!
//! UI Automation can raise an event on a property change, and subscribing would be
//! more elegant. It requires implementing a COM callback interface, keeping it alive
//! across an apartment boundary, and unsubscribing correctly on every exit path — and
//! the failure mode of getting that wrong is a leaked cross-process reference into
//! another application, which outlives our own bug.
//!
//! Polling a value every couple of seconds for a minute is a handful of cross-process
//! reads, it is trivially cancellable, and it cannot leak. The watch window is short by
//! design, so there is no ongoing cost to optimise away.
//!
//! # Three things the first version got wrong
//!
//! The first version read the field exactly once, immediately after the paste, and
//! gave up if the transcript was not there. In practice it was never there, and the
//! feature learned nothing — every dictation logged "the focused control does not
//! contain the transcript". Three separate causes, each fixed here:
//!
//! * **The paste had not landed yet.** A synthetic Ctrl+V is a queued keystroke; the
//!   target processes it when it gets round to it, and a browser or Electron app then
//!   applies it in a renderer process and updates its accessibility tree later still.
//!   So the field is now re-read for up to [`ACQUIRE_FOR`] until the transcript shows
//!   up, instead of judged on a single read taken milliseconds after the keystroke.
//! * **Chromium answers the wrong pattern first.** When the focused element is a web
//!   document, its `ValuePattern` is the page's **URL**, not its text. Reading Value
//!   first and stopping there compared the transcript against a URL. Both patterns are
//!   now read, and whichever one actually holds the transcript is the one watched.
//! * **A word half-typed was judged as a correction.** The watch stopped at the first
//!   poll that showed a difference, and a poll landing mid-edit sees `Pan` where the
//!   user is typing `Paninis`. Now a reading is only judged once the text has stopped
//!   changing between two polls ([`Settle`]), so a correction is evaluated when the
//!   user has finished making it.
//!
//! # Why a dedicated thread
//!
//! COM interface pointers are neither [`Send`] nor [`Sync`], so the automation client
//! and the element reference cannot be moved between threads or stored in shared
//! state. One thread owns them for the life of the app, receives jobs over a channel,
//! and does all the reading. It also means a cross-process read that blocks — because
//! the target application is busy or hung — blocks nothing but this thread.
//!
//! # Platform support
//!
//! | Platform | Mechanism | Status |
//! |---|---|---|
//! | Windows | UI Automation (`IUIAutomation`) | native |
//! | macOS | none yet — needs the Accessibility API and its own permission | unsupported |
//! | Linux | none yet — needs AT-SPI over D-Bus | unsupported |
//!
//! macOS and Linux report [`supported`] as false rather than silently never firing,
//! following [`crate::meetings::call_detect`]'s precedent: a feature that is quietly
//! inert is indistinguishable from a broken one, and the UI can say so out loud.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use log::debug;

/// How long after a paste to keep watching.
///
/// A correction happens within seconds — the user sees the wrong word and fixes it.
/// A minute is generous cover for reading the sentence back first, and it bounds the
/// window in which the app reads anything at all.
pub const WATCH_FOR: Duration = Duration::from_secs(60);

/// How often to re-read the field once it is being watched.
///
/// Slow on purpose. Each read is a cross-process call, and catching a correction two
/// seconds late costs nothing: nothing is waiting on the result.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long to keep looking for the transcript in the focused field after the paste.
///
/// The paste is a keystroke the target handles asynchronously, and Chromium-based apps
/// (browsers, Electron editors and chat apps) then apply it in a renderer and refresh
/// their accessibility tree later again — the first query can even be what switches
/// that tree on. A few seconds covers a busy app without reading a field long after
/// the user may have clicked somewhere else.
pub const ACQUIRE_FOR: Duration = Duration::from_secs(4);

/// How often to re-check for the transcript while acquiring.
///
/// Short, because acquisition is the only part of this that anything is racing: the
/// sooner the field is found, the less chance the user has moved focus first.
pub const ACQUIRE_RETRY: Duration = Duration::from_millis(250);

/// Longest text this will read out of a field.
///
/// A dictation is a sentence or a paragraph. Reading a whole document would be both
/// pointless — the transcript cannot be located reliably in it — and exactly the kind
/// of overreach this feature has to avoid.
const MAX_TEXT_LEN: usize = 20_000;

/// Whether the field just dictated into can be read on this platform.
pub fn supported() -> bool {
    cfg!(target_os = "windows")
}

/// A background reader for the field the app last pasted into.
///
/// Owns the thread that owns the COM objects. Dropping it stops the thread.
pub struct CorrectionWatcher {
    tx: Sender<Msg>,
    handle: Option<std::thread::JoinHandle<()>>,
}

enum Msg {
    /// Start watching the currently focused control, expecting `pasted` to be in it.
    Watch {
        pasted: String,
    },
    /// Abandon the current watch without starting another — the user began a new
    /// dictation, so the old field is no longer the one that matters.
    Cancel,
    Stop,
}

impl CorrectionWatcher {
    /// Start the reader thread.
    ///
    /// `on_text` is called with the field's contents each time they have **settled**
    /// (stopped changing between two polls), on the watcher thread. It must not block
    /// for long and must not panic. It is given the text and the transcript that was
    /// pasted, and returns `true` when it is satisfied — which stops the watch early
    /// rather than reading for the full window.
    ///
    /// Returns `None` where reading a field is not possible, so the caller can say so
    /// rather than wonder why nothing is ever learned.
    pub fn start<F>(mut on_text: F) -> Option<Self>
    where
        F: FnMut(&str, &str) -> bool + Send + 'static,
    {
        if !supported() {
            debug!(
                "Auto-learn from corrections is not available on this platform; \
                 dictated words can still be added to the dictionary by hand"
            );
            return None;
        }

        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("autolearn-watch".into())
            .spawn(move || run(rx, &mut on_text))
            .map_err(|e| log::error!("Could not start the correction watcher: {e}"))
            .ok()?;

        Some(Self {
            tx,
            handle: Some(handle),
        })
    }

    /// Watch the focused control, expecting it to contain `pasted`.
    ///
    /// Call immediately after the paste, while the target still has focus. A later call
    /// replaces an earlier watch: the newest dictation is the one whose corrections
    /// are worth having, and watching two fields at once would attribute one field's
    /// edit to the other's transcript.
    pub fn watch(&self, pasted: String) {
        let _ = self.tx.send(Msg::Watch { pasted });
    }

    /// Stop watching without starting a new watch.
    pub fn cancel(&self) {
        let _ = self.tx.send(Msg::Cancel);
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                log::warn!("Correction watcher thread panicked on shutdown");
            }
        }
    }
}

impl Drop for CorrectionWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Decides when a field's text is worth judging.
///
/// A poll can land in the middle of an edit — the user has deleted `panangini` and
/// typed `Pan` so far — and judging that reading would either learn a fragment or,
/// worse, end the watch before the real correction arrives. So a reading is only
/// handed on once it matches the previous reading, which means the user paused for at
/// least one poll interval. The same settled text is never handed on twice, so an
/// untouched field costs one comparison rather than one per poll.
///
/// Pure, so the debounce can be tested without a text field or a clock.
#[derive(Debug, Default)]
pub struct Settle {
    last: Option<String>,
    judged: Option<String>,
}

impl Settle {
    /// Start from a reading already taken, so the next identical poll is enough to
    /// count as settled.
    pub fn seeded(text: String) -> Self {
        Self {
            last: Some(text),
            judged: None,
        }
    }

    /// Feed one reading. Answers the text when it has settled and has not been judged
    /// in this form before.
    pub fn observe(&mut self, text: String) -> Option<&str> {
        let stable = self.last.as_deref() == Some(text.as_str());
        self.last = Some(text);
        if !stable || self.judged == self.last {
            return None;
        }
        self.judged = self.last.clone();
        self.judged.as_deref()
    }
}

/// The reader thread.
///
/// Structured as one loop rather than a thread per watch: COM initialisation and the
/// automation client are per-thread and worth creating once, and a single loop makes
/// "a new dictation supersedes the previous watch" fall out of the message order
/// instead of needing cancellation plumbing.
#[cfg(target_os = "windows")]
fn run<F>(rx: Receiver<Msg>, on_text: &mut F)
where
    F: FnMut(&str, &str) -> bool,
{
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Instant;

    /// Where the current watch is. Replaced as a whole, so the parts cannot disagree.
    enum Phase {
        Idle,
        /// The paste was sent; waiting for the transcript to appear in the focused
        /// control.
        Acquiring {
            pasted: String,
            started: Instant,
            last_seen: Option<windows_impl::Probe>,
        },
        /// Found it. Reading this control through this pattern until the window ends.
        Watching {
            target: windows_impl::Target,
            source: windows_impl::Source,
            pasted: String,
            started: Instant,
            settle: Settle,
        },
    }

    let _com = windows_impl::Apartment::new();
    let automation = match windows_impl::Automation::new() {
        Ok(automation) => automation,
        Err(e) => {
            log::warn!("Auto-learn cannot read text fields: {e}");
            return;
        }
    };

    let mut phase = Phase::Idle;

    loop {
        // While idle, block until something arrives, so an idle app costs nothing.
        let timeout = match phase {
            Phase::Idle => Duration::from_secs(3_600),
            Phase::Acquiring { .. } => ACQUIRE_RETRY,
            Phase::Watching { .. } => POLL_INTERVAL,
        };

        match rx.recv_timeout(timeout) {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Msg::Cancel) => {
                phase = Phase::Idle;
                continue;
            }
            Ok(Msg::Watch { pasted }) => {
                // Try straight away — a plain Win32 edit has usually applied the paste
                // already — and fall back to retrying on the next tick.
                phase = Phase::Acquiring {
                    pasted,
                    started: Instant::now(),
                    last_seen: None,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        phase = match phase {
            Phase::Idle => Phase::Idle,
            Phase::Acquiring {
                pasted,
                started,
                last_seen,
            } => match automation.acquire(&pasted, MAX_TEXT_LEN) {
                Ok(windows_impl::Acquired::Found {
                    target,
                    source,
                    text,
                    probe,
                }) => {
                    debug!(
                        "Auto-learn: watching {probe} via {source:?} (found after {} ms)",
                        started.elapsed().as_millis()
                    );
                    Phase::Watching {
                        target,
                        source,
                        pasted,
                        started: Instant::now(),
                        settle: Settle::seeded(text),
                    }
                }
                Ok(windows_impl::Acquired::NotYet(probe)) => {
                    if started.elapsed() >= ACQUIRE_FOR {
                        // Say what was focused and what it exposed, never its text, so
                        // "why did nothing get learned" has an answer in the log.
                        match probe.or(last_seen) {
                            Some(probe) => debug!(
                                "Auto-learn: the transcript never appeared in the focused \
                                 control ({probe}), so it is not being watched"
                            ),
                            None => debug!(
                                "Auto-learn: no focused control could be read, so nothing \
                                 is being watched"
                            ),
                        }
                        Phase::Idle
                    } else {
                        Phase::Acquiring {
                            pasted,
                            started,
                            last_seen: probe.or(last_seen),
                        }
                    }
                }
                Err(e) => {
                    if started.elapsed() >= ACQUIRE_FOR {
                        debug!("Auto-learn could not read the focused control: {e}");
                        Phase::Idle
                    } else {
                        Phase::Acquiring {
                            pasted,
                            started,
                            last_seen,
                        }
                    }
                }
            },
            Phase::Watching {
                target,
                source,
                pasted,
                started,
                mut settle,
            } => {
                if started.elapsed() >= WATCH_FOR {
                    debug!("Auto-learn stopped watching: the window elapsed");
                    Phase::Idle
                } else {
                    match target.read(source, MAX_TEXT_LEN) {
                        Ok(Some(text)) => {
                            let done = match settle.observe(text) {
                                Some(settled) => on_text(settled, &pasted),
                                None => false,
                            };
                            if done {
                                Phase::Idle
                            } else {
                                Phase::Watching {
                                    target,
                                    source,
                                    pasted,
                                    started,
                                    settle,
                                }
                            }
                        }
                        // The control is gone: the window closed, or the app tore down
                        // its tree. Nothing more to read.
                        Ok(None) | Err(_) => {
                            debug!("Auto-learn stopped watching: the control went away");
                            Phase::Idle
                        }
                    }
                }
            }
        };
    }
}

/// Never runs: [`CorrectionWatcher::start`] returns before spawning where
/// [`supported`] is false. Present so the spawn site compiles everywhere.
#[cfg(not(target_os = "windows"))]
fn run<F>(rx: Receiver<Msg>, _on_text: &mut F)
where
    F: FnMut(&str, &str) -> bool,
{
    while let Ok(msg) = rx.recv() {
        if matches!(msg, Msg::Stop) {
            return;
        }
    }
}

/// Whether a field's contents plausibly contain the transcript we pasted.
///
/// A containment check on the exact string would be too strict — the paste may have
/// been reformatted by the target app, and by the time of the first poll a word may
/// already have been corrected. So this asks whether *most* of the transcript's longer
/// words are present, which survives both.
///
/// Pure, so the judgement can be tested without a text field.
pub fn resembles(field: &str, pasted: &str) -> bool {
    let lowered = field.to_lowercase();
    // Short words are in every text and prove nothing.
    let anchors: Vec<String> = pasted
        .split_whitespace()
        .filter(|word| word.chars().count() >= 4)
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();

    if anchors.is_empty() {
        // Nothing long enough to look for. Fall back to the exact text, which is all
        // there is to go on for "ok" or "yes".
        return lowered.contains(&pasted.trim().to_lowercase());
    }

    let found = anchors
        .iter()
        .filter(|anchor| lowered.contains(anchor.as_str()))
        .count();
    // Half, not all: a correction has by definition changed at least one of them, and
    // a short transcript has few to spare.
    found * 2 >= anchors.len()
}

/* ───────────────────────────── Windows ───────────────────────────── */

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::fmt;

    use windows::core::Interface;
    use windows::Win32::Foundation::S_OK;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
        IUIAutomationValuePattern, UIA_TextPatternId, UIA_ValuePatternId,
    };

    use super::resembles;

    /// Owns this thread's COM apartment for as long as the watcher runs.
    ///
    /// Same discipline as `managers::audio::set_mute` and `call_detect`: `S_OK` means we
    /// initialised COM and therefore owe a matching `CoUninitialize`; `S_FALSE` means
    /// somebody else did, and unbalancing their count would break them.
    pub struct Apartment {
        owned: bool,
    }

    impl Apartment {
        pub fn new() -> Self {
            let owned = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } == S_OK;
            Self { owned }
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.owned {
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Which UI Automation pattern the control's text is read through.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Source {
        /// `ValuePattern` — plain edit controls, search boxes, most inputs.
        Value,
        /// `TextPattern` — rich text, documents, code editors, web content.
        Text,
    }

    /// What the focused control looked like, for the log. Deliberately carries no text:
    /// the control's kind and the *lengths* of what it exposed are enough to explain a
    /// failure, and the contents are the user's.
    #[derive(Clone, Debug)]
    pub struct Probe {
        control_type: i32,
        class_name: String,
        framework: String,
        value_len: Option<usize>,
        text_len: Option<usize>,
    }

    impl fmt::Display for Probe {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let len = |len: Option<usize>| match len {
                Some(len) => format!("{len} chars"),
                None => "none".to_string(),
            };
            write!(
                f,
                "control type {}, class \"{}\", framework \"{}\", value {}, text {}",
                self.control_type,
                self.class_name,
                self.framework,
                len(self.value_len),
                len(self.text_len)
            )
        }
    }

    /// The result of looking for the transcript in the focused control.
    pub enum Acquired {
        Found {
            target: Target,
            source: Source,
            text: String,
            probe: Probe,
        },
        /// Not there (yet). Carries what was focused, when anything was.
        NotYet(Option<Probe>),
    }

    /// The UI Automation client. Created once per thread.
    pub struct Automation {
        client: IUIAutomation,
    }

    impl Automation {
        pub fn new() -> windows::core::Result<Self> {
            let client: IUIAutomation =
                unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }?;
            Ok(Self { client })
        }

        /// Find the transcript in the control that has keyboard focus.
        ///
        /// Both patterns are read and the one that actually contains the transcript
        /// wins. Taking `ValuePattern` on trust was the original bug: on a Chromium web
        /// document it is the page URL, so the comparison was against an address.
        ///
        /// Confirming the transcript is there before watching is also what stops a diff
        /// against an unrelated field: a paste that went somewhere unreadable, or a
        /// focus change in the moments after it, would otherwise make every word of the
        /// transcript look like a correction.
        pub fn acquire(&self, pasted: &str, max_len: usize) -> windows::core::Result<Acquired> {
            let element = unsafe { self.client.GetFocusedElement() }?;
            let target = Target { element };

            let value = target.read(Source::Value, max_len).ok().flatten();
            let text = target.read(Source::Text, max_len).ok().flatten();
            let probe = target.probe(value.as_deref(), text.as_deref());

            let found = match (value, text) {
                (Some(value), _) if resembles(&value, pasted) => Some((Source::Value, value)),
                (_, Some(text)) if resembles(&text, pasted) => Some((Source::Text, text)),
                _ => None,
            };

            Ok(match found {
                Some((source, text)) => Acquired::Found {
                    target,
                    source,
                    text,
                    probe,
                },
                None => Acquired::NotYet(Some(probe)),
            })
        }
    }

    /// One control being watched.
    ///
    /// Holds a cross-process reference to the element rather than re-resolving focus on
    /// every poll. Re-resolving would read whatever the user has clicked into since,
    /// which is how a correction in one field gets attributed to another field's
    /// transcript.
    pub struct Target {
        element: IUIAutomationElement,
    }

    impl Target {
        /// The control's text through one pattern, truncated to `max_len` characters.
        ///
        /// `None` when the control does not support that pattern. `CurrentName` is
        /// deliberately never used: for an edit control the name is its label
        /// ("Search"), not its contents, so it would produce a confident comparison
        /// against the wrong string.
        pub fn read(
            &self,
            source: Source,
            max_len: usize,
        ) -> windows::core::Result<Option<String>> {
            let text = match source {
                Source::Value => self.value_text()?,
                Source::Text => self.document_text()?,
            };
            Ok(text.map(|text| clip(text, max_len)))
        }

        fn value_text(&self) -> windows::core::Result<Option<String>> {
            // A control that does not support the pattern answers with a null
            // interface rather than an error, so the cast is what decides.
            let pattern = unsafe { self.element.GetCurrentPattern(UIA_ValuePatternId) };
            let Ok(unknown) = pattern else {
                return Ok(None);
            };
            let Ok(pattern) = unknown.cast::<IUIAutomationValuePattern>() else {
                return Ok(None);
            };
            let value = unsafe { pattern.CurrentValue() }?;
            Ok(Some(value.to_string()))
        }

        fn document_text(&self) -> windows::core::Result<Option<String>> {
            let pattern = unsafe { self.element.GetCurrentPattern(UIA_TextPatternId) };
            let Ok(unknown) = pattern else {
                return Ok(None);
            };
            let Ok(pattern) = unknown.cast::<IUIAutomationTextPattern>() else {
                return Ok(None);
            };
            let range = unsafe { pattern.DocumentRange() }?;
            // -1 means "no limit"; the result is clipped on our side instead, because a
            // per-call limit would silently truncate mid-word.
            let value = unsafe { range.GetText(-1) }?;
            Ok(Some(value.to_string()))
        }

        fn probe(&self, value: Option<&str>, text: Option<&str>) -> Probe {
            let element = &self.element;
            Probe {
                control_type: unsafe { element.CurrentControlType() }
                    .map(|id| id.0)
                    .unwrap_or_default(),
                class_name: unsafe { element.CurrentClassName() }
                    .map(|name| name.to_string())
                    .unwrap_or_default(),
                framework: unsafe { element.CurrentFrameworkId() }
                    .map(|name| name.to_string())
                    .unwrap_or_default(),
                value_len: value.map(|value| value.chars().count()),
                text_len: text.map(|text| text.chars().count()),
            }
        }
    }

    fn clip(text: String, max_len: usize) -> String {
        if text.chars().count() <= max_len {
            return text;
        }
        text.chars().take(max_len).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /* ───────────────────── is this the right field? ───────────────────── */

    /// The check that stops a diff against an unrelated field, where every word would
    /// look like a correction.
    #[test]
    fn the_field_holding_the_transcript_is_recognised() {
        assert!(resembles(
            "please email Barali about the invoice",
            "please email brali about the invoice"
        ));
    }

    /// Dictating into a half-written message: the transcript is in there, surrounded by
    /// the user's own text.
    #[test]
    fn surrounding_text_does_not_break_recognition() {
        assert!(resembles(
            "Hi team. please email Barali about the invoice. Thanks.",
            "please email brali about the invoice"
        ));
    }

    #[test]
    fn an_unrelated_field_is_rejected() {
        assert!(!resembles(
            "https://example.com/some/unrelated/page",
            "please email brali about the invoice"
        ));
    }

    /// What Chromium's `ValuePattern` answers for a focused web document: the URL. It
    /// must never be taken for the field the transcript went into.
    #[test]
    fn a_page_url_is_not_the_field() {
        assert!(!resembles("https://www.reddit.com/", "I like panangini."));
    }

    #[test]
    fn an_empty_field_is_rejected() {
        assert!(!resembles("", "please email brali about the invoice"));
    }

    /// A correction has by definition changed a word, so requiring every anchor would
    /// reject the field the moment it became interesting.
    #[test]
    fn one_corrected_word_does_not_reject_the_field() {
        assert!(resembles(
            "send Kubernetes config to Barali today",
            "send kubernets config to brali today"
        ));
    }

    /// A one-word dictation has no long anchors, so the exact text is all there is.
    #[test]
    fn a_very_short_transcript_falls_back_to_exact_text() {
        assert!(resembles("yes", "yes"));
        assert!(resembles("I said yes to it", "yes"));
        assert!(!resembles("no", "yes"));
    }

    #[test]
    fn recognition_ignores_casing() {
        assert!(resembles(
            "PLEASE EMAIL BARALI ABOUT THE INVOICE",
            "please email brali about the invoice"
        ));
    }

    /* ───────────────────────────── settling ───────────────────────────── */

    /// A poll mid-edit must not be judged: `Pan` is not what the user is typing.
    #[test]
    fn a_changing_field_is_not_judged() {
        let mut settle = Settle::seeded("I like panangini.".into());
        assert_eq!(settle.observe("I like Pan".into()), None);
        assert_eq!(settle.observe("I like Panini".into()), None);
    }

    /// Once the text holds still for one interval, it is handed on.
    #[test]
    fn a_settled_field_is_judged() {
        let mut settle = Settle::seeded("I like panangini.".into());
        assert_eq!(settle.observe("I like Pan".into()), None);
        assert_eq!(settle.observe("I like Paninis.".into()), None);
        assert_eq!(
            settle.observe("I like Paninis.".into()),
            Some("I like Paninis.")
        );
    }

    /// The acquisition read counts as the first of the two, so an edit that was
    /// already finished when the field was found is judged on the first poll.
    #[test]
    fn the_seed_counts_as_a_reading() {
        let mut settle = Settle::seeded("I like Paninis.".into());
        assert_eq!(
            settle.observe("I like Paninis.".into()),
            Some("I like Paninis.")
        );
    }

    /// An untouched field is judged once, not on every poll for a minute.
    #[test]
    fn the_same_settled_text_is_judged_once() {
        let mut settle = Settle::seeded("I like panangini.".into());
        assert!(settle.observe("I like panangini.".into()).is_some());
        assert_eq!(settle.observe("I like panangini.".into()), None);
        assert_eq!(settle.observe("I like panangini.".into()), None);
    }

    /// After one settled state is judged, a later edit that settles is judged too.
    #[test]
    fn a_later_edit_is_judged_after_an_earlier_one() {
        let mut settle = Settle::seeded("a".into());
        assert!(settle.observe("a".into()).is_some());
        assert_eq!(settle.observe("b".into()), None);
        assert_eq!(settle.observe("b".into()), Some("b"));
    }

    /// Editing back to a state already judged is not judged again: nothing new.
    #[test]
    fn returning_to_a_judged_text_is_not_rejudged() {
        let mut settle = Settle::seeded("a".into());
        assert!(settle.observe("a".into()).is_some());
        assert_eq!(settle.observe("b".into()), None);
        assert_eq!(settle.observe("a".into()), None);
        assert_eq!(settle.observe("a".into()), None);
    }

    /* ───────────────────────── platform support ───────────────────────── */

    /// Unsupported platforms must say so rather than silently never firing, which is
    /// indistinguishable from a broken feature.
    #[test]
    fn support_is_reported_honestly() {
        assert_eq!(supported(), cfg!(target_os = "windows"));
    }

    /// The watch window bounds how long the app reads anything at all, so it must stay
    /// short and must be longer than one poll — long enough, in fact, for a correction
    /// to settle (two equal readings) several times over.
    #[test]
    fn the_watch_window_is_short_and_polls_several_times() {
        assert!(WATCH_FOR <= Duration::from_secs(120));
        assert!(POLL_INTERVAL < WATCH_FOR);
        assert!(WATCH_FOR.as_secs() / POLL_INTERVAL.as_secs() >= 4);
    }

    /// Acquisition has to survive a slow renderer, but must not stretch into a period
    /// where the user has plausibly moved on to another field.
    #[test]
    fn acquisition_retries_several_times_within_a_few_seconds() {
        assert!(ACQUIRE_FOR <= Duration::from_secs(10));
        assert!(ACQUIRE_RETRY < ACQUIRE_FOR);
        assert!(ACQUIRE_FOR.as_millis() / ACQUIRE_RETRY.as_millis() >= 4);
        assert!(ACQUIRE_FOR < WATCH_FOR);
    }
}
