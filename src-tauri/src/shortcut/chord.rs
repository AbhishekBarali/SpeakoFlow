//! A recording whose shortcut press may still turn out to be the start of a
//! longer shortcut.
//!
//! With Hold to talk, a modifier-only shortcut has to start recording the
//! moment its keys go down, or the first words are lost. But the same keys can
//! be the first half of a longer shortcut: Ctrl+Alt is the ask and Ctrl+Alt+C
//! the call, and on a Mac Fn dictates while Fn+Ctrl asks. The hotkey engine
//! sorts that out by withdrawing the shorter one when the rest of the chord
//! follows within `handy_keys::CHORD_CANCEL_WINDOW` (see
//! `HotkeyState::Cancelled`), but by then the shorter one's window was already
//! on screen, so every call opened with the ask's panel flashing up first.
//!
//! So such a press records at once and only *shows* itself once the window has
//! passed: [`reveal_when_settled`] holds back the visible part, releasing the
//! keys reveals it at once ([`reveal_now`]), and a withdrawn press is dropped
//! without ever having appeared ([`forget`]). With Tap to toggle none of this
//! is needed: the engine fires those shortcuts when the keys are let go, by
//! which point it already knows what was pressed.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use tauri::AppHandle;

use crate::settings::{self, AppSettings, KeyboardImplementation};

type Reveal = Box<dyn FnOnce(&AppHandle) + Send>;

static PENDING: Lazy<Mutex<Option<(u64, Reveal)>>> = Lazy::new(|| Mutex::new(None));
/// Held while a reveal runs, so a release arriving just as the timer fires
/// waits for that reveal instead of racing past it.
static RUNNING: Mutex<()> = Mutex::new(());
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn keys(binding: &str) -> BTreeSet<String> {
    binding
        .split('+')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Whether a shortcut that is live right now starts with every key of
/// `binding_id`'s and adds more.
fn extended_by_another(settings: &AppSettings, binding_id: &str) -> bool {
    let Some(binding) = settings.bindings.get(binding_id) else {
        return false;
    };
    let base = keys(&binding.current_binding);
    if base.is_empty() {
        return false;
    }
    settings.bindings.iter().any(|(id, other)| {
        let live = if id == settings::CLEANUP_BINDING_ID {
            settings::cleanup_binding_active(settings)
        } else if crate::assistant::is_assistant_binding(id) {
            settings.assistant_enabled
        } else {
            id != "cancel"
        };
        let longer = keys(&other.current_binding);
        id != binding_id && live && longer.len() > base.len() && base.is_subset(&longer)
    })
}

/// Whether this press should keep quiet until the chord window has passed.
///
/// Only a held press from the hotkey engine that can withdraw it, of a shortcut
/// another live one extends. `shortcut_str` is how a programmatic start (the
/// pill, the CLI) is told apart: it is the binding's own keys only when a
/// hotkey fired it.
pub fn holds_back(settings: &AppSettings, binding_id: &str, shortcut_str: &str) -> bool {
    settings.push_to_talk
        && settings.keyboard_implementation == KeyboardImplementation::HandyKeys
        && settings
            .bindings
            .get(binding_id)
            .is_some_and(|b| b.current_binding == shortcut_str)
        && extended_by_another(settings, binding_id)
}

/// Run `reveal` now, or, when `hold_back`, once the chord window has passed
/// with the press still standing.
pub fn reveal_when_settled(
    app: &AppHandle,
    hold_back: bool,
    reveal: impl FnOnce(&AppHandle) + Send + 'static,
) {
    if !hold_back {
        reveal(app);
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut pending) = PENDING.lock() {
        *pending = Some((generation, Box::new(reveal)));
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(::handy_keys::CHORD_CANCEL_WINDOW);
        run_pending(&app, Some(generation));
    });
}

/// The keys were let go: show whatever is still held back, right away.
pub fn reveal_now(app: &AppHandle) {
    run_pending(app, None);
}

/// The press was withdrawn or cancelled: drop what it was holding back, so it
/// never appears. Returns whether there was something to drop, which means the
/// recording it belonged to was never shown.
pub fn forget() -> bool {
    PENDING
        .lock()
        .map(|mut pending| pending.take().is_some())
        .unwrap_or(false)
}

fn run_pending(app: &AppHandle, only: Option<u64>) {
    let _running = RUNNING.lock();
    let reveal = PENDING.lock().ok().and_then(|mut pending| {
        let matches = match (pending.as_ref(), only) {
            (Some((generation, _)), Some(wanted)) => *generation == wanted,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if matches {
            pending.take().map(|(_, reveal)| reveal)
        } else {
            None
        }
    });
    if let Some(reveal) = reveal {
        reveal(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(bindings: &[(&str, &str)]) -> AppSettings {
        let mut settings = settings::get_default_settings();
        settings.push_to_talk = true;
        settings.keyboard_implementation = KeyboardImplementation::HandyKeys;
        settings.assistant_enabled = true;
        for (id, keys) in bindings {
            settings.bindings.get_mut(*id).unwrap().current_binding = keys.to_string();
        }
        settings
    }

    #[test]
    fn the_ask_holds_back_while_the_call_extends_it() {
        let settings = with(&[
            ("assistant", "ctrl_left+alt_left"),
            ("assistant_call", "ctrl_left+alt_left+c"),
        ]);
        assert!(holds_back(&settings, "assistant", "ctrl_left+alt_left"));
    }

    #[test]
    fn nothing_holds_back_once_nothing_extends_it() {
        let settings = with(&[("assistant", "ctrl_left+alt_left"), ("assistant_call", "")]);
        assert!(!holds_back(&settings, "assistant", "ctrl_left+alt_left"));
    }

    #[test]
    fn a_tap_or_a_programmatic_start_never_holds_back() {
        let mut settings = with(&[
            ("assistant", "ctrl_left+alt_left"),
            ("assistant_call", "ctrl_left+alt_left+c"),
        ]);
        assert!(!holds_back(&settings, "assistant", "signal"));
        settings.push_to_talk = false;
        assert!(!holds_back(&settings, "assistant", "ctrl_left+alt_left"));
    }

    #[test]
    fn the_mac_dictation_key_is_extended_by_the_ask() {
        let settings = with(&[
            ("transcribe", "fn"),
            ("assistant", "fn+ctrl"),
            ("assistant_call", "fn+ctrl+c"),
        ]);
        assert!(holds_back(&settings, "transcribe", "fn"));
        assert!(holds_back(&settings, "assistant", "fn+ctrl"));
    }

    #[test]
    fn a_switched_off_assistant_extends_nothing() {
        let mut settings = with(&[
            ("transcribe", "fn"),
            ("assistant", "fn+ctrl"),
            ("assistant_call", "fn+ctrl+c"),
        ]);
        settings.assistant_enabled = false;
        settings.post_process_enabled = false;
        assert!(!holds_back(&settings, "transcribe", "fn"));
    }
}
