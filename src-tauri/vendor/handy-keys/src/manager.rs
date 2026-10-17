//! Platform-agnostic hotkey manager built on top of KeyboardListener

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::listener::{BlockingHotkeys, KeyboardListener};
use crate::types::{Hotkey, HotkeyEvent, HotkeyId, HotkeyState, Key, KeyEvent, Modifiers};

/// After a modifier-only hotkey has fired on press, a key typed on top of it
/// within this window means the modifiers were the start of some other
/// shortcut — Ctrl+Alt+C for the call while the ask is Ctrl+Alt, VS Code's
/// `Ctrl+Alt+↑`, Windows' `Ctrl+Win+→`, a German layout's `Ctrl+Alt+Q` for
/// `@`. The hotkey is then withdrawn with [`HotkeyState::Cancelled`] instead of
/// being left running behind a shortcut the user meant for someone else. Past
/// the window the hotkey is plainly being held on purpose and a stray key no
/// longer ends it. Only used while hotkeys fire on press; see
/// [`HotkeyManager::set_fire_on_release`].
pub const CHORD_CANCEL_WINDOW: Duration = Duration::from_millis(400);

/// A keyboard key, as opposed to a mouse button. Clicking while holding a
/// modifier-only hotkey to talk is ordinary use, not the start of another
/// shortcut.
fn is_keyboard_key(key: Key) -> bool {
    !key.is_mouse_button()
}

/// Internal state shared between the manager and the processing thread
struct ManagerState {
    hotkeys: HashMap<HotkeyId, Hotkey>,
    next_id: u32,
    /// Track which hotkeys are currently pressed
    pressed_hotkeys: HashSet<HotkeyId>,
    /// When each pressed modifier-only hotkey fired, for [`CHORD_CANCEL_WINDOW`].
    pressed_at: HashMap<HotkeyId, Instant>,
    /// Modifier-only hotkeys fire when their modifiers are let go, as a press
    /// and a release together, instead of when they go down. The default for
    /// every hotkey without a setting of its own in `release_overrides`.
    fire_on_release: bool,
    /// Hotkeys that fire on release, or on press, whatever the default: a host
    /// where some shortcuts are held and others tapped (dictation held to talk
    /// while the ask is tapped). See [`HotkeyManager::set_fire_on_release_for`].
    release_overrides: HashMap<HotkeyId, bool>,
    /// With `fire_on_release`: modifier-only hotkeys whose modifiers are held
    /// right now. Letting go fires one; pressing any other key first means it
    /// was the start of a longer shortcut, and it never fires.
    pending: HashSet<HotkeyId>,
    /// Modifier-only hotkeys that turned out to be part of a chord. They stay
    /// quiet until their modifiers stop matching, so letting go of the chord's
    /// last key while still holding the modifiers does not fire them.
    suppressed: HashSet<HotkeyId>,
    /// Hotkeys that carry on a held modifier-only hotkey instead of replacing
    /// it (see [`HotkeyManager::register_continuation`]).
    continuations: HashSet<HotkeyId>,
}

impl ManagerState {
    fn new() -> Self {
        Self {
            hotkeys: HashMap::new(),
            next_id: 0,
            pressed_hotkeys: HashSet::new(),
            pressed_at: HashMap::new(),
            fire_on_release: false,
            release_overrides: HashMap::new(),
            pending: HashSet::new(),
            suppressed: HashSet::new(),
            continuations: HashSet::new(),
        }
    }

    /// Forget everything about a hotkey that is being unregistered.
    fn forget(&mut self, id: HotkeyId) {
        self.pressed_hotkeys.remove(&id);
        self.pressed_at.remove(&id);
        self.pending.remove(&id);
        self.suppressed.remove(&id);
        self.continuations.remove(&id);
        self.release_overrides.remove(&id);
    }

    /// Whether `key` going down with `modifiers` held completes a continuation.
    fn completes_continuation(&self, key: Key, modifiers: Modifiers) -> bool {
        self.continuations.iter().any(|id| {
            self.hotkeys
                .get(id)
                .is_some_and(|h| h.key == Some(key) && h.modifiers.matches(modifiers))
        })
    }

    /// Whether hotkey `id` fires once its modifiers are let go.
    fn fires_on_release(&self, id: HotkeyId) -> bool {
        self.release_overrides
            .get(&id)
            .copied()
            .unwrap_or(self.fire_on_release)
    }

    fn set_fire_on_release(&mut self, on: bool) {
        self.fire_on_release = on;
        self.pending.clear();
    }

    fn set_fire_on_release_for(&mut self, id: HotkeyId, on: bool) {
        self.release_overrides.insert(id, on);
        self.pending.remove(&id);
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

    fn within_cancel_window(&self, id: HotkeyId, now: Instant) -> bool {
        self.pressed_at
            .get(&id)
            .is_some_and(|&at| now.saturating_duration_since(at) < CHORD_CANCEL_WINDOW)
    }

    /// Whether a mouse button, with these modifiers held, is itself a
    /// registered hotkey.
    fn is_bound_button(&self, key: Key, modifiers: Modifiers) -> bool {
        self.hotkeys
            .values()
            .any(|h| h.key == Some(key) && h.modifiers.matches(modifiers))
    }

    /// Process a key event and return any matching hotkey events
    fn process_event(&mut self, event: &KeyEvent) -> Vec<HotkeyEvent> {
        self.process_event_at(event, Instant::now())
    }

    /// Process a key event observed at `now`.
    fn process_event_at(&mut self, event: &KeyEvent, now: Instant) -> Vec<HotkeyEvent> {
        let mut results = Vec::new();

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
                    // A click counts as "something else" only for a hotkey
                    // waiting for its release. While a hotkey is being held to
                    // talk it is ordinary use of the mouse, unless the button
                    // is a hotkey of its own: Ctrl+Alt+Back registered while
                    // Ctrl+Alt asks is the rest of a longer shortcut, exactly
                    // as a typed C is for Ctrl+Alt+C, so it counts as a key.
                    let keyboard =
                        is_keyboard_key(key) || self.is_bound_button(key, event.modifiers);
                    if keyboard || !self.pending.is_empty() {
                        let continues = self.completes_continuation(key, event.modifiers);
                        self.yield_to_chord(now, continues, keyboard, &mut results);
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
            // A waiting hotkey whose modifiers were let go with nothing else
            // pressed in between: that was the hotkey, tapped.
            if event.key.is_none() {
                let tapped: Vec<HotkeyId> = self
                    .pending
                    .iter()
                    .copied()
                    .filter(|id| {
                        !self
                            .hotkeys
                            .get(id)
                            .is_some_and(|h| h.modifiers.matches(event.modifiers))
                    })
                    .collect();
                for id in tapped {
                    self.pending.remove(&id);
                    results.push(HotkeyEvent {
                        id,
                        state: HotkeyState::Pressed,
                    });
                    results.push(HotkeyEvent {
                        id,
                        state: HotkeyState::Released,
                    });
                }
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

    /// Another key went down. Whatever modifier-only hotkey the held modifiers
    /// were waiting to fire, or fired a moment ago, was the start of another
    /// shortcut instead. A click (not a `keyboard` key) only means that for a
    /// hotkey still waiting: one held to talk is not ended by using the mouse.
    ///
    /// Unless the key `continues` it: then the one already running keeps
    /// running. A tap still waiting to fire never does, though, because the
    /// continuation has taken its place, and letting go of the modifiers
    /// afterwards must not fire the hotkey as well.
    fn yield_to_chord(
        &mut self,
        now: Instant,
        continues: bool,
        keyboard: bool,
        results: &mut Vec<HotkeyEvent>,
    ) {
        let waiting: Vec<HotkeyId> = self.pending.drain().collect();
        self.suppressed.extend(waiting);
        if continues || !keyboard {
            return;
        }

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

        // Whatever was waiting no longer matches the modifiers now held
        // (Ctrl+Win on the way to Ctrl+Win+Shift).
        self.pending.retain(|id| matched.contains(id));
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

        // Each one that is held fires now; each one that is tapped waits for
        // its keys to be let go.
        for id in matched {
            if self.fires_on_release(id) {
                self.pending.insert(id);
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
                || {
                    // Windows shutdown closes the native sender, waking recv
                    // without a timer. Other platforms retain their stop poll.
                    #[cfg(target_os = "windows")]
                    {
                        listener.recv()
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        listener.recv_timeout(Duration::from_millis(100))
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

    /// Event processing loop
    fn event_loop(
        next_event: impl Fn() -> Result<KeyEvent>,
        state: Arc<Mutex<ManagerState>>,
        deliver: Box<dyn Fn(HotkeyEvent) -> bool + Send>,
        running: Arc<std::sync::atomic::AtomicBool>,
    ) {
        while running.load(std::sync::atomic::Ordering::SeqCst) {
            let hotkey_events = match next_event() {
                Ok(key_event) => match state.lock() {
                    Ok(mut state) => state.process_event(&key_event),
                    Err(_) => continue,
                },
                Err(crate::error::Error::Timeout) => {
                    // No event received, loop continues to check running flag
                    continue;
                }
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
        self.register_with(hotkey, false)
    }

    /// Register a hotkey that carries on the modifier-only hotkey it extends,
    /// instead of taking over from it.
    ///
    /// Normally a key typed onto a modifier-only hotkey that fired a moment ago
    /// withdraws it ([`HotkeyState::Cancelled`]): Ctrl+Alt was only the start
    /// of Ctrl+Alt+C. A continuation is the opposite case, Ctrl+Win+Space while
    /// Ctrl+Win is being held to dictate: Space changes how the running hotkey
    /// behaves, so that hotkey keeps its press and the continuation fires
    /// beside it, however quickly the key follows. With
    /// [`HotkeyManager::set_fire_on_release`] on, a waiting tap of the shorter
    /// hotkey is still dropped, since the continuation replaced it.
    pub fn register_continuation(&self, hotkey: Hotkey) -> Result<HotkeyId> {
        self.register_with(hotkey, true)
    }

    fn register_with(&self, hotkey: Hotkey, continues: bool) -> Result<HotkeyId> {
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
        if continues {
            state.continuations.insert(id);
        }

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

    /// Fire modifier-only hotkeys when their modifiers are let go instead of
    /// when they go down.
    ///
    /// For a host where these hotkeys are taps rather than holds. A press that
    /// fires on the way down cannot know what comes next, so a tap of Ctrl+Alt
    /// started the ask before the C of Ctrl+Alt+C arrived, and the ask's panel
    /// flashed up on the way to every call. Fired on release, nothing happens
    /// until the user lets go, and by then it is known: modifiers alone were
    /// the tap; another key in between was another shortcut, and this one
    /// never fires. Off (the default) suits push-to-talk, where the press has
    /// to start the recording at once and a quick chord withdraws it instead
    /// ([`HotkeyState::Cancelled`]).
    pub fn set_fire_on_release(&self, on: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.set_fire_on_release(on);
        }
    }

    /// [`Self::set_fire_on_release`] for one hotkey, for a host whose
    /// shortcuts are not all held or all tapped: dictation held to talk while
    /// the ask is tapped. The hotkey keeps this whatever the default becomes,
    /// until it is unregistered.
    pub fn set_fire_on_release_for(&self, id: HotkeyId, on: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.set_fire_on_release_for(id, on);
        }
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
            || key_rx.recv().map_err(|_| Error::EventLoopNotRunning),
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
            || {
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
        fn windows_defaults(fire_on_release: bool) -> (ManagerState, [HotkeyId; 4]) {
            let mut state = ManagerState::new();
            state.set_fire_on_release(fire_on_release);
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

        fn ctrl_down() -> KeyEvent {
            make_modifier_event(Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT)
        }

        // Tap to toggle: modifier-only hotkeys fire when let go.

        #[test]
        fn tapped_ask_keys_fire_once_let_go() {
            let (mut state, [_, _, ask, _]) = windows_defaults(true);
            let t0 = Instant::now();
            assert!(state.process_event_at(&ctrl_down(), t0).is_empty());
            let down = make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT);
            assert!(state.process_event_at(&down, at(t0, 10)).is_empty());
            let up = state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                at(t0, 900),
            );
            assert_eq!(
                states(&up),
                vec![(ask, HotkeyState::Pressed), (ask, HotkeyState::Released)]
            );
        }

        #[test]
        fn the_call_never_opens_the_ask_on_the_way() {
            let (mut state, [_, _, ask, call]) = windows_defaults(true);
            let t0 = Instant::now();
            state.process_event_at(&ctrl_down(), t0);
            state.process_event_at(
                &make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT),
                at(t0, 20),
            );
            // However slowly the C follows.
            let c = state.process_event_at(
                &make_key_event(CTRL_ALT, Some(Key::C), true),
                at(t0, 3000),
            );
            assert_eq!(states(&c), vec![(call, HotkeyState::Pressed)]);
            let c_up = state.process_event_at(
                &make_key_event(CTRL_ALT, Some(Key::C), false),
                at(t0, 3100),
            );
            assert_eq!(states(&c_up), vec![(call, HotkeyState::Released)]);
            // Letting go of the modifiers afterwards fires no ask.
            let alt_up = state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                at(t0, 3200),
            );
            assert!(alt_up.is_empty());
            assert!(!state.pressed_hotkeys.contains(&ask));
        }

        #[test]
        fn another_apps_ctrl_alt_shortcut_fires_nothing_when_tapping() {
            let (mut state, _) = windows_defaults(true);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            // Ctrl+Alt+L (reformat in JetBrains).
            assert!(state
                .process_event_at(&make_key_event(CTRL_ALT, Some(Key::L), true), at(t0, 50))
                .is_empty());
            state.process_event_at(&make_key_event(CTRL_ALT, Some(Key::L), false), at(t0, 120));
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::empty(), false, Modifiers::CTRL_LEFT),
                    at(t0, 200),
                )
                .is_empty());
        }

        #[test]
        fn a_click_while_waiting_means_ctrl_click_not_the_hotkey() {
            let (mut state, _) = windows_defaults(true);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::MouseLeft), true),
                at(t0, 50),
            );
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::CMD_LEFT),
                    at(t0, 100),
                )
                .is_empty());
        }

        #[test]
        fn adding_shift_while_tapping_picks_cleanup_over_dictation() {
            let (mut state, [dictate, cleanup, _, _]) = windows_defaults(true);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let shift = CTRL_WIN | Modifiers::SHIFT_LEFT;
            state.process_event_at(
                &make_modifier_event(shift, true, Modifiers::SHIFT_LEFT),
                at(t0, 60),
            );
            // Let go of Shift first: still nothing, the chord is Ctrl+Win+Shift.
            let first = state.process_event_at(
                &make_modifier_event(CTRL_WIN, false, Modifiers::SHIFT_LEFT),
                at(t0, 200),
            );
            assert_eq!(
                states(&first),
                vec![(cleanup, HotkeyState::Pressed), (cleanup, HotkeyState::Released)]
            );
            assert!(!state.pending.contains(&dictate));
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::CMD_LEFT),
                    at(t0, 250),
                )
                .is_empty());
        }

        // Hold to talk: modifier-only hotkeys fire at once.

        #[test]
        fn held_ask_keys_fire_at_once_and_release_on_let_go() {
            let (mut state, [_, _, ask, _]) = windows_defaults(false);
            let t0 = Instant::now();
            state.process_event_at(&ctrl_down(), t0);
            let down = state.process_event_at(
                &make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT),
                at(t0, 10),
            );
            assert_eq!(states(&down), vec![(ask, HotkeyState::Pressed)]);
            let up = state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                at(t0, 2000),
            );
            assert_eq!(states(&up), vec![(ask, HotkeyState::Released)]);
        }

        #[test]
        fn holding_ask_keys_then_c_withdraws_the_ask_for_the_call() {
            let (mut state, [_, _, ask, call]) = windows_defaults(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            let c = state.process_event_at(
                &make_key_event(CTRL_ALT, Some(Key::C), true),
                at(t0, 90),
            );
            assert_eq!(
                states(&c),
                vec![(ask, HotkeyState::Cancelled), (call, HotkeyState::Pressed)]
            );
            state.process_event_at(&make_key_event(CTRL_ALT, Some(Key::C), false), at(t0, 150));
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                    at(t0, 300),
                )
                .is_empty());
        }

        #[test]
        fn another_apps_shortcut_typed_quickly_withdraws_a_held_hotkey() {
            let (mut state, [dictate, _, _, _]) = windows_defaults(false);
            let t0 = Instant::now();
            let pressed = state.process_event_at(
                &make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT),
                t0,
            );
            assert_eq!(states(&pressed), vec![(dictate, HotkeyState::Pressed)]);
            let arrow = state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::RightArrow), true),
                at(t0, 150),
            );
            assert_eq!(states(&arrow), vec![(dictate, HotkeyState::Cancelled)]);
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::CMD_LEFT),
                    at(t0, 300),
                )
                .is_empty());
        }

        #[test]
        fn a_stray_key_long_into_a_hold_does_not_end_it() {
            let (mut state, [dictate, _, _, _]) = windows_defaults(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let late = t0 + CHORD_CANCEL_WINDOW + Duration::from_millis(1);
            assert!(state
                .process_event_at(&make_key_event(CTRL_WIN, Some(Key::A), true), late)
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
        }

        #[test]
        fn a_click_during_a_hold_is_not_a_chord() {
            let (mut state, [dictate, _, _, _]) = windows_defaults(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            assert!(state
                .process_event_at(
                    &make_key_event(CTRL_WIN, Some(Key::MouseLeft), true),
                    at(t0, 50),
                )
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
        }

        /// A mouse button registered as a hotkey of its own (another
        /// shortcut's other keys) is a chord like any typed key: Ctrl+Alt
        /// fires the ask, Back on top within the window withdraws it for the
        /// longer one. A button nobody registered stays ordinary use.
        #[test]
        fn a_bound_button_during_a_hold_is_a_chord() {
            let (mut state, [_, _, ask, _]) = windows_defaults(false);
            let back = HotkeyId(4);
            state
                .hotkeys
                .insert(back, "ctrl_left+alt_left+mousex1".parse().unwrap());
            let t0 = Instant::now();
            state.process_event_at(
                &make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT),
                t0,
            );
            let click = state.process_event_at(
                &make_key_event(CTRL_ALT, Some(Key::MouseX1), true),
                at(t0, 120),
            );
            assert_eq!(
                states(&click),
                vec![(ask, HotkeyState::Cancelled), (back, HotkeyState::Pressed)]
            );
            let release = state.process_event_at(
                &make_key_event(CTRL_ALT, Some(Key::MouseX1), false),
                at(t0, 400),
            );
            assert_eq!(states(&release), vec![(back, HotkeyState::Released)]);

            // The forward button is nobody's: holding the ask, it is a click.
            let (mut state, [_, _, ask, _]) = windows_defaults(false);
            state.process_event_at(
                &make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT),
                t0,
            );
            assert!(state
                .process_event_at(
                    &make_key_event(CTRL_ALT, Some(Key::MouseX2), true),
                    at(t0, 120),
                )
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&ask));
        }

        /// A button on its own, bound with no modifiers, is pressed and
        /// released like a key.
        #[test]
        fn a_bound_button_alone_is_pressed_and_released() {
            let (mut state, _) = windows_defaults(false);
            let back = HotkeyId(4);
            state.hotkeys.insert(back, "mousex1".parse().unwrap());
            let t0 = Instant::now();
            let down = state.process_event_at(
                &make_key_event(Modifiers::empty(), Some(Key::MouseX1), true),
                t0,
            );
            assert_eq!(states(&down), vec![(back, HotkeyState::Pressed)]);
            let up = state.process_event_at(
                &make_key_event(Modifiers::empty(), Some(Key::MouseX1), false),
                at(t0, 2000),
            );
            assert_eq!(states(&up), vec![(back, HotkeyState::Released)]);
        }

        #[test]
        fn adding_shift_just_after_dictation_fired_switches_to_cleanup() {
            let (mut state, [dictate, cleanup, _, _]) = windows_defaults(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let shift = state.process_event_at(
                &make_modifier_event(CTRL_WIN | Modifiers::SHIFT_LEFT, true, Modifiers::SHIFT_LEFT),
                at(t0, 100),
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
            let (mut state, [dictate, cleanup, _, _]) = windows_defaults(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let late = t0 + CHORD_CANCEL_WINDOW;
            assert!(state
                .process_event_at(
                    &make_modifier_event(CTRL_WIN | Modifiers::SHIFT_LEFT, true, Modifiers::SHIFT_LEFT),
                    late,
                )
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
            assert!(!state.pressed_hotkeys.contains(&cleanup));
        }

        // Continuations: a key that carries on the held hotkey.

        /// The Windows defaults plus their hands-free keys: dictation's
        /// Ctrl+Win+Space and cleanup's Ctrl+Win+Shift+Space.
        fn with_hands_free(fire_on_release: bool) -> (ManagerState, [HotkeyId; 4], [HotkeyId; 2]) {
            let (mut state, ids) = windows_defaults(fire_on_release);
            let continuations = [HotkeyId(4), HotkeyId(5)];
            for (id, hotkey) in continuations
                .iter()
                .zip(["ctrl_left+super+space", "ctrl_left+super+shift+space"])
            {
                state.hotkeys.insert(*id, hotkey.parse().unwrap());
                state.continuations.insert(*id);
            }
            (state, ids, continuations)
        }

        #[test]
        fn space_right_after_dictation_fired_keeps_dictation_running() {
            let (mut state, [dictate, ..], [hands_free, _]) = with_hands_free(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            // Well inside the window in which any other key withdraws it.
            let space = state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::Space), true),
                at(t0, 80),
            );
            assert_eq!(states(&space), vec![(hands_free, HotkeyState::Pressed)]);
            assert!(state.pressed_hotkeys.contains(&dictate));
            // Letting go releases each once; nothing is withdrawn.
            let space_up = state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::Space), false),
                at(t0, 200),
            );
            assert_eq!(states(&space_up), vec![(hands_free, HotkeyState::Released)]);
            let ctrl_up = state.process_event_at(
                &make_modifier_event(Modifiers::CMD_LEFT, false, Modifiers::CTRL_LEFT),
                at(t0, 260),
            );
            assert_eq!(states(&ctrl_up), vec![(dictate, HotkeyState::Released)]);
        }

        #[test]
        fn another_key_right_after_dictation_still_withdraws_it() {
            let (mut state, [dictate, ..], _) = with_hands_free(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let arrow = state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::RightArrow), true),
                at(t0, 80),
            );
            assert_eq!(states(&arrow), vec![(dictate, HotkeyState::Cancelled)]);
        }

        #[test]
        fn space_while_tapping_fires_the_continuation_and_no_tap() {
            let (mut state, _, [hands_free, _]) = with_hands_free(true);
            let t0 = Instant::now();
            assert!(state
                .process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0)
                .is_empty());
            let space = state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::Space), true),
                at(t0, 60),
            );
            assert_eq!(states(&space), vec![(hands_free, HotkeyState::Pressed)]);
            state.process_event_at(&make_key_event(CTRL_WIN, Some(Key::Space), false), at(t0, 120));
            // Letting go of Ctrl+Win is not also a tap of dictation, which would
            // stop the recording Space just started.
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::CMD_LEFT, false, Modifiers::CTRL_LEFT),
                    at(t0, 180),
                )
                .is_empty());
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::empty(), false, Modifiers::CMD_LEFT),
                    at(t0, 200),
                )
                .is_empty());
        }

        #[test]
        fn a_continuation_answers_only_to_its_own_modifiers() {
            let (mut state, [_, cleanup, ..], [_, cleanup_hands_free]) = with_hands_free(false);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let shift = CTRL_WIN | Modifiers::SHIFT_LEFT;
            state.process_event_at(&make_modifier_event(shift, true, Modifiers::SHIFT_LEFT), at(t0, 50));
            assert!(state.pressed_hotkeys.contains(&cleanup));
            let space = state.process_event_at(
                &make_key_event(shift, Some(Key::Space), true),
                at(t0, 120),
            );
            assert_eq!(states(&space), vec![(cleanup_hands_free, HotkeyState::Pressed)]);
            assert!(state.pressed_hotkeys.contains(&cleanup));
        }

        #[test]
        fn unregistering_a_continuation_forgets_it() {
            let (mut state, [dictate, ..], [hands_free, _]) = with_hands_free(false);
            state.hotkeys.remove(&hands_free);
            state.forget(hands_free);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t0);
            let space = state.process_event_at(
                &make_key_event(CTRL_WIN, Some(Key::Space), true),
                at(t0, 80),
            );
            assert_eq!(states(&space), vec![(dictate, HotkeyState::Cancelled)]);
        }

        // Both modes.

        #[test]
        fn altgr_never_reaches_a_left_side_hotkey() {
            // AltGr reports as Left Ctrl + Right Alt.
            for fire_on_release in [false, true] {
                let (mut state, _) = windows_defaults(fire_on_release);
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
        }

        #[test]
        fn unregistering_drops_a_waiting_tap() {
            let (mut state, [_, _, ask, _]) = windows_defaults(true);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            state.hotkeys.remove(&ask);
            state.forget(ask);
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::empty(), false, Modifiers::OPT_LEFT),
                    at(t0, 100),
                )
                .is_empty());
        }

        #[test]
        fn switching_modes_forgets_a_waiting_tap() {
            let (mut state, _) = windows_defaults(true);
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            state.set_fire_on_release(false);
            assert!(state.pending.is_empty());
        }

        // Each hotkey on its own: dictation held to talk while the ask is tapped.

        fn held_dictation_tapped_ask() -> (ManagerState, [HotkeyId; 4]) {
            let (mut state, ids) = windows_defaults(false);
            state.set_fire_on_release_for(ids[2], true);
            (state, ids)
        }

        #[test]
        fn a_held_dictation_fires_at_once_beside_a_tapped_ask() {
            let (mut state, [dictate, _, ask, _]) = held_dictation_tapped_ask();
            let t0 = Instant::now();
            let win = state.process_event_at(
                &make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT),
                t0,
            );
            assert_eq!(states(&win), vec![(dictate, HotkeyState::Pressed)]);
            let win_up = state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::CMD_LEFT),
                at(t0, 2000),
            );
            assert_eq!(states(&win_up), vec![(dictate, HotkeyState::Released)]);
            // The ask, on the same Ctrl, waits until its keys are let go.
            let alt = state.process_event_at(
                &make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT),
                at(t0, 2100),
            );
            assert!(alt.is_empty());
            let alt_up = state.process_event_at(
                &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                at(t0, 2300),
            );
            assert_eq!(
                states(&alt_up),
                vec![(ask, HotkeyState::Pressed), (ask, HotkeyState::Released)]
            );
        }

        #[test]
        fn a_tapped_ask_still_never_opens_on_the_way_to_the_call() {
            let (mut state, [_, _, ask, call]) = held_dictation_tapped_ask();
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            let c = state.process_event_at(
                &make_key_event(CTRL_ALT, Some(Key::C), true),
                at(t0, 3000),
            );
            assert_eq!(states(&c), vec![(call, HotkeyState::Pressed)]);
            state.process_event_at(&make_key_event(CTRL_ALT, Some(Key::C), false), at(t0, 3100));
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::empty(), false, Modifiers::OPT_LEFT),
                    at(t0, 3200),
                )
                .is_empty());
            assert!(!state.pressed_hotkeys.contains(&ask));
        }

        #[test]
        fn a_click_drops_a_waiting_tap_but_not_a_held_hotkey() {
            let (mut state, [dictate, _, _, _]) = held_dictation_tapped_ask();
            let t0 = Instant::now();
            // Ctrl+Alt+click is a click, not the ask.
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            state.process_event_at(
                &make_key_event(CTRL_ALT, Some(Key::MouseLeft), true),
                at(t0, 50),
            );
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::CTRL_LEFT, false, Modifiers::OPT_LEFT),
                    at(t0, 100),
                )
                .is_empty());
            // Clicking while holding Ctrl+Win to talk keeps the dictation.
            let t1 = at(t0, 1000);
            state.process_event_at(&make_modifier_event(CTRL_WIN, true, Modifiers::CMD_LEFT), t1);
            assert!(state
                .process_event_at(
                    &make_key_event(CTRL_WIN, Some(Key::MouseLeft), true),
                    at(t1, 50),
                )
                .is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
        }

        /// The Mac defaults: dictate Fn, ask Fn+Ctrl, so the ask starts with
        /// the dictation key.
        fn mac_defaults_held_dictation_tapped_ask() -> (ManagerState, HotkeyId, HotkeyId) {
            let mut state = ManagerState::new();
            let (dictate, ask) = (HotkeyId(0), HotkeyId(1));
            state.hotkeys.insert(dictate, "fn".parse().unwrap());
            state.hotkeys.insert(ask, "fn+ctrl".parse().unwrap());
            state.set_fire_on_release_for(ask, true);
            (state, dictate, ask)
        }

        #[test]
        fn a_tapped_ask_takes_over_from_the_held_dictation_it_starts_with() {
            let (mut state, dictate, ask) = mac_defaults_held_dictation_tapped_ask();
            let t0 = Instant::now();
            let fn_down = state.process_event_at(
                &make_modifier_event(Modifiers::FN, true, Modifiers::FN),
                t0,
            );
            assert_eq!(states(&fn_down), vec![(dictate, HotkeyState::Pressed)]);
            let ctrl = state.process_event_at(
                &make_modifier_event(Modifiers::FN | Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT),
                at(t0, 120),
            );
            assert_eq!(states(&ctrl), vec![(dictate, HotkeyState::Cancelled)]);
            let ctrl_up = state.process_event_at(
                &make_modifier_event(Modifiers::FN, false, Modifiers::CTRL_LEFT),
                at(t0, 400),
            );
            assert_eq!(
                states(&ctrl_up),
                vec![(ask, HotkeyState::Pressed), (ask, HotkeyState::Released)]
            );
        }

        #[test]
        fn ctrl_long_into_a_held_dictation_starts_no_tapped_ask() {
            let (mut state, dictate, _) = mac_defaults_held_dictation_tapped_ask();
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(Modifiers::FN, true, Modifiers::FN), t0);
            let late = t0 + CHORD_CANCEL_WINDOW;
            assert!(state
                .process_event_at(
                    &make_modifier_event(Modifiers::FN | Modifiers::CTRL_LEFT, true, Modifiers::CTRL_LEFT),
                    late,
                )
                .is_empty());
            assert!(state.pending.is_empty());
            assert!(state.pressed_hotkeys.contains(&dictate));
        }

        #[test]
        fn holding_one_hotkey_again_forgets_its_waiting_tap() {
            let (mut state, [_, _, ask, _]) = held_dictation_tapped_ask();
            let t0 = Instant::now();
            state.process_event_at(&make_modifier_event(CTRL_ALT, true, Modifiers::OPT_LEFT), t0);
            assert!(state.pending.contains(&ask));
            state.set_fire_on_release_for(ask, false);
            assert!(state.pending.is_empty());
        }

        #[test]
        fn unregistering_forgets_a_hotkeys_own_setting() {
            let (mut state, [_, _, ask, _]) = held_dictation_tapped_ask();
            state.forget(ask);
            assert!(!state.fires_on_release(ask));
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
