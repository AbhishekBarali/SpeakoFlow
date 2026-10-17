//! What the hotkey logic knows about the keys held on each input device, and
//! which key events reach the compositor. No I/O here, so every rule is
//! unit-tested; `engine.rs` feeds it.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::types::{Key, KeyEvent, Modifiers};

/// An input device, as the engine numbers them. Never reused within a run.
pub(crate) type DeviceId = u64;

/// evdev key event values.
pub(crate) const RELEASE: i32 = 0;
pub(crate) const PRESS: i32 = 1;
pub(crate) const REPEAT: i32 = 2;

/// Whether a key event from a taken-over keyboard is passed on to the
/// compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Forward,
    Swallow,
}

/// What a device is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// Taken over (`EVIOCGRAB`) and re-sent through a uinput mirror, so the
    /// key a hotkey ends in can be withheld from the focused app.
    Grab,
    /// Only read; the compositor keeps receiving it directly.
    Listen,
}

/// A device's role, from what it has.
///
/// `hotkey_keys`: a key a hotkey can end in (anything but a modifier).
/// `followed`: a modifier or a mouse button the hotkey logic follows.
/// `made_by_uinput`: another program created it in software.
///
/// A device another program made is only read, never taken over. It is a
/// remapper's output (keyd), a typing tool's keyboard (ydotool, dotool) or
/// another listener's mirror; taking those over gets in the way of the program
/// that made them, and two mirroring listeners would mirror each other's
/// mirrors without end.
pub(crate) fn role_for(hotkey_keys: bool, followed: bool, made_by_uinput: bool) -> Option<Role> {
    if hotkey_keys && !made_by_uinput {
        Some(Role::Grab)
    } else if hotkey_keys || followed {
        Some(Role::Listen)
    } else {
        None
    }
}

/// When to try opening a device node again after it refused us. udev hands a
/// new node to the `input` group about 100 ms after it appears and says so
/// with an attribute change, which retries at once; this schedule is the
/// fallback. `None`: wait for the node to change.
pub(crate) fn open_retry_delay(attempt: u32) -> Option<Duration> {
    const SCHEDULE_MS: [u64; 6] = [100, 250, 500, 1_000, 2_000, 4_000];
    SCHEDULE_MS
        .get(attempt.saturating_sub(1) as usize)
        .map(|&ms| Duration::from_millis(ms))
}

/// When to try again after a keyboard could not be taken over (another
/// program holds it) or mirrored.
pub(crate) fn takeover_retry_delay(attempt: u32) -> Duration {
    let doublings = attempt.saturating_sub(1).min(5);
    Duration::from_secs(1 << doublings).min(Duration::from_secs(30))
}

struct DeviceKeys {
    /// Modifiers held on this device.
    modifiers: Modifiers,
    /// Keys whose press was withheld. Their repeats and release are withheld
    /// too, whatever is held by then. Deciding again on release is how a key
    /// ends up pressed forever on the other side: pressed with no hotkey
    /// matching (passed on), released once Ctrl was down (withheld).
    withheld: HashSet<u16>,
}

impl Default for DeviceKeys {
    fn default() -> Self {
        Self {
            modifiers: Modifiers::empty(),
            withheld: HashSet::new(),
        }
    }
}

/// Key state across every device the engine reads.
pub(crate) struct Tracker {
    devices: HashMap<DeviceId, DeviceKeys>,
    /// Modifiers held on any device.
    modifiers: Modifiers,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            devices: HashMap::new(),
            modifiers: Modifiers::empty(),
        }
    }
}

impl Tracker {
    /// Recompute the combined modifiers; returns the previous ones.
    fn recombine(&mut self) -> Modifiers {
        let previous = self.modifiers;
        self.modifiers = self
            .devices
            .values()
            .fold(Modifiers::empty(), |held, keys| held | keys.modifiers);
        previous
    }

    /// A modifier key went down or up on `device`. Returns the event for the
    /// listeners when the modifiers held across all devices changed: holding
    /// Ctrl on two keyboards and letting go of one changes nothing.
    pub(crate) fn modifier(
        &mut self,
        device: DeviceId,
        modifier: Modifiers,
        down: bool,
    ) -> Option<KeyEvent> {
        self.devices
            .entry(device)
            .or_default()
            .modifiers
            .set(modifier, down);
        let previous = self.recombine();
        (self.modifiers != previous).then_some(KeyEvent {
            modifiers: self.modifiers,
            key: None,
            is_key_down: down,
            changed_modifier: Some(modifier),
        })
    }

    /// A key a hotkey can end in, with its evdev `code` and event `value`.
    /// `block_press` decides a press, from the modifiers held and the key; a
    /// repeat or release follows its press. Repeats read as the key going down
    /// again, as they did with rdev.
    pub(crate) fn key(
        &mut self,
        device: DeviceId,
        code: u16,
        key: Key,
        value: i32,
        block_press: impl FnOnce(Modifiers, Key) -> bool,
    ) -> (Verdict, KeyEvent) {
        let modifiers = self.modifiers;
        let keys = self.devices.entry(device).or_default();
        let withheld = match value {
            RELEASE => keys.withheld.remove(&code),
            PRESS => {
                let block = block_press(modifiers, key);
                if block {
                    keys.withheld.insert(code);
                } else {
                    keys.withheld.remove(&code);
                }
                block
            }
            _ => keys.withheld.contains(&code),
        };
        let verdict = if withheld {
            Verdict::Swallow
        } else {
            Verdict::Forward
        };
        let event = KeyEvent {
            modifiers,
            key: Some(key),
            is_key_down: value != RELEASE,
            changed_modifier: None,
        };
        (verdict, event)
    }

    /// A mouse button. Plain left and right clicks only matter with a
    /// modifier held (Ctrl+click is not a hotkey tap).
    pub(crate) fn button(&self, key: Key, down: bool) -> Option<KeyEvent> {
        let plain_click = matches!(key, Key::MouseLeft | Key::MouseRight);
        (!plain_click || !self.modifiers.is_empty()).then_some(KeyEvent {
            modifiers: self.modifiers,
            key: Some(key),
            is_key_down: down,
            changed_modifier: None,
        })
    }

    /// The device was handed back to the compositor: the releases of keys
    /// withheld so far now go straight to it, which never saw their presses
    /// (harmless), so stop expecting them here.
    pub(crate) fn forget_withheld(&mut self, device: DeviceId) {
        if let Some(keys) = self.devices.get_mut(&device) {
            keys.withheld.clear();
        }
    }

    /// The device is gone. Returns a release for any modifier only it held.
    pub(crate) fn remove(&mut self, device: DeviceId) -> Option<KeyEvent> {
        self.devices.remove(&device)?;
        let previous = self.recombine();
        let lost = previous & !self.modifiers;
        (!lost.is_empty()).then_some(KeyEvent {
            modifiers: self.modifiers,
            key: None,
            is_key_down: false,
            changed_modifier: Some(lost),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: DeviceId = 1;
    const B: DeviceId = 2;
    const SEMICOLON: u16 = 39;

    fn never(_: Modifiers, _: Key) -> bool {
        false
    }

    fn always(_: Modifiers, _: Key) -> bool {
        true
    }

    #[test]
    fn a_withheld_press_withholds_its_repeats_and_release_whatever_is_held_by_then() {
        let mut keys = Tracker::default();
        keys.modifier(A, Modifiers::CTRL_LEFT, true);
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, PRESS, always).0,
            Verdict::Swallow
        );
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, REPEAT, never).0,
            Verdict::Swallow
        );
        // Ctrl let go first: the hotkey no longer matches, the release is
        // still withheld, or the compositor would see `;` come up unpressed.
        keys.modifier(A, Modifiers::CTRL_LEFT, false);
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, RELEASE, never).0,
            Verdict::Swallow
        );
        // That press is over: the next one is decided afresh.
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, PRESS, never).0,
            Verdict::Forward
        );
    }

    #[test]
    fn a_press_let_through_lets_its_release_through_even_once_it_matches() {
        let mut keys = Tracker::default();
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, PRESS, never).0,
            Verdict::Forward
        );
        keys.modifier(A, Modifiers::CTRL_LEFT, true);
        // Withholding these would leave `;` held in the compositor for good.
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, REPEAT, always).0,
            Verdict::Forward
        );
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, RELEASE, always).0,
            Verdict::Forward
        );
    }

    #[test]
    fn a_press_is_judged_with_the_modifiers_held_on_every_keyboard() {
        let mut keys = Tracker::default();
        keys.modifier(A, Modifiers::CTRL_LEFT, true);
        let mut judged = None;
        keys.key(B, SEMICOLON, Key::Semicolon, PRESS, |modifiers, _| {
            judged = Some(modifiers);
            false
        });
        assert_eq!(judged, Some(Modifiers::CTRL_LEFT));
    }

    #[test]
    fn key_events_carry_the_modifiers_held() {
        let mut keys = Tracker::default();
        keys.modifier(A, Modifiers::SHIFT_LEFT, true);
        let (_, event) = keys.key(A, SEMICOLON, Key::Semicolon, PRESS, never);
        assert_eq!(event.modifiers, Modifiers::SHIFT_LEFT);
        assert_eq!(event.key, Some(Key::Semicolon));
        assert!(event.is_key_down);
        let (_, event) = keys.key(A, SEMICOLON, Key::Semicolon, REPEAT, never);
        assert!(event.is_key_down);
        let (_, event) = keys.key(A, SEMICOLON, Key::Semicolon, RELEASE, never);
        assert!(!event.is_key_down);
    }

    #[test]
    fn a_modifier_held_on_two_keyboards_is_released_by_the_last() {
        let mut keys = Tracker::default();
        let down = keys
            .modifier(A, Modifiers::CTRL_LEFT, true)
            .expect("first Ctrl");
        assert!(down.is_key_down);
        assert_eq!(down.changed_modifier, Some(Modifiers::CTRL_LEFT));
        assert!(keys.modifier(B, Modifiers::CTRL_LEFT, true).is_none());
        assert!(
            keys.modifier(A, Modifiers::CTRL_LEFT, false).is_none(),
            "still held on B"
        );
        let up = keys
            .modifier(B, Modifiers::CTRL_LEFT, false)
            .expect("last Ctrl");
        assert!(!up.is_key_down);
        assert!(up.modifiers.is_empty());
    }

    #[test]
    fn a_keyboard_that_goes_away_releases_what_only_it_held() {
        let mut keys = Tracker::default();
        keys.modifier(A, Modifiers::CMD_LEFT, true);
        keys.modifier(B, Modifiers::CTRL_LEFT, true);
        let event = keys
            .remove(A)
            .expect("Super was held on the keyboard that went away");
        assert_eq!(event.modifiers, Modifiers::CTRL_LEFT);
        assert_eq!(event.changed_modifier, Some(Modifiers::CMD_LEFT));
        assert!(!event.is_key_down);
        assert!(keys.remove(A).is_none());
        assert!(keys.remove(B).is_some());
    }

    #[test]
    fn a_keyboard_handed_back_lets_its_withheld_releases_through() {
        let mut keys = Tracker::default();
        keys.key(A, SEMICOLON, Key::Semicolon, PRESS, always);
        keys.forget_withheld(A);
        assert_eq!(
            keys.key(A, SEMICOLON, Key::Semicolon, RELEASE, never).0,
            Verdict::Forward
        );
    }

    #[test]
    fn plain_clicks_are_only_reported_with_a_modifier_held() {
        let mut keys = Tracker::default();
        assert!(keys.button(Key::MouseLeft, true).is_none());
        assert!(keys.button(Key::MouseMiddle, true).is_some());
        keys.modifier(A, Modifiers::CTRL_LEFT, true);
        let click = keys.button(Key::MouseLeft, true).expect("Ctrl+click");
        assert_eq!(click.modifiers, Modifiers::CTRL_LEFT);
    }

    #[test]
    fn only_hardware_keyboards_are_taken_over() {
        assert_eq!(role_for(true, true, false), Some(Role::Grab));
        assert_eq!(role_for(true, false, false), Some(Role::Grab));
        assert_eq!(
            role_for(true, true, true),
            Some(Role::Listen),
            "keyd, ydotool, a mirror"
        );
        assert_eq!(
            role_for(false, true, false),
            Some(Role::Listen),
            "a mouse or touchpad"
        );
        assert_eq!(
            role_for(false, false, false),
            None,
            "a lid switch, a power button"
        );
    }

    #[test]
    fn retries_back_off_and_stop() {
        let opens: Vec<_> = (1..=7)
            .map(|attempt| open_retry_delay(attempt).map(|delay| delay.as_millis()))
            .collect();
        assert_eq!(
            opens,
            [
                Some(100),
                Some(250),
                Some(500),
                Some(1_000),
                Some(2_000),
                Some(4_000),
                None
            ]
        );
        let takeovers: Vec<_> = (1..=7)
            .map(|attempt| takeover_retry_delay(attempt).as_secs())
            .collect();
        assert_eq!(takeovers, [1, 2, 4, 8, 16, 30, 30]);
    }
}
