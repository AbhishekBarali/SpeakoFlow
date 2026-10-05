//! Modifier keys that a blocking listener in this process is keeping from the
//! operating system right now.
//!
//! A blocking hook withholds the key-downs of a modifier-only hotkey (Windows'
//! `Ctrl+Alt`, say) so the focused application never sees them. The OS then
//! believes those keys are up: `GetAsyncKeyState` reports them released while
//! the user is holding them. A hook knows which keys *it* withheld and corrects
//! for that, but a second listener in the same process — the non-blocking one
//! that records a new shortcut in Settings — had no way to know, so its
//! reconciliation read the held `Ctrl+Alt` as released the moment the next key
//! went down. It emitted synthetic releases, the recorder took the first
//! release as "the user let go", and recording `Ctrl+Alt+C` committed
//! `Ctrl+Alt` instead, which then collided with the assistant's own `Ctrl+Alt`
//! ("Hotkey already registered").
//!
//! Every blocking listener publishes what it withholds here; reconciliation in
//! any listener treats the union as held.

// Only the Windows hook withholds keys today; the record is still compiled
// everywhere so its logic is tested on every platform.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::sync::Mutex;
use std::thread::ThreadId;

use crate::types::Modifiers;

/// What each blocking listener (identified by its hook thread) withholds.
pub(crate) struct WithheldModifiers {
    by_listener: Mutex<Vec<(ThreadId, Modifiers)>>,
}

impl WithheldModifiers {
    pub(crate) const fn new() -> Self {
        Self {
            by_listener: Mutex::new(Vec::new()),
        }
    }

    /// Record what `listener` withholds now, replacing what it published before.
    pub(crate) fn publish(&self, listener: ThreadId, withheld: Modifiers) {
        let Ok(mut all) = self.by_listener.lock() else {
            return;
        };
        all.retain(|(id, _)| *id != listener);
        if !withheld.is_empty() {
            all.push((listener, withheld));
        }
    }

    /// Forget `listener`, whose hook is gone.
    pub(crate) fn clear(&self, listener: ThreadId) {
        self.publish(listener, Modifiers::empty());
    }

    /// Every modifier some blocking listener is withholding.
    pub(crate) fn union(&self) -> Modifiers {
        self.by_listener
            .lock()
            .map(|all| {
                all.iter()
                    .fold(Modifiers::empty(), |acc, (_, withheld)| acc | *withheld)
            })
            .unwrap_or(Modifiers::empty())
    }
}

/// The process-wide record the platform listeners share.
pub(crate) static PROCESS: WithheldModifiers = WithheldModifiers::new();

#[cfg(test)]
mod tests {
    use super::*;

    fn other_thread_id() -> ThreadId {
        std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap()
    }

    #[test]
    fn nothing_is_withheld_until_a_listener_says_so() {
        let record = WithheldModifiers::new();
        assert!(record.union().is_empty());
    }

    /// The case behind the bug: the blocking manager withholds Ctrl+Alt, and
    /// the recorder, a different listener, must see them as held.
    #[test]
    fn another_listeners_withheld_keys_are_visible() {
        let record = WithheldModifiers::new();
        let manager = other_thread_id();
        record.publish(manager, Modifiers::CTRL_LEFT | Modifiers::OPT_LEFT);
        assert_eq!(record.union(), Modifiers::CTRL_LEFT | Modifiers::OPT_LEFT);
    }

    /// A listener's latest report replaces its previous one, so a key it lets
    /// go of (or replays to the OS) stops counting as withheld.
    #[test]
    fn a_new_report_replaces_the_old_one() {
        let record = WithheldModifiers::new();
        let manager = other_thread_id();
        record.publish(manager, Modifiers::CTRL_LEFT | Modifiers::OPT_LEFT);
        record.publish(manager, Modifiers::CTRL_LEFT);
        assert_eq!(record.union(), Modifiers::CTRL_LEFT);
        record.publish(manager, Modifiers::empty());
        assert!(record.union().is_empty());
    }

    #[test]
    fn listeners_are_combined_and_cleared_independently() {
        let record = WithheldModifiers::new();
        let first = other_thread_id();
        let second = other_thread_id();
        record.publish(first, Modifiers::CTRL_LEFT);
        record.publish(second, Modifiers::CMD_LEFT);
        assert_eq!(record.union(), Modifiers::CTRL_LEFT | Modifiers::CMD_LEFT);
        record.clear(first);
        assert_eq!(record.union(), Modifiers::CMD_LEFT);
    }
}
