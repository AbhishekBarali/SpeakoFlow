//! Keeping the recording overlay on the display the user is working on.
//!
//! The overlay used to be placed once, on the monitor under the cursor at the
//! moment a state was shown, and then stay put. Start dictating with the pointer
//! on a left-hand monitor, move it to the right, and the live transcript kept
//! running on a screen nobody was looking at. So while the overlay is up it
//! follows the cursor from display to display.
//!
//! The rule: once the cursor has stayed on another display for [`FOLLOW_DWELL`],
//! the overlay **hops** there. The webview fades the pill out for
//! [`HOP_FADE_OUT`], the window moves while nothing is drawn, and the pill fades
//! back in on the new display. Sweeping the pointer across a monitor edge on the
//! way to something else moves nothing, because the dwell restarts every time the
//! cursor changes its mind.
//!
//! Two earlier versions each got half of this wrong. The first glided, with an
//! eased 360 ms animation at 16 ms frames; every frame was a round trip through
//! the event loop, so the glide stuttered whenever the main thread was busy —
//! which during a dictation it usually is. The second teleported after a 300 ms
//! dwell sampled at 50 ms, so the overlay arrived up to a third of a second after
//! the user's eyes had, and then appeared in a single frame. Both read as lag.
//! The dwell is now short enough to land inside the saccade to the other screen,
//! and the fade makes the one move look deliberate instead of like a glitch —
//! while still costing a single window move, not an animation.
//!
//! Everything here is pure — the clock and the displays are parameters — so the
//! behaviour is unit-tested without a window or a sleep. The thread that drives
//! it lives in `overlay.rs`.

use std::time::{Duration, Instant};

/// How long the cursor must stay on another display before the overlay hops
/// there. Long enough that overshooting a monitor edge by a few pixels, or
/// flicking straight through a display, moves nothing; short enough that the
/// hop is finished about when the user's eyes arrive on the new screen.
pub(crate) const FOLLOW_DWELL: Duration = Duration::from_millis(60);

/// How often the cursor is sampled. Only decides how soon a display change is
/// *noticed*; the dwell above is what gates the move. One `GetCursorPos` per
/// tick on Windows, so sampling this often is free.
pub(crate) const FOLLOW_POLL: Duration = Duration::from_millis(20);

/// How long the webview takes to fade the overlay out before the window moves.
/// Matches the `.is-hopping` transition in `RecordingOverlay.css`, so the move
/// happens while nothing is on screen.
pub(crate) const HOP_FADE_OUT: Duration = Duration::from_millis(60);

pub(crate) type Point = (f64, f64);

/// A display's bounds in physical pixels, with its scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MonitorBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

impl MonitorBounds {
    /// Is a point in physical pixels on this display? Half-open, so a point on
    /// the seam between two displays belongs to exactly one of them.
    fn contains_physical(&self, (x, y): Point) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// The same test with the display converted to logical points first.
    fn contains_logical(&self, (x, y): Point) -> bool {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let (mx, my) = (self.x / scale, self.y / scale);
        let (mw, mh) = (self.width / scale, self.height / scale);
        x >= mx && x < mx + mw && y >= my && y < my + mh
    }
}

/// The display a cursor position falls on, or `None` when it is on none of them.
///
/// Tauri reports monitor bounds in physical pixels, but the cursor is physical on
/// Windows and X11 (`GetCursorPos` / `XQueryPointer` in a DPI-aware process) and
/// logical points on macOS (`NSEvent::mouseLocation`). So the hit test runs in
/// each space, physical first, instead of guessing which one this platform is in.
///
/// Physical first because it is the only reading that is correct on a scaled
/// multi-monitor Windows desktop: pre-scaling the displays down to logical while
/// the cursor stays physical made every display look smaller than it is, so a
/// cursor in the right half of a 150% display matched nothing. The logical pass
/// is a no-op on an unscaled display, where the two spaces are identical.
pub(crate) fn display_under(displays: &[MonitorBounds], cursor: Point) -> Option<MonitorBounds> {
    displays
        .iter()
        .find(|display| display.contains_physical(cursor))
        .or_else(|| {
            displays
                .iter()
                .find(|display| display.contains_logical(cursor))
        })
        .copied()
}

/// Which screen edge the overlay sits against, with its offset in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Edge {
    Top(f64),
    Bottom(f64),
}

/// Where along that edge: centred, or in a corner with a margin in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Side {
    Center,
    Left(f64),
    Right(f64),
}

/// Top-left of an overlay of logical `size`, centred on `bounds` against `edge`.
///
/// `physical` picks the coordinate space of the answer, and it has to match the
/// space the platform positions windows in. On Windows that is physical pixels:
/// a *logical* position is converted with the scale factor of the display the
/// window is currently on, which is the wrong display whenever the overlay is
/// about to move to a monitor with a different scale — the old code computed a
/// point in the target's logical space and had it rescaled by the source's.
/// On macOS the native space is points, so logical is exact there.
#[cfg(test)]
pub(crate) fn placement_on(
    bounds: MonitorBounds,
    size: (f64, f64),
    edge: Edge,
    physical: bool,
) -> Point {
    placement_at(bounds, size, edge, Side::Center, physical)
}

/// [`placement_on`], with the overlay in a corner of that edge when `side`
/// asks for one. The margin scales with the display exactly as the edge offset
/// does, so a corner overlay keeps the same distance from both edges.
pub(crate) fn placement_at(
    bounds: MonitorBounds,
    size: (f64, f64),
    edge: Edge,
    side: Side,
    physical: bool,
) -> Point {
    // Everything is converted into one space before any arithmetic: the
    // monitor from physical to logical, or the overlay from logical to physical.
    let (to_space, from_physical) = if physical {
        (bounds.scale, 1.0)
    } else {
        (1.0, 1.0 / bounds.scale)
    };
    let (mx, my) = (bounds.x * from_physical, bounds.y * from_physical);
    let (mw, mh) = (bounds.width * from_physical, bounds.height * from_physical);
    let (w, h) = (size.0 * to_space, size.1 * to_space);
    let x = match side {
        Side::Center => mx + (mw - w) / 2.0,
        Side::Left(margin) => mx + margin * to_space,
        Side::Right(margin) => mx + mw - w - margin * to_space,
    };
    let y = match edge {
        Edge::Top(offset) => my + offset * to_space,
        Edge::Bottom(offset) => my + mh - h - offset * to_space,
    };
    (x, y)
}

/// Is `point` inside the rectangle at `origin` of `size`, give or take `slack`?
/// All four in the same space.
pub(crate) fn rect_contains(origin: Point, size: (f64, f64), slack: f64, point: Point) -> bool {
    point.0 >= origin.0 - slack
        && point.0 <= origin.0 + size.0 + slack
        && point.1 >= origin.1 - slack
        && point.1 <= origin.1 + size.1 + slack
}

/// The decision half of cursor following: given the display the overlay is on
/// and the display under the cursor, whether to move now, and where.
#[derive(Debug, Default)]
pub(crate) struct DisplayFollower {
    /// A display the cursor has settled on, and since when. Promoted to a move
    /// once it has held for [`FOLLOW_DWELL`].
    pending: Option<(MonitorBounds, Instant)>,
}

impl DisplayFollower {
    /// Advance one tick. `current` is the display the overlay is on;
    /// `under_cursor` is the display the cursor is on, or `None` when the cursor
    /// cannot be located (the overlay then stays where it is — falling back to
    /// the primary display would drag it away from the user). Returns the display
    /// to move the overlay to, once, when it should move.
    pub fn step(
        &mut self,
        current: MonitorBounds,
        under_cursor: Option<MonitorBounds>,
        now: Instant,
    ) -> Option<MonitorBounds> {
        let Some(target) = under_cursor.filter(|display| *display != current) else {
            // On the overlay's own display, or nowhere we can see: nothing to do,
            // and any half-finished dwell is void.
            self.pending = None;
            return None;
        };
        match self.pending {
            Some((wanted, since)) if wanted == target => {
                if now.saturating_duration_since(since) >= FOLLOW_DWELL {
                    self.pending = None;
                    Some(target)
                } else {
                    None
                }
            }
            // A new destination, or the cursor changed its mind: the dwell starts
            // over from now.
            _ => {
                self.pending = Some((target, now));
                None
            }
        }
    }
}

/// Where a hop that has already faded the overlay out should land, decided from
/// the cursor as it is *after* the fade, or `None` to fade back in where it is.
///
/// The fade takes [`HOP_FADE_OUT`], and the cursor keeps moving during it. Moving
/// to the display the follower picked regardless would put the overlay on a
/// screen the user has already left; so the cursor is read again, and a cursor
/// that went back to the overlay's own display cancels the move. A cursor that
/// cannot be located keeps the original decision — that sample was fine a moment
/// ago, and the overlay is already invisible.
pub(crate) fn hop_destination(
    current: MonitorBounds,
    chosen: MonitorBounds,
    under_cursor_now: Option<MonitorBounds>,
) -> Option<MonitorBounds> {
    let destination = under_cursor_now.unwrap_or(chosen);
    (destination != current).then_some(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: MonitorBounds = MonitorBounds {
        x: -1920.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
        scale: 1.0,
    };
    const MIDDLE: MonitorBounds = MonitorBounds {
        x: 0.0,
        y: 0.0,
        width: 2560.0,
        height: 1440.0,
        scale: 1.0,
    };
    const RIGHT: MonitorBounds = MonitorBounds {
        x: 2560.0,
        y: 0.0,
        width: 3840.0,
        height: 2160.0,
        scale: 1.5,
    };

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// The dwell in milliseconds, so the tests describe the rule rather than
    /// one particular tuning of it.
    fn dwell() -> u64 {
        FOLLOW_DWELL.as_millis() as u64
    }

    #[test]
    fn the_dwell_is_short_and_the_poll_can_see_it() {
        // The follow used to wait 300 ms sampled at 50 ms, which read as lag.
        // Anything near that again brings the complaint back.
        assert!(FOLLOW_DWELL <= Duration::from_millis(100));
        // Sampled more often than the dwell, or the poll is the real delay.
        assert!(FOLLOW_POLL < FOLLOW_DWELL);
        // Dwell plus fade must still land inside the eye's move to the new screen.
        assert!(FOLLOW_DWELL + FOLLOW_POLL + HOP_FADE_OUT <= Duration::from_millis(200));
    }

    #[test]
    fn logical_placement_matches_the_original_formula() {
        // 1920x1080 at 150%: 1280x720 logical.
        let bounds = MonitorBounds {
            x: 1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            scale: 1.5,
        };
        let (x, y) = placement_on(bounds, (400.0, 120.0), Edge::Bottom(40.0), false);
        assert_eq!(x, 1280.0 + (1280.0 - 400.0) / 2.0);
        assert_eq!(y, 720.0 - 120.0 - 40.0);
    }

    #[test]
    fn physical_placement_scales_the_overlay_by_the_target_display() {
        // A 150% display to the right of a 100% primary. The overlay must be
        // centred using the *target's* scale, whatever display it is on now.
        let (x, y) = placement_on(RIGHT, (400.0, 120.0), Edge::Bottom(40.0), true);
        assert_eq!(x, 2560.0 + (3840.0 - 600.0) / 2.0);
        assert_eq!(y, 2160.0 - 180.0 - 60.0);
        let (_, top) = placement_on(RIGHT, (400.0, 120.0), Edge::Top(4.0), true);
        assert_eq!(top, 6.0);
    }

    #[test]
    fn physical_and_logical_agree_on_an_unscaled_display() {
        let size = (96.0, 44.0);
        assert_eq!(
            placement_on(LEFT, size, Edge::Bottom(40.0), true),
            placement_on(LEFT, size, Edge::Bottom(40.0), false)
        );
    }

    #[test]
    fn a_corner_keeps_its_margin_from_the_side_edge() {
        // 1920x1080 at 150%: 1280x720 logical, starting at 1920 physical.
        let bounds = MonitorBounds {
            x: 1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            scale: 1.5,
        };
        let size = (96.0, 44.0);
        let (left, top) = placement_at(bounds, size, Edge::Top(4.0), Side::Left(16.0), false);
        assert_eq!((left, top), (1280.0 + 16.0, 4.0));
        let (right, bottom) =
            placement_at(bounds, size, Edge::Bottom(40.0), Side::Right(16.0), false);
        assert_eq!(right, 1280.0 + 1280.0 - 96.0 - 16.0);
        assert_eq!(bottom, 720.0 - 44.0 - 40.0);
        // In physical pixels the margin scales with the display, like the offset.
        let (left, _) = placement_at(bounds, size, Edge::Top(4.0), Side::Left(16.0), true);
        assert_eq!(left, 1920.0 + 24.0);
        let (right, _) = placement_at(bounds, size, Edge::Top(4.0), Side::Right(16.0), true);
        assert_eq!(right, 1920.0 + 1920.0 - 144.0 - 24.0);
    }

    #[test]
    fn centred_is_what_it_always_was() {
        let size = (400.0, 120.0);
        for physical in [true, false] {
            assert_eq!(
                placement_at(RIGHT, size, Edge::Bottom(40.0), Side::Center, physical),
                placement_on(RIGHT, size, Edge::Bottom(40.0), physical)
            );
        }
    }

    #[test]
    fn the_cursor_is_found_on_the_display_it_is_on() {
        let displays = [LEFT, MIDDLE, RIGHT];
        assert_eq!(display_under(&displays, (-5.0, 500.0)), Some(LEFT));
        assert_eq!(display_under(&displays, (0.0, 0.0)), Some(MIDDLE));
        // The right half of a 150% display — the case pre-scaling used to miss.
        assert_eq!(display_under(&displays, (6000.0, 2000.0)), Some(RIGHT));
        assert_eq!(display_under(&displays, (0.0, 5000.0)), None);
    }

    #[test]
    fn a_logical_cursor_still_finds_a_scaled_display() {
        // macOS reports points: 3000pt on a 2x display that starts at 4000px.
        let retina = MonitorBounds {
            x: 4000.0,
            y: 0.0,
            width: 3000.0,
            height: 2000.0,
            scale: 2.0,
        };
        assert_eq!(display_under(&[retina], (2100.0, 500.0)), Some(retina));
    }

    #[test]
    fn stays_put_while_the_cursor_is_on_its_display() {
        let mut follower = DisplayFollower::default();
        let start = Instant::now();
        for ms in (0..2000).step_by(50) {
            assert_eq!(follower.step(MIDDLE, Some(MIDDLE), at(start, ms)), None);
        }
    }

    #[test]
    fn an_unknown_cursor_never_moves_the_overlay() {
        let mut follower = DisplayFollower::default();
        let start = Instant::now();
        assert_eq!(follower.step(MIDDLE, None, start), None);
        assert_eq!(follower.step(MIDDLE, None, at(start, 1000)), None);
    }

    #[test]
    fn moves_in_one_step_once_the_cursor_has_settled() {
        let mut follower = DisplayFollower::default();
        let start = Instant::now();
        let d = dwell();
        // Noticed, but not yet acted on.
        assert_eq!(follower.step(MIDDLE, Some(RIGHT), start), None);
        assert_eq!(follower.step(MIDDLE, Some(RIGHT), at(start, d / 3)), None);
        assert_eq!(follower.step(MIDDLE, Some(RIGHT), at(start, d - 1)), None);
        // Dwell elapsed: one move, straight to the destination.
        assert_eq!(
            follower.step(MIDDLE, Some(RIGHT), at(start, d)),
            Some(RIGHT)
        );
        // Arrived: nothing more to do, however long the cursor stays.
        assert_eq!(follower.step(RIGHT, Some(RIGHT), at(start, d + 20)), None);
        assert_eq!(follower.step(RIGHT, Some(RIGHT), at(start, d + 5000)), None);
    }

    #[test]
    fn a_brief_crossing_does_not_drag_the_overlay_along() {
        let mut follower = DisplayFollower::default();
        let start = Instant::now();
        let d = dwell();
        assert_eq!(follower.step(MIDDLE, Some(RIGHT), start), None);
        // Back before the dwell ran out.
        assert_eq!(follower.step(MIDDLE, Some(MIDDLE), at(start, d / 2)), None);
        // Crossing again restarts the dwell rather than inheriting the old one.
        let again = d / 2 + 10;
        assert_eq!(follower.step(MIDDLE, Some(RIGHT), at(start, again)), None);
        assert_eq!(
            follower.step(MIDDLE, Some(RIGHT), at(start, again + d - 1)),
            None
        );
        assert_eq!(
            follower.step(MIDDLE, Some(RIGHT), at(start, again + d)),
            Some(RIGHT)
        );
    }

    #[test]
    fn passing_through_a_display_on_the_way_to_another_restarts_the_dwell() {
        let mut follower = DisplayFollower::default();
        let start = Instant::now();
        let d = dwell();
        // Left edge → through the middle → settles on the right.
        assert_eq!(follower.step(LEFT, Some(MIDDLE), start), None);
        let arrived = d * 2 / 3;
        assert_eq!(follower.step(LEFT, Some(RIGHT), at(start, arrived)), None);
        // A full dwell after first seeing the middle display, but not yet a
        // full dwell on the right one: not yet.
        assert_eq!(follower.step(LEFT, Some(RIGHT), at(start, d)), None);
        assert_eq!(
            follower.step(LEFT, Some(RIGHT), at(start, arrived + d)),
            Some(RIGHT)
        );
    }

    #[test]
    fn losing_the_cursor_mid_dwell_cancels_the_move() {
        let mut follower = DisplayFollower::default();
        let start = Instant::now();
        let d = dwell();
        assert_eq!(follower.step(MIDDLE, Some(RIGHT), start), None);
        assert_eq!(follower.step(MIDDLE, None, at(start, d / 2)), None);
        // The cursor reappears on the right: that is a fresh dwell.
        let back = d / 2 + 20;
        assert_eq!(follower.step(MIDDLE, Some(RIGHT), at(start, back)), None);
        assert_eq!(
            follower.step(MIDDLE, Some(RIGHT), at(start, back + d)),
            Some(RIGHT)
        );
    }

    #[test]
    fn a_hop_lands_where_the_cursor_is_after_the_fade() {
        // Still on the chosen display: go there.
        assert_eq!(hop_destination(MIDDLE, RIGHT, Some(RIGHT)), Some(RIGHT));
        // Kept going to a third display during the fade: go there instead.
        assert_eq!(hop_destination(LEFT, MIDDLE, Some(RIGHT)), Some(RIGHT));
        // Came back to the overlay's own display: fade back in, no move.
        assert_eq!(hop_destination(MIDDLE, RIGHT, Some(MIDDLE)), None);
        // Lost the cursor: the decision from a moment ago stands.
        assert_eq!(hop_destination(MIDDLE, RIGHT, None), Some(RIGHT));
    }

    #[test]
    fn the_overlay_rect_test_allows_a_little_slack() {
        let origin = (100.0, 200.0);
        let size = (400.0, 120.0);
        assert!(rect_contains(origin, size, 0.0, (100.0, 200.0)));
        assert!(rect_contains(origin, size, 0.0, (500.0, 320.0)));
        assert!(!rect_contains(origin, size, 0.0, (99.0, 250.0)));
        assert!(rect_contains(origin, size, 4.0, (97.0, 250.0)));
        assert!(!rect_contains(origin, size, 4.0, (300.0, 330.0)));
    }
}
