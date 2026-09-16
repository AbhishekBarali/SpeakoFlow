//! Suppressing the host application's *own* synthetic keystrokes.
//!
//! A low-level keyboard hook sees every event on the system, including the ones
//! the hosting application injects itself with `SendInput`. That is a problem
//! whenever an app both registers a hotkey and synthesizes keystrokes, because
//! its own injected modifier release is indistinguishable from the user letting
//! go of the hotkey.
//!
//! It is not hypothetical. SpeakoFlow harvests the focused app's selection with a
//! synthetic Ctrl+C at the moment an assistant recording starts. With a
//! modifier-only hotkey such as `Ctrl+Alt`, the injected Ctrl key-up came back
//! through the hook, matched the held hotkey, and ended the recording about 30 ms
//! after it began — so hold-to-talk recorded nothing at all. The app's existing
//! guard against this asked Windows (`GetAsyncKeyState`) whether the user was
//! still holding Alt, and the answer was "no", because this very hook had
//! *blocked* that Alt key-down from ever reaching Windows.
//!
//! The fix is a window, not a blanket rule. Injected events are only ignored
//! while [`ignore_injected_input`]'s guard is alive, i.e. for the few
//! milliseconds the host is actually synthesizing keys. Outside that window an
//! injected event is treated exactly as before, so a macro keyboard, AutoHotkey
//! remap, or accessibility tool that fires a registered hotkey keeps working.
//!
//! The counter is a depth rather than a flag so nested or overlapping synthetic
//! sequences cannot have an inner one's exit re-enable matching for an outer one
//! that is still running.

use std::sync::atomic::{AtomicUsize, Ordering};

static DEPTH: AtomicUsize = AtomicUsize::new(0);

/// Whether the host is currently synthesizing keystrokes, and injected keyboard
/// events should therefore not be matched against registered hotkeys.
pub fn ignoring_injected_input() -> bool {
    DEPTH.load(Ordering::SeqCst) > 0
}

/// Ignore injected keyboard events until the returned guard is dropped.
///
/// Wrap every synthetic key sequence in this. Injected events still reach the
/// rest of the system untouched — they are only withheld from hotkey matching,
/// so a paste still pastes and a synthetic copy still copies.
#[must_use = "injected input is only ignored while the guard is alive"]
pub fn ignore_injected_input() -> InjectedInputGuard {
    DEPTH.fetch_add(1, Ordering::SeqCst);
    InjectedInputGuard(())
}

/// Restores injected-event matching when dropped. See [`ignore_injected_input`].
pub struct InjectedInputGuard(());

impl Drop for InjectedInputGuard {
    fn drop(&mut self) {
        // `fetch_update` rather than `fetch_sub`, so a stray extra drop can never
        // wrap the counter around to a huge number and suppress every injected
        // event for the rest of the process's life.
        let _ = DEPTH.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |depth| {
            Some(depth.saturating_sub(1))
        });
    }
}

/// Whether one keyboard event should be withheld from hotkey matching.
///
/// Split out from the platform hook so the rule is testable without synthesizing
/// system input.
pub(crate) fn should_ignore_event(is_injected: bool) -> bool {
    is_injected && ignoring_injected_input()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counter is process-wide, so these tests cannot run beside each other.
    /// Without this they pass alone and fail at random under the default test
    /// harness, which is the worst kind of test.
    static SERIALIZE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        SERIALIZE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The whole point: a real key press is never ignored, and an injected one is
    /// ignored only inside the guard's window.
    #[test]
    fn only_injected_events_inside_the_window_are_ignored() {
        let _lock = exclusive();
        assert!(!should_ignore_event(true), "no guard is active");
        assert!(!should_ignore_event(false));
        {
            let _guard = ignore_injected_input();
            assert!(should_ignore_event(true), "our own synthetic keystroke");
            assert!(
                !should_ignore_event(false),
                "a physical key must still reach hotkey matching, even mid-injection"
            );
        }
        assert!(
            !should_ignore_event(true),
            "the window must close with the guard, or an external macro keyboard \
             would stop working for the rest of the run"
        );
    }

    /// Nested sequences: the inner guard's exit must not re-enable matching while
    /// the outer one is still synthesizing.
    #[test]
    fn nested_guards_keep_the_window_open_until_the_outermost_exits() {
        let _lock = exclusive();
        let outer = ignore_injected_input();
        {
            let _inner = ignore_injected_input();
            assert!(should_ignore_event(true));
        }
        assert!(should_ignore_event(true), "the outer sequence is still running");
        drop(outer);
        assert!(!should_ignore_event(true));
    }

    /// The counter must not underflow into "suppress everything forever".
    #[test]
    fn the_depth_never_wraps_below_zero() {
        let _lock = exclusive();
        drop(ignore_injected_input());
        drop(ignore_injected_input());
        assert!(!ignoring_injected_input());
        assert_eq!(DEPTH.load(Ordering::SeqCst), 0);
    }
}
