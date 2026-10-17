//! Linux key codes (evdev, `linux/input-event-codes.h`) to handy-keys types.
//!
//! [`KEYS`] is the table the rdev-based listener used, with rdev's own step
//! taken out: rdev 0.5.3's evdev → `rdev::Key` table composed with the old
//! `rdev::Key` → [`Key`] table. Keys neither of them knew (Pause, Print Screen,
//! F13 and up, the keypad decimal) stay unmapped, so no stored binding changes
//! meaning.

use evdev_rs::enums::EV_KEY;

use crate::types::{Key, Modifiers};

/// Every key a hotkey can end in, by evdev code.
static KEYS: [(EV_KEY, Key); 92] = [
    (EV_KEY::KEY_ESC, Key::Escape),
    (EV_KEY::KEY_1, Key::Num1),
    (EV_KEY::KEY_2, Key::Num2),
    (EV_KEY::KEY_3, Key::Num3),
    (EV_KEY::KEY_4, Key::Num4),
    (EV_KEY::KEY_5, Key::Num5),
    (EV_KEY::KEY_6, Key::Num6),
    (EV_KEY::KEY_7, Key::Num7),
    (EV_KEY::KEY_8, Key::Num8),
    (EV_KEY::KEY_9, Key::Num9),
    (EV_KEY::KEY_0, Key::Num0),
    (EV_KEY::KEY_MINUS, Key::Minus),
    (EV_KEY::KEY_EQUAL, Key::Equal),
    (EV_KEY::KEY_BACKSPACE, Key::Delete),
    (EV_KEY::KEY_TAB, Key::Tab),
    (EV_KEY::KEY_Q, Key::Q),
    (EV_KEY::KEY_W, Key::W),
    (EV_KEY::KEY_E, Key::E),
    (EV_KEY::KEY_R, Key::R),
    (EV_KEY::KEY_T, Key::T),
    (EV_KEY::KEY_Y, Key::Y),
    (EV_KEY::KEY_U, Key::U),
    (EV_KEY::KEY_I, Key::I),
    (EV_KEY::KEY_O, Key::O),
    (EV_KEY::KEY_P, Key::P),
    (EV_KEY::KEY_LEFTBRACE, Key::LeftBracket),
    (EV_KEY::KEY_RIGHTBRACE, Key::RightBracket),
    (EV_KEY::KEY_ENTER, Key::Return),
    (EV_KEY::KEY_A, Key::A),
    (EV_KEY::KEY_S, Key::S),
    (EV_KEY::KEY_D, Key::D),
    (EV_KEY::KEY_F, Key::F),
    (EV_KEY::KEY_G, Key::G),
    (EV_KEY::KEY_H, Key::H),
    (EV_KEY::KEY_J, Key::J),
    (EV_KEY::KEY_K, Key::K),
    (EV_KEY::KEY_L, Key::L),
    (EV_KEY::KEY_SEMICOLON, Key::Semicolon),
    (EV_KEY::KEY_APOSTROPHE, Key::Quote),
    (EV_KEY::KEY_GRAVE, Key::Grave),
    (EV_KEY::KEY_BACKSLASH, Key::Backslash),
    (EV_KEY::KEY_Z, Key::Z),
    (EV_KEY::KEY_X, Key::X),
    (EV_KEY::KEY_C, Key::C),
    (EV_KEY::KEY_V, Key::V),
    (EV_KEY::KEY_B, Key::B),
    (EV_KEY::KEY_N, Key::N),
    (EV_KEY::KEY_M, Key::M),
    (EV_KEY::KEY_COMMA, Key::Comma),
    (EV_KEY::KEY_DOT, Key::Period),
    (EV_KEY::KEY_SLASH, Key::Slash),
    (EV_KEY::KEY_KPASTERISK, Key::KeypadMultiply),
    (EV_KEY::KEY_SPACE, Key::Space),
    (EV_KEY::KEY_CAPSLOCK, Key::CapsLock),
    (EV_KEY::KEY_F1, Key::F1),
    (EV_KEY::KEY_F2, Key::F2),
    (EV_KEY::KEY_F3, Key::F3),
    (EV_KEY::KEY_F4, Key::F4),
    (EV_KEY::KEY_F5, Key::F5),
    (EV_KEY::KEY_F6, Key::F6),
    (EV_KEY::KEY_F7, Key::F7),
    (EV_KEY::KEY_F8, Key::F8),
    (EV_KEY::KEY_F9, Key::F9),
    (EV_KEY::KEY_F10, Key::F10),
    (EV_KEY::KEY_NUMLOCK, Key::NumLock),
    (EV_KEY::KEY_SCROLLLOCK, Key::ScrollLock),
    (EV_KEY::KEY_KP7, Key::Keypad7),
    (EV_KEY::KEY_KP8, Key::Keypad8),
    (EV_KEY::KEY_KP9, Key::Keypad9),
    (EV_KEY::KEY_KPMINUS, Key::KeypadMinus),
    (EV_KEY::KEY_KP4, Key::Keypad4),
    (EV_KEY::KEY_KP5, Key::Keypad5),
    (EV_KEY::KEY_KP6, Key::Keypad6),
    (EV_KEY::KEY_KPPLUS, Key::KeypadPlus),
    (EV_KEY::KEY_KP1, Key::Keypad1),
    (EV_KEY::KEY_KP2, Key::Keypad2),
    (EV_KEY::KEY_KP3, Key::Keypad3),
    (EV_KEY::KEY_KP0, Key::Keypad0),
    (EV_KEY::KEY_F11, Key::F11),
    (EV_KEY::KEY_F12, Key::F12),
    (EV_KEY::KEY_KPENTER, Key::KeypadEnter),
    (EV_KEY::KEY_KPSLASH, Key::KeypadDivide),
    (EV_KEY::KEY_HOME, Key::Home),
    (EV_KEY::KEY_UP, Key::UpArrow),
    (EV_KEY::KEY_PAGEUP, Key::PageUp),
    (EV_KEY::KEY_LEFT, Key::LeftArrow),
    (EV_KEY::KEY_RIGHT, Key::RightArrow),
    (EV_KEY::KEY_END, Key::End),
    (EV_KEY::KEY_DOWN, Key::DownArrow),
    (EV_KEY::KEY_PAGEDOWN, Key::PageDown),
    (EV_KEY::KEY_INSERT, Key::Insert),
    (EV_KEY::KEY_DELETE, Key::ForwardDelete),
];

/// The modifier keys, each with the side-specific modifier it holds.
static MODIFIERS: [(EV_KEY, Modifiers); 8] = [
    (EV_KEY::KEY_LEFTSHIFT, Modifiers::SHIFT_LEFT),
    (EV_KEY::KEY_RIGHTSHIFT, Modifiers::SHIFT_RIGHT),
    (EV_KEY::KEY_LEFTCTRL, Modifiers::CTRL_LEFT),
    (EV_KEY::KEY_RIGHTCTRL, Modifiers::CTRL_RIGHT),
    (EV_KEY::KEY_LEFTALT, Modifiers::OPT_LEFT),
    (EV_KEY::KEY_RIGHTALT, Modifiers::OPT_RIGHT),
    (EV_KEY::KEY_LEFTMETA, Modifiers::CMD_LEFT),
    (EV_KEY::KEY_RIGHTMETA, Modifiers::CMD_RIGHT),
];

/// The mouse buttons the hotkey logic follows.
static BUTTONS: [(EV_KEY, Key); 3] = [
    (EV_KEY::BTN_LEFT, Key::MouseLeft),
    (EV_KEY::BTN_RIGHT, Key::MouseRight),
    (EV_KEY::BTN_MIDDLE, Key::MouseMiddle),
];

/// The key a hotkey can end in, for a key code.
pub(crate) fn key_for(code: &EV_KEY) -> Option<Key> {
    KEYS.iter().find(|(c, _)| c == code).map(|&(_, key)| key)
}

/// The side-specific modifier a modifier key holds.
pub(crate) fn modifier_for(code: &EV_KEY) -> Option<Modifiers> {
    MODIFIERS
        .iter()
        .find(|(c, _)| c == code)
        .map(|&(_, modifier)| modifier)
}

/// The mouse button a button code is, if the hotkey logic follows it.
pub(crate) fn button_for(code: &EV_KEY) -> Option<Key> {
    BUTTONS.iter().find(|(c, _)| c == code).map(|&(_, key)| key)
}

/// The numeric code, for bookkeeping that outlives the event.
pub(crate) fn code_of(code: &EV_KEY) -> u16 {
    code.clone() as u16
}

/// Whether a mouse button may be withheld from the app under the pointer when
/// a shortcut uses it: every button the hotkey logic follows but the primary
/// clicks, which no shortcut may take away from the mouse.
pub(crate) fn is_withholdable_button(key: Key) -> bool {
    BUTTONS.iter().any(|&(_, button)| button == key)
        && !matches!(key, Key::MouseLeft | Key::MouseRight)
}

/// Whether a device has a mouse button a shortcut can use.
pub(crate) fn has_bindable_button(has: impl Fn(&EV_KEY) -> bool) -> bool {
    BUTTONS
        .iter()
        .any(|(code, key)| is_withholdable_button(*key) && has(code))
}

/// Whether a device has a key a hotkey can end in. `has` answers for one code.
pub(crate) fn has_hotkey_key(has: impl Fn(&EV_KEY) -> bool) -> bool {
    KEYS.iter().any(|(code, _)| has(code))
}

/// Whether a device has a modifier or a mouse button the hotkey logic follows.
pub(crate) fn has_followed_key(has: impl Fn(&EV_KEY) -> bool) -> bool {
    MODIFIERS.iter().any(|(code, _)| has(code)) || BUTTONS.iter().any(|(code, _)| has(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_hotkey_key_has_one_code_and_every_code_one_key() {
        let codes: HashSet<u16> = KEYS.iter().map(|(code, _)| code_of(code)).collect();
        let keys: HashSet<Key> = KEYS.iter().map(|&(_, key)| key).collect();
        assert_eq!(codes.len(), KEYS.len());
        assert_eq!(keys.len(), KEYS.len());
    }

    #[test]
    fn keys_keep_the_meaning_they_had_under_rdev() {
        // Where evdev and handy-keys name the same key differently.
        assert_eq!(key_for(&EV_KEY::KEY_SEMICOLON), Some(Key::Semicolon));
        assert_eq!(key_for(&EV_KEY::KEY_BACKSPACE), Some(Key::Delete));
        assert_eq!(key_for(&EV_KEY::KEY_DELETE), Some(Key::ForwardDelete));
        assert_eq!(key_for(&EV_KEY::KEY_DOT), Some(Key::Period));
        assert_eq!(key_for(&EV_KEY::KEY_APOSTROPHE), Some(Key::Quote));
        assert_eq!(key_for(&EV_KEY::KEY_GRAVE), Some(Key::Grave));
        assert_eq!(key_for(&EV_KEY::KEY_ENTER), Some(Key::Return));
        assert_eq!(key_for(&EV_KEY::KEY_KPENTER), Some(Key::KeypadEnter));
        assert_eq!(key_for(&EV_KEY::KEY_KPASTERISK), Some(Key::KeypadMultiply));
        // Never reported on Linux before, so still not.
        for code in [
            EV_KEY::KEY_PAUSE,
            EV_KEY::KEY_SYSRQ,
            EV_KEY::KEY_PRINT,
            EV_KEY::KEY_KPDOT,
            EV_KEY::KEY_F13,
            EV_KEY::KEY_102ND,
        ] {
            assert_eq!(key_for(&code), None, "{code:?}");
        }
    }

    #[test]
    fn modifiers_are_never_hotkey_keys() {
        for (code, modifier) in MODIFIERS.iter() {
            assert_eq!(modifier_for(code), Some(*modifier));
            assert_eq!(key_for(code), None, "{code:?}");
        }
        assert_eq!(
            modifier_for(&EV_KEY::KEY_RIGHTALT),
            Some(Modifiers::OPT_RIGHT)
        );
        assert_eq!(
            modifier_for(&EV_KEY::KEY_LEFTMETA),
            Some(Modifiers::CMD_LEFT)
        );
        assert_eq!(modifier_for(&EV_KEY::KEY_A), None);
    }

    #[test]
    fn devices_are_judged_by_the_keys_they_have() {
        let keyboard = |code: &EV_KEY| *code == EV_KEY::KEY_A || *code == EV_KEY::KEY_LEFTCTRL;
        let mouse = |code: &EV_KEY| *code == EV_KEY::BTN_LEFT;
        let lid = |_: &EV_KEY| false;
        assert!(has_hotkey_key(keyboard) && has_followed_key(keyboard));
        assert!(!has_hotkey_key(mouse) && has_followed_key(mouse));
        assert!(!has_hotkey_key(lid) && !has_followed_key(lid));
        let wheel_mouse = |code: &EV_KEY| *code == EV_KEY::BTN_LEFT || *code == EV_KEY::BTN_MIDDLE;
        assert!(has_bindable_button(wheel_mouse));
        assert!(
            !has_bindable_button(mouse),
            "a touchpad's single button is a primary click"
        );
    }

    #[test]
    fn only_the_middle_button_can_be_withheld() {
        assert!(is_withholdable_button(Key::MouseMiddle));
        assert!(!is_withholdable_button(Key::MouseLeft));
        assert!(!is_withholdable_button(Key::MouseRight));
        // Never read on Linux, so a shortcut on them cannot fire here.
        assert!(!is_withholdable_button(Key::MouseX1));
        assert!(!is_withholdable_button(Key::A));
    }
}
