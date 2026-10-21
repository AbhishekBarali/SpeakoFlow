//! The sets of keys a shortcut answers to.
//!
//! A shortcut has its own keys (`current_binding`) and, optionally, other keys
//! that do the same thing (`alternate_binding`): typically a mouse button, or a
//! combo nobody would press by hand that a mouse's own software sends for one
//! of its buttons. Both are registered with the hotkey engine under the
//! shortcut's id, so nothing downstream of the engines needs to know there are
//! two. What has to know lives here: which of them an engine should listen for
//! right now, which keys a shortcut may be given at all, and which keys are
//! already taken.

use handy_keys::Key;
use log::debug;

use crate::settings::{AppSettings, KeyboardImplementation, ShortcutBinding};

/// Which of a shortcut's two sets of keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Its own keys, the ones "Use default" restores.
    Primary,
    /// The other keys someone added.
    Alternate,
}

fn parts(combo: &str) -> impl Iterator<Item = &str> {
    combo
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn buttons(combo: &str) -> impl Iterator<Item = Key> + '_ {
    parts(combo).filter_map(|part| part.parse::<Key>().ok().filter(|key| key.is_mouse_button()))
}

/// Whether `combo` includes a mouse button.
pub fn has_mouse_button(combo: &str) -> bool {
    buttons(combo).next().is_some()
}

/// Whether `combo` includes the left or right button, which no shortcut may
/// take: they are how a mouse is used at all.
pub fn uses_click(combo: &str) -> bool {
    buttons(combo).any(|key| matches!(key, Key::MouseLeft | Key::MouseRight))
}

/// One spelling per set of keys: lower case, aliases folded, order ignored. A
/// side stays part of the key (`ctrl_left` is not `ctrl`), because the engine
/// registers those as different hotkeys. Mirrors `canonicalShortcut` in
/// `src/lib/utils/shortcutConflict.ts`, which makes the same call in the editor.
pub fn canonical(combo: &str) -> String {
    let mut keys: Vec<String> = parts(combo)
        .map(|part| {
            let part = part.to_lowercase();
            let (base, side) = if let Some(base) = part.strip_suffix("_left") {
                (base, "_left")
            } else if let Some(base) = part.strip_suffix("_right") {
                (base, "_right")
            } else {
                (part.as_str(), "")
            };
            let base = match base {
                "control" => "ctrl",
                "option" | "opt" => "alt",
                "cmd" | "command" | "meta" | "win" | "windows" => "super",
                "mouse1" | "leftclick" | "lmb" => "mouseleft",
                "mouse2" | "rightclick" | "rmb" => "mouseright",
                "mouse3" | "middleclick" | "mmb" => "mousemiddle",
                "mouse4" | "back" | "xbutton1" => "mousex1",
                "mouse5" | "forward" | "xbutton2" => "mousex2",
                other => other,
            };
            format!("{base}{side}")
        })
        .collect();
    keys.sort();
    keys.join("+")
}

/// Why `combo` cannot be a shortcut's keys on `engine` right now, or `None`
/// when it can. Only mouse buttons are ever refused here; whether the engine
/// can parse the keys is its own validator's call.
///
/// The same rules decide what is armed ([`armed`]), so a mouse button stored
/// while it was allowed simply goes quiet once it no longer is, and comes
/// back when it is allowed again.
pub fn refusal(
    settings: &AppSettings,
    combo: &str,
    engine: KeyboardImplementation,
) -> Option<&'static str> {
    if !has_mouse_button(combo) {
        return None;
    }
    if uses_click(combo) {
        return Some("Left and right click can't be part of a shortcut.");
    }
    if engine != KeyboardImplementation::HandyKeys {
        return Some(
            "Mouse buttons need the SpeakoFlow Keys engine (Settings → Advanced → \
             Keyboard implementation).",
        );
    }
    if !settings.mouse_button_shortcuts {
        // Only Linux still keeps the switch under Experimental.
        #[cfg(target_os = "linux")]
        return Some(
            "Mouse buttons are turned off. Turn on \"Mouse buttons as shortcuts\" in \
             Settings → Advanced → Experimental.",
        );
        #[cfg(not(target_os = "linux"))]
        return Some(
            "Mouse buttons are turned off. Turn on \"Mouse buttons as shortcuts\" in \
             Settings → Shortcuts.",
        );
    }
    None
}

/// Why `combo` cannot be a shortcut's *own* keys, or `None` when it can.
///
/// A mouse button is only ever a shortcut's other keys. Its own keys are the
/// ones it is known by, that "Use default" restores and that every other
/// engine has to be able to listen for, so they stay on the keyboard: a mouse
/// button in their place used to replace a working Ctrl + Win with one button,
/// and turning mouse buttons off then had to put the default keys back behind
/// the user's back. Kept as other keys, a button goes quiet with the setting
/// and the shortcut's keys keep working throughout.
pub fn own_keys_refusal(combo: &str) -> Option<&'static str> {
    has_mouse_button(combo).then_some(
        "A mouse button can't be a shortcut's own keys. Add it next to them with \
         \"Add another\".",
    )
}

/// The keys of `binding` that `engine` should listen for right now, its own
/// first. Nothing while the shortcut is turned off; other keys that are the
/// same as its own are not listened for twice; and a mouse button only while
/// [`refusal`] allows it.
pub fn armed(
    settings: &AppSettings,
    binding: &ShortcutBinding,
    engine: KeyboardImplementation,
) -> Vec<(Slot, String)> {
    let combos = binding.combos();
    let Some(primary) = combos.first() else {
        return Vec::new();
    };
    let primary_keys = canonical(primary);
    combos
        .iter()
        .enumerate()
        .map(|(index, combo)| {
            let slot = if index == 0 {
                Slot::Primary
            } else {
                Slot::Alternate
            };
            (slot, *combo)
        })
        .filter(|(slot, combo)| *slot == Slot::Primary || canonical(combo) != primary_keys)
        .filter(|(_, combo)| match refusal(settings, combo, engine) {
            None => true,
            Some(reason) => {
                debug!("Not listening for '{combo}' of '{}': {reason}", binding.id);
                false
            }
        })
        .map(|(slot, combo)| (slot, combo.to_string()))
        .collect()
}

/// What registering one shortcut's keys did. `None` for keys that were not
/// registered because there is nothing to listen for (see [`armed`]).
#[derive(Debug)]
pub struct Armed {
    pub primary: Option<Result<(), String>>,
    pub alternate: Option<Result<(), String>>,
}

impl Armed {
    /// Nothing was registered, and nothing failed.
    pub fn nothing() -> Self {
        Self {
            primary: None,
            alternate: None,
        }
    }

    /// The registration could not even be attempted.
    pub fn failed(error: String) -> Self {
        Self {
            primary: Some(Err(error)),
            alternate: None,
        }
    }

    /// Whether the shortcut's own keys failed: that is what every caller has
    /// to handle, since those are the keys the shortcut is known by. Its other
    /// keys failing (another app took them) is logged, and the shortcut keeps
    /// working on its own keys.
    pub fn into_result(self, binding_id: &str) -> Result<(), String> {
        if let Some(Err(e)) = &self.alternate {
            log::warn!("Could not register the other keys of '{binding_id}': {e}");
        }
        self.primary.unwrap_or(Ok(()))
    }
}

/// Register `binding`'s keys with `register`, its own first.
///
/// When its own keys fail, its other keys are not tried, so a failed
/// registration leaves nothing behind for the caller to undo — the same
/// all-or-nothing a shortcut with one set of keys always had. Its other keys
/// failing never costs it its own.
pub fn arm_with(
    settings: &AppSettings,
    binding: &ShortcutBinding,
    engine: KeyboardImplementation,
    mut register: impl FnMut(&str) -> Result<(), String>,
) -> Armed {
    let mut armed_keys = Armed::nothing();
    for (slot, combo) in armed(settings, binding, engine) {
        let result = register(&combo);
        match slot {
            Slot::Primary => {
                let failed = result.is_err();
                armed_keys.primary = Some(result);
                if failed {
                    break;
                }
            }
            Slot::Alternate => armed_keys.alternate = Some(result),
        }
    }
    armed_keys
}

/// Who already answers to a set of keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// Another shortcut, by id: its own keys or its other keys.
    Other(String),
    /// The same shortcut's other set.
    OwnOther,
}

impl Conflict {
    /// Says so, for an error the editor did not already catch.
    pub fn describe(&self, settings: &AppSettings, combo: &str) -> String {
        match self {
            Conflict::Other(id) => {
                let name = settings
                    .bindings
                    .get(id)
                    .map(|b| b.name.as_str())
                    .unwrap_or(id.as_str());
                format!("{combo} is already used by the {name} shortcut")
            }
            Conflict::OwnOther => format!("{combo} is already this shortcut's other keys"),
        }
    }
}

/// Who already answers to `combo`, if `id`'s `slot` were given it.
///
/// Two shortcuts cannot share keys (the engines refuse the second
/// registration, or worse, take whichever registers first), and a shortcut's
/// two sets of keys must differ or the second is pointless. A shortcut that is
/// turned off holds no keys, so its keys are free.
pub fn conflict(settings: &AppSettings, id: &str, combo: &str, slot: Slot) -> Option<Conflict> {
    let wanted = canonical(combo);
    if wanted.is_empty() {
        return None;
    }
    let mut ids: Vec<&String> = settings.bindings.keys().collect();
    // A stable answer when more than one shortcut holds the keys.
    ids.sort();
    for other_id in ids {
        let other = &settings.bindings[other_id];
        if other_id == id {
            let own_other = match slot {
                Slot::Primary => other.alternate_binding.as_str(),
                Slot::Alternate => other.current_binding.as_str(),
            };
            if !own_other.trim().is_empty() && canonical(own_other) == wanted {
                return Some(Conflict::OwnOther);
            }
            continue;
        }
        if other
            .combos()
            .iter()
            .any(|taken| canonical(taken) == wanted)
        {
            return Some(Conflict::Other(other_id.clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    fn with_keys(keys: &[(&str, &str, &str)]) -> AppSettings {
        let mut settings = get_default_settings();
        for (id, own, other) in keys {
            let binding = settings.bindings.get_mut(*id).unwrap();
            binding.current_binding = own.to_string();
            binding.alternate_binding = other.to_string();
        }
        settings
    }

    fn combos_of(armed: Vec<(Slot, String)>) -> Vec<String> {
        armed.into_iter().map(|(_, combo)| combo).collect()
    }

    #[test]
    fn both_sets_of_keys_are_armed_own_keys_first() {
        let settings = with_keys(&[("transcribe", "ctrl_left+super", "ctrl+alt+f12")]);
        let armed = armed(
            &settings,
            &settings.bindings["transcribe"],
            KeyboardImplementation::HandyKeys,
        );
        assert_eq!(
            armed,
            vec![
                (Slot::Primary, "ctrl_left+super".to_string()),
                (Slot::Alternate, "ctrl+alt+f12".to_string()),
            ]
        );
    }

    #[test]
    fn a_shortcut_turned_off_arms_neither() {
        let settings = with_keys(&[("cancel", "", "f13")]);
        assert!(armed(
            &settings,
            &settings.bindings["cancel"],
            KeyboardImplementation::HandyKeys
        )
        .is_empty());
    }

    #[test]
    fn other_keys_that_are_the_same_keys_are_not_armed_twice() {
        let settings = with_keys(&[("transcribe", "ctrl+alt+k", "Alt+Control+K")]);
        assert_eq!(
            combos_of(armed(
                &settings,
                &settings.bindings["transcribe"],
                KeyboardImplementation::HandyKeys
            )),
            vec!["ctrl+alt+k"]
        );
    }

    #[test]
    fn a_mouse_button_is_armed_only_while_mouse_buttons_are_on() {
        let mut settings = with_keys(&[("transcribe", "ctrl_left+super", "mousex1")]);
        let dictate = settings.bindings["transcribe"].clone();
        settings.mouse_button_shortcuts = false;
        assert_eq!(
            combos_of(armed(
                &settings,
                &dictate,
                KeyboardImplementation::HandyKeys
            )),
            vec!["ctrl_left+super"]
        );
        settings.mouse_button_shortcuts = true;
        assert_eq!(
            combos_of(armed(
                &settings,
                &dictate,
                KeyboardImplementation::HandyKeys
            )),
            vec!["ctrl_left+super", "mousex1"]
        );
    }

    #[test]
    fn tauris_engine_never_arms_a_mouse_button() {
        let mut settings = with_keys(&[("transcribe", "ctrl+space", "ctrl+mousemiddle")]);
        settings.mouse_button_shortcuts = true;
        assert_eq!(
            combos_of(armed(
                &settings,
                &settings.bindings["transcribe"],
                KeyboardImplementation::Tauri
            )),
            vec!["ctrl+space"]
        );
    }

    /// Even typed into the settings file by hand, a click never becomes a
    /// shortcut.
    #[test]
    fn a_click_is_never_armed() {
        let mut settings = with_keys(&[("transcribe", "ctrl_left+super", "ctrl+mouseleft")]);
        settings.mouse_button_shortcuts = true;
        assert_eq!(
            combos_of(armed(
                &settings,
                &settings.bindings["transcribe"],
                KeyboardImplementation::HandyKeys
            )),
            vec!["ctrl_left+super"]
        );
    }

    #[test]
    fn mouse_buttons_are_refused_with_the_reason() {
        let mut settings = get_default_settings();
        settings.mouse_button_shortcuts = false;
        let off = refusal(&settings, "mousex1", KeyboardImplementation::HandyKeys);
        assert!(off.is_some_and(|r| r.contains("turned off")), "{off:?}");
        settings.mouse_button_shortcuts = true;
        assert_eq!(
            refusal(&settings, "mousex1", KeyboardImplementation::HandyKeys),
            None
        );
        assert_eq!(
            refusal(
                &settings,
                "ctrl_left+mousex2",
                KeyboardImplementation::HandyKeys
            ),
            None
        );
        let tauri = refusal(&settings, "mousex1", KeyboardImplementation::Tauri);
        assert!(
            tauri.is_some_and(|r| r.contains("SpeakoFlow Keys")),
            "{tauri:?}"
        );
        let click = refusal(
            &settings,
            "ctrl+mouseright",
            KeyboardImplementation::HandyKeys,
        );
        assert!(click.is_some_and(|r| r.contains("click")), "{click:?}");
        // Keys are never this function's business.
        assert_eq!(
            refusal(&settings, "ctrl+back_slash", KeyboardImplementation::Tauri),
            None
        );
        assert_eq!(
            refusal(&settings, "f13", KeyboardImplementation::Tauri),
            None
        );
    }

    #[test]
    fn a_mouse_button_is_never_a_shortcuts_own_keys() {
        assert!(own_keys_refusal("mousex1").is_some());
        assert!(own_keys_refusal("ctrl_left+mousemiddle").is_some());
        assert!(own_keys_refusal("Back").is_some());
        assert_eq!(own_keys_refusal("ctrl_left+super"), None);
        assert_eq!(own_keys_refusal("backspace"), None);
        assert_eq!(own_keys_refusal(""), None);
    }

    #[test]
    fn mouse_buttons_are_told_apart_by_any_of_their_names() {
        assert!(has_mouse_button("mousex1"));
        assert!(has_mouse_button("ctrl_left+back"));
        assert!(has_mouse_button("Mouse5"));
        assert!(!has_mouse_button("ctrl_left+super"));
        assert!(!has_mouse_button("backspace"));
        assert!(uses_click("shift+LeftClick"));
        assert!(!uses_click("mousemiddle"));
    }

    #[test]
    fn canonical_folds_case_aliases_order_and_mouse_names() {
        assert_eq!(
            canonical("Control+Option+Space"),
            canonical("space+alt+ctrl")
        );
        assert_eq!(canonical("command_left+k"), "k+super_left");
        assert_eq!(canonical("back"), canonical("MouseX1"));
        assert_eq!(canonical("ctrl+mouse5"), canonical("forward+control"));
        // A side is part of the key.
        assert_ne!(canonical("ctrl+alt"), canonical("ctrl_left+alt_left"));
        assert_eq!(canonical(" "), "");
    }

    #[test]
    fn keys_another_shortcut_holds_are_taken_either_way_round() {
        let settings = with_keys(&[
            ("transcribe", "ctrl_left+super", "mousex1"),
            ("assistant", "ctrl_left+alt_left", ""),
        ]);
        // Its other keys are as taken as its own.
        assert_eq!(
            conflict(&settings, "assistant", "back", Slot::Alternate),
            Some(Conflict::Other("transcribe".to_string()))
        );
        assert_eq!(
            conflict(&settings, "assistant", "super+ctrl_left", Slot::Alternate),
            Some(Conflict::Other("transcribe".to_string()))
        );
        assert_eq!(
            conflict(
                &settings,
                "transcribe",
                "ctrl_left+alt_left",
                Slot::Alternate
            ),
            Some(Conflict::Other("assistant".to_string()))
        );
        // Starting with another shortcut's keys is not having them.
        assert_eq!(
            conflict(
                &settings,
                "transcribe",
                "ctrl_left+alt_left+d",
                Slot::Alternate
            ),
            None
        );
    }

    #[test]
    fn a_shortcuts_two_sets_of_keys_must_differ() {
        let settings = with_keys(&[("transcribe", "ctrl_left+super", "mousex1")]);
        assert_eq!(
            conflict(&settings, "transcribe", "ctrl_left+super", Slot::Alternate),
            Some(Conflict::OwnOther)
        );
        assert_eq!(
            conflict(&settings, "transcribe", "mouse4", Slot::Primary),
            Some(Conflict::OwnOther)
        );
        // Its own keys are its own: setting them again is no conflict.
        assert_eq!(
            conflict(&settings, "transcribe", "ctrl_left+super", Slot::Primary),
            None
        );
    }

    #[test]
    fn a_shortcut_turned_off_holds_no_keys() {
        let settings = with_keys(&[("cancel", "", "f13")]);
        assert_eq!(
            conflict(&settings, "transcribe", "escape", Slot::Alternate),
            None
        );
        assert_eq!(
            conflict(&settings, "transcribe", "f13", Slot::Alternate),
            None
        );
    }

    /// An engine that refuses some keys, and remembers what it was asked for.
    struct Engine {
        taken: &'static [&'static str],
        asked: Vec<String>,
    }

    impl Engine {
        fn register(&mut self, combo: &str) -> Result<(), String> {
            self.asked.push(combo.to_string());
            if self.taken.contains(&combo) {
                Err(format!("{combo} is taken"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn own_keys_failing_registers_nothing_else() {
        let settings = with_keys(&[("transcribe", "ctrl+space", "f13")]);
        let mut engine = Engine {
            taken: &["ctrl+space"],
            asked: Vec::new(),
        };
        let armed = arm_with(
            &settings,
            &settings.bindings["transcribe"],
            KeyboardImplementation::HandyKeys,
            |combo| engine.register(combo),
        );
        assert_eq!(engine.asked, vec!["ctrl+space"]);
        assert!(armed.alternate.is_none());
        assert!(armed.into_result("transcribe").is_err());
    }

    #[test]
    fn other_keys_failing_keeps_the_shortcut_on_its_own_keys() {
        let settings = with_keys(&[("transcribe", "ctrl+space", "f13")]);
        let mut engine = Engine {
            taken: &["f13"],
            asked: Vec::new(),
        };
        let armed = arm_with(
            &settings,
            &settings.bindings["transcribe"],
            KeyboardImplementation::HandyKeys,
            |combo| engine.register(combo),
        );
        assert_eq!(engine.asked, vec!["ctrl+space", "f13"]);
        assert!(matches!(armed.alternate, Some(Err(_))));
        assert!(armed.into_result("transcribe").is_ok());
    }
}
