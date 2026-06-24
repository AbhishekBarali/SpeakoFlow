//! Tap-to-lock watcher.
//!
//! While a push-to-talk (hold) recording is active, a quick tap of **Shift**
//! converts it to hands-free (locked) mode, so the user can let go of the
//! hotkey and keep talking. Pressing the hotkey again (or the overlay/panel
//! "done" tick) then finishes the recording.
//!
//! Detection uses a global, **non-blocking** [`handy_keys::KeyboardListener`]
//! running on a dedicated thread. Non-blocking means it only *observes* key
//! events without consuming them, so normal typing and the base hotkey keep
//! working and any active `HotkeyManager` still fires. The listener acts only
//! on the rising edge of Shift, and only while the transcription coordinator
//! has "armed" it for the current hold recording.
//!
//! Privacy: this never inspects which character keys are pressed and never logs
//! or stores key contents — it looks at the Shift modifier state only. The app
//! already runs a persistent global keyboard hook for its hotkeys, so this adds
//! no new capability beyond detecting a Shift tap.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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

/// Watches for a Shift tap to flip an active hold recording to hands-free.
///
/// Held in Tauri managed state. The coordinator calls [`LockWatch::arm`] when a
/// push-to-talk recording starts and [`LockWatch::disarm`] once it locks,
/// finishes, or is cancelled.
pub struct LockWatch {
    armed: Arc<AtomicBool>,
}

impl LockWatch {
    /// Spawn the background listener thread and return the handle.
    pub fn new(app: AppHandle) -> Self {
        let armed = Arc::new(AtomicBool::new(false));
        let armed_for_thread = Arc::clone(&armed);
        thread::spawn(move || run(app, armed_for_thread));
        Self { armed }
    }

    /// Start listening: a Shift tap will now lock the current hold recording.
    pub fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    /// Stop listening (recording locked, finished, or cancelled).
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

/// Background thread: own the listener and translate Shift taps into a single
/// lock command while armed.
fn run(app: AppHandle, armed: Arc<AtomicBool>) {
    let listener = match create_listener() {
        Some(listener) => listener,
        None => {
            warn!("LockWatch: keyboard listener unavailable; tap-to-lock disabled until restart");
            return;
        }
    };
    debug!("LockWatch: listening for Shift taps");

    // Track the physical Shift state so we only fire on a press (rising edge),
    // never on hold/auto-repeat or release.
    let mut shift_down = false;
    while let Ok(event) = listener.recv() {
        let shift_now = event.modifiers.intersects(Modifiers::SHIFT);
        let pressed = shift_now && !shift_down;
        shift_down = shift_now;

        if pressed && event.is_key_down && armed.load(Ordering::SeqCst) {
            // One lock per arm: clear immediately so a burst of key events can't
            // queue a second command before the coordinator disarms us.
            armed.store(false, Ordering::SeqCst);
            if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                debug!("LockWatch: Shift tap -> locking recording hands-free");
                coordinator.notify_lock();
            }
        }
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
