//! Tap-to-lock watcher.
//!
//! While a push-to-talk (hold) recording is active, a quick tap of the user's
//! **lock shortcut** (Shift by default, or empty to disable) converts it to
//! hands-free (locked) mode, so the user can let go of the hotkey and keep
//! talking. Pressing the hotkey again (or the overlay/panel "done" tick) then
//! finishes the recording. The lock shortcut is captured like any other
//! shortcut, so it can be whatever the user wants (or nothing).
//!
//! Detection uses a global, **non-blocking** [`handy_keys::KeyboardListener`]
//! running on a dedicated thread. Non-blocking means it only *observes* key
//! events without consuming them, so normal typing and the base hotkey keep
//! working and any active `HotkeyManager` still fires. It fires only while the
//! transcription coordinator has "armed" it for the current hold recording, and
//! only when the lock shortcut is freshly satisfied — extra modifiers from the
//! record shortcut you're holding are ignored, so you just add the lock key on
//! top.
//!
//! Privacy: this never logs or stores key contents — it compares only against
//! the single configured lock shortcut.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use handy_keys::{KeyboardListener, Modifiers};
use log::{debug, warn};
use tauri::{AppHandle, Manager};

use crate::TranscriptionCoordinator;

/// Number of times to retry creating the keyboard listener at startup. This
/// covers transient failures and the macOS case where accessibility permission
/// is granted slightly after launch.
const LISTENER_CREATE_ATTEMPTS: u32 = 5;
/// Delay between listener creation attempts.
const LISTENER_RETRY_DELAY: Duration = Duration::from_secs(2);

/// A parsed lock shortcut: the modifiers that must be held plus an optional
/// main (non-modifier) key. A "tap" fires when this combination becomes
/// satisfied while armed. Extra modifiers held for the record shortcut are
/// ignored — only the configured parts must be present.
#[derive(Clone, Debug)]
struct LockCombo {
    mods: Modifiers,
    key: Option<String>,
}

/// Map a modifier alias to its handy-keys flag, or `None` for non-modifiers.
fn modifier_bit(token: &str) -> Option<Modifiers> {
    match token {
        "shift" => Some(Modifiers::SHIFT),
        "ctrl" | "control" => Some(Modifiers::CTRL),
        "alt" | "option" | "opt" => Some(Modifiers::OPT),
        "super" | "cmd" | "command" | "meta" | "win" | "windows" => Some(Modifiers::CMD),
        "fn" => Some(Modifiers::FN),
        _ => None,
    }
}

/// Normalize a key name so a captured token and an observed key match despite
/// small naming differences (e.g. "escape" vs "esc").
fn normalize_key_name(name: &str) -> String {
    let n = name.trim().to_lowercase();
    let n = n
        .strip_prefix("left ")
        .or_else(|| n.strip_prefix("right "))
        .unwrap_or(&n);
    match n {
        "escape" => "esc".to_string(),
        "return" => "enter".to_string(),
        "spacebar" => "space".to_string(),
        other => other.to_string(),
    }
}

/// Parse a captured shortcut string (e.g. `"shift"`, `"ctrl+shift"`, `"tab"`)
/// into a [`LockCombo`]. Returns `None` for empty/blank input (tap-to-lock off).
fn parse_lock_combo(raw: &str) -> Option<LockCombo> {
    if raw.trim().is_empty() {
        return None;
    }
    let mut combo = LockCombo {
        mods: Modifiers::empty(),
        key: None,
    };
    for part in raw.split('+') {
        let token = part.trim().to_lowercase();
        let token = token
            .strip_suffix("_left")
            .or_else(|| token.strip_suffix("_right"))
            .unwrap_or(&token);
        if token.is_empty() {
            continue;
        }
        if let Some(m) = modifier_bit(token) {
            combo.mods |= m;
        } else {
            combo.key = Some(normalize_key_name(token));
        }
    }
    if combo.mods.is_empty() && combo.key.is_none() {
        None
    } else {
        Some(combo)
    }
}

/// Watches for a lock-shortcut tap to flip an active hold recording to
/// hands-free.
///
/// Held in Tauri managed state. The coordinator calls [`LockWatch::arm`] (with
/// the captured lock shortcut) when a push-to-talk recording starts and
/// [`LockWatch::disarm`] once it locks, finishes, or is cancelled.
pub struct LockWatch {
    /// `Some(combo)` while armed for the current hold recording; `None` when
    /// disarmed or the shortcut is empty. Read on every observed key event.
    armed: Arc<Mutex<Option<LockCombo>>>,
}

impl LockWatch {
    /// Spawn the background listener thread and return the handle.
    pub fn new(app: AppHandle) -> Self {
        let armed = Arc::new(Mutex::new(None));
        let armed_for_thread = Arc::clone(&armed);
        thread::spawn(move || run(app, armed_for_thread));
        Self { armed }
    }

    /// Start listening: a tap of `shortcut` will now lock the current hold
    /// recording. A blank/unparseable shortcut leaves the watcher disarmed.
    pub fn arm(&self, shortcut: &str) {
        if let Ok(mut guard) = self.armed.lock() {
            *guard = parse_lock_combo(shortcut);
        }
    }

    /// Stop listening (recording locked, finished, or cancelled).
    pub fn disarm(&self) {
        if let Ok(mut guard) = self.armed.lock() {
            *guard = None;
        }
    }
}

/// Background thread: own the listener and translate a lock-shortcut tap into a
/// single lock command while armed.
fn run(app: AppHandle, armed: Arc<Mutex<Option<LockCombo>>>) {
    let listener = match create_listener() {
        Some(listener) => listener,
        None => {
            warn!("LockWatch: keyboard listener unavailable; tap-to-lock disabled until restart");
            return;
        }
    };
    debug!("LockWatch: listening for lock-shortcut taps");

    // Track the previous modifier set so a modifier-only lock shortcut fires on
    // its rising edge, never on hold/auto-repeat or release. Updated every
    // iteration (armed or not) so the edge stays correct across arm cycles.
    let mut prev_mods = Modifiers::empty();
    while let Ok(event) = listener.recv() {
        let combo = armed.lock().ok().and_then(|guard| guard.clone());

        if let Some(combo) = combo {
            let mods_now = event.modifiers.contains(combo.mods);
            let fire = match &combo.key {
                // A main key: fire on its key-down while the required modifiers
                // are held (naturally a rising edge — it's the key-down event).
                Some(k) => {
                    event.is_key_down
                        && mods_now
                        && event
                            .key
                            .as_ref()
                            .map(|ek| normalize_key_name(&ek.to_string()) == *k)
                            .unwrap_or(false)
                }
                // Modifier-only: fire when the whole modifier set becomes held.
                None => mods_now && !prev_mods.contains(combo.mods),
            };

            if fire {
                // One lock per arm: clear immediately so a burst of events (or
                // auto-repeat) can't queue a second command before the
                // coordinator disarms us.
                if let Ok(mut guard) = armed.lock() {
                    *guard = None;
                }
                if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                    debug!("LockWatch: lock-shortcut tap -> locking recording hands-free");
                    coordinator.notify_lock();
                }
            }
        }

        prev_mods = event.modifiers;
    }

    debug!("LockWatch: listener stopped");
}

/// Try to create the keyboard listener, retrying a few times to ride out a
/// transient failure or a just-granted permission.
fn create_listener() -> Option<KeyboardListener> {
    for attempt in 1..=LISTENER_CREATE_ATTEMPTS {
        match KeyboardListener::new() {
            Ok(listener) => return Some(listener),
            Err(e) => {
                warn!(
                    "LockWatch: listener attempt {}/{} failed: {}",
                    attempt, LISTENER_CREATE_ATTEMPTS, e
                );
                thread::sleep(LISTENER_RETRY_DELAY);
            }
        }
    }
    None
}
