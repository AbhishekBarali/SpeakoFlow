//! Hands-free dictation: a dictation that keeps listening after the keys are let
//! go, until they are pressed again (or the pill's ✓, or Esc / ✕ to cancel).
//!
//! It is a state a dictation can be in, not a third recording mode. With Tap to
//! toggle every dictation is hands-free from the start. With Hold to talk, the
//! hands-free key (`settings::HANDS_FREE_BINDING_ID`, Space unless changed)
//! makes one so: pressed while the dictation keys are held, it keeps the
//! dictation going after they are let go. That is Wispr Flow's gesture
//! (Ctrl+Win+Space, Fn+Space), and our dictation keys are already Wispr's, so
//! someone switching keeps the habit.
//!
//! The key is stored on its own and registered on top of each dictation
//! shortcut's keys, so it follows them when they change and works the same for
//! dictation with cleanup. It only exists for dictation keys that are modifiers
//! alone: the hotkey engine has one main key per hotkey, so Ctrl+Space plus
//! another key is not a combination it can register, and a shortcut like that
//! has Tap to toggle instead. It is registered as a continuation
//! (`HotkeyManager::register_continuation`), because to the engine any key
//! typed onto a fresh modifier-only press is the start of another shortcut,
//! which would throw the dictation away. And only with Hold to talk: with Tap
//! there is nothing to make hands-free, and Ctrl+Win+Space stays Windows' own.
//!
//! Only dictation has any of this. The ask is held or tapped on its own
//! setting (`assistant_push_to_talk`), and a call is a call.

use handy_keys::Hotkey;

use crate::settings::{
    holds_to_record, AppSettings, KeyboardImplementation, CLEANUP_BINDING_ID, HANDS_FREE_BINDING_ID,
};

/// What a hands-free hotkey's binding id ends with: `transcribe` has
/// `transcribe#hands_free`. Never stored; it only names the derived hotkey.
const KEY_SUFFIX: &str = "#hands_free";

/// The shortcuts that dictate: plain dictation and dictation with cleanup.
pub fn is_dictation_binding(id: &str) -> bool {
    id == "transcribe" || id == CLEANUP_BINDING_ID
}

/// The id the hands-free hotkey of dictation shortcut `binding_id` is
/// registered under.
pub fn key_id(binding_id: &str) -> String {
    format!("{binding_id}{KEY_SUFFIX}")
}

/// The dictation shortcut a hands-free hotkey belongs to, or `None` when `id`
/// is not one.
pub fn dictation_of(id: &str) -> Option<&str> {
    id.strip_suffix(KEY_SUFFIX)
        .filter(|base| is_dictation_binding(base))
}

/// The one main key in `raw`, which is all the hands-free key is: the dictation
/// keys supply the modifiers. Recorded in Settings while the dictation keys are
/// held, it arrives as `ctrl_left+super+space`, which means Space. `None` for
/// modifiers alone, an empty binding, or keys the engine cannot read.
pub fn main_key(raw: &str) -> Option<String> {
    let hotkey: Hotkey = raw.trim().parse().ok()?;
    hotkey.key.map(|key| key.to_string().to_lowercase())
}

/// `keys` with the hands-free key on top, when `keys` are modifier keys alone:
/// `ctrl_left+super` and `space` give `ctrl_left+super+space`. `None` for keys
/// with a main key of their own, or a hands-free key that is none.
pub fn extend(keys: &str, hands_free_key: &str) -> Option<String> {
    let keys = keys.trim();
    let hotkey: Hotkey = keys.parse().ok()?;
    if hotkey.key.is_some() || hotkey.modifiers.is_empty() {
        return None;
    }
    let extended = format!("{keys}+{}", main_key(hands_free_key)?);
    extended.parse::<Hotkey>().ok()?;
    Some(extended)
}

/// The hands-free hotkey to register for dictation shortcut `binding_id` on
/// `keys`, if there should be one. `live` is every shortcut registered right
/// now, as (id, keys).
///
/// The keys come from what is registered rather than from `settings`: a
/// shortcut being changed is registered before its new keys are saved, and a
/// stored shortcut that is switched off holds nothing. A shortcut of the user's
/// own on the same keys wins, because the derived hotkey must never take a
/// combination someone chose on purpose.
pub fn keys_for<'a>(
    settings: &AppSettings,
    binding_id: &str,
    keys: &str,
    live: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    if settings.keyboard_implementation != KeyboardImplementation::HandyKeys
        || !is_dictation_binding(binding_id)
        || !holds_to_record(settings, binding_id)
    {
        return None;
    }
    let key = &settings
        .bindings
        .get(HANDS_FREE_BINDING_ID)?
        .current_binding;
    let extended = extend(keys, key)?;
    let hotkey: Hotkey = extended.parse().ok()?;
    let taken = live.into_iter().any(|(id, other)| {
        id != binding_id
            && other
                .parse::<Hotkey>()
                .is_ok_and(|other| same_keys(&other, &hotkey))
    });
    (!taken).then_some(extended)
}

/// Whether one press would fire both: `ctrl+super+space` (either Ctrl) and
/// `ctrl_left+super+space` are different hotkeys on the same keys.
fn same_keys(a: &Hotkey, b: &Hotkey) -> bool {
    a.key == b.key && (a.modifiers.matches(b.modifiers) || b.modifiers.matches(a.modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    fn on_hotkey_engine() -> AppSettings {
        let mut settings = get_default_settings();
        settings.keyboard_implementation = KeyboardImplementation::HandyKeys;
        settings.push_to_talk = true;
        settings
    }

    fn with_key(mut settings: AppSettings, key: &str) -> AppSettings {
        settings
            .bindings
            .get_mut(HANDS_FREE_BINDING_ID)
            .unwrap()
            .current_binding = key.to_string();
        settings
    }

    /// What `transcribe` gets on `keys` while `live` are registered.
    fn for_dictation(settings: &AppSettings, keys: &str, live: &[(&str, &str)]) -> Option<String> {
        keys_for(settings, "transcribe", keys, live.iter().copied())
    }

    #[test]
    fn the_hands_free_key_is_one_main_key() {
        assert_eq!(main_key("space").as_deref(), Some("space"));
        assert_eq!(main_key("Space").as_deref(), Some("space"));
        // Recorded while holding the dictation keys.
        assert_eq!(main_key("ctrl_left+super+space").as_deref(), Some("space"));
        assert_eq!(main_key("enter").as_deref(), Some("return"));
        assert_eq!(main_key("f13").as_deref(), Some("f13"));
        assert_eq!(main_key("shift"), None);
        assert_eq!(main_key(""), None);
        assert_eq!(main_key("not a key"), None);
    }

    #[test]
    fn modifier_keys_gain_the_hands_free_key() {
        assert_eq!(
            extend("ctrl_left+super", "space").as_deref(),
            Some("ctrl_left+super+space")
        );
        assert_eq!(extend("fn", "space").as_deref(), Some("fn+space"));
        assert_eq!(
            extend("ctrl_left+super+shift", "return").as_deref(),
            Some("ctrl_left+super+shift+return")
        );
        assert_eq!(extend("alt_right", "h").as_deref(), Some("alt_right+h"));
    }

    #[test]
    fn keys_with_a_main_key_have_no_hands_free_hotkey() {
        // Linux's default: Space is already the main key.
        assert_eq!(extend("ctrl+space", "space"), None);
        assert_eq!(extend("ctrl+shift+d", "space"), None);
        assert_eq!(extend("f13", "space"), None);
        assert_eq!(extend("", "space"), None);
        // Nor without a hands-free key, or with a modifier for one.
        assert_eq!(extend("ctrl_left+super", ""), None);
        assert_eq!(extend("ctrl_left+super", "shift"), None);
    }

    #[test]
    fn the_hands_free_key_names_its_dictation_shortcut() {
        assert_eq!(dictation_of(&key_id("transcribe")), Some("transcribe"));
        assert_eq!(
            dictation_of(&key_id(CLEANUP_BINDING_ID)),
            Some(CLEANUP_BINDING_ID)
        );
        assert_eq!(dictation_of("transcribe"), None);
        assert_eq!(dictation_of(&key_id("assistant")), None);
    }

    #[test]
    fn only_held_dictation_on_the_hotkey_engine_gets_one() {
        let settings = on_hotkey_engine();
        let live = [("transcribe", "ctrl_left+super")];
        assert_eq!(
            for_dictation(&settings, "ctrl_left+super", &live).as_deref(),
            Some("ctrl_left+super+space")
        );
        assert_eq!(
            keys_for(&settings, "assistant", "ctrl_left+alt_left", live),
            None
        );

        // Tapped, every dictation already is hands-free.
        let mut tapped = settings.clone();
        tapped.push_to_talk = false;
        assert_eq!(for_dictation(&tapped, "ctrl_left+super", &live), None);
        // The ask's own mode has nothing to do with it.
        let mut ask_tapped = settings.clone();
        ask_tapped.assistant_push_to_talk = false;
        assert!(for_dictation(&ask_tapped, "ctrl_left+super", &live).is_some());

        let mut tauri = settings;
        tauri.keyboard_implementation = KeyboardImplementation::Tauri;
        assert_eq!(for_dictation(&tauri, "ctrl_left+super", &live), None);
    }

    #[test]
    fn the_hands_free_key_is_the_users_to_change_or_remove() {
        let live = [("transcribe", "ctrl_left+super")];
        let enter = with_key(on_hotkey_engine(), "return");
        assert_eq!(
            for_dictation(&enter, "ctrl_left+super", &live).as_deref(),
            Some("ctrl_left+super+return")
        );
        let removed = with_key(on_hotkey_engine(), "");
        assert_eq!(for_dictation(&removed, "ctrl_left+super", &live), None);
    }

    #[test]
    fn a_shortcut_of_the_users_own_on_those_keys_wins() {
        let settings = on_hotkey_engine();
        let mine = |assistant| [("transcribe", "ctrl_left+super"), ("assistant", assistant)];
        assert_eq!(
            for_dictation(&settings, "ctrl_left+super", &mine("super+ctrl_left+space")),
            None
        );
        // Either Ctrl still fires on Left Ctrl.
        assert_eq!(
            for_dictation(&settings, "ctrl_left+super", &mine("ctrl+super+space")),
            None
        );
        // Something else on Space is no conflict.
        assert!(for_dictation(&settings, "ctrl_left+super", &mine("ctrl+alt+space")).is_some());
    }

    #[test]
    fn only_live_shortcuts_can_be_in_the_way() {
        // Stored but switched off (the assistant's master switch, a cleanup
        // shortcut not in use) is not registered, so not listed here.
        let settings = on_hotkey_engine();
        assert!(for_dictation(&settings, "ctrl_left+super", &[]).is_some());
    }
}
