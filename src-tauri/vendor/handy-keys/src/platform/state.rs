//! Shared state for platform-specific keyboard listeners

use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::types::{Hotkey, Key, KeyEvent, Modifiers};

/// Hotkeys that should be blocked when triggered
pub type BlockingHotkeys = Arc<Mutex<HashSet<Hotkey>>>;

/// Internal state shared with platform-specific event callbacks.
///
/// Linux keeps its own (`platform::linux::tracker`): one engine reads every
/// device for every listener.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub struct ListenerState {
    pub event_sender: Sender<KeyEvent>,
    /// Track which modifiers are currently held
    pub current_modifiers: Modifiers,
    /// Hotkeys to block (if any)
    pub blocking_hotkeys: Option<BlockingHotkeys>,
    /// Mouse buttons whose press was kept from other applications, so their
    /// release is too. See [`swallow_mouse_button`].
    pub swallowed_buttons: HashSet<Key>,
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
impl ListenerState {
    pub fn new(event_sender: Sender<KeyEvent>, blocking_hotkeys: Option<BlockingHotkeys>) -> Self {
        Self {
            event_sender,
            current_modifiers: Modifiers::empty(),
            blocking_hotkeys,
            swallowed_buttons: HashSet::new(),
        }
    }

    /// Check if an event matches a blocking hotkey
    pub fn should_block(&self, modifiers: Modifiers, key: Option<Key>) -> bool {
        if let Some(ref hotkeys) = self.blocking_hotkeys {
            if let Ok(set) = hotkeys.lock() {
                return set
                    .iter()
                    .any(|h| h.modifiers.matches(modifiers) && h.key == key);
            }
        }
        false
    }

    /// Whether this mouse button event must be kept from other applications.
    /// See [`swallow_mouse_button`].
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub fn swallow_button(&mut self, modifiers: Modifiers, key: Key, is_down: bool) -> bool {
        swallow_mouse_button(
            &mut self.swallowed_buttons,
            self.blocking_hotkeys.as_ref(),
            modifiers,
            key,
            is_down,
        )
    }
}

/// Whether a mouse button event belongs to a registered hotkey and must be
/// kept from the application under the pointer.
///
/// A mouse button bound to dictation must not also do its usual job: Back
/// would leave the page being dictated into. Keys have done this all along
/// (a blocking listener keeps a matched hotkey from the focused app), and
/// buttons used to pass through regardless.
///
/// Decided when the button goes down and mirrored when it comes up, so an
/// application sees both halves of a click or neither. Re-deciding on the
/// release, against modifiers that may have changed in between, could hand an
/// application a release with no press or the reverse, and a button it then
/// believes is held. Left and right are never swallowed, whatever is
/// registered: no shortcut may take the primary clicks away from the mouse.
pub(crate) fn swallow_mouse_button(
    swallowed: &mut HashSet<Key>,
    blocking: Option<&BlockingHotkeys>,
    modifiers: Modifiers,
    key: Key,
    is_down: bool,
) -> bool {
    if !key.is_mouse_button() || matches!(key, Key::MouseLeft | Key::MouseRight) {
        return false;
    }
    if !is_down {
        return swallowed.remove(&key);
    }
    let bound = blocking.is_some_and(|hotkeys| {
        hotkeys.lock().is_ok_and(|set| {
            set.iter()
                .any(|h| h.key == Some(key) && h.modifiers.matches(modifiers))
        })
    });
    if bound {
        swallowed.insert(key);
    } else {
        swallowed.remove(&key);
    }
    bound
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(hotkeys: &[&str]) -> BlockingHotkeys {
        Arc::new(Mutex::new(
            hotkeys.iter().map(|h| h.parse().unwrap()).collect(),
        ))
    }

    #[test]
    fn a_bound_button_is_swallowed_down_and_up() {
        let blocking = registered(&["mousex1"]);
        let mut swallowed = HashSet::new();
        assert!(swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::empty(),
            Key::MouseX1,
            true
        ));
        assert!(swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::empty(),
            Key::MouseX1,
            false
        ));
        assert!(swallowed.is_empty());
    }

    #[test]
    fn an_unbound_button_passes_through() {
        let blocking = registered(&["mousex1"]);
        let mut swallowed = HashSet::new();
        for is_down in [true, false] {
            assert!(!swallow_mouse_button(
                &mut swallowed,
                Some(&blocking),
                Modifiers::empty(),
                Key::MouseX2,
                is_down
            ));
        }
    }

    /// The modifiers belong to the hotkey: Ctrl+Back is bound, so a plain Back
    /// is the browser's.
    #[test]
    fn modifiers_have_to_match() {
        let blocking = registered(&["ctrl_left+mousex1"]);
        let mut swallowed = HashSet::new();
        assert!(!swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::empty(),
            Key::MouseX1,
            true
        ));
        assert!(swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::CTRL_LEFT,
            Key::MouseX1,
            true
        ));
    }

    /// Letting go of Ctrl before the button must not hand the application a
    /// release whose press it never saw, nor keep the release when it did.
    #[test]
    fn the_release_follows_the_press_whatever_the_modifiers_do() {
        let blocking = registered(&["ctrl_left+mousex1"]);
        let mut swallowed = HashSet::new();
        assert!(swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::CTRL_LEFT,
            Key::MouseX1,
            true
        ));
        assert!(swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::empty(),
            Key::MouseX1,
            false
        ));

        // And the other way round: pressed plain, released with Ctrl held.
        assert!(!swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::empty(),
            Key::MouseX1,
            true
        ));
        assert!(!swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::CTRL_LEFT,
            Key::MouseX1,
            false
        ));
    }

    #[test]
    fn left_and_right_click_are_never_swallowed() {
        let blocking = registered(&["ctrl_left+mouseleft", "mouseright"]);
        let mut swallowed = HashSet::new();
        assert!(!swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::CTRL_LEFT,
            Key::MouseLeft,
            true
        ));
        assert!(!swallow_mouse_button(
            &mut swallowed,
            Some(&blocking),
            Modifiers::empty(),
            Key::MouseRight,
            true
        ));
    }

    /// A listener with nothing to block (the shortcut recorder) only watches.
    #[test]
    fn a_listening_only_listener_swallows_nothing() {
        let mut swallowed = HashSet::new();
        assert!(!swallow_mouse_button(
            &mut swallowed,
            None,
            Modifiers::empty(),
            Key::MouseMiddle,
            true
        ));
    }
}
