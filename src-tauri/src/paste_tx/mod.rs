//! Clipboard paste that restores the user's clipboard only once the target
//! application has read the dictation. Ported from Handy's "reliable paste"
//! (cjpais/Handy #1812, #1847).
//!
//! The plain clipboard paste (`clipboard::paste_via_clipboard`) puts the
//! dictation on the clipboard, sends Ctrl+V, waits a fixed 50 ms and puts the
//! user's clipboard back. The keystroke is only *queued* at that point: the
//! target reads the clipboard whenever its event loop gets to it, and a busy
//! app (a browser tab, an Electron editor, anything on a loaded machine) lost
//! that race and pasted whatever the user had copied before.
//!
//! Here the dictation goes on the clipboard as a *promise*, and the operating
//! system tells us when a consumer actually reads it, which is the receipt:
//!
//! - Windows: delayed rendering (`SetClipboardData(CF_UNICODETEXT, NULL)`);
//!   the owner window receives `WM_RENDERFORMAT` on each read.
//! - macOS: `declareTypes:owner:`; the pasteboard calls
//!   `pasteboard:provideDataForType:` on each read.
//!
//! Two rules make a receipt trustworthy:
//!
//! 1. Only reads after the paste keystroke was sent count. Earlier ones are a
//!    clipboard manager, a remote-desktop client or antivirus reacting to the
//!    clipboard change itself.
//! 2. The clipboard is only put back while it is still ours (sequence number /
//!    changeCount unchanged, no ownership-lost event). If the user copied
//!    something else in the meantime, their copy wins.
//!
//! The restore waits for a short quiet period after the *last* read, because
//! some apps read several times per paste (Chromium probes, then reads), and a
//! bounded timeout caps how long the dictation can sit on the clipboard. The
//! failure mode is always "the dictation stays on the clipboard a few seconds
//! longer", never "the old clipboard gets pasted".
//!
//! Auto-submit moves into the transaction as well: Enter is sent only once the
//! target has read the dictation, so it can never submit a field the paste has
//! not reached yet. Linux keeps the plain paste.

// The shared transaction state is compiled everywhere (for the unit tests),
// but only the Windows and macOS modules use all of it.
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]

use std::time::{Duration, Instant};

use crate::settings::{AutoSubmitKey, PasteMethod};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// How long after the *last* read the dictation stays on the clipboard before
/// the user's content is put back. Covers apps that read several times per
/// paste (Chromium's probe-then-read).
pub(crate) const QUIET_PERIOD: Duration = Duration::from_millis(200);

/// The longest the dictation may occupy the clipboard when no read arrives.
/// Long enough that a realistically loaded target reads first; short enough
/// that a keystroke the target never saw does not strand the dictation there.
pub(crate) const RESTORE_TIMEOUT: Duration = Duration::from_secs(8);

/// When the keystroke could not be sent at all, no read can be legitimate, so
/// the clipboard is put back quickly instead of waiting out the full timeout.
pub(crate) const FAILED_INJECTION_TIMEOUT: Duration = Duration::from_millis(500);

/// How long after the keystroke the clipboard is put back when the promise was
/// already read *before* it.
///
/// A promise is rendered once: the first reader gets the data, and from then
/// on it is ordinary clipboard data that no later read reports. SpeakoFlow
/// leaves clipboard monitoring on (remote-desktop and VM clients forward the
/// clipboard into their session, see `clipboard::write_transient_clipboard_text`),
/// so a clipboard manager or a VM client reacting to the change can take that
/// one read in the paste-delay gap, and the target's own read then goes
/// unseen. Waiting the full [`RESTORE_TIMEOUT`] and skipping auto-submit would
/// make every paste worse for anyone running such a tool, so this case falls
/// back to a fixed wait: twenty times the 50 ms the plain paste used, and
/// auto-submit still goes, as it always did.
pub(crate) const BLIND_RESTORE_DELAY: Duration = Duration::from_millis(1000);

/// How long after the keystroke the auto-submit Enter goes when the target's
/// read cannot be seen (a promise read before the keystroke, see
/// [`BLIND_RESTORE_DELAY`]). Only the clipboard restore needs the long wait;
/// Enter does not touch the clipboard, and the target handles it after the
/// paste keystroke it queued first. Holding Enter for the restore's full
/// second made every auto-submitted dictation feel a second slow on machines
/// with a clipboard manager or a VM/remote-desktop client, and it could land
/// while the next shortcut's keys were already held. The plain paste sent it
/// 50 ms after the keystroke.
pub(crate) const BLIND_SUBMIT_DELAY: Duration = Duration::from_millis(150);

/// How long the paste waits for the dictation to be published before giving
/// up and using the plain paste instead. Publishing first snapshots the
/// clipboard, which can ask another app to render data it promised; a hung
/// owner must not hold the paste (and the main thread) for long. Windows only:
/// macOS publishes on the calling thread and has nothing to wait for.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) const PUBLISH_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a settling transaction tries to take the input lock for the
/// auto-submit Enter. The paste that started the transaction holds it until it
/// returns, which is long before the quiet period ends; anything else holding
/// it for longer is busy typing, and Enter is skipped rather than interleaved.
pub(crate) const AUTO_SUBMIT_LOCK_WAIT: Duration = Duration::from_millis(500);

/// One paste the transaction should perform.
pub(crate) struct PasteRequest<'a> {
    /// What to paste.
    pub text: &'a str,
    /// The same content as HTML, for an assistant answer whose Markdown a
    /// rich editor should show as formatting (`clipboard::paste_rich_with_behavior`).
    /// Promised beside the text; each app reads the one it understands.
    pub html: Option<&'a str>,
    /// Ctrl+V, Ctrl+Shift+V or Shift+Insert.
    pub paste_method: PasteMethod,
    /// The user's "paste delay": time between publishing and the keystroke,
    /// for targets (remote desktops, VMs) that forward the clipboard first.
    pub paste_delay: Duration,
    /// The Enter to send once the target has read the dictation, if any.
    pub auto_submit: Option<AutoSubmitKey>,
    /// "Copy to clipboard" after pasting: leave this text on the clipboard as
    /// ordinary text instead of restoring the user's content.
    pub leave_on_clipboard: Option<String>,
}

/// Shared, cross-thread record of one paste transaction.
#[derive(Debug)]
pub(crate) struct TxState {
    /// When the dictation was published to the clipboard.
    pub published_at: Instant,
    /// When the paste keystroke was sent. Only reads after this count as
    /// evidence the target read the dictation.
    pub injected_at: Option<Instant>,
    /// The keystroke could not be sent; the wait is cut short.
    pub injection_failed: bool,
    /// When consumers read the clipboard.
    pub receipts: Vec<Instant>,
    /// Someone else took the clipboard (the user copied something, ...).
    pub ownership_lost: bool,
    /// A newer paste settled this one early.
    pub cancelled: bool,
    /// The first read that counts has been logged.
    pub logged_receipt: bool,
    /// The auto-submit Enter has already gone (early, see
    /// [`blind_submit_due`]), so settling must not send it again.
    pub submitted: bool,
}

impl TxState {
    pub fn new() -> Self {
        Self {
            published_at: Instant::now(),
            injected_at: None,
            injection_failed: false,
            receipts: Vec::new(),
            ownership_lost: false,
            cancelled: false,
            logged_receipt: false,
            submitted: false,
        }
    }

    /// Records a read, logging the first one that counts as evidence.
    pub fn record_receipt(&mut self, at: Instant) {
        self.receipts.push(at);
        if self.logged_receipt {
            return;
        }
        if let Some(injected) = self.injected_at {
            if at >= injected {
                self.logged_receipt = true;
                log::info!(
                    "Paste: the target read the clipboard {} ms after the keystroke",
                    at.duration_since(injected).as_millis()
                );
            }
        }
    }

    pub fn last_receipt_after_injection(&self) -> Option<Instant> {
        let injected = self.injected_at?;
        self.receipts.iter().copied().rev().find(|t| *t >= injected)
    }

    pub fn any_receipt_after_injection(&self) -> bool {
        self.last_receipt_after_injection().is_some()
    }

    /// The promise was read before the keystroke, so it is ordinary data now
    /// and the target's read will not be reported (see
    /// [`BLIND_RESTORE_DELAY`]).
    pub fn rendered_before_injection(&self) -> bool {
        match self.injected_at {
            Some(injected) => self.receipts.iter().any(|t| *t < injected),
            None => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WaitDecision {
    KeepWaiting,
    /// Stop waiting and settle the transaction.
    Finish,
}

/// Keep waiting for the target to read, or settle now. Both platforms' wait
/// loops call this; it is pure so every branch is testable.
pub(crate) fn evaluate(state: &TxState, now: Instant) -> WaitDecision {
    if state.ownership_lost || state.cancelled {
        return WaitDecision::Finish;
    }
    if let Some(last) = state.last_receipt_after_injection() {
        if now.duration_since(last) >= QUIET_PERIOD {
            return WaitDecision::Finish;
        }
    }
    if state.rendered_before_injection() && !state.injection_failed {
        if let Some(injected) = state.injected_at {
            if now.duration_since(injected) >= BLIND_RESTORE_DELAY {
                return WaitDecision::Finish;
            }
        }
    }
    let deadline = if state.injection_failed {
        FAILED_INJECTION_TIMEOUT
    } else {
        RESTORE_TIMEOUT
    };
    if now.duration_since(state.published_at) >= deadline {
        return WaitDecision::Finish;
    }
    WaitDecision::KeepWaiting
}

/// What settling does to the clipboard.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClipboardSettle {
    /// Put back what the user had.
    Restore,
    /// Replace the promise with the dictation as ordinary text ("Copy to
    /// clipboard" after pasting).
    LeaveCopy,
    /// The clipboard is no longer ours: the user's newer copy stays.
    Untouched,
}

/// How one transaction settles.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Settlement {
    /// Send the auto-submit Enter.
    pub submit: bool,
    pub clipboard: ClipboardSettle,
}

/// Decide how a finished transaction settles.
///
/// Enter goes only after the target demonstrably read the dictation: pressing
/// it after an unconfirmed paste could submit a field without the text, or
/// with whatever the field held before. The one exception is a promise read
/// before the keystroke, whose target read can never be seen (see
/// [`BLIND_RESTORE_DELAY`]); there Enter goes as it did with the plain paste.
/// The clipboard is only touched while it is still ours.
pub(crate) fn settlement(
    state: &TxState,
    still_ours: bool,
    wants_submit: bool,
    leaves_copy: bool,
) -> Settlement {
    let clipboard = if state.ownership_lost || !still_ours {
        ClipboardSettle::Untouched
    } else if leaves_copy {
        ClipboardSettle::LeaveCopy
    } else {
        ClipboardSettle::Restore
    };
    let delivered = state.any_receipt_after_injection()
        || (state.rendered_before_injection() && !state.injection_failed);
    Settlement {
        submit: wants_submit && delivered && !state.submitted,
        clipboard,
    }
}

/// Whether the auto-submit Enter of a blind paste (the promise was read
/// before the keystroke) is due now, ahead of the clipboard restore. See
/// [`BLIND_SUBMIT_DELAY`]. A paste whose target read is seen keeps sending
/// Enter when it settles, a quiet period after that read.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn blind_submit_due(state: &TxState, now: Instant) -> bool {
    !state.submitted
        && !state.injection_failed
        && !state.ownership_lost
        && !state.any_receipt_after_injection()
        && state.rendered_before_injection()
        && state
            .injected_at
            .is_some_and(|injected| now.duration_since(injected) >= BLIND_SUBMIT_DELAY)
}

/// Why a transaction settled, for the log.
pub(crate) fn settle_reason(state: &TxState) -> &'static str {
    if state.ownership_lost {
        "the clipboard changed hands"
    } else if state.cancelled {
        "a newer paste started"
    } else if state.any_receipt_after_injection() {
        "the target read the dictation"
    } else if state.injection_failed {
        "the keystroke could not be sent"
    } else if state.rendered_before_injection() {
        "another app read the clipboard before the keystroke"
    } else {
        "no read arrived in time"
    }
}

/// Send the paste keystroke for `paste_method`. Every `input::send_paste_*`
/// already holds `input::synthesizing()` around its keys, so the hotkey engine
/// never reads them back as a shortcut.
pub(crate) fn send_chord(
    enigo: &mut enigo::Enigo,
    paste_method: PasteMethod,
) -> Result<(), String> {
    match paste_method {
        PasteMethod::CtrlV => crate::input::send_paste_ctrl_v(enigo),
        PasteMethod::CtrlShiftV => crate::input::send_paste_ctrl_shift_v(enigo),
        PasteMethod::ShiftInsert => crate::input::send_paste_shift_insert(enigo),
        other => Err(format!(
            "Invalid paste method for clipboard paste: {:?}",
            other
        )),
    }
}

/// Send the auto-submit Enter from a settling transaction, which runs off the
/// paste's own thread and so has to take the input lock itself. Bounded (see
/// [`AUTO_SUBMIT_LOCK_WAIT`]): a settle never waits on someone else's typing.
pub(crate) fn send_auto_submit_later(app_handle: &tauri::AppHandle, key: AutoSubmitKey) {
    use tauri::Manager;
    let Some(enigo_state) = app_handle.try_state::<crate::input::EnigoState>() else {
        return;
    };
    let deadline = Instant::now() + AUTO_SUBMIT_LOCK_WAIT;
    loop {
        match enigo_state.0.try_lock() {
            Ok(mut enigo) => {
                if let Err(e) = crate::clipboard::send_return_key(&mut enigo, key) {
                    log::warn!("Paste: could not send the auto-submit key: {e}");
                }
                return;
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                let mut enigo = poisoned.into_inner();
                if let Err(e) = crate::clipboard::send_return_key(&mut enigo, key) {
                    log::warn!("Paste: could not send the auto-submit key: {e}");
                }
                return;
            }
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                log::warn!("Paste: skipped auto-submit, input is busy");
                return;
            }
        }
    }
}

/// Paste through a clipboard transaction. On `Ok`, the dictation is on the
/// clipboard and the keystroke has been sent; the restore and any auto-submit
/// finish on their own.
///
/// Must be called with the input lock held (`enigo`), from the thread the
/// paste runs on: on macOS that is the main thread, which then has to be
/// released for AppKit to answer the target's read.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn paste(
    app_handle: &tauri::AppHandle,
    enigo: &mut enigo::Enigo,
    request: PasteRequest<'_>,
) -> Result<(), PasteTxError> {
    platform::run(app_handle, enigo, request)
}

/// Settle a paste transaction still waiting for its target, now. Used before
/// anything else snapshots the clipboard, so it records the user's content
/// rather than a dictation in flight.
///
/// Its auto-submit Enter, if still owed, is dropped rather than sent: the
/// only caller is an ask harvesting the selection, at the moment its keys go
/// down, so Enter would reach the target with those keys held (Ctrl+Enter,
/// a page break in Word).
///
/// Windows settles on the calling thread. macOS has to settle on the main
/// thread, so it is queued there and waited for briefly; a pending
/// transaction would otherwise settle on its own within [`RESTORE_TIMEOUT`],
/// after the harvest had already taken the dictation for the user's
/// clipboard and "restored" it.
pub(crate) fn settle_pending(app_handle: &tauri::AppHandle) {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    platform::settle_pending(app_handle);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = app_handle;
}

/// Settle a pending paste before the app exits, so the user's clipboard is
/// put back instead of being left holding the dictation (and on Windows, a
/// promise that dies unrendered with its window). An owed auto-submit still
/// goes. Bounded: quitting never waits on it for long.
pub(crate) fn settle_before_exit(app_handle: &tauri::AppHandle) {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    platform::settle_before_exit(app_handle);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = app_handle;
}

/// Windows' `HTML Format` for `fragment`: the header of byte offsets that Word
/// and browsers parse, then a document holding the fragment between the
/// `StartFragment` / `EndFragment` markers, NUL-terminated for the clipboard.
/// What arboard writes for the plain paste, built here because a promised
/// format is rendered by hand. Shared rather than Windows-only so its offsets
/// are unit-tested on every platform.
pub(crate) fn cf_html(fragment: &str) -> Vec<u8> {
    const START: &str = "<html><body>\r\n<!--StartFragment-->";
    const END: &str = "<!--EndFragment-->\r\n</body></html>";
    // Every offset is printed with ten digits, so the header's length is known
    // before the offsets are.
    let header_len = format!(
        "Version:0.9\r\nStartHTML:{0:010}\r\nEndHTML:{0:010}\r\nStartFragment:{0:010}\r\nEndFragment:{0:010}\r\n",
        0
    )
    .len();
    let start_html = header_len;
    let start_fragment = start_html + START.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + END.len();
    let mut out = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n{START}{fragment}{END}"
    )
    .into_bytes();
    out.push(0);
    out
}

/// Why a transaction paste did not complete.
#[derive(Debug)]
pub(crate) enum PasteTxError {
    /// Nothing was published; the plain paste should be used instead.
    Unavailable(String),
    /// The dictation was published but the keystroke failed. The transaction
    /// restores the clipboard on its own; a second paste must not be tried,
    /// since a keystroke that failed once can have half-arrived.
    Keystroke(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn published(ago: Duration) -> TxState {
        let mut s = TxState::new();
        s.published_at = Instant::now() - ago;
        s
    }

    #[test]
    fn keeps_waiting_without_a_read_within_the_timeout() {
        let s = published(Duration::from_millis(100));
        assert_eq!(evaluate(&s, Instant::now()), WaitDecision::KeepWaiting);
    }

    #[test]
    fn finishes_a_quiet_period_after_the_read() {
        let now = Instant::now();
        let mut s = published(Duration::from_millis(300));
        s.injected_at = Some(now - Duration::from_millis(250));
        s.receipts.push(now - QUIET_PERIOD);
        assert_eq!(evaluate(&s, now), WaitDecision::Finish);
    }

    #[test]
    fn waits_out_the_quiet_period_after_a_recent_read() {
        let now = Instant::now();
        let mut s = published(Duration::from_millis(300));
        s.injected_at = Some(now - Duration::from_millis(100));
        s.receipts.push(now - Duration::from_millis(50));
        assert_eq!(evaluate(&s, now), WaitDecision::KeepWaiting);
    }

    #[test]
    fn a_second_read_restarts_the_quiet_period() {
        // Chromium probes, then reads: the second read must keep the
        // dictation on the clipboard for another quiet period.
        let now = Instant::now();
        let mut s = published(Duration::from_millis(600));
        s.injected_at = Some(now - Duration::from_millis(500));
        s.receipts.push(now - Duration::from_millis(400));
        s.receipts.push(now - Duration::from_millis(20));
        assert_eq!(evaluate(&s, now), WaitDecision::KeepWaiting);
    }

    #[test]
    fn a_read_before_the_keystroke_does_not_count() {
        let now = Instant::now();
        let mut s = published(Duration::from_millis(300));
        s.receipts.push(now - Duration::from_millis(200));
        s.injected_at = Some(now - Duration::from_millis(100));
        assert!(!s.any_receipt_after_injection());
        assert_eq!(evaluate(&s, now), WaitDecision::KeepWaiting);
    }

    #[test]
    fn a_promise_read_before_the_keystroke_settles_after_the_blind_delay() {
        // A clipboard manager took the one read in the paste-delay gap: the
        // target's read will not be reported, so the wait is the fixed one,
        // and auto-submit still goes as it did with the plain paste.
        let now = Instant::now();
        let mut s = published(BLIND_RESTORE_DELAY + Duration::from_millis(100));
        s.receipts
            .push(now - BLIND_RESTORE_DELAY - Duration::from_millis(80));
        s.injected_at = Some(now - BLIND_RESTORE_DELAY);
        assert!(s.rendered_before_injection());
        assert_eq!(evaluate(&s, now), WaitDecision::Finish);
        assert!(settlement(&s, true, true, false).submit);
        assert_eq!(
            settle_reason(&s),
            "another app read the clipboard before the keystroke"
        );
    }

    #[test]
    fn a_failed_keystroke_never_submits_even_when_read_early() {
        let now = Instant::now();
        let mut s = published(FAILED_INJECTION_TIMEOUT);
        s.receipts.push(now - FAILED_INJECTION_TIMEOUT);
        s.injected_at = Some(now - Duration::from_millis(400));
        s.injection_failed = true;
        assert!(!settlement(&s, true, true, false).submit);
    }

    #[test]
    fn finishes_on_timeout_without_a_read() {
        let s = published(RESTORE_TIMEOUT);
        assert_eq!(evaluate(&s, Instant::now()), WaitDecision::Finish);
    }

    #[test]
    fn a_failed_keystroke_uses_the_short_timeout() {
        let mut s = published(FAILED_INJECTION_TIMEOUT);
        s.injection_failed = true;
        assert_eq!(evaluate(&s, Instant::now()), WaitDecision::Finish);
        let mut s = published(FAILED_INJECTION_TIMEOUT / 2);
        s.injection_failed = true;
        assert_eq!(evaluate(&s, Instant::now()), WaitDecision::KeepWaiting);
    }

    #[test]
    fn losing_the_clipboard_or_a_newer_paste_finishes_at_once() {
        let mut s = published(Duration::from_millis(10));
        s.ownership_lost = true;
        assert_eq!(evaluate(&s, Instant::now()), WaitDecision::Finish);
        let mut s = published(Duration::from_millis(10));
        s.cancelled = true;
        assert_eq!(evaluate(&s, Instant::now()), WaitDecision::Finish);
    }

    fn read_after_keystroke() -> TxState {
        let now = Instant::now();
        let mut s = published(Duration::from_millis(400));
        s.injected_at = Some(now - Duration::from_millis(300));
        s.receipts.push(now - Duration::from_millis(250));
        s
    }

    #[test]
    fn a_read_paste_restores_and_submits() {
        let s = read_after_keystroke();
        assert_eq!(
            settlement(&s, true, true, false),
            Settlement {
                submit: true,
                clipboard: ClipboardSettle::Restore
            }
        );
    }

    #[test]
    fn an_unread_paste_restores_but_never_submits() {
        let s = published(RESTORE_TIMEOUT);
        assert_eq!(
            settlement(&s, true, true, false),
            Settlement {
                submit: false,
                clipboard: ClipboardSettle::Restore
            }
        );
    }

    #[test]
    fn a_newer_copy_by_the_user_is_never_overwritten() {
        let mut s = read_after_keystroke();
        s.ownership_lost = true;
        assert_eq!(
            settlement(&s, true, false, true).clipboard,
            ClipboardSettle::Untouched
        );
        let s = read_after_keystroke();
        assert_eq!(
            settlement(&s, false, false, false).clipboard,
            ClipboardSettle::Untouched
        );
    }

    #[test]
    fn copy_to_clipboard_leaves_the_dictation() {
        let s = read_after_keystroke();
        assert_eq!(
            settlement(&s, true, false, true),
            Settlement {
                submit: false,
                clipboard: ClipboardSettle::LeaveCopy
            }
        );
    }

    #[test]
    fn a_blind_paste_submits_early_and_only_once() {
        // A clipboard manager took the one read before the keystroke: Enter
        // goes shortly after the keystroke, not with the restore a second
        // later, and settling does not send it again.
        let now = Instant::now();
        let mut s = published(Duration::from_millis(400));
        s.receipts.push(now - Duration::from_millis(390));
        s.injected_at = Some(now - BLIND_SUBMIT_DELAY / 2);
        assert!(!blind_submit_due(&s, now), "not before the delay");
        s.injected_at = Some(now - BLIND_SUBMIT_DELAY);
        assert!(blind_submit_due(&s, now));
        assert_eq!(evaluate(&s, now), WaitDecision::KeepWaiting);
        s.submitted = true;
        assert!(!blind_submit_due(&s, now));
        assert!(!settlement(&s, true, true, false).submit);
    }

    #[test]
    fn a_seen_read_or_a_failed_keystroke_never_submits_early() {
        let now = Instant::now();
        // The target's read was seen: Enter waits for the settle.
        let mut s = read_after_keystroke();
        assert!(!blind_submit_due(&s, now));
        // A failed keystroke never submits.
        s = published(Duration::from_millis(400));
        s.receipts.push(now - Duration::from_millis(390));
        s.injected_at = Some(now - BLIND_SUBMIT_DELAY);
        s.injection_failed = true;
        assert!(!blind_submit_due(&s, now));
    }

    #[test]
    fn settle_reasons_name_the_cause() {
        let mut s = published(Duration::ZERO);
        assert_eq!(settle_reason(&s), "no read arrived in time");
        s.injection_failed = true;
        assert_eq!(settle_reason(&s), "the keystroke could not be sent");
        let s = read_after_keystroke();
        assert_eq!(settle_reason(&s), "the target read the dictation");
    }

    /// The offsets in the header point exactly at the document and at the
    /// fragment, counted in bytes (UTF-8), which is what Word and browsers
    /// slice by.
    #[test]
    fn cf_html_offsets_point_at_the_fragment() {
        let fragment = "<ol>\n<li>Open <strong>Settings → Advanced</strong>.</li>\n</ol>";
        let bytes = cf_html(fragment);
        assert_eq!(bytes.last(), Some(&0));
        let doc = std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap();
        let offset = |key: &str| -> usize {
            let at = doc.find(key).unwrap() + key.len();
            doc[at..at + 10].parse().unwrap()
        };
        let (start_html, end_html) = (offset("StartHTML:"), offset("EndHTML:"));
        let (start, end) = (offset("StartFragment:"), offset("EndFragment:"));
        assert_eq!(&doc[start..end], fragment);
        assert!(doc[start_html..].starts_with("<html>"));
        assert_eq!(end_html, doc.len());
        assert!(doc[..start].ends_with("<!--StartFragment-->"));
        assert!(doc[end..].starts_with("<!--EndFragment-->"));
    }
}
