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
//! Polling a value four times over a minute is a handful of cross-process reads, it is
//! trivially cancellable, and it cannot leak. The watch window is short by design, so
//! there is no ongoing cost to optimise away.
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

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use log::debug;

/// How long after a paste to keep watching.
///
/// A correction happens within seconds — the user sees the wrong word and fixes it.
/// A minute is generous cover for reading the sentence back first, and it bounds the
/// window in which the app reads anything at all.
pub const WATCH_FOR: Duration = Duration::from_secs(60);

/// How often to re-read the field.
///
/// Slow on purpose. Each read is a cross-process call, and catching a correction two
/// seconds late costs nothing: nothing is waiting on the result.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

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
    /// `on_text` is called with the field's current contents on every poll, on the
    /// watcher thread. It must not block for long and must not panic. It is given the
    /// text and the transcript that was pasted, and returns `true` when it is
    /// satisfied — which stops the watch early rather than reading for the full
    /// window.
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

/// The reader thread.
///
/// Structured as one loop rather than a thread per watch: COM initialisation and the
/// automation client are per-thread and worth creating once, and a single loop makes
/// "a new dictation supersedes the previous watch" fall out of the message order
/// instead of needing cancellation plumbing.
fn run<F>(rx: Receiver<Msg>, on_text: &mut F)
where
    F: FnMut(&str, &str) -> bool,
{
    #[cfg(target_os = "windows")]
    let _com = windows_impl::Apartment::new();
    #[cfg(target_os = "windows")]
    let automation = match windows_impl::Automation::new() {
        Ok(automation) => automation,
        Err(e) => {
            log::warn!("Auto-learn cannot read text fields: {e}");
            return;
        }
    };

    // The control being watched, the transcript that went into it, and when the watch
    // started. All three are replaced together, so they cannot disagree.
    #[cfg(target_os = "windows")]
    let mut watching: Option<(windows_impl::Target, String, Instant)> = None;
    #[cfg(not(target_os = "windows"))]
    let mut watching: Option<((), String, Instant)> = None;

    loop {
        // While watching, wake on the poll interval; while idle, block until something
        // arrives. An idle app therefore costs nothing at all.
        let timeout = if watching.is_some() {
            POLL_INTERVAL
        } else {
            Duration::from_secs(3_600)
        };

        match rx.recv_timeout(timeout) {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Msg::Cancel) => {
                watching = None;
                continue;
            }
            Ok(Msg::Watch { pasted }) => {
                #[cfg(target_os = "windows")]
                {
                    watching = match automation.focused_target() {
                        Ok(Some(target)) => {
                            // Confirm the control actually holds what we pasted before
                            // watching it. Without this check, a paste that went
                            // somewhere unreadable — or a focus change in the
                            // milliseconds after it — would leave us diffing our
                            // transcript against an unrelated field, and every word in
                            // it would look like a correction.
                            match target.text(MAX_TEXT_LEN) {
                                Ok(Some(text)) if resembles(&text, &pasted) => {
                                    Some((target, pasted, Instant::now()))
                                }
                                Ok(_) => {
                                    debug!(
                                        "Auto-learn: the focused control does not contain the \
                                         transcript, so it is not being watched"
                                    );
                                    None
                                }
                                Err(e) => {
                                    debug!("Auto-learn could not read the focused control: {e}");
                                    None
                                }
                            }
                        }
                        Ok(None) => None,
                        Err(e) => {
                            debug!("Auto-learn could not find the focused control: {e}");
                            None
                        }
                    };
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = pasted;
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        // A poll tick.
        #[cfg(target_os = "windows")]
        {
            let Some((target, pasted, started)) = watching.as_ref() else {
                continue;
            };
            if started.elapsed() >= WATCH_FOR {
                debug!("Auto-learn stopped watching: the window elapsed");
                watching = None;
                continue;
            }
            match target.text(MAX_TEXT_LEN) {
                Ok(Some(text)) => {
                    if on_text(&text, pasted) {
                        watching = None;
                    }
                }
                // The control is gone: the window closed, or the app tore down its
                // tree. Nothing more to read.
                Ok(None) | Err(_) => {
                    watching = None;
                }
            }
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

        /// The control that currently has keyboard focus, if it can hold text.
        ///
        /// `None` rather than an error when nothing is focused or the focused element
        /// exposes no text: both are ordinary, and neither says anything is wrong.
        pub fn focused_target(&self) -> windows::core::Result<Option<Target>> {
            let element = unsafe { self.client.GetFocusedElement() }?;
            Ok(Some(Target { element }))
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
        /// The control's text, truncated to `max_len` characters.
        ///
        /// Two patterns are tried, in order of how likely they are to be the *editable*
        /// text rather than a label:
        ///
        /// 1. `ValuePattern` — plain edit controls, search boxes, most inputs.
        /// 2. `TextPattern` — rich text, documents, code editors, web content.
        ///
        /// `CurrentName` is deliberately **not** used as a third fallback. For an edit
        /// control the name is its label ("Search"), not its contents, so falling back
        /// to it would produce a confident comparison against the wrong string.
        pub fn text(&self, max_len: usize) -> windows::core::Result<Option<String>> {
            if let Some(text) = self.value_text()? {
                return Ok(Some(clip(text, max_len)));
            }
            if let Some(text) = self.document_text()? {
                return Ok(Some(clip(text, max_len)));
            }
            Ok(None)
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
            let text = unsafe { value.to_string() };
            Ok(Some(text))
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
            let text = unsafe { value.to_string() };
            Ok(Some(text))
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

    /* ───────────────────────── platform support ───────────────────────── */

    /// Unsupported platforms must say so rather than silently never firing, which is
    /// indistinguishable from a broken feature.
    #[test]
    fn support_is_reported_honestly() {
        assert_eq!(supported(), cfg!(target_os = "windows"));
    }

    /// The watch window bounds how long the app reads anything at all, so it must stay
    /// short and must be longer than one poll.
    #[test]
    fn the_watch_window_is_short_and_polls_several_times() {
        assert!(WATCH_FOR <= Duration::from_secs(120));
        assert!(POLL_INTERVAL < WATCH_FOR);
        assert!(WATCH_FOR.as_secs() / POLL_INTERVAL.as_secs() >= 4);
    }
}
