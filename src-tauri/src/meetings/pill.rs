//! The floating meeting pill: a small always-on-top window that is on screen for
//! as long as a meeting is being recorded.
//!
//! # Why this is a window and not a panel in Settings
//!
//! Before this existed, the live transcript rendered inside Settings → Meetings.
//! That is the one place the user is guaranteed *not* to be looking during a
//! call — they are in Zoom, or in a browser tab, or taking notes somewhere else.
//! A recording with no visible indicator is indistinguishable from a recording
//! that silently stopped, which is the worst property a recorder can have. So the
//! pill exists to answer one question continuously and from anywhere: *is this
//! still capturing, and is it hearing what I think it is?*
//!
//! Expanded, it answers two more: *what has it heard so far*, and *what does that
//! add up to* — the ask input in [`crate::meetings::chat`].
//!
//! # Two modes, and why the difference is not cosmetic
//!
//! | | collapsed | expanded |
//! |---|---|---|
//! | width | [`PILL_WIDTH_COLLAPSED`] | [`PILL_WIDTH_EXPANDED`] |
//! | height | measured, small | measured, clamped to [`MAX_HEIGHT_FRACTION`] of the display |
//! | focusable | **no** (Windows: `WS_EX_NOACTIVATE`) | **yes** |
//! | pointer | only the drawn pill | the whole card |
//!
//! The focusable row is the load-bearing one. A window that can take focus will
//! take it on a click, and a click that moves focus off the field the user is
//! typing into — mid-call, mid-sentence — is a worse bug than anything the pill
//! fixes. So collapsed it is unfocusable outright on Windows.
//!
//! But an unfocusable window's text input can never receive the caret, which
//! makes "ask anything" impossible. The reminder popup sidesteps this by having
//! only buttons; the pill cannot. So focusability is toggled per mode, following
//! [`crate::assistant`]'s panel rather than [`crate::reminders`]'s popup — and
//! the webview blurs its input *before* asking to collapse, because
//! `set_focusable(false)` on a window that currently holds focus is the one case
//! the platform will not honour (see the note in `overlay.rs`).
//!
//! # Lifecycle
//!
//! Built lazily on the first meeting, then hidden and reused — never destroyed
//! while the app runs. The build happens **inline on the calling thread**,
//! mirroring [`crate::assistant::open_snip_overlay`]: dispatching a webview build
//! to the main thread from inside a Tauri command's call stack deadlocks WebView2
//! on Windows. Everything *after* the build — show, hide, size, position,
//! focusability — is queued onto the main thread, because those are ordinary
//! window operations and the callers are audio and shortcut threads.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use log::{debug, error, warn};
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};

/// Window label. Must also appear in `capabilities/default.json`, or the window
/// renders and can `invoke` but every `listen` is denied by the ACL — which
/// looks like a pill that never updates rather than like a permissions problem.
pub const PILL_LABEL: &str = "meeting_pill";

/// Emitted to the pill when the user asks to expand or collapse it from
/// somewhere else (the tray, a shortcut), so the webview and the window agree on
/// which mode they are in.
pub const PILL_MODE_EVENT: &str = "meeting-pill-mode";

/// Emitted when a call appears to be in progress and no meeting is recording, and
/// again with `active: false` when the offer is withdrawn.
///
/// `active` is explicit rather than being implied by a null app name. Reading the
/// process name is allowed to fail and the detection does not depend on it, so
/// "offer with no name" is a normal case — collapsing it with "no offer" would make
/// the card impossible to show for exactly the processes we cannot inspect.
pub const CALL_OFFER_EVENT: &str = "meeting-call-detected";

/// Payload of [`CALL_OFFER_EVENT`].
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct CallOffer {
    pub active: bool,
    /// The capturing process's file name, when it could be read.
    pub app: Option<String>,
}

const PILL_WIDTH_COLLAPSED: f64 = 276.0;
const PILL_WIDTH_EXPANDED: f64 = 452.0;

/// Used only until the webview reports its measured height.
const COLLAPSED_FALLBACK_HEIGHT: f64 = 52.0;
const EXPANDED_FALLBACK_HEIGHT: f64 = 420.0;

/// Floor, so a measurement that arrives mid-render cannot collapse the window to
/// nothing and leave the user with no way to click it.
const MIN_HEIGHT: f64 = 44.0;

/// Ceiling on the expanded card, as a fraction of the display's height.
///
/// The clamp lives here rather than in CSS because the webview cannot see the
/// display: its own `100vh` is only ever the window it is already in, so a
/// transcript that keeps growing would keep growing the window. Same reasoning as
/// `assistant::fit_ask_card`.
const MAX_HEIGHT_FRACTION: f64 = 0.62;

/// Gap between the pill and the bottom of the work area.
///
/// Deliberately larger than `overlay::OVERLAY_BOTTOM_OFFSET`: the dictation
/// overlay lives at the bottom centre too, and a user who dictates a note during
/// a call would otherwise have the two windows stacked on the same pixels. This
/// puts the meeting pill above it.
#[cfg(target_os = "macos")]
const PILL_BOTTOM_OFFSET: f64 = 78.0;
#[cfg(not(target_os = "macos"))]
const PILL_BOTTOM_OFFSET: f64 = 104.0;

/// Mirrors the window's real visibility.
///
/// Read instead of `window.is_visible()`, which is a blocking round-trip to the
/// event loop. The meeting recorder calls into here from the thread that is also
/// opening audio devices, and `assistant.rs` documents what that costs there: a
/// blocking visibility query on the recording path delays the microphone and
/// swallows the first words.
static PILL_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Whether the pill is showing the expanded card.
static PILL_EXPANDED: AtomicBool = AtomicBool::new(false);

/// Last height the webview measured, in logical pixels. Zero means "not yet".
static PILL_HEIGHT: AtomicU32 = AtomicU32::new(0);

/// Generation counter for the topmost guard. Bumped on both start and stop, so a
/// tick can tell whether the window it is about to raise is still meant to be up.
#[cfg(target_os = "windows")]
static PILL_GUARD: AtomicU64 = AtomicU64::new(0);

/// Slow enough to cost nothing over an hour, fast enough that a displaced pill is
/// back before the user notices it went.
#[cfg(target_os = "windows")]
const PILL_GUARD_INTERVAL_MS: u64 = 700;

/// Non-Windows builds do not run the guard, but the counter keeps the
/// `expanded`/`visible` bookkeeping symmetrical across platforms.
#[cfg(not(target_os = "windows"))]
static PILL_GUARD: AtomicU64 = AtomicU64::new(0);

pub fn is_visible() -> bool {
    PILL_VISIBLE.load(Ordering::SeqCst)
}

pub fn is_expanded() -> bool {
    PILL_EXPANDED.load(Ordering::SeqCst)
}

/* ─────────────────────────────── build ─────────────────────────────── */

/// Create the pill window, hidden, if it does not exist yet.
///
/// **Builds inline on the calling thread on purpose.** Queuing a webview build
/// onto the main thread from inside a Tauri command's call stack deadlocks
/// WebView2 on Windows; `assistant::open_snip_overlay` carries the same note and
/// the same treatment. Call this from a `spawn_blocking` closure or a background
/// thread, never by dispatching it to the main thread.
///
/// Idempotent, and never destroys: the pill is rebuilt at most once per app run,
/// so starting a second meeting costs a `show()` rather than a webview boot.
pub fn ensure_pill_window(app: &AppHandle) {
    if app.get_webview_window(PILL_LABEL).is_some() {
        return;
    }

    let builder = WebviewWindowBuilder::new(
        app,
        PILL_LABEL,
        tauri::WebviewUrl::App("src/meeting/index.html".into()),
    )
    // Must match every other window's args, or this webview gets a different
    // WebView2 environment and fails to share the user data directory.
    .additional_browser_args(crate::WEBVIEW2_BROWSER_ARGS)
    .title("Meeting")
    .inner_size(PILL_WIDTH_COLLAPSED, COLLAPSED_FALLBACK_HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    // A click lands on the button it was aimed at rather than being spent
    // activating the window first.
    .accept_first_mouse(true)
    // Never arrives focused. It appears while the user is mid-call and possibly
    // mid-sentence in another app; taking the caret then is unforgivable.
    .focused(false)
    .visible(false);

    // Collapsed is the mode it is built in, and collapsed must not take focus.
    // Windows only: on macOS a non-activating window makes WebView controls
    // unreliable, and the pill's stop button is the one thing that must always
    // work.
    #[cfg(target_os = "windows")]
    let builder = builder.focusable(false);

    let mut builder = builder;
    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    match builder.build() {
        Ok(window) => {
            // Excluded from screen capture. This matters most for the call offer:
            // the moment it appears is very often the moment the user is sharing
            // their screen, and "should I record this call?" is not a question to
            // put in front of the other participants. The recording indicator is
            // hidden by the same call, which is the right default too — it is the
            // user's own status, not part of what they are presenting.
            //
            // Best-effort: unsupported on some Linux compositors, and a failure
            // there must not cost the window.
            if let Err(e) = window.set_content_protected(true) {
                debug!("Could not content-protect the meeting pill: {e}");
            }

            let app_handle = app.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // The X on the pill means "get out of my way", not "stop
                    // recording" — a meeting is still being captured and
                    // destroying the window would leave nothing to stop it with.
                    // Hide, and let the tray and Settings remain the way back.
                    api.prevent_close();
                    hide_pill(&app_handle);
                }
            });
            debug!("Meeting pill window created (hidden)");
        }
        Err(e) => error!("Failed to create the meeting pill window: {e}"),
    }
}

/* ─────────────────────────────── show / hide ─────────────────────────────── */

/// Put the pill on screen, building it first if this is the first meeting.
///
/// Safe from any thread. The build runs inline here (see
/// [`ensure_pill_window`]); everything after it is queued onto the main thread.
pub fn show_pill(app: &AppHandle) {
    ensure_pill_window(app);

    // A fresh show should not inherit the previous meeting's expansion or its
    // measured height — the transcript is empty again.
    PILL_EXPANDED.store(false, Ordering::SeqCst);
    PILL_HEIGHT.store(0, Ordering::SeqCst);

    let app_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        let Some(window) = app_main.get_webview_window(PILL_LABEL) else {
            return;
        };
        // Size first, then position for that exact size, then show — otherwise it
        // flashes at the wrong geometry before settling.
        apply_geometry(&app_main, &window);
        let _ = window.show();
        // Re-assert after showing: `always_on_top` at build time is a request,
        // and another process may already hold the topmost slot.
        let _ = window.set_always_on_top(true);
        PILL_VISIBLE.store(true, Ordering::SeqCst);
        #[cfg(target_os = "windows")]
        start_topmost_guard(&app_main);
    }) {
        warn!("Could not queue the meeting pill onto the main thread: {e}");
    }
}

/// Take the pill off screen. Does not stop the meeting.
pub fn hide_pill(app: &AppHandle) {
    // Retire the guard *before* the window goes down. A tick already queued on
    // the main thread would otherwise put a deliberately hidden pill back on
    // screen, which is exactly the bug `overlay.rs` documents.
    #[cfg(target_os = "windows")]
    stop_topmost_guard();

    PILL_VISIBLE.store(false, Ordering::SeqCst);
    PILL_EXPANDED.store(false, Ordering::SeqCst);
    PILL_HEIGHT.store(0, Ordering::SeqCst);

    let app_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        let Some(window) = app_main.get_webview_window(PILL_LABEL) else {
            return;
        };
        let _ = window.hide();
        // Back to unfocusable, so the next show cannot arrive able to steal the
        // caret. Safe here precisely because the window is now hidden and holds
        // no focus.
        #[cfg(target_os = "windows")]
        let _ = window.set_focusable(false);
    }) {
        warn!("Could not queue hiding the meeting pill: {e}");
    }
}

/* ─────────────────────────── the call offer ─────────────────────────── */

/// Show the pill as an offer to record a call that appears to be happening.
///
/// The same window as a live recording, deliberately. Three reasons, and the third
/// is the one that decided it:
///
/// * It is already always-on-top, already positioned, already permitted in
///   `capabilities/default.json`.
/// * The offer appears exactly where the recording indicator will be if it is
///   accepted, so accepting is visually continuous rather than one card vanishing
///   and a different thing appearing elsewhere.
/// * **An OS notification would be the wrong surface.** A meeting is precisely when
///   Focus / Do Not Disturb is on, and when notifications are muted for screen
///   sharing — so a system notification is suppressed at the only moment it
///   matters. Drawing it ourselves is the only way it reliably arrives.
///
/// Nothing here can start a recording. The window is shown; the user's click is what
/// calls `start_meeting`.
pub fn show_call_offer(app: &AppHandle, app_label: Option<String>) {
    ensure_pill_window(app);

    PILL_EXPANDED.store(false, Ordering::SeqCst);
    PILL_HEIGHT.store(0, Ordering::SeqCst);

    let app_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        let Some(window) = app_main.get_webview_window(PILL_LABEL) else {
            return;
        };
        // Emitted before the window is shown, so the webview has the offer to render
        // on its first paint rather than flashing an empty pill.
        let _ = app_main.emit_to(
            PILL_LABEL,
            CALL_OFFER_EVENT,
            CallOffer {
                active: true,
                app: app_label.clone(),
            },
        );
        apply_geometry(&app_main, &window);
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        PILL_VISIBLE.store(true, Ordering::SeqCst);
        #[cfg(target_os = "windows")]
        start_topmost_guard(&app_main);
    }) {
        warn!("Could not queue the call offer onto the main thread: {e}");
    }
}

/// Take the offer down.
///
/// Clears the offer payload *before* hiding, so a window reused for a real
/// recording a moment later cannot render the stale card for a frame.
pub fn hide_call_offer(app: &AppHandle) {
    let app_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        let _ = app_main.emit_to(
            PILL_LABEL,
            CALL_OFFER_EVENT,
            CallOffer {
                active: false,
                app: None,
            },
        );
    }) {
        debug!("Could not clear the call offer: {e}");
    }
    hide_pill(app);
}

/* ─────────────────────────────── expand / collapse ────────────────────────── */

/// Switch between the compact pill and the expanded card.
///
/// The webview is expected to have blurred its input already when collapsing;
/// `set_focusable(false)` is not honoured for a window that currently holds
/// focus, and a pill stuck focusable is a pill that steals the caret.
pub fn set_pill_expanded(app: &AppHandle, expanded: bool) {
    if PILL_EXPANDED.swap(expanded, Ordering::SeqCst) == expanded {
        return;
    }
    // The measured height belongs to the mode that produced it.
    PILL_HEIGHT.store(0, Ordering::SeqCst);

    let app_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        let Some(window) = app_main.get_webview_window(PILL_LABEL) else {
            return;
        };

        // Focusability before geometry: the expanded card should be typeable the
        // moment it is the right size, not a frame later.
        #[cfg(target_os = "windows")]
        let _ = window.set_focusable(expanded);

        apply_geometry(&app_main, &window);

        if expanded {
            // Asked for, so focus is wanted. Without this the caret never lands
            // in the ask input even though the window can now hold it.
            let _ = window.set_focus();
        }
        let _ = window.set_always_on_top(true);
        let _ = app_main.emit_to(PILL_LABEL, PILL_MODE_EVENT, expanded);
    }) {
        warn!("Could not queue the meeting pill mode change: {e}");
    }
}

/* ─────────────────────────────── geometry ─────────────────────────────── */

/// Adopt a height the webview measured for its content.
///
/// Idempotent by design: the webview reports on every `ResizeObserver` callback,
/// and a resize that triggers another measurement would otherwise oscillate.
pub fn fit_pill(app: &AppHandle, requested: f64) {
    if requested <= 0.0 {
        return;
    }
    let clamped = requested.max(MIN_HEIGHT);
    let previous = PILL_HEIGHT.swap(clamped.round() as u32, Ordering::SeqCst) as f64;
    if (previous - clamped).abs() < 0.5 {
        return;
    }
    // Nothing to resize until it is on screen; the stored height is applied by
    // the next show.
    if !PILL_VISIBLE.load(Ordering::SeqCst) {
        return;
    }

    let app_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if let Some(window) = app_main.get_webview_window(PILL_LABEL) {
            apply_geometry(&app_main, &window);
        }
    }) {
        debug!("Could not queue the meeting pill resize: {e}");
    }
}

/// Size and place the window for the current mode. Main thread only.
fn apply_geometry(app: &AppHandle, window: &tauri::WebviewWindow) {
    let expanded = PILL_EXPANDED.load(Ordering::SeqCst);
    let width = if expanded {
        PILL_WIDTH_EXPANDED
    } else {
        PILL_WIDTH_COLLAPSED
    };

    // The monitor is resolved once and used for both the clamp and the position,
    // so a pill clamped against one display cannot be placed on another.
    let monitor = crate::overlay::get_monitor_with_cursor(app)
        .or_else(|| window.current_monitor().ok().flatten());

    let height = resolve_height(expanded, monitor.as_ref());
    let _ = window.set_size(tauri::LogicalSize::new(width, height));

    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let monitor_x = monitor.position().x as f64 / scale;
        let monitor_y = monitor.position().y as f64 / scale;
        let monitor_w = monitor.size().width as f64 / scale;
        let monitor_h = monitor.size().height as f64 / scale;

        let x = monitor_x + ((monitor_w - width) / 2.0).max(0.0);
        let y = monitor_y + (monitor_h - height - PILL_BOTTOM_OFFSET).max(monitor_y);

        // Logical, never physical: tao converts a `PhysicalPosition` using the
        // scale factor of the monitor the window is *currently* on, which is the
        // wrong one whenever the window is moving between displays.
        let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    }
}

/// The height to use, given the mode and the display to clamp against.
///
/// Pure so the clamp can be tested without a window: every bug this has had was
/// in the arithmetic, not in the `set_size` call.
fn resolve_height(expanded: bool, monitor: Option<&tauri::Monitor>) -> f64 {
    let fallback = if expanded {
        EXPANDED_FALLBACK_HEIGHT
    } else {
        COLLAPSED_FALLBACK_HEIGHT
    };
    let measured = match PILL_HEIGHT.load(Ordering::SeqCst) {
        0 => fallback,
        value => (value as f64).max(MIN_HEIGHT),
    };
    clamp_height(measured, expanded, monitor.map(available_height))
}

/// Available logical height of a monitor.
fn available_height(monitor: &tauri::Monitor) -> f64 {
    monitor.size().height as f64 / monitor.scale_factor()
}

/// Clamp a measured height against the display.
///
/// Only the expanded card is clamped. The collapsed pill is a fixed strip whose
/// measurement cannot run away, and clamping it would mean a tiny display could
/// shrink it below the point where its buttons are hittable.
fn clamp_height(measured: f64, expanded: bool, monitor_height: Option<f64>) -> f64 {
    if !expanded {
        return measured.max(MIN_HEIGHT);
    }
    let ceiling = monitor_height
        .map(|available| (available * MAX_HEIGHT_FRACTION).max(MIN_HEIGHT))
        .unwrap_or(EXPANDED_FALLBACK_HEIGHT);
    measured.max(MIN_HEIGHT).min(ceiling)
}

/* ─────────────────────────────── topmost guard ────────────────────────────── */

/// Keep the pill above other windows for the whole meeting.
///
/// `WS_EX_TOPMOST` is a position another process can take, and the pill is up for
/// an hour rather than the four seconds a dictation overlay lives — so it is
/// displaced far more often. The guard re-asserts on a slow tick and logs once
/// per displacement, which is what turns "the meeting window disappears
/// sometimes" into a line naming whether topmost or visibility was lost.
///
/// A separate generation counter from the recording overlay's on purpose: the two
/// windows come and go independently, and sharing one would mean a dictation
/// ending retired the meeting pill's watcher.
#[cfg(target_os = "windows")]
fn start_topmost_guard(app: &AppHandle) {
    let generation = PILL_GUARD.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    static LAST_REPORTED: AtomicU64 = AtomicU64::new(0);

    std::thread::spawn(move || {
        while PILL_GUARD.load(Ordering::SeqCst) == generation {
            std::thread::sleep(std::time::Duration::from_millis(PILL_GUARD_INTERVAL_MS));
            if PILL_GUARD.load(Ordering::SeqCst) != generation {
                break;
            }
            let Some(window) = app.get_webview_window(PILL_LABEL) else {
                break;
            };
            let _ = window.clone().run_on_main_thread(move || {
                // Checked again on the main thread: a hide may have landed while
                // this closure sat in the queue, and resurrecting a hidden pill
                // is worse than leaving a visible one displaced.
                if PILL_GUARD.load(Ordering::SeqCst) != generation {
                    return;
                }
                reassert_on_top(&window, &LAST_REPORTED, generation);
            });
        }
    });
}

#[cfg(target_os = "windows")]
fn stop_topmost_guard() {
    PILL_GUARD.fetch_add(1, Ordering::SeqCst);
}

#[cfg(target_os = "windows")]
fn reassert_on_top(window: &tauri::WebviewWindow, last_reported: &AtomicU64, generation: u64) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, IsWindowVisible, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
    };

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    unsafe {
        let lost_topmost = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 == 0;
        let hidden = !IsWindowVisible(hwnd).as_bool();
        if lost_topmost || hidden {
            if last_reported.swap(generation, Ordering::SeqCst) != generation {
                warn!(
                    "Meeting pill was displaced (lost topmost: {lost_topmost}, \
                     hidden: {hidden}); restoring it"
                );
            }
            // No SWP_SHOWWINDOW: an unmapped window is restored through Tauri's
            // own `show()` below, so this call can never reveal a window the app
            // meant to keep hidden.
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            if hidden {
                let _ = window.show();
            }
            return;
        }
        // Still flagged topmost, but possibly no longer first among topmost
        // windows. Re-asking is a no-op when we are already there.
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4K display must not let the expanded card fill the screen.
    #[test]
    fn the_expanded_card_is_clamped_to_the_display() {
        let clamped = clamp_height(2_000.0, true, Some(1_000.0));
        assert!(
            clamped <= 1_000.0 * MAX_HEIGHT_FRACTION + 0.001,
            "expected a clamp to {}, got {clamped}",
            1_000.0 * MAX_HEIGHT_FRACTION
        );
    }

    /// A card that fits must not be stretched to the ceiling.
    #[test]
    fn a_short_card_keeps_its_measured_height() {
        assert_eq!(clamp_height(200.0, true, Some(1_000.0)), 200.0);
    }

    /// The collapsed strip is a fixed shape; clamping it against a small display
    /// could shrink it below the point where its buttons are hittable.
    #[test]
    fn the_collapsed_pill_is_not_clamped() {
        assert_eq!(clamp_height(52.0, false, Some(100.0)), 52.0);
    }

    /// A measurement that arrives mid-render must not collapse the window to
    /// nothing, leaving the user no way to click it.
    #[test]
    fn a_degenerate_measurement_is_floored() {
        assert_eq!(clamp_height(0.0, false, Some(1_000.0)), MIN_HEIGHT);
        assert_eq!(clamp_height(1.0, true, Some(1_000.0)), MIN_HEIGHT);
    }

    /// With no monitor resolved the expanded card falls back to a fixed size
    /// rather than to "unbounded".
    #[test]
    fn an_unknown_display_still_bounds_the_card() {
        assert_eq!(clamp_height(5_000.0, true, None), EXPANDED_FALLBACK_HEIGHT);
    }

    /// A display so short that the fraction lands under the floor must still
    /// produce a clickable window.
    #[test]
    fn a_tiny_display_does_not_produce_a_zero_height_card() {
        assert_eq!(clamp_height(300.0, true, Some(10.0)), MIN_HEIGHT);
    }

    #[test]
    fn the_expanded_card_is_wider_than_the_collapsed_pill() {
        assert!(PILL_WIDTH_EXPANDED > PILL_WIDTH_COLLAPSED);
    }

    /// The pill sits above where the dictation overlay draws, so dictating during
    /// a call does not stack two windows on the same pixels.
    #[test]
    fn the_pill_clears_the_dictation_overlay() {
        assert!(PILL_BOTTOM_OFFSET > 40.0);
    }
}
