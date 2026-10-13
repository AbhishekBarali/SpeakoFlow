//! Hands-free dictation: a dictation that keeps listening after the keys are let
//! go, until they are pressed again (or the pill's ✓, or Esc / ✕ to cancel).
//!
//! It is a state a dictation can be in, not a third recording mode. With Tap to
//! toggle every dictation is hands-free from the start. With Hold to talk a
//! dictation becomes hands-free in one of two ways:
//!
//! - **The dictation keys plus Space.** Pressed while the dictation keys are
//!   held, Space keeps the running dictation going after they are let go;
//!   pressed together from idle, the three start a hands-free dictation. This
//!   is Wispr Flow's hands-free key (Ctrl+Win+Space, Fn+Space), and our
//!   dictation keys are already Wispr's, so someone switching keeps the habit.
//!   It is derived from the dictation keys rather than stored as a shortcut of
//!   its own, so it follows them when they change. It only exists for a
//!   shortcut of modifier keys alone: Ctrl+Space plus Space is not a chord,
//!   and the hotkey engine has one main key per hotkey. Registered as a
//!   continuation (`HotkeyManager::register_continuation`), because to the
//!   engine any key typed onto a fresh modifier-only press is the start of
//!   another shortcut, which would throw the dictation away.
//! - **A quick tap** (`dictation_hands_free_tap`, off by default). For the
//!   shortcuts Space cannot extend: Linux's Ctrl+Space, a key with a letter in
//!   it. Off by default because it turns a stray tap into an open microphone.
//!   The press and release are timed where the hotkey engine delivers them
//!   (`TranscriptionCoordinator::send_input`), never when the coordinator gets
//!   to them: starting a recording opens the microphone on the coordinator's
//!   thread, and a cold or Bluetooth microphone can take longer than the whole
//!   tap, which would make every tap look like a hold.
//!
//! Only dictation has any of this. The assistant follows Hold or Tap as before.

use std::time::Duration;

use handy_keys::Hotkey;

use crate::settings::{AppSettings, KeyboardImplementation, CLEANUP_BINDING_ID};

/// A press of the dictation keys shorter than this is a tap, when
/// `dictation_hands_free_tap` is on. Holding to say even one word takes longer,
/// and a deliberate tap of two keys is well inside it.
pub const QUICK_TAP: Duration = Duration::from_millis(300);

/// What a hands-free key's binding id ends with: `transcribe` has
/// `transcribe#hands_free`. Never stored; it only names the derived hotkey.
const KEY_SUFFIX: &str = "#hands_free";

/// The shortcuts that dictate: plain dictation and dictation with cleanup.
pub fn is_dictation_binding(id: &str) -> bool {
    id == "transcribe" || id == CLEANUP_BINDING_ID
}

/// The id the hands-free key of dictation shortcut `binding_id` is registered
/// under.
pub fn key_id(binding_id: &str) -> String {
    format!("{binding_id}{KEY_SUFFIX}")
}

/// The dictation shortcut a hands-free key belongs to, or `None` when `id` is
/// not a hands-free key.
pub fn dictation_of(id: &str) -> Option<&str> {
    id.strip_suffix(KEY_SUFFIX)
        .filter(|base| is_dictation_binding(base))
}

/// The keys plus Space, when `keys` are modifier keys alone (`ctrl_left+super`
/// gives `ctrl_left+super+space`). `None` for anything with a main key of its
/// own, an empty binding, or one the engine cannot read.
pub fn space_keys(keys: &str) -> Option<String> {
    let keys = keys.trim();
    let hotkey: Hotkey = keys.parse().ok()?;
    if hotkey.key.is_some() || hotkey.modifiers.is_empty() {
        return None;
    }
    let extended = format!("{keys}+space");
    extended.parse::<Hotkey>().ok()?;
    Some(extended)
}

/// The hands-free key to register for dictation shortcut `binding_id` on
/// `keys`, if there should be one. `live` is every shortcut registered right
/// now, as (id, keys).
///
/// The keys come from what is registered rather than from `settings`: a
/// shortcut being changed is registered before its new keys are saved, and a
/// stored shortcut that is switched off holds nothing. A shortcut of the user's
/// own on the same keys wins, because the derived key must never take a
/// combination someone chose on purpose.
pub fn space_keys_for<'a>(
    settings: &AppSettings,
    binding_id: &str,
    keys: &str,
    live: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    if !settings.dictation_hands_free_space
        || settings.keyboard_implementation != KeyboardImplementation::HandyKeys
        || !is_dictation_binding(binding_id)
    {
        return None;
    }
    let extended = space_keys(keys)?;
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

/// Whether letting go of `binding_id` after holding it for `held` keeps the
/// dictation listening instead of ending it.
pub fn tap_goes_hands_free(settings: &AppSettings, binding_id: &str, held: Duration) -> bool {
    settings.dictation_hands_free_tap
        && settings.push_to_talk
        && is_dictation_binding(binding_id)
        && held < QUICK_TAP
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    fn on_hotkey_engine() -> AppSettings {
        let mut settings = get_default_settings();
        settings.keyboard_implementation = KeyboardImplementation::HandyKeys;
        settings.dictation_hands_free_space = true;
        settings
    }

    /// What `transcribe` gets on `keys` while `live` are registered.
    fn for_dictation(settings: &AppSettings, keys: &str, live: &[(&str, &str)]) -> Option<String> {
        space_keys_for(settings, "transcribe", keys, live.iter().copied())
    }

    #[test]
    fn modifier_keys_gain_space() {
        assert_eq!(
            space_keys("ctrl_left+super").as_deref(),
            Some("ctrl_left+super+space")
        );
        assert_eq!(space_keys("fn").as_deref(), Some("fn+space"));
        assert_eq!(
            space_keys("ctrl_left+super+shift").as_deref(),
            Some("ctrl_left+super+shift+space")
        );
    }

    #[test]
    fn keys_with_a_main_key_have_no_space_variant() {
        // Linux's default: Space is already the main key.
        assert_eq!(space_keys("ctrl+space"), None);
        assert_eq!(space_keys("ctrl+shift+d"), None);
        assert_eq!(space_keys("f13"), None);
        assert_eq!(space_keys(""), None);
        assert_eq!(space_keys("not a key"), None);
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
    fn only_dictation_on_the_hotkey_engine_gets_one() {
        let settings = on_hotkey_engine();
        let live = [("transcribe", "ctrl_left+super")];
        assert_eq!(
            for_dictation(&settings, "ctrl_left+super", &live).as_deref(),
            Some("ctrl_left+super+space")
        );
        assert_eq!(
            space_keys_for(&settings, "assistant", "ctrl_left+alt_left", live),
            None
        );

        let mut tauri = settings.clone();
        tauri.keyboard_implementation = KeyboardImplementation::Tauri;
        assert_eq!(for_dictation(&tauri, "ctrl_left+super", &live), None);

        let mut off = settings;
        off.dictation_hands_free_space = false;
        assert_eq!(for_dictation(&off, "ctrl_left+super", &live), None);
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

    #[test]
    fn a_quick_tap_goes_hands_free_only_when_asked_and_held() {
        let mut settings = get_default_settings();
        settings.push_to_talk = true;
        settings.dictation_hands_free_tap = true;
        let tap = Duration::from_millis(120);
        assert!(tap_goes_hands_free(&settings, "transcribe", tap));
        assert!(tap_goes_hands_free(&settings, CLEANUP_BINDING_ID, tap));
        assert!(!tap_goes_hands_free(&settings, "assistant", tap));
        assert!(!tap_goes_hands_free(&settings, "transcribe", QUICK_TAP));
        assert!(!tap_goes_hands_free(
            &settings,
            "transcribe",
            Duration::from_secs(2)
        ));

        settings.dictation_hands_free_tap = false;
        assert!(!tap_goes_hands_free(&settings, "transcribe", tap));
    }
}
