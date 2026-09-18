//! Learning a spelling from a correction the user already made.
//!
//! When someone dictates "email brali about it" and then fixes `brali` to `Barali`,
//! they have supplied the highest-quality vocabulary signal the app can get: a word
//! the recogniser got wrong, and the exact spelling it should have used. They did it
//! anyway, because they wanted the text right. Learning it means the next dictation is
//! right on its own.
//!
//! Three parts, split so that only one of them knows about the app:
//!
//! * [`detect`] — pure. Pasted text and edited text in, learnable words out. This is
//!   where the guards live, and the guards are the feature: telling a correction from a
//!   rewrite is the whole problem.
//! * [`watch`] — native. Reads the one control the app just pasted into, a few times
//!   over the following minute, then stops.
//! * [`learner`] — the wiring. Settings, the on/off switch, the cap.
//!
//! Off by default, and it must stay that way: unlike every other setting in the app,
//! turning it on means reading the contents of a text field in another application.

pub mod detect;
pub mod learner;
pub mod watch;
