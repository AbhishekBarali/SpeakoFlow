//! The listeners subscribed to the process's one evdev engine.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use super::keycode;
use crate::platform::state::BlockingHotkeys;
use crate::types::{Key, KeyEvent, Modifiers};

/// One listener's end of the engine.
pub(crate) struct Subscriber {
    pub(crate) sender: Sender<KeyEvent>,
    /// Cleared when the listener is dropped.
    pub(crate) running: Arc<AtomicBool>,
    /// Hotkeys whose key is withheld from the focused app, if this listener
    /// withholds any (the hotkey manager does, the Settings recorder not).
    pub(crate) blocking: Option<BlockingHotkeys>,
}

impl Subscriber {
    fn is_live(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Whether this listener withholds a mouse button the engine can read.
    fn withholds_a_button(&self) -> bool {
        self.blocking.as_ref().is_some_and(|hotkeys| {
            hotkeys.lock().is_ok_and(|set| {
                set.iter()
                    .any(|hotkey| hotkey.key.is_some_and(keycode::is_withholdable_button))
            })
        })
    }

    fn blocks(&self, modifiers: Modifiers, key: Key) -> bool {
        self.blocking.as_ref().is_some_and(|hotkeys| {
            hotkeys.lock().is_ok_and(|set| {
                set.iter()
                    .any(|hotkey| hotkey.key == Some(key) && hotkey.modifiers.matches(modifiers))
            })
        })
    }
}

pub(crate) struct Hub {
    subscribers: Mutex<Vec<Subscriber>>,
    /// Signalled when a listener subscribes, for an engine waiting for one.
    arrived: Condvar,
    /// An eventfd in the engine's poll set, so a listener coming or going
    /// wakes it.
    wake: OwnedFd,
}

impl Hub {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: eventfd takes no pointers.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            subscribers: Mutex::new(Vec::new()),
            arrived: Condvar::new(),
            // SAFETY: `fd` is a new descriptor nothing else owns.
            wake: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    /// A panic while the lock was held must not end capture for everyone.
    fn lock(&self) -> MutexGuard<'_, Vec<Subscriber>> {
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn subscribe(&self, subscriber: Subscriber) {
        self.lock().push(subscriber);
        self.arrived.notify_all();
        self.wake();
    }

    /// Deliver an event to every live listener, forgetting the ones that left.
    pub(crate) fn broadcast(&self, event: KeyEvent) {
        self.lock()
            .retain(|subscriber| subscriber.is_live() && subscriber.sender.send(event).is_ok());
    }

    /// Whether any live listener withholds `key` with `modifiers` held.
    pub(crate) fn should_block(&self, modifiers: Modifiers, key: Key) -> bool {
        self.lock()
            .iter()
            .any(|subscriber| subscriber.is_live() && subscriber.blocks(modifiers, key))
    }

    /// Whether any live listener withholds a mouse button, which is when the
    /// pointers that have it are taken over.
    pub(crate) fn withholds_buttons(&self) -> bool {
        self.lock()
            .iter()
            .any(|subscriber| subscriber.is_live() && subscriber.withholds_a_button())
    }

    pub(crate) fn has_live_subscribers(&self) -> bool {
        let mut subscribers = self.lock();
        subscribers.retain(Subscriber::is_live);
        !subscribers.is_empty()
    }

    pub(crate) fn wait_for_subscriber(&self) {
        let mut subscribers = self.lock();
        loop {
            subscribers.retain(Subscriber::is_live);
            if !subscribers.is_empty() {
                return;
            }
            subscribers = self
                .arrived
                .wait(subscribers)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(crate) fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: writes the 8 bytes of `one`. It only fails when the counter
        // is about to overflow, and then the engine is already woken.
        let _ = unsafe { libc::write(self.wake.as_raw_fd(), (&one as *const u64).cast(), 8) };
    }

    pub(crate) fn clear_wake(&self) {
        let mut count: u64 = 0;
        // SAFETY: reads at most 8 bytes into `count`.
        let _ = unsafe { libc::read(self.wake.as_raw_fd(), (&mut count as *mut u64).cast(), 8) };
    }

    pub(crate) fn wake_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Hotkey;
    use std::collections::HashSet;
    use std::sync::mpsc;

    fn ctrl_k() -> KeyEvent {
        KeyEvent {
            modifiers: Modifiers::CTRL_LEFT,
            key: Some(Key::K),
            is_key_down: true,
            changed_modifier: None,
        }
    }

    #[test]
    fn listeners_that_left_stop_receiving_and_stop_withholding() {
        let hub = Hub::new().expect("eventfd");
        let hotkey = Hotkey::new(Modifiers::CTRL, Key::K).unwrap();
        let blocking: BlockingHotkeys = Arc::new(Mutex::new(HashSet::from([hotkey])));
        let (manager_tx, manager_rx) = mpsc::channel();
        let manager_running = Arc::new(AtomicBool::new(true));
        let (recorder_tx, recorder_rx) = mpsc::channel();
        hub.subscribe(Subscriber {
            sender: manager_tx,
            running: Arc::clone(&manager_running),
            blocking: Some(blocking),
        });
        hub.subscribe(Subscriber {
            sender: recorder_tx,
            running: Arc::new(AtomicBool::new(true)),
            blocking: None,
        });

        assert!(hub.should_block(Modifiers::CTRL_LEFT, Key::K));
        assert!(!hub.should_block(Modifiers::empty(), Key::K));
        hub.broadcast(ctrl_k());
        assert_eq!(manager_rx.try_iter().count(), 1);
        assert_eq!(recorder_rx.try_iter().count(), 1);

        manager_running.store(false, Ordering::SeqCst);
        assert!(
            !hub.should_block(Modifiers::CTRL_LEFT, Key::K),
            "a listener that left withholds nothing"
        );
        hub.broadcast(ctrl_k());
        assert_eq!(manager_rx.try_iter().count(), 0);
        assert_eq!(recorder_rx.try_iter().count(), 1);
        assert!(hub.has_live_subscribers());

        assert!(
            !hub.withholds_buttons(),
            "Ctrl+K withholds a key, not a button"
        );
        drop(recorder_rx);
        hub.broadcast(ctrl_k());
        assert!(
            !hub.has_live_subscribers(),
            "a listener whose receiver is gone is dropped too"
        );
    }

    #[test]
    fn pointers_are_wanted_only_while_a_readable_button_is_a_shortcut() {
        let hub = Hub::new().expect("eventfd");
        let blocking: BlockingHotkeys = Arc::new(Mutex::new(HashSet::new()));
        let (sender, _events) = mpsc::channel();
        hub.subscribe(Subscriber {
            sender,
            running: Arc::new(AtomicBool::new(true)),
            blocking: Some(Arc::clone(&blocking)),
        });
        assert!(!hub.withholds_buttons());
        blocking
            .lock()
            .unwrap()
            .insert(Hotkey::new(Modifiers::empty(), Key::MouseX1).unwrap());
        assert!(!hub.withholds_buttons(), "Back is never read on Linux");
        blocking
            .lock()
            .unwrap()
            .insert(Hotkey::new(Modifiers::CTRL_LEFT, Key::MouseMiddle).unwrap());
        assert!(hub.withholds_buttons());
        blocking.lock().unwrap().clear();
        assert!(!hub.withholds_buttons());
    }
}
