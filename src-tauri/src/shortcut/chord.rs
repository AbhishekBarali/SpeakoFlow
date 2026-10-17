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

use super::combos;

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

/// Whether a set of keys that is live right now starts with every key of
/// `pressed` (one of `binding_id`'s) and adds more.
///
/// Every set of keys counts, a shortcut's other keys as much as its own, and
/// that includes `binding_id`'s own other keys: with Ctrl+Win to dictate and
/// Ctrl+Win+F12 as the same shortcut's other keys, the F12 withdraws the
/// Ctrl+Win press like any longer shortcut would. A mouse button on top counts
/// too while one is being listened for, since the engine takes a registered
/// button for the rest of a chord. A press that is itself a button is never
/// withdrawn (only modifier-only presses are), so it never waits.
fn extended_by_another(settings: &AppSettings, binding_id: &str, pressed: &str) -> bool {
    let base = keys(pressed);
    if base.is_empty() || combos::has_mouse_button(pressed) {
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
        live && other.combos().into_iter().any(|combo| {
            let longer = keys(combo);
            !(id == binding_id && longer == base)
                && combos::refusal(settings, combo, KeyboardImplementation::HandyKeys).is_none()
                && longer.len() > base.len()
                && base.is_subset(&longer)
        })
    })
}

/// Whether this press should keep quiet until the chord window has passed.
///
/// Only a held press from the hotkey engine that can withdraw it, of a shortcut
/// another live one extends. `shortcut_str` is how a programmatic start (the
/// pill, the CLI) is told apart: it is one of the binding's own sets of keys
/// only when a hotkey fired it.
pub fn holds_back(settings: &AppSettings, binding_id: &str, shortcut_str: &str) -> bool {
    settings.push_to_talk
        && settings.keyboard_implementation == KeyboardImplementation::HandyKeys
        && settings
            .bindings
            .get(binding_id)
            .is_some_and(|b| b.combos().contains(&shortcut_str))
        && extended_by_another(settings, binding_id, shortcut_str)
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

    fn with_other_keys(settings: &mut AppSettings, id: &str, keys: &str) {
        settings.bindings.get_mut(id).unwrap().alternate_binding = keys.to_string();
    }

    /// Other keys are a press the engine can fire, so they hold back the same
    /// way, and they are a longer shortcut like any other.
    #[test]
    fn other_keys_hold_back_and_extend_like_own_keys() {
        let mut settings = with(&[
            ("transcribe", "f13"),
            ("assistant", "fn+ctrl"),
            ("assistant_call", ""),
        ]);
        with_other_keys(&mut settings, "transcribe", "ctrl_left+alt_left");
        with_other_keys(&mut settings, "assistant", "ctrl_left+alt_left+d");
        // Dictation's other keys are extended by the ask's other keys.
        assert!(holds_back(&settings, "transcribe", "ctrl_left+alt_left"));
        // Its own keys are not.
        assert!(!holds_back(&settings, "transcribe", "f13"));
    }

    /// Ctrl+Win to dictate, and Ctrl+Win+F12 as the same shortcut's other keys
    /// (sent by a mouse button's software): the F12 withdraws the Ctrl+Win
    /// press, so its pill must not flash up on the way.
    #[test]
    fn a_shortcuts_own_other_keys_extend_its_keys() {
        let mut settings = with(&[("transcribe", "ctrl_left+super"), ("assistant_call", "")]);
        with_other_keys(&mut settings, "transcribe", "ctrl_left+super+f12");
        assert!(holds_back(&settings, "transcribe", "ctrl_left+super"));
        assert!(!holds_back(&settings, "transcribe", "ctrl_left+super+f12"));
    }

    /// A press that is a mouse button is never withdrawn, so it never waits.
    /// A button added on top of a press is a longer shortcut, but only while
    /// something listens for it.
    #[test]
    fn a_pressed_button_never_waits_and_a_button_on_top_extends_while_on() {
        let mut settings = with(&[
            ("transcribe", "ctrl_left+super"),
            ("assistant", "ctrl_left+alt_left"),
            ("assistant_call", ""),
        ]);
        settings.mouse_button_shortcuts = true;
        with_other_keys(&mut settings, "transcribe", "mousex1");
        with_other_keys(&mut settings, "assistant", "mousex1+ctrl_left+alt_left");
        assert!(!holds_back(&settings, "transcribe", "mousex1"));
        assert!(holds_back(&settings, "assistant", "ctrl_left+alt_left"));
        settings.mouse_button_shortcuts = false;
        assert!(!holds_back(&settings, "assistant", "ctrl_left+alt_left"));
    }

    /// Keys that are not one of the shortcut's own are a programmatic start.
    #[test]
    fn only_the_shortcuts_own_keys_count_as_its_press() {
        let mut settings = with(&[("assistant", "fn+ctrl"), ("assistant_call", "fn+ctrl+c")]);
        with_other_keys(&mut settings, "assistant", "f14");
        assert!(holds_back(&settings, "assistant", "fn+ctrl"));
        assert!(!holds_back(&settings, "assistant", "signal"));
        assert!(!holds_back(&settings, "assistant", "f14"));
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
