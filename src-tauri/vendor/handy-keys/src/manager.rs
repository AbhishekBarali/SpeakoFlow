//! Platform-agnostic hotkey manager built on top of KeyboardListener

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::listener::{BlockingHotkeys, KeyboardListener};
use crate::types::{Hotkey, HotkeyEvent, HotkeyId, HotkeyState, Key, KeyEvent, Modifiers};

/// How long a modifier-only hotkey is held back when another registered hotkey
/// starts with the same modifiers.
///
/// `Ctrl+Alt` (ask) and `Ctrl+Alt+C` (call) share their first two keys, so the
/// moment Ctrl+Alt goes down it is not yet known which one is meant. Firing at
/// once started an ask — panel, microphone, a synthetic copy of the selection —
/// on the way to every call, and that is why the call used to live on an
/// unrelated combo. A third key inside this window is the longer shortcut and
/// the shorter one never fires; no third key and it fires when the window ends.
/// A hotkey nothing else extends is not held back at all.
pub(crate) const CHORD_GRACE: Duration = Duration::from_millis(200);

/// After a modifier-only hotkey has fired, a key typed on top of it within this
/// window means the modifiers were the start of some other shortcut — VS Code's
/// `Ctrl+Alt+↑`, Windows' `Ctrl+Win+→`, a German layout's `Ctrl+Alt+Q` for `@`.
/// The hotkey is then withdrawn with [`HotkeyState::Cancelled`] instead of
/// being left running behind a shortcut the user meant for someone else. Past
/// the window the hotkey is plainly being held on purpose and a stray key no
/// longer ends it.
pub(crate) const CHORD_CANCEL_WINDOW: Duration = Duration::from_millis(500);

/// The four side-paired modifier groups, in the order `Modifiers::matches` uses.
const MODIFIER_GROUPS: [Modifiers; 4] = [
    Modifiers::CMD,
    Modifiers::SHIFT,
    Modifiers::CTRL,
    Modifiers::OPT,
];

/// Whether `longer` can be reached by holding `shorter` and pressing more:
/// every modifier `shorter` needs, `longer` needs too (on a compatible side),
/// and `longer` adds a key or another modifier on top.
fn extends(longer: &Hotkey, shorter: &Hotkey) -> bool {
    if longer == shorter || shorter.key.is_some() {
        return false;
    }
    let mut adds_modifier = false;
    for group in MODIFIER_GROUPS {
        let short = shorter.modifiers & group;
        let long = longer.modifiers & group;
        if !short.is_empty() && !short.intersects(long) {
            return false;
        }
        adds_modifier |= short.is_empty() && !long.is_empty();
    }
    let short_fn = shorter.modifiers.contains(Modifiers::FN);
    let long_fn = longer.modifiers.contains(Modifiers::FN);
    if short_fn && !long_fn {
        return false;
    }
    adds_modifier |= long_fn && !short_fn;
    longer.key.is_some() || adds_modifier
}

/// A keyboard key, as opposed to a mouse button. Clicking while holding a
/// modifier-only hotkey is ordinary use, not the start of another shortcut.
fn is_keyboard_key(key: Key) -> bool {
    !matches!(
        key,
        Key::MouseLeft | Key::MouseRight | Key::MouseMiddle | Key::MouseX1 | Key::MouseX2
    )
}

/// Internal state shared between the manager and the processing thread
struct ManagerState {
    hotkeys: HashMap<HotkeyId, Hotkey>,
    next_id: u32,
    /// Track which hotkeys are currently pressed
    pressed_hotkeys: HashSet<HotkeyId>,
    /// When each pressed modifier-only hotkey fired, for [`CHORD_CANCEL_WINDOW`].
    pressed_at: HashMap<HotkeyId, Instant>,
    /// Modifier-only hotkeys held back by [`CHORD_GRACE`], with the moment they
    /// fire if nothing else is pressed first.
    pending: HashMap<HotkeyId, Instant>,
    /// Modifier-only hotkeys that turned out to be part of a chord. They stay
    /// quiet until their modifiers stop matching, so letting go of the chord's
    /// last key while still holding the modifiers does not fire them.
    suppressed: HashSet<HotkeyId>,
}

impl ManagerState {
    fn new() -> Self {
        Self {
            hotkeys: HashMap::new(),
            next_id: 0,
            pressed_hotkeys: HashSet::new(),
            pressed_at: HashMap::new(),
            pending: HashMap::new(),
            suppressed: HashSet::new(),
        }
    }

    /// Forget everything about a hotkey that is being unregistered.
    fn forget(&mut self, id: HotkeyId) {
        self.pressed_hotkeys.remove(&id);
        self.pressed_at.remove(&id);
        self.pending.remove(&id);
        self.suppressed.remove(&id);
    }

    /// The next moment [`Self::tick`] has something to do.
    fn next_deadline(&self) -> Option<Instant> {
        self.pending.values().min().copied()
    }

    /// Fire every held-back hotkey whose grace has run out.
    fn tick(&mut self, now: Instant) -> Vec<HotkeyEvent> {
        let due: Vec<HotkeyId> = self
            .pending
            .iter()
            .filter(|(_, &at)| at <= now)
            .map(|(&id, _)| id)
            .collect();
        due.into_iter()
            .map(|id| {
                self.pending.remove(&id);
                self.press(id, now)
            })
            .collect()
    }

    fn press(&mut self, id: HotkeyId, now: Instant) -> HotkeyEvent {
        self.pressed_hotkeys.insert(id);
        self.pressed_at.insert(id, now);
        HotkeyEvent {
            id,
            state: HotkeyState::Pressed,
        }
    }

    fn is_modifier_only(&self, id: HotkeyId) -> bool {
        self.hotkeys.get(&id).is_some_and(|h| h.key.is_none())
    }

    /// Whether another registered hotkey starts with this one's modifiers.
    fn is_extended(&self, id: HotkeyId) -> bool {
        let Some(hotkey) = self.hotkeys.get(&id) else {
            return false;
        };
        self.hotkeys
            .iter()
            .any(|(&other, longer)| other != id && extends(longer, hotkey))
    }

    fn within_cancel_window(&self, id: HotkeyId, now: Instant) -> bool {
        self.pressed_at
            .get(&id)
            .is_some_and(|&at| now.saturating_duration_since(at) < CHORD_CANCEL_WINDOW)
    }

    /// Process a key event and return any matching hotkey events
    #[cfg(test)]
    fn process_event(&mut self, event: &KeyEvent) -> Vec<HotkeyEvent> {
        self.process_event_at(event, Instant::now())
    }

    /// Process a key event observed at `now`.
    fn process_event_at(&mut self, event: &KeyEvent, now: Instant) -> Vec<HotkeyEvent> {
        // Anything whose grace ran out before this event fires first, so the
        // order of events is the order things happened in.
        let mut results = self.tick(now);

        // A chord's modifier-only hotkey is quiet until its modifiers change.
        let hotkeys = &self.hotkeys;
        self.suppressed.retain(|id| {
            hotkeys
                .get(id)
                .is_some_and(|h| h.modifiers.matches(event.modifiers))
        });

        if event.is_key_down {
            match event.key {
                Some(key) => {
                    if is_keyboard_key(key) {
                        self.yield_to_chord(now, &mut results);
                    }
                    let to_press: Vec<HotkeyId> = self
                        .hotkeys
                        .iter()
                        .filter(|(&id, hotkey)| {
                            hotkey.key == Some(key)
                                && hotkey.modifiers.matches(event.modifiers)
                                && !self.pressed_hotkeys.contains(&id)
                        })
                        .map(|(&id, _)| id)
                        .collect();
                    for id in to_press {
                        self.pressed_hotkeys.insert(id);
                        results.push(HotkeyEvent {
                            id,
                            state: HotkeyState::Pressed,
                        });
                    }
                }
                None => self.modifier_down(event.modifiers, now, &mut results),
            }
        } else {
            // A held-back hotkey whose modifiers were let go before its grace
            // ran out was a quick tap: it fires and releases at once, which is
            // what a tap-to-toggle shortcut needs.
            let tapped: Vec<HotkeyId> = self
                .pending
                .keys()
                .copied()
                .filter(|id| {
                    event.key.is_none()
                        && !self
                            .hotkeys
                            .get(id)
                            .is_some_and(|h| h.modifiers.matches(event.modifiers))
                })
                .collect();
            for id in tapped {
                self.pending.remove(&id);
                results.push(self.press(id, now));
                self.pressed_hotkeys.remove(&id);
                self.pressed_at.remove(&id);
                results.push(HotkeyEvent {
                    id,
                    state: HotkeyState::Released,
                });
            }
            // Check for hotkeys that should be released
            // A hotkey is released when its key is released, or — for modifier
            // events — when the modifiers no longer match. A modifier event
            // (key == None) whose modifiers still match must not release a
            // modifier-only hotkey: `hotkey.key == event.key` is true when both
            // are None, so tapping Shift (or our own injected Ctrl key-up)
            // while Ctrl+Win was held used to end the recording. Both Windows
            // defaults (Ctrl+Win dictation, Ctrl+Alt assistant) are
            // modifier-only. Port of upstream handy-keys #23.
            let to_release: Vec<HotkeyId> = self
                .hotkeys
                .iter()
                .filter(|(&id, hotkey)| {
                    self.pressed_hotkeys.contains(&id)
                        && ((event.key.is_some() && hotkey.key == event.key)
                            || (event.key.is_none() && !hotkey.modifiers.matches(event.modifiers)))
                })
                .map(|(&id, _)| id)
                .collect();

            for id in to_release {
                self.pressed_hotkeys.remove(&id);
                self.pressed_at.remove(&id);
                results.push(HotkeyEvent {
                    id,
                    state: HotkeyState::Released,
                });
            }
        }

        results
    }

    /// A keyboard key went down. Whatever modifier-only hotkey the held
    /// modifiers were about to fire, or fired a moment ago, was the start of
    /// another shortcut instead.
    fn yield_to_chord(&mut self, now: Instant, results: &mut Vec<HotkeyEvent>) {
        let held_back: Vec<HotkeyId> = self.pending.drain().map(|(id, _)| id).collect();
        self.suppressed.extend(held_back);

        let withdrawn: Vec<HotkeyId> = self
            .pressed_hotkeys
            .iter()
            .copied()
            .filter(|&id| self.is_modifier_only(id) && self.within_cancel_window(id, now))
            .collect();
        for id in withdrawn {
            self.pressed_hotkeys.remove(&id);
            self.pressed_at.remove(&id);
            self.suppressed.insert(id);
            results.push(HotkeyEvent {
                id,
                state: HotkeyState::Cancelled,
            });
        }
    }

    /// A modifier went down and the held modifiers are now `modifiers`.
    fn modifier_down(&mut self, modifiers: Modifiers, now: Instant, results: &mut Vec<HotkeyEvent>) {
        let matched: Vec<HotkeyId> = self
            .hotkeys
            .iter()
            .filter(|(&id, hotkey)| {
                hotkey.key.is_none()
                    && hotkey.modifiers.matches(modifiers)
                    && !self.pressed_hotkeys.contains(&id)
                    && !self.suppressed.contains(&id)
            })
            .map(|(&id, _)| id)
            .collect();

        // Whatever was held back no longer matches the modifiers now held.
        self.pending.retain(|id, _| matched.contains(id));
        if matched.is_empty() {
            return;
        }

        // Another modifier-only hotkey is already running. Fired a moment ago,
        // it was the first half of this longer one (Ctrl+Win on the way to
        // Ctrl+Win+Shift) and gives way. Held for longer, it owns the keyboard
        // and a modifier pressed on top of it is not a new shortcut.
        let running: Vec<HotkeyId> = self
            .pressed_hotkeys
            .iter()
            .copied()
            .filter(|&id| self.is_modifier_only(id))
            .collect();
        if running
            .iter()
            .any(|&id| !self.within_cancel_window(id, now))
        {
            return;
        }
        for id in running {
            self.pressed_hotkeys.remove(&id);
            self.pressed_at.remove(&id);
            results.push(HotkeyEvent {
                id,
                state: HotkeyState::Cancelled,
            });
        }

        for id in matched {
            if self.is_extended(id) {
                self.pending.entry(id).or_insert(now + CHORD_GRACE);
            } else {
                results.push(self.press(id, now));
            }
        }
    }
}

/// Platform-agnostic Hotkey Manager
///
/// This manager wraps a `KeyboardListener` and filters events against
/// registered hotkeys, emitting `HotkeyEvent`s when matches occur.
///
/// Registered hotkeys are blocked from reaching other applications.
/// Note: On Linux/Wayland, blocking may not work due to compositor restrictions.
pub struct HotkeyManager {
    state: Arc<Mutex<ManagerState>>,
    event_receiver: Receiver<HotkeyEvent>,
    _thread_handle: Option<JoinHandle<()>>,
    running: Arc<std::sync::atomic::AtomicBool>,
    /// Shared set of hotkeys to block
    blocking_hotkeys: Option<BlockingHotkeys>,
    #[cfg(target_os = "windows")]
    shutdown_event: Arc<std::os::windows::io::OwnedHandle>,
}

impl HotkeyManager {
    /// Create a new HotkeyManager (non-blocking mode)
    ///
    /// On macOS, this will check for accessibility permissions and fail if not granted.
    pub fn new() -> Result<Self> {
        Self::new_internal(false, None)
    }

    /// Create a new HotkeyManager with blocking support
    ///
    /// On macOS, this will check for accessibility permissions and fail if not granted.
    /// Registered hotkeys will be blocked from reaching other applications.
    ///
    /// Note: On Linux/Wayland, blocking may not work due to compositor restrictions.
    pub fn new_with_blocking() -> Result<Self> {
        Self::new_internal(true, None)
    }

    /// Deliver matching hotkeys directly to a handler instead of the receive
    /// queue. The handler should only enqueue the event and return promptly;
    /// returning false stops delivery. This lets an application wait on one
    /// channel for commands and hotkeys without polling either queue.
    pub fn new_with_blocking_handler(
        handler: impl Fn(HotkeyEvent) -> bool + Send + 'static,
    ) -> Result<Self> {
        Self::new_internal(true, Some(Box::new(handler)))
    }

    fn new_internal(
        blocking: bool,
        handler: Option<Box<dyn Fn(HotkeyEvent) -> bool + Send>>,
    ) -> Result<Self> {
        let blocking_hotkeys: Option<BlockingHotkeys> =
            blocking.then(|| Arc::new(Mutex::new(HashSet::new())));
        let listener = match &blocking_hotkeys {
            Some(hotkeys) => KeyboardListener::new_with_blocking(hotkeys.clone())?,
            None => KeyboardListener::new()?,
        };
        #[cfg(target_os = "windows")]
        let shutdown_event = listener.shutdown_event();

        let (tx, rx) = mpsc::channel();
        let deliver = handler.unwrap_or_else(|| Box::new(move |event| tx.send(event).is_ok()));
        let state = Arc::new(Mutex::new(ManagerState::new()));
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));

        let thread_state = Arc::clone(&state);
        let thread_running = Arc::clone(&running);

        let handle = thread::spawn(move || {
            Self::event_loop(
                |timeout: Option<Duration>| {
                    // Windows shutdown closes the native sender, waking recv
                    // without a timer, so it only sleeps with a timeout while a
                    // held-back hotkey is waiting to fire. Other platforms keep
                    // their stop poll.
                    #[cfg(target_os = "windows")]
                    {
                        match timeout {
                            Some(timeout) => listener.recv_timeout(timeout),
                            None => listener.recv(),
                        }
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        let poll = Duration::from_millis(100);
                        listener.recv_timeout(timeout.map_or(poll, |t| t.min(poll)))
                    }
                },
                thread_state,
                deliver,
                thread_running,
            );
        });

        Ok(Self {
            state,
            event_receiver: rx,
            _thread_handle: Some(handle),
            running,
            blocking_hotkeys,
            #[cfg(target_os = "windows")]
            shutdown_event,
        })
    }

    /// Event processing loop. `next_event` waits for the next key event, for at
    /// most the given time when one is given.
    fn event_loop(
        next_event: impl Fn(Option<Duration>) -> Result<KeyEvent>,
        state: Arc<Mutex<ManagerState>>,
        deliver: Box<dyn Fn(HotkeyEvent) -> bool + Send>,
        running: Arc<std::sync::atomic::AtomicBool>,
    ) {
        while running.load(std::sync::atomic::Ordering::SeqCst) {
            let timeout = state.lock().ok().and_then(|s| s.next_deadline()).map(|at| {
                at.saturating_duration_since(Instant::now())
            });
            let hotkey_events = match next_event(timeout) {
                Ok(key_event) => match state.lock() {
                    Ok(mut state) => state.process_event_at(&key_event, Instant::now()),
                    Err(_) => continue,
                },
                Err(crate::error::Error::Timeout) => match state.lock() {
                    // A held-back hotkey's grace may have run out.
                    Ok(mut state) => state.tick(Instant::now()),
                    Err(_) => continue,
                },
                Err(_) => {
                    // Listener disconnected, exit
                    return;
                }
            };
            for event in hotkey_events {
                if !deliver(event) {
                    // Receiver dropped, exit
                    return;
                }
            }
        }
    }

    /// Register a hotkey and return its unique ID
    ///
    /// Returns an error if the hotkey is already registered.
    pub fn register(&self, hotkey: Hotkey) -> Result<HotkeyId> {
        let mut state = self.state.lock().map_err(|_| Error::MutexPoisoned)?;

        // Check if already registered
        for (id, existing) in &state.hotkeys {
            if existing == &hotkey {
                return Err(Error::HotkeyAlreadyRegistered(format!(
                    "{} (id: {:?})",
                    hotkey, id
                )));
            }
        }

        let id = HotkeyId(state.next_id);
        state.next_id += 1;
        state.hotkeys.insert(id, hotkey);

        // Add to blocking set
        if let Some(blocking_hotkeys) = &self.blocking_hotkeys {
            if let Ok(mut blocking) = blocking_hotkeys.lock() {
                blocking.insert(hotkey);
            }
        }

        Ok(id)
    }

    /// Unregister a hotkey by its ID
    ///
    /// Returns an error if the hotkey ID is not found.
    pub fn unregister(&self, id: HotkeyId) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| Error::MutexPoisoned)?;

        let hotkey = state.hotkeys.remove(&id);
        if hotkey.is_none() {
            return Err(Error::HotkeyNotFound(id));
        }
        state.forget(id);

        // Remove from blocking set
        if let Some(blocking_hotkeys) = &self.blocking_hotkeys {
            if let Some(hotkey) = hotkey {
                if let Ok(mut blocking) = blocking_hotkeys.lock() {
                    blocking.remove(&hotkey);
                }
            }
        }

        Ok(())
    }

    /// Get the hotkey definition associated with an ID
    ///
    /// Returns `None` if the ID is not found.
    pub fn get_hotkey(&self, id: HotkeyId) -> Option<Hotkey> {
        let state = self.state.lock().ok()?;
        state.hotkeys.get(&id).copied()
    }

    /// Blocking receive for hotkey events
    ///
    /// Blocks until a hotkey event is received or the event loop stops.
    pub fn recv(&self) -> Result<HotkeyEvent> {
        self.event_receiver
            .recv()
            .map_err(|_| Error::EventLoopNotRunning)
    }

    /// Non-blocking receive for hotkey events
    ///
    /// Returns `Some(event)` if an event is available, `None` otherwise.
    pub fn try_recv(&self) -> Option<HotkeyEvent> {
        match self.event_receiver.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Get the number of currently registered hotkeys
    pub fn hotkey_count(&self) -> usize {
        let state = if let Ok(s) = self.state.lock() {
            s
        } else {
            return 0;
        };
        state.hotkeys.len()
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::{Foundation::HANDLE, System::Threading::SetEvent};
            // End the native listener first: closing its sender wakes the
            // processing thread even if no hotkey has ever been pressed.
            unsafe {
                let _ = SetEvent(HANDLE(self.shutdown_event.as_raw_handle()));
            }
        }
        // Join the thread to ensure clean shutdown
        if let Some(handle) = self._thread_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Key, Modifiers};

    #[test]
    fn handler_delivers_press_and_release_in_order_without_polling() {
        let mut state = ManagerState::new();
        state
            .hotkeys
            .insert(HotkeyId(0), Hotkey::new(Modifiers::CTRL, Key::K).unwrap());
        let (key_tx, key_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        for down in [true, true, false] {
            key_tx
                .send(make_key_event(Modifiers::CTRL_LEFT, Some(Key::K), down))
                .unwrap();
        }
        drop(key_tx);
        HotkeyManager::event_loop(
            |_| key_rx.recv().map_err(|_| Error::EventLoopNotRunning),
            Arc::new(Mutex::new(state)),
            Box::new(move |event| event_tx.send(event).is_ok()),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        let events: Vec<_> = event_rx.try_iter().map(|e| (e.id, e.state)).collect();
        assert_eq!(
            events,
            vec![
                (HotkeyId(0), HotkeyState::Pressed),
                (HotkeyId(0), HotkeyState::Released),
            ]
        );
    }

    #[test]
    fn handler_disconnect_stops_event_processing() {
        let mut state = ManagerState::new();
        state
            .hotkeys
            .insert(HotkeyId(0), Hotkey::new(Modifiers::CTRL, Key::K).unwrap());
        let reads = std::cell::Cell::new(0);
        HotkeyManager::event_loop(
            |_| {
                reads.set(reads.get() + 1);
                assert_eq!(reads.get(), 1, "read again after delivery stopped");
                Ok(make_key_event(Modifiers::CTRL_LEFT, Some(Key::K), true))
            },
            Arc::new(Mutex::new(state)),
            Box::new(|_| false),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn idle_native_manager_shuts_down_without_keyboard_input() {
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            // Immediate drops also exercise shutdown before native hook setup
            // finishes. No hotkeys are registered and no input is synthesized.
            for _ in 0..8 {
                drop(HotkeyManager::new_with_blocking_handler(|_| true).unwrap());
            }
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        worker.join().unwrap();
    }

    fn make_key_event(modifiers: Modifiers, key: Option<Key>, is_key_down: bool) -> KeyEvent {
        KeyEvent {
            modifiers,
            key,
            is_key_down,
            changed_modifier: None,
        }
    }

    fn make_modifier_event(
        modifiers: Modifiers,
        is_key_down: bool,
        changed: Modifiers,
    ) -> KeyEvent {
        KeyEvent {
            modifiers,
            key: None,
            is_key_down,
            changed_modifier: Some(changed),
        }
    }

    mod manager_state {
        use super::*;

        // ---- Chords: shortcuts that share their first keys -----------------

        const CTRL_ALT: Modifiers = Modifiers::CTRL_LEFT.union(Modifiers::OPT_LEFT);
        const CTRL_WIN: Modifiers = Modifiers::CTRL_LEFT.union(Modifiers::CMD_LEFT);

        /// The Windows defaults: dictate Ctrl+Win, cleanup Ctrl+Win+Shift, ask
        /// Ctrl+Alt, call Ctrl+Alt+C.
        fn windows_defaults() -> (ManagerState, [HotkeyId; 4]) {
            let mut state = ManagerState::new();
            let ids = [HotkeyId(0), HotkeyId(1), HotkeyId(2), HotkeyId(3)];
            for (id, hotkey) in ids.iter().zip([
                "ctrl_left+super",
                "ctrl_left+super+shift",
                "ctrl_left+alt_left",
                "ctrl_left+alt_left+c",
            ]) {
                state.hotkeys.insert(*id, hotkey.parse().unwrap());
            }
            (state, ids)
        }

        fn states(events: &[HotkeyEvent]) -> Vec<(HotkeyId, HotkeyState)> {
            events.iter().map(|e| (e.id, e.state)).collect()
        }

        fn at(start: Instant, ms: u64) -> Instant {
            start + Duration::from_millis(ms)
        }

        #[test]
        fn a_hotkey_another_one_extends_waits_out_its_grace() {
            let (mut state, [_, _, ask, _]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT),
                t0,
            );
            let down = make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT);
            assert!(state.process_event_at(&down, at(t0, 10)).is_empty());
            assert_eq!(state.next_deadline(), Some(at(t0, 10) + CHORD_GRACE));
            assert!(state.tick(at(t0, 100)).is_empty());
            assert_eq!(
                states(&state.tick(at(t0, 10) + CHORD_GRACE)),
                vec![(ask, HotkeyState::Pressed)]
            );
        }

        #[test]
        fn ask_keys_plus_c_starts_a_call_and_never_an_ask() {
            let (mut state, [_, _, ask, call]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT),
                t0,
            );
            state.process_event_at(
                &make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT),
                at(t0, 20),
            );
            let c = state.process_event_at(&make_key_event(CTRL_ALT, Some(Key::C), true), at(t0, 90));
            assert_eq!(states(&c), vec![(call, HotkeyState::Pressed)]);
            // Grace would have run out by now; the ask must stay quiet.
            assert!(state.tick(at(t0, 1000)).is_empty());
            // Letting go of C while still on Ctrl+Alt does not start an ask.
            let c_up = state.process_event_at(&make_key_event(CTRL_ALT, Some(Key::C), false), at(t0, 1100));
            assert_eq!(states(&c_up), vec![(call, HotkeyState::Released)]);
            assert!(state.tick(at(t0, 2000)).is_empty());
            assert!(!state.pressed_hotkeys.contains(&ask));
        }

        #[test]
        fn a_quick_tap_inside_the_grace_still_fires() {
            let (mut state, [_, _, ask, _]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT),
                t0,
            );
            state.process_event_at(
                &make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT),
                at(t0, 10),
            );
            let up = state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                at(t0, 80),
            );
            assert_eq!(
                states(&up),
                vec![(ask, HotkeyState::Pressed), (ask, HotkeyState::Released)]
            );
        }

        #[test]
        fn another_apps_shortcut_typed_quickly_withdraws_the_hotkey() {
            // Ctrl+Win is not extended by anything with cleanup off, so it
            // fires at once; Ctrl+Win+Right (switch desktop) then withdraws it.
            let mut state = ManagerState::new();
            let dictate = HotkeyId(0);
            state.hotkeys.insert(dictate, "ctrl_left+super".parse().unwrap());
            let t0 = Instant::now();
            state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT),
                t0,
            );
            let pressed = state.process_event_at(
                &make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT),
                at(t0, 10),
            );
            assert_eq!(states(&pressed), vec![(dictate, HotkeyState::Pressed)]);
            let arrow = state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::RightArrow), true),
                at(t0, 150),
            );
            assert_eq!(states(&arrow), vec![(dictate, HotkeyState::Cancelled)]);
            // No release follows the cancellation.
            let up = state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::CMD_LEFT),
                at(t0, 300),
            );
            assert!(up.is_empty());
        }

        #[test]
        fn a_stray_key_long_into_a_hold_does_not_end_it() {
            let mut state = ManagerState::new();
            let dictate = HotkeyId(0);
            state.hotkeys.insert(dictate, "ctrl_left+super".parse().unwrap());
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let late = at(t0, 0) + CHORD_CANCEL_WINDOW + Duration::from_millis(1);
            assert!(state
                .process_event_at(&make_key_event(CTRL_WIN, Some(Key::A), true), late)
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
        }

        #[test]
        fn a_click_during_a_hold_is_not_a_chord() {
            let mut state = ManagerState::new();
            let dictate = HotkeyId(0);
            state.hotkeys.insert(dictate, "ctrl_left+super".parse().unwrap());
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            assert!(state
                .process_event_at(&make_key_event(CTRL_WIN, Some(Key::MouseLeft), true), at(t0, 50))
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
        }

        #[test]
        fn adding_shift_inside_the_grace_picks_cleanup_over_dictation() {
            let (mut state, [dictate, cleanup, _, _]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT),
                t0,
            );
            // Dictation is extended by cleanup, so it waits.
            assert!(state
                .process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), at(t0, 10))
                .is_empty());
            let shift = state.process_event_at(
                &make_modifier_event(CTRL_WIN | Modifiers::SHIFT_LEFT, true, Modifiers::SHIFT_LEFT),
                at(t0, 60),
            );
            assert_eq!(states(&shift), vec![(cleanup, HotkeyState::Pressed)]);
            assert!(state.tick(at(t0, 1000)).is_empty());
            assert!(!state.pressed_hotkeys.contains(&dictate));
        }

        #[test]
        fn adding_shift_just_after_dictation_fired_switches_to_cleanup() {
            let (mut state, [dictate, cleanup, _, _]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            assert_eq!(
                states(&state.tick(at(t0, 0) + CHORD_GRACE)),
                vec![(dictate, HotkeyState::Pressed)]
            );
            let shift = state.process_event_at(
                &make_modifier_event(CTRL_WIN | Modifiers::SHIFT_LEFT, true, Modifiers::SHIFT_LEFT),
                at(t0, 300),
            );
            assert_eq!(
                states(&shift),
                vec![
                    (dictate, HotkeyState::Cancelled),
                    (cleanup, HotkeyState::Pressed)
                ]
            );
        }

        #[test]
        fn shift_pressed_long_into_a_dictation_starts_nothing_new() {
            let (mut state, [dictate, cleanup, _, _]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            state.tick(at(t0, 0) + CHORD_GRACE);
            let late = at(t0, 0) + CHORD_GRACE + CHORD_CANCEL_WINDOW;
            assert!(state
                .process_event_at(
                    &make_modifier_event(CTRL_WIN | Modifiers::SHIFT_LEFT, true, Modifiers::SHIFT_LEFT),
                    late,
                )
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
            assert!(!state.pressed_hotkeys.contains(&cleanup));
        }

        #[test]
        fn a_chord_stays_quiet_until_its_modifiers_are_let_go() {
            let (mut state, [_, _, ask, _]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            // Ctrl+Alt+L (reformat in JetBrains): no ask, then or later.
            state.process_event_at(&make_key_event(CTRL_ALT, Some(Key::L), true), at(t0, 50));
            state.process_event_at(&make_key_event(CTRL_ALT, Some(Key::L), false), at(t0, 120));
            assert!(state.tick(at(t0, 2000)).is_empty());
            // Let go of Alt and press it again: a fresh ask.
            state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                at(t0, 2100),
            );
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), at(t0, 2200));
            assert_eq!(
                states(&state.tick(at(t0, 2200) + CHORD_GRACE)),
                vec![(ask, HotkeyState::Pressed)]
            );
        }

        #[test]
        fn altgr_never_reaches_a_left_side_hotkey() {
            // AltGr reports as Left Ctrl + Right Alt.
            let (mut state, _) = windows_defaults();
            let t0 = Instant::now();
            let altgr = Modifiers::CTRL_LEFT | Modifiers::OPT_RIGHT;
            assert!(state
                .process_event_at(&make_modifier_event(altgr, true, Modifiers::OPT_RIGHT), t0)
                .is_empty());
            assert!(state.pending.is_empty());
            assert!(state
                .process_event_at(&make_key_event(altgr, Some(Key::C), true), at(t0, 50))
                .is_empty());
        }

        #[test]
        fn unregistering_drops_a_held_back_press() {
            let (mut state, [_, _, ask, _]) = windows_defaults();
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            state.hotkeys.remove(&ask);
            state.forget(ask);
            assert!(state.tick(at(t0, 1000)).is_empty());
        }

        #[test]
        fn extends_compares_sides_and_groups() {
            let ask: Hotkey = "ctrl_left+alt_left".parse().unwrap();
            let call: Hotkey = "ctrl_left+alt_left+c".parse().unwrap();
            let compound_call: Hotkey = "ctrl+alt+c".parse().unwrap();
            let right_call: Hotkey = "ctrl_right+alt_right+c".parse().unwrap();
            let dictate: Hotkey = "fn".parse().unwrap();
            let mac_ask: Hotkey = "fn+ctrl".parse().unwrap();
            assert!(extends(&call, &ask));
            assert!(extends(&compound_call, &ask));
            assert!(!extends(&right_call, &ask));
            assert!(!extends(&ask, &call));
            assert!(!extends(&ask, &ask));
            assert!(extends(&mac_ask, &dictate));
            assert!(!extends(&dictate, &mac_ask));
        }

        // ---- Port of upstream handy-keys #23 ------------------------------

        #[test]
        fn modifier_only_hotkey_not_released_by_unrelated_modifier() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, None).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Cmd down — hotkey pressed
            let event = make_modifier_event(Modifiers::CMD_LEFT, true, Modifiers::CMD_LEFT);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Pressed);

            // Shift down while Cmd held — no state change
            let event = make_modifier_event(
                Modifiers::CMD_LEFT | Modifiers::SHIFT_LEFT,
                true,
                Modifiers::SHIFT_LEFT,
            );
            assert_eq!(state.process_event(&event).len(), 0);

            // Shift up — Cmd is still held and still matches, so the hotkey
            // must NOT be released
            let event = make_modifier_event(Modifiers::CMD_LEFT, false, Modifiers::SHIFT_LEFT);
            assert_eq!(state.process_event(&event).len(), 0);
            assert!(state.pressed_hotkeys.contains(&id));

            // Cmd up — now it releases
            let event = make_modifier_event(Modifiers::empty(), false, Modifiers::CMD_LEFT);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Released);
            assert!(!state.pressed_hotkeys.contains(&id));
        }

        #[test]
        fn windows_default_ctrl_win_survives_an_unrelated_modifier_tap() {
            // The SpeakoFlow Windows dictation default is ctrl_left+super.
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CTRL_LEFT | Modifiers::CMD_LEFT, None).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            let held = Modifiers::CTRL_LEFT | Modifiers::CMD_LEFT;
            state.process_event(&make_modifier_event(
                Modifiers::CTRL_LEFT,
                true,
                Modifiers::CTRL_LEFT,
            ));
            let pressed =
                state.process_event(&make_modifier_event(held, true, Modifiers::CMD_LEFT));
            assert_eq!(pressed.len(), 1);

            // Alt tapped and released while still holding Ctrl+Win.
            state.process_event(&make_modifier_event(
                held | Modifiers::OPT_LEFT,
                true,
                Modifiers::OPT_LEFT,
            ));
            assert!(state
                .process_event(&make_modifier_event(held, false, Modifiers::OPT_LEFT))
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&id));
        }

        #[test]
        fn modifier_only_hotkey_releases_on_own_modifier_release() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, None).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            let event = make_modifier_event(Modifiers::CMD_LEFT, true, Modifiers::CMD_LEFT);
            state.process_event(&event);
            assert!(state.pressed_hotkeys.contains(&id));

            let event = make_modifier_event(Modifiers::empty(), false, Modifiers::CMD_LEFT);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Released);
        }

        #[test]
        fn compound_modifier_only_hotkey_releases_on_partial_release() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD | Modifiers::SHIFT, None).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            let event = make_modifier_event(Modifiers::CMD_LEFT, true, Modifiers::CMD_LEFT);
            assert_eq!(state.process_event(&event).len(), 0);
            let event = make_modifier_event(
                Modifiers::CMD_LEFT | Modifiers::SHIFT_LEFT,
                true,
                Modifiers::SHIFT_LEFT,
            );
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Pressed);

            let event = make_modifier_event(Modifiers::SHIFT_LEFT, false, Modifiers::CMD_LEFT);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Released);
        }

        #[test]
        fn keyed_hotkey_not_released_by_unrelated_key_release() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), true);
            state.process_event(&event);
            assert!(state.pressed_hotkeys.contains(&id));

            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::J), false);
            assert_eq!(state.process_event(&event).len(), 0);
            assert!(state.pressed_hotkeys.contains(&id));
        }

        #[test]
        fn register_and_lookup_hotkey() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();

            let id = HotkeyId(state.next_id);
            state.next_id += 1;
            state.hotkeys.insert(id, hotkey);

            assert_eq!(state.hotkeys.get(&id), Some(&hotkey));
            assert_eq!(state.hotkeys.len(), 1);
        }

        #[test]
        fn hotkey_press_generates_event() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Simulate Cmd+K key down (event uses side-specific modifier)
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), true);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, id);
            assert_eq!(results[0].state, HotkeyState::Pressed);
            assert!(state.pressed_hotkeys.contains(&id));
        }

        #[test]
        fn hotkey_release_generates_event() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Press first
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), true);
            state.process_event(&event);

            // Then release the key
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), false);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, id);
            assert_eq!(results[0].state, HotkeyState::Released);
            assert!(!state.pressed_hotkeys.contains(&id));
        }

        #[test]
        fn no_duplicate_press_events() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Press once
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), true);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);

            // Press again (key repeat) - should not generate another event
            let results = state.process_event(&event);
            assert_eq!(results.len(), 0);
        }

        #[test]
        fn modifier_release_triggers_hotkey_release() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Press Cmd+K
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), true);
            state.process_event(&event);
            assert!(state.pressed_hotkeys.contains(&id));

            // Release Cmd (while K is still held) - modifier event
            let event = make_modifier_event(Modifiers::empty(), false, Modifiers::CMD_LEFT);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Released);
            assert!(!state.pressed_hotkeys.contains(&id));
        }

        #[test]
        fn wrong_modifiers_dont_trigger() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            state.hotkeys.insert(HotkeyId(0), hotkey);

            // Press Shift+K instead of Cmd+K
            let event = make_key_event(Modifiers::SHIFT_LEFT, Some(Key::K), true);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 0);
        }

        #[test]
        fn modifier_only_hotkey() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD | Modifiers::SHIFT, None).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Press Cmd+Shift (no key) — events use side-specific modifiers
            let event = make_modifier_event(
                Modifiers::CMD_LEFT | Modifiers::SHIFT_LEFT,
                true,
                Modifiers::SHIFT_LEFT,
            );
            let results = state.process_event(&event);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Pressed);
        }

        #[test]
        fn multiple_hotkeys_same_key() {
            let mut state = ManagerState::new();

            // Cmd+K and Ctrl+K
            let hotkey1 = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            let hotkey2 = Hotkey::new(Modifiers::CTRL, Key::K).unwrap();
            let id1 = HotkeyId(0);
            let id2 = HotkeyId(1);
            state.hotkeys.insert(id1, hotkey1);
            state.hotkeys.insert(id2, hotkey2);

            // Press Cmd+K
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), true);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, id1);

            // Press Ctrl+K (release Cmd first)
            state.pressed_hotkeys.clear();
            let event = make_key_event(Modifiers::CTRL_LEFT, Some(Key::K), true);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, id2);
        }

        #[test]
        fn key_only_hotkey() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::empty(), Key::F1).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Press F1 with no modifiers
            let event = make_key_event(Modifiers::empty(), Some(Key::F1), true);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Pressed);

            // F1 with modifiers should NOT trigger
            state.pressed_hotkeys.clear();
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::F1), true);
            let results = state.process_event(&event);

            assert_eq!(results.len(), 0);
        }

        #[test]
        fn side_specific_hotkey_matches_correct_side() {
            let mut state = ManagerState::new();
            // Register CtrlRight+Space
            let hotkey = Hotkey::new(Modifiers::CTRL_RIGHT, Key::Space).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Left ctrl should not trigger
            let event = make_key_event(Modifiers::CTRL_LEFT, Some(Key::Space), true);
            assert_eq!(state.process_event(&event).len(), 0);

            // Right ctrl should trigger
            let event = make_key_event(Modifiers::CTRL_RIGHT, Some(Key::Space), true);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].state, HotkeyState::Pressed);
        }

        #[test]
        fn compound_hotkey_matches_either_side() {
            let mut state = ManagerState::new();
            let hotkey = Hotkey::new(Modifiers::CMD, Key::K).unwrap();
            let id = HotkeyId(0);
            state.hotkeys.insert(id, hotkey);

            // Left Cmd triggers
            let event = make_key_event(Modifiers::CMD_LEFT, Some(Key::K), true);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);

            // Release
            state.pressed_hotkeys.clear();

            // Right Cmd also triggers
            let event = make_key_event(Modifiers::CMD_RIGHT, Some(Key::K), true);
            let results = state.process_event(&event);
            assert_eq!(results.len(), 1);
        }
    }
}
