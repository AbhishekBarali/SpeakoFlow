//! Keeping the recording overlay on the display the user is working on.
//!
//! The overlay used to be placed once, on the monitor under the cursor at the
//! moment a state was shown, and then stay put. Start dictating with the pointer
//! on a left-hand monitor, move it to the right, and the live transcript kept
//! running on a screen nobody was looking at. So while the overlay is up it now
//! follows the cursor from display to display.
//!
//! Two things make following tolerable rather than irritating, and both live in
//! [`OverlayFollower`]: it waits for the cursor to *settle* on another display
//! before setting off ([`FOLLOW_DWELL`]), so sweeping the pointer across a
//! monitor edge on the way to something else drags nothing along; and it
//! *glides* there ([`FOLLOW_GLIDE`], eased at both ends) instead of teleporting,
//! so the eye can track where it went.
//!
//! Everything here is pure — the clock and the window's position are parameters
//! — so the behaviour is unit-tested without a window or a sleep. The thread that
//! drives it lives in `overlay.rs`.

use std::time::{Duration, Instant};

/// How long the cursor must stay on another display before the overlay sets off
/// after it. Long enough that crossing a monitor edge in passing does not move
/// anything; short enough that it still reads as following, not catching up.
pub(crate) const FOLLOW_DWELL: Duration = Duration::from_millis(160);

/// The move itself. Eased at both ends so it neither jumps off the mark nor
/// slams into place — the difference between a window that glides and one that
/// is thrown across the desk.
pub(crate) const FOLLOW_GLIDE: Duration = Duration::from_millis(360);

/// How often the cursor is sampled while nothing is moving. Only decides how
/// soon a display change is *noticed*; the dwell above is what gates the move.
pub(crate) const FOLLOW_POLL: Duration = Duration::from_millis(60);

/// Frame interval while a glide is running.
pub(crate) const FOLLOW_FRAME: Duration = Duration::from_millis(16);

/// Distances under this are the same place: DPI conversion and rounding to
/// whole pixels never land exactly on the computed point.
const SAME_PLACE: f64 = 2.0;

/// A window that did not arrive this far from where a glide sent it was not
/// moved at all — the window manager ignores client positioning.
const DID_NOT_ARRIVE: f64 = 8.0;

/// Two glides in a row that went nowhere, and the follower stops trying rather
/// than asking a compositor that ignores it to move a window at 60 Hz.
const MAX_IGNORED_GLIDES: u8 = 2;

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

/// Which screen edge the overlay sits against, with its offset in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Edge {
    Top(f64),
    Bottom(f64),
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
pub(crate) fn placement_on(
    bounds: MonitorBounds,
    size: (f64, f64),
    edge: Edge,
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
    let x = mx + (mw - w) / 2.0;
    let y = match edge {
        Edge::Top(offset) => my + offset * to_space,
        Edge::Bottom(offset) => my + mh - h - offset * to_space,
    };
    (x, y)
}

fn distance(a: Point, b: Point) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

#[derive(Clone, Copy, Debug)]
struct Glide {
    from: Point,
    to: Point,
    started: Instant,
}

impl Glide {
    fn progress(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f64();
        (elapsed / FOLLOW_GLIDE.as_secs_f64()).clamp(0.0, 1.0)
    }

    fn position_at(&self, now: Instant) -> Point {
        let k = ease_in_out_cubic(self.progress(now));
        (
            self.from.0 + (self.to.0 - self.from.0) * k,
            self.from.1 + (self.to.1 - self.from.1) * k,
        )
    }
}

/// The decision half of cursor following: given where the window is and where
/// it belongs, what to do this tick.
#[derive(Debug, Default)]
pub(crate) struct OverlayFollower {
    glide: Option<Glide>,
    /// A destination the cursor has asked for, and since when. Promoted to a
    /// glide once it has held for [`FOLLOW_DWELL`].
    pending: Option<(Point, Instant)>,
    /// Where the last completed glide put the window, checked on the next tick.
    landed: Option<Point>,
    ignored: u8,
}

impl OverlayFollower {
    pub fn is_gliding(&self) -> bool {
        self.glide.is_some()
    }

    /// True once the window manager has shown it ignores our positioning.
    pub fn given_up(&self) -> bool {
        self.ignored >= MAX_IGNORED_GLIDES
    }

    /// Advance one tick. `current` is where the window actually is; `target` is
    /// where it belongs for the display under the cursor, or `None` when the
    /// cursor cannot be located (the overlay then stays where it is — falling
    /// back to the primary display would drag it away from the user). Returns
    /// the position to move the window to now, if it should move.
    pub fn step(&mut self, current: Point, target: Option<Point>, now: Instant) -> Option<Point> {
        if self.given_up() {
            return None;
        }
        if let Some(landed) = self.landed.take() {
            if distance(current, landed) > DID_NOT_ARRIVE {
                self.ignored += 1;
                if self.given_up() {
                    self.glide = None;
                    self.pending = None;
                    return None;
                }
            } else {
                self.ignored = 0;
            }
        }

        let (here, heading) = match &self.glide {
            Some(glide) => (glide.position_at(now), glide.to),
            None => (current, current),
        };
        match target {
            Some(target) if distance(target, heading) > SAME_PLACE => match self.pending {
                Some((wanted, since)) if distance(wanted, target) <= SAME_PLACE => {
                    if now.saturating_duration_since(since) >= FOLLOW_DWELL {
                        self.pending = None;
                        // Retargeting mid-glide starts from wherever the window
                        // is now, so turning back never jumps.
                        self.glide = Some(Glide {
                            from: here,
                            to: target,
                            started: now,
                        });
                    }
                }
                // A new destination, or the cursor changed its mind: the dwell
                // starts over.
                _ => self.pending = Some((target, now)),
            },
            // Already there or already on the way, or the cursor is unknown.
            _ => self.pending = None,
        }

        let glide = self.glide?;
        if glide.progress(now) >= 1.0 {
            self.glide = None;
            self.landed = Some(glide.to);
            Some(glide.to)
        } else {
            Some(glide.position_at(now))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: Point = (-1500.0, 1000.0);
    const RIGHT: Point = (1000.0, 1000.0);

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
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
        let bounds = MonitorBounds {
            x: 2560.0,
            y: 0.0,
            width: 3840.0,
            height: 2160.0,
            scale: 1.5,
        };
        let (x, y) = placement_on(bounds, (400.0, 120.0), Edge::Bottom(40.0), true);
        assert_eq!(x, 2560.0 + (3840.0 - 600.0) / 2.0);
        assert_eq!(y, 2160.0 - 180.0 - 60.0);
        let (_, top) = placement_on(bounds, (400.0, 120.0), Edge::Top(4.0), true);
        assert_eq!(top, 6.0);
    }

    #[test]
    fn physical_and_logical_agree_on_an_unscaled_display() {
        let bounds = MonitorBounds {
            x: -1920.0,
            y: 120.0,
            width: 1920.0,
            height: 1080.0,
            scale: 1.0,
        };
        let size = (96.0, 44.0);
        assert_eq!(
            placement_on(bounds, size, Edge::Bottom(40.0), true),
            placement_on(bounds, size, Edge::Bottom(40.0), false)
        );
    }

    #[test]
    fn stays_put_while_the_cursor_is_on_its_display() {
        let mut follower = OverlayFollower::default();
        let start = Instant::now();
        for ms in (0..2000).step_by(60) {
            assert_eq!(follower.step(LEFT, Some(LEFT), at(start, ms)), None);
        }
    }

    #[test]
    fn an_unknown_cursor_never_moves_the_overlay() {
        let mut follower = OverlayFollower::default();
        let start = Instant::now();
        assert_eq!(follower.step(LEFT, None, start), None);
        assert_eq!(follower.step(LEFT, None, at(start, 1000)), None);
    }

    #[test]
    fn waits_for_the_cursor_to_settle_then_glides_smoothly_there() {
        let mut follower = OverlayFollower::default();
        let start = Instant::now();
        // Noticed, but not yet acted on.
        assert_eq!(follower.step(LEFT, Some(RIGHT), start), None);
        assert_eq!(follower.step(LEFT, Some(RIGHT), at(start, 100)), None);
        // Dwell elapsed: the glide begins where the window is.
        let t0 = at(start, FOLLOW_DWELL.as_millis() as u64);
        assert_eq!(follower.step(LEFT, Some(RIGHT), t0), Some(LEFT));
        assert!(follower.is_gliding());

        // Every frame moves forward, never past the target, and the first and
        // last frames move less than the middle ones (eased at both ends).
        let mut last = LEFT;
        let mut steps = Vec::new();
        let frames = FOLLOW_GLIDE.as_millis() as u64 / 16;
        for frame in 1..=frames + 1 {
            let now = t0 + Duration::from_millis(frame * 16);
            let next = follower.step(last, Some(RIGHT), now).expect("gliding");
            assert!(next.0 >= last.0 && next.0 <= RIGHT.0);
            steps.push(next.0 - last.0);
            last = next;
        }
        assert_eq!(last, RIGHT);
        assert!(!follower.is_gliding());
        let peak = steps.iter().cloned().fold(0.0, f64::max);
        assert!(steps[0] < peak / 4.0);
        assert!(steps[steps.len() - 2] < peak / 4.0);

        // Arrived: nothing more to do.
        let after = t0 + FOLLOW_GLIDE + Duration::from_millis(100);
        assert_eq!(follower.step(RIGHT, Some(RIGHT), after), None);
    }

    #[test]
    fn a_brief_crossing_does_not_drag_the_overlay_along() {
        let mut follower = OverlayFollower::default();
        let start = Instant::now();
        assert_eq!(follower.step(LEFT, Some(RIGHT), start), None);
        // Back before the dwell ran out.
        assert_eq!(follower.step(LEFT, Some(LEFT), at(start, 100)), None);
        // Crossing again restarts the dwell rather than inheriting the old one.
        assert_eq!(follower.step(LEFT, Some(RIGHT), at(start, 200)), None);
        assert_eq!(follower.step(LEFT, Some(RIGHT), at(start, 300)), None);
        assert!(!follower.is_gliding());
    }

    #[test]
    fn turning_back_mid_glide_starts_from_where_the_window_is() {
        let mut follower = OverlayFollower::default();
        let start = Instant::now();
        follower.step(LEFT, Some(RIGHT), start);
        let t0 = at(start, FOLLOW_DWELL.as_millis() as u64);
        follower.step(LEFT, Some(RIGHT), t0);
        let halfway = t0 + FOLLOW_GLIDE / 2;
        let mid = follower.step(LEFT, Some(RIGHT), halfway).unwrap();

        // The cursor goes back. The glide keeps going until the dwell says the
        // new destination is meant...
        let back_noticed = halfway + Duration::from_millis(16);
        let still_right = follower.step(mid, Some(LEFT), back_noticed).unwrap();
        assert!(still_right.0 > mid.0);
        // ...then the return trip starts exactly where the outbound glide had
        // got to — no jump — and heads back.
        let back = back_noticed + FOLLOW_DWELL;
        let from = follower.step(still_right, Some(LEFT), back).unwrap();
        assert!(from.0 >= still_right.0 && from.0 <= RIGHT.0);
        let next = follower
            .step(from, Some(LEFT), back + Duration::from_millis(48))
            .unwrap();
        assert!(next.0 < from.0);
        let end = back + FOLLOW_GLIDE;
        assert_eq!(follower.step(next, Some(LEFT), end), Some(LEFT));
    }

    #[test]
    fn gives_up_when_the_window_manager_ignores_positioning() {
        let mut follower = OverlayFollower::default();
        let mut now = Instant::now();
        for _ in 0..MAX_IGNORED_GLIDES {
            follower.step(LEFT, Some(RIGHT), now);
            now += FOLLOW_DWELL;
            follower.step(LEFT, Some(RIGHT), now);
            now += FOLLOW_GLIDE;
            assert_eq!(follower.step(LEFT, Some(RIGHT), now), Some(RIGHT));
            // The window never moved.
            now += FOLLOW_FRAME;
            follower.step(LEFT, Some(RIGHT), now);
        }
        assert!(follower.given_up());
        assert_eq!(follower.step(LEFT, Some(RIGHT), now + FOLLOW_DWELL), None);
    }
}
