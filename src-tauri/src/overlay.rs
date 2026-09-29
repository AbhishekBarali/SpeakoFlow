use crate::input;
use crate::overlay_follow::{
    display_under, hop_destination, placement_on, rect_contains, DisplayFollower, Edge,
    MonitorBounds, Point, FOLLOW_POLL, HOP_FADE_OUT,
};
use crate::overlay_lifecycle::OverlayLifecycle;
use crate::settings;
use crate::settings::OverlayPosition;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

/// Bumped on every overlay state change. The delayed hide behind a brief
/// notice only fires when nothing newer (e.g. a fresh recording) replaced it.
static OVERLAY_LIFECYCLE: OverlayLifecycle = OverlayLifecycle::new();

#[cfg(not(target_os = "macos"))]
use log::debug;

#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;

#[cfg(target_os = "macos")]
use tauri::WebviewUrl;

#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelBuilder, PanelLevel};

#[cfg(target_os = "linux")]
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

#[cfg(target_os = "linux")]
use std::env;

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(RecordingOverlayPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

// The window is intentionally a little larger than the visible pill: the pill
// hugs its content (auto width) and floats centered inside this transparent
// frame, so these are the *maximum* bounds across all states rather than the
// chip's actual size. Keeping them tight is what makes the overlay read as a
// small, unobtrusive lozenge — the compact pill is only ~86pt wide, and every
// point of frame beyond that is invisible padding that still swallows clicks.
const OVERLAY_WIDTH: f64 = 96.0;
const OVERLAY_HEIGHT: f64 = 44.0;

// Labeled pill states (Flow generating / looking at the screen / a brief
// notice) carry a written line, so the transparent frame is widened and given
// room for a second line. Truncating a notice destroys the only thing it exists
// to say, so the text wraps here rather than ellipsizing; the pill itself still
// hugs its content, so a short label stays a short pill.
const OVERLAY_LABEL_WIDTH: f64 = 300.0;
const OVERLAY_LABEL_HEIGHT: f64 = 60.0;

// The opt-in live-transcription window (see `live_transcription_window_enabled`)
// reuses this same overlay window, resized into a larger card so the running
// committed + tentative transcript is readable during dictation. Mirrors
// Handy's 400×120 streaming overlay. When the window setting is off the overlay
// stays the compact pill above.
const OVERLAY_STREAM_WIDTH: f64 = 400.0;
const OVERLAY_STREAM_HEIGHT: f64 = 120.0;

// The Undo / Try again pill left behind by a dismissed or failed dictation.
// Unlike every other state it takes the pointer, so the frame *is* the pill:
// the pill fills it edge to edge (see `.overlay-pill.recovery`), and no
// transparent margin sits in front of the app underneath catching clicks meant
// for it. "Try again" is longer than "Undo"; a failure that explains itself
// (a cloud key, Flow) needs the two-line labelled frame.
const OVERLAY_RECOVERY_WIDTH: f64 = 184.0;
const OVERLAY_RECOVERY_WIDE_WIDTH: f64 = 248.0;
const OVERLAY_RECOVERY_HEIGHT: f64 = 40.0;

/// How long the Undo / Try again pill waits for a click. Short on purpose — it
/// is there for the second after a slip, and History keeps the dictation for
/// anything later — and a pointer resting on it holds it open.
const RECOVERY_LINGER: std::time::Duration = std::time::Duration::from_secs(6);

fn is_recovery_state(state: &str) -> bool {
    matches!(state, "dismissed" | "failed")
}

/// Frame for a recovery pill.
fn recovery_overlay_size(state: &str, has_notice: bool) -> (f64, f64) {
    if has_notice {
        (OVERLAY_LABEL_WIDTH, OVERLAY_LABEL_HEIGHT)
    } else if state == "failed" {
        (OVERLAY_RECOVERY_WIDE_WIDTH, OVERLAY_RECOVERY_HEIGHT)
    } else {
        (OVERLAY_RECOVERY_WIDTH, OVERLAY_RECOVERY_HEIGHT)
    }
}

/// Windows accessibility text size (Settings → Accessibility → Text size).
///
/// A separate axis from display scaling: WebView2 applies it as a document zoom
/// *inside* the window, while the window itself is sized only by DPI. The
/// overlay's frame is kept tight around the pill (see `OVERLAY_WIDTH`), so at
/// 125% text the pill and card were larger than their window and got clipped on
/// every dictation. The value is absent until the slider is moved off 100%, and
/// is stored as a percentage. Backport of Handy #2059.
#[cfg(target_os = "windows")]
fn windows_text_scale_factor() -> f64 {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Accessibility")
        .and_then(|key| key.get_value::<u32, _>("TextScaleFactor"))
        .map(|percent| clamp_text_scale(percent as f64 / 100.0))
        .unwrap_or(1.0)
}

/// Windows allows 100%–225%; anything outside that is a corrupt value.
fn clamp_text_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        scale.clamp(1.0, 2.25)
    } else {
        1.0
    }
}

/// Content zoom the overlay's webview renders at, beyond display scaling.
fn overlay_text_scale() -> f64 {
    #[cfg(target_os = "windows")]
    {
        windows_text_scale_factor()
    }
    #[cfg(not(target_os = "windows"))]
    {
        1.0
    }
}

/// A logical overlay size grown by the text scale, so the zoomed content fits.
/// Placement offsets are not scaled: the overlay keeps its distance from the
/// screen edge and grows away from it.
fn scale_overlay_size((width, height): (f64, f64), text_scale: f64) -> (f64, f64) {
    (width * text_scale, height * text_scale)
}

/// Only completed live cards linger. All waiting is event-driven.
///
/// The card holds still at full opacity for the whole linger — that is the
/// window in which the transcript is readable and the copy button is a real
/// target — and then leaves in one short movement. A long fade is the worst of
/// both: it is neither readable nor gone, and a card that spends most of a
/// second dissolving reads as the app being slow rather than as a dismissal.
/// Hovering during either phase cancels it and restarts the linger, so the exit
/// being brief costs nothing (see `OverlayLifecycle::wait_for_dismissal`). How
/// long the linger is belongs to the user (`settings::OverlayLinger`).
static OVERLAY_STREAMING: AtomicBool = AtomicBool::new(false);
const OVERLAY_FADE_MS: u64 = 220;

pub fn set_overlay_hovered(hovered: bool) {
    OVERLAY_LIFECYCLE.set_hovered(hovered);
}

/// Payload of the "show-overlay" event. Carries the visual `state`
/// (recording / transcribing / processing / generating / vision / notice)
/// plus whether the larger live-transcription card should be rendered
/// (`streaming_window`). `notice` carries an i18n key suffix for the brief
/// `notice` state (e.g. a Flow error). Serialized camelCase so the overlay
/// reads `event.payload.streamingWindow`.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowOverlayPayload {
    state: String,
    streaming_window: bool,
    /// Whether the card takes the pointer in this state (see
    /// [`live_card_takes_pointer`]). The webview only offers the copy button and
    /// text selection when it does, so it never shows a control that a
    /// click-through window would pass straight to the app underneath.
    interactive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    notice: Option<String>,
    /// The overlay lifetime this show began. The recovery pill hands it back
    /// with its click (`recover_dictation`), so a click meant for a pill that
    /// has since been replaced can never act on a different dictation.
    epoch: u64,
}

/// Whether the live card can take the pointer mid-recording without being able
/// to take keyboard focus.
///
/// The pointer is what makes live text worth showing — scrolling back through
/// it, selecting a phrase, the copy button — and a click-through card showed all
/// of that and let none of it work: the cursor over it was the app's underneath.
/// But the overlay must never become the window a paste lands in (see
/// `create_recording_overlay`), so input is only enabled where the window is
/// structurally unable to take focus: Windows sets `WS_EX_NOACTIVATE`, macOS
/// uses a panel that can never become key, and a GTK layer surface is created
/// with `KeyboardMode::None`. An ordinary X11 window has none of these — a click
/// focuses it — so there the card stays click-through until the paste is done.
fn live_card_takes_pointer() -> bool {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        LAYER_SHELL_ACTIVE.load(Ordering::SeqCst)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
const OVERLAY_TOP_OFFSET: f64 = 46.0;
#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_TOP_OFFSET: f64 = 4.0;

#[cfg(target_os = "macos")]
const OVERLAY_BOTTOM_OFFSET: f64 = 15.0;

#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_BOTTOM_OFFSET: f64 = 40.0;

#[cfg(target_os = "linux")]
fn update_gtk_layer_shell_anchors(overlay_window: &tauri::webview::WebviewWindow) {
    let window_clone = overlay_window.clone();
    let _ = overlay_window.run_on_main_thread(move || {
        // Try to get the GTK window from the Tauri webview
        if let Ok(gtk_window) = window_clone.gtk_window() {
            let settings = settings::get_settings(window_clone.app_handle());
            match settings.overlay_position {
                OverlayPosition::Top => {
                    gtk_window.set_anchor(Edge::Top, true);
                    gtk_window.set_anchor(Edge::Bottom, false);
                }
                OverlayPosition::Bottom | OverlayPosition::None => {
                    gtk_window.set_anchor(Edge::Bottom, true);
                    gtk_window.set_anchor(Edge::Top, false);
                }
            }
        }
    });
}

/// Returns true when the environment variable is set to a truthy value
/// (e.g. "1", "true", "yes", "on").
/// "0", "false", "no", "off" and empty string are treated as falsy (case-insensitive).
/// Returns false when the variable is not set.
#[cfg(target_os = "linux")]
fn env_flag_enabled(name: &str) -> bool {
    match env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        ),
        Err(_) => false,
    }
}

/// Initializes GTK layer shell for Linux overlay window
/// Returns true if layer shell was successfully initialized, false otherwise
#[cfg(target_os = "linux")]
fn init_gtk_layer_shell(overlay_window: &tauri::webview::WebviewWindow) -> bool {
    if env_flag_enabled("SPEAKOFLOW_NO_GTK_LAYER_SHELL") {
        debug!("Skipping GTK layer shell init (SPEAKOFLOW_NO_GTK_LAYER_SHELL is enabled)");
        return false;
    }

    if !gtk_layer_shell::is_supported() {
        return false;
    }

    // Try to get the GTK window from the Tauri webview
    if let Ok(gtk_window) = overlay_window.gtk_window() {
        // Initialize layer shell
        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        gtk_window.set_keyboard_mode(KeyboardMode::None);
        gtk_window.set_exclusive_zone(0);

        update_gtk_layer_shell_anchors(overlay_window);

        LAYER_SHELL_ACTIVE.store(true, Ordering::SeqCst);
        return true;
    }
    false
}

/// A layer surface is placed by the compositor from its anchors; client
/// positioning does nothing, so there is nothing for cursor following to move.
#[cfg(target_os = "linux")]
static LAYER_SHELL_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Forces a window to be topmost using Win32 API (Windows only)
/// This is more reliable than Tauri's set_always_on_top which can be overridden
#[cfg(target_os = "windows")]
fn force_overlay_topmost(overlay_window: &tauri::webview::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    // Clone because run_on_main_thread takes 'static
    let overlay_clone = overlay_window.clone();

    // Make sure the Win32 call happens on the UI thread
    let _ = overlay_clone.clone().run_on_main_thread(move || {
        if let Ok(hwnd) = overlay_clone.hwnd() {
            unsafe {
                // Force Z-order: make this window topmost without changing size/pos or stealing focus
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    });
}

/// Keep the overlay on screen for as long as it is meant to be there.
///
/// `force_overlay_topmost` used to run exactly once per show, and that is not
/// enough on Windows: `WS_EX_TOPMOST` is a property another process can displace
/// after the fact. A full-screen exclusive app, a shell flyout, or any window
/// that raises itself into the topmost band takes the position we asked for, and
/// the pill silently vanishes for the rest of the recording with nothing in the
/// log to say so — the "the dictation window gets hidden randomly" report.
///
/// So the assertion is re-run on a slow tick while the overlay is up, and the
/// tick *reports*: a lost `WS_EX_TOPMOST` or an unexpectedly invisible window is
/// logged once per occurrence, which turns an unreproducible complaint into a
/// line naming which of the two failed.
///
/// The generation counter is the whole lifecycle: starting bumps it (retiring
/// any previous watcher), and hiding bumps it again so nothing is left ticking
/// against a window that is deliberately down.
#[cfg(target_os = "windows")]
static OVERLAY_GUARD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Slow enough to be free, fast enough that a displaced overlay is back before
/// the user finishes the sentence they were speaking when it happened.
#[cfg(target_os = "windows")]
const OVERLAY_GUARD_INTERVAL_MS: u64 = 700;

#[cfg(target_os = "windows")]
fn start_overlay_topmost_guard(app_handle: &AppHandle) {
    use std::sync::atomic::AtomicU64;
    let generation = OVERLAY_GUARD.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app_handle.clone();
    // Reported once per displacement rather than every tick: the repair is
    // immediate, so a window that is taken repeatedly would otherwise fill the
    // log with the same line.
    static LAST_REPORTED: AtomicU64 = AtomicU64::new(0);
    std::thread::spawn(move || {
        while OVERLAY_GUARD.load(Ordering::SeqCst) == generation {
            std::thread::sleep(std::time::Duration::from_millis(OVERLAY_GUARD_INTERVAL_MS));
            if OVERLAY_GUARD.load(Ordering::SeqCst) != generation {
                break;
            }
            let Some(window) = app.get_webview_window("recording_overlay") else {
                break;
            };
            let _ = window.clone().run_on_main_thread(move || {
                // Re-check on the main thread: a hide may have landed while this
                // closure was queued, and resurrecting the overlay then would be
                // worse than leaving it displaced.
                if OVERLAY_GUARD.load(Ordering::SeqCst) != generation {
                    return;
                }
                reassert_overlay_on_top(&window, &LAST_REPORTED, generation);
            });
        }
    });
}

#[cfg(target_os = "windows")]
fn stop_overlay_topmost_guard() {
    OVERLAY_GUARD.fetch_add(1, Ordering::SeqCst);
}

#[cfg(target_os = "windows")]
fn reassert_overlay_on_top(
    window: &tauri::webview::WebviewWindow,
    last_reported: &std::sync::atomic::AtomicU64,
    generation: u64,
) {
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
                log::warn!(
                    "Recording overlay was displaced (lost topmost: {lost_topmost}, \
                     hidden: {hidden}); restoring it"
                );
            }
            // Deliberately no SWP_SHOWWINDOW: an unmapped window is put back
            // through Tauri's own `show()` below, so this call can never reveal a
            // window the app meant to keep hidden.
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
        // Still flagged topmost but possibly no longer *first* among topmost
        // windows. Re-asking is a no-op when we are already there and costs one
        // message otherwise, which is cheaper than working out the difference.
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

/// Linux fallback for keeping the recording overlay above other windows when
/// GTK layer shell is unavailable (e.g. GNOME/Mutter, or an X11 session). Tauri
/// maps `always_on_top` to GTK's `keep_above`, but that hint can be dropped
/// after a hide/show, so re-assert it — mirroring the Windows `force_overlay_topmost`
/// re-assert. Effective on X11/Xorg (and XWayland); a harmless no-op under
/// layer shell and under GNOME Wayland, which never honors client-set stacking.
/// GTK calls must run on the main thread.
#[cfg(target_os = "linux")]
fn force_overlay_keep_above(overlay_window: &tauri::webview::WebviewWindow) {
    use gtk::prelude::GtkWindowExt;

    let overlay_clone = overlay_window.clone();
    let _ = overlay_window.run_on_main_thread(move || {
        if let Ok(gtk_window) = overlay_clone.gtk_window() {
            gtk_window.set_keep_above(true);
        }
    });
}

/// Returns the `tauri::Monitor` currently under the mouse cursor (fallback:
/// primary). Shared by the recording overlay and the assistant region-snip
/// overlay so both place their windows with the same proven, multi-monitor-safe
/// logic.
pub(crate) fn get_monitor_with_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
    monitor_under_cursor(app_handle).or_else(|| app_handle.primary_monitor().ok().flatten())
}

/// The monitor under the cursor, with **no** fallback. Cursor following needs
/// the difference: "the cursor can't be located" (Wayland, a locked session)
/// must leave the overlay where it is, not send it to the primary display.
///
/// The hit test itself (physical space first, logical second, and why) is
/// `overlay_follow::display_under`, shared with the follower's cached list.
fn monitor_under_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
    let (x, y) = input::get_cursor_position(app_handle)?;
    let monitors = app_handle.available_monitors().ok()?;
    let displays: Vec<MonitorBounds> = monitors.iter().map(monitor_bounds).collect();
    let hit = display_under(&displays, (x as f64, y as f64))?;
    monitors.into_iter().find(|m| monitor_bounds(m) == hit)
}

/// Overlay positions are computed and applied in the platform's native window
/// space: physical pixels on Windows, logical points elsewhere. See
/// `overlay_follow::placement_on` for why a logical position is wrong on a
/// mixed-DPI Windows desktop — tao converts it with the scale factor of the
/// display the window is *leaving*.
///
/// macOS positions in points natively, and Linux (GTK) in logical pixels with
/// one scale for the whole screen, so logical is exact on both.
#[cfg(target_os = "windows")]
const PLACE_IN_PHYSICAL: bool = true;
#[cfg(not(target_os = "windows"))]
const PLACE_IN_PHYSICAL: bool = false;

fn monitor_bounds(monitor: &tauri::Monitor) -> MonitorBounds {
    MonitorBounds {
        x: monitor.position().x as f64,
        y: monitor.position().y as f64,
        width: monitor.size().width as f64,
        height: monitor.size().height as f64,
        scale: monitor.scale_factor(),
    }
}

/// Uses monitor position/size directly rather than work_area(), which can
/// return incorrect coordinates on macOS for monitors with negative positions.
/// The per-platform OVERLAY_TOP_OFFSET / OVERLAY_BOTTOM_OFFSET constants
/// already account for system chrome (menu bar, taskbar).
fn overlay_edge(position: OverlayPosition) -> Edge {
    match position {
        OverlayPosition::Top => Edge::Top(OVERLAY_TOP_OFFSET),
        OverlayPosition::Bottom | OverlayPosition::None => Edge::Bottom(OVERLAY_BOTTOM_OFFSET),
    }
}

/// Where an overlay of logical `width`x`height` belongs on `monitor`, in the
/// native placement space (see `PLACE_IN_PHYSICAL`).
fn placement_for(
    monitor: &tauri::Monitor,
    width: f64,
    height: f64,
    position: OverlayPosition,
) -> Point {
    placement_on(
        monitor_bounds(monitor),
        (width, height),
        overlay_edge(position),
        PLACE_IN_PHYSICAL,
    )
}

fn set_overlay_placement(window: &tauri::webview::WebviewWindow, (x, y): Point) {
    let position = if PLACE_IN_PHYSICAL {
        tauri::Position::Physical(PhysicalPosition::new(x.round() as i32, y.round() as i32))
    } else {
        tauri::Position::Logical(tauri::LogicalPosition { x, y })
    };
    let _ = window.set_position(position);
}

/// Generation of the running cursor follower. Starting bumps it (retiring any
/// previous follower), and every hide bumps it again, so nothing keeps moving a
/// window that is deliberately down — the same lifecycle as the topmost guard.
static OVERLAY_FOLLOW: AtomicU64 = AtomicU64::new(0);

/// Whether the cursor and the overlay's placement are in the same coordinate
/// space, so "is the pointer on the card?" can be answered from our own numbers.
/// True on Windows (both physical) and macOS (both logical points). Not on
/// Linux, where X11 reports a physical cursor while GTK places in logical pixels.
const CURSOR_SHARES_PLACEMENT_SPACE: bool = cfg!(any(target_os = "windows", target_os = "macos"));

/// A hover the webview reported but the cursor contradicts, for this long, is
/// dropped. The completed card holds itself open while hovered and takes the
/// pointer while it does; a `mouseleave` that never arrived used to hold it —
/// and the clicks landing on it — for as long as the app ran.
const STALE_HOVER_AFTER: std::time::Duration = std::time::Duration::from_millis(400);

/// Keep the overlay on the display the cursor is on for as long as it is up.
///
/// The decision — wait for the cursor to settle on another display, then hop
/// there — is `overlay_follow::DisplayFollower`; this is only the loop that
/// samples the cursor and applies what it says. A hop is three steps: the webview
/// fades the overlay out (`overlay-hop` `"out"`), the window moves while nothing
/// is drawn, and it fades back in (`"in"`). Both a new state and a hide reset the
/// webview's hop state, so a hop interrupted between the two can never leave the
/// overlay invisible.
///
/// Everything the loop needs is captured when the overlay is shown: the display
/// it was placed on, its size, and one enumeration of the displays. Asking the
/// window instead (position, size, scale, monitors) cost four round trips
/// through the event loop on every poll, and those stall exactly when the main
/// thread is busy — which is what made following feel laggy. What is left per
/// poll is one cursor read, which does not touch the event loop at all.
fn start_overlay_follow(
    app_handle: &AppHandle,
    placed_on: Option<tauri::Monitor>,
    size: (f64, f64),
) {
    let generation = OVERLAY_FOLLOW.fetch_add(1, Ordering::SeqCst) + 1;
    #[cfg(target_os = "linux")]
    if LAYER_SHELL_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    // Nowhere to follow from: the overlay could not be placed on any display.
    let Some(placed_on) = placed_on else {
        return;
    };
    let edge = overlay_edge(settings::get_settings(app_handle).overlay_position);
    let app = app_handle.clone();
    std::thread::spawn(move || {
        let alive = || OVERLAY_FOLLOW.load(Ordering::SeqCst) == generation;
        // Once per overlay lifetime. A display plugged in mid-dictation is picked
        // up by the next show, which is at most one state change away.
        let displays: Vec<MonitorBounds> = match app.available_monitors() {
            Ok(monitors) => monitors.iter().map(monitor_bounds).collect(),
            Err(_) => return,
        };
        let mut current = monitor_bounds(&placed_on);
        let mut follower = DisplayFollower::default();
        let mut stale_hover_since: Option<std::time::Instant> = None;
        while alive() {
            std::thread::sleep(FOLLOW_POLL);
            if !alive() {
                break;
            }
            let Some((x, y)) = input::get_cursor_position(&app) else {
                continue;
            };
            let cursor = (x as f64, y as f64);
            let now = std::time::Instant::now();

            // The pointer is on the completed card, reaching for its copy button:
            // never pull the card out from under it. But check the claim — the
            // hover comes from the webview's `mouseenter`/`mouseleave`, and one
            // missed `mouseleave` would otherwise keep a clickable card on screen
            // indefinitely.
            if OVERLAY_LIFECYCLE.is_hovered() {
                if CURSOR_SHARES_PLACEMENT_SPACE {
                    let origin = placement_on(current, size, edge, PLACE_IN_PHYSICAL);
                    let extent = if PLACE_IN_PHYSICAL {
                        (size.0 * current.scale, size.1 * current.scale)
                    } else {
                        size
                    };
                    if rect_contains(origin, extent, 6.0, cursor) {
                        stale_hover_since = None;
                    } else if now.saturating_duration_since(*stale_hover_since.get_or_insert(now))
                        >= STALE_HOVER_AFTER
                    {
                        log::debug!("Overlay hover outlived the pointer; releasing it");
                        stale_hover_since = None;
                        set_overlay_hovered(false);
                    }
                }
                continue;
            }
            stale_hover_since = None;

            if let Some(target) = follower.step(current, display_under(&displays, cursor), now) {
                // A hide or a new state may have landed while this tick ran.
                if !alive() {
                    break;
                }
                let Some(window) = app.get_webview_window("recording_overlay") else {
                    break;
                };
                let _ = window.emit("overlay-hop", "out");
                std::thread::sleep(HOP_FADE_OUT);
                // Whoever retired this follower (a hide, or the next state's
                // show) also cleared the fade in the webview, and placed the
                // window themselves.
                if !alive() {
                    break;
                }
                let now_under = input::get_cursor_position(&app)
                    .and_then(|(x, y)| display_under(&displays, (x as f64, y as f64)));
                if let Some(destination) = hop_destination(current, target, now_under) {
                    set_overlay_placement(
                        &window,
                        placement_on(destination, size, edge, PLACE_IN_PHYSICAL),
                    );
                    current = destination;
                }
                let _ = window.emit("overlay-hop", "in");
            }
        }
    });
}

fn stop_overlay_follow() {
    OVERLAY_FOLLOW.fetch_add(1, Ordering::SeqCst);
}

/// Returns the overlay position (native placement space) for a window of the
/// given logical `width`/`height` on the monitor under the cursor.
///
/// Parameterized by size so the same monitor-under-cursor placement works for
/// both the compact pill and the larger live-transcription card (which must be
/// centered on its own width, not the pill's).
fn calculate_overlay_position_sized(
    app_handle: &AppHandle,
    width: f64,
    height: f64,
) -> Option<(f64, f64)> {
    let monitor = get_monitor_with_cursor(app_handle)?;
    let settings = settings::get_settings(app_handle);
    Some(placement_for(
        &monitor,
        width,
        height,
        settings.overlay_position,
    ))
}

/// Convenience wrapper: position for the compact pill (the default size).
fn calculate_overlay_position(app_handle: &AppHandle) -> Option<(f64, f64)> {
    calculate_overlay_position_sized(app_handle, OVERLAY_WIDTH, OVERLAY_HEIGHT)
}

/// The overlay window's current size in logical pixels, or the compact pill
/// size as a fallback. Lets `update_overlay_position` re-center correctly
/// regardless of whether the overlay is currently the pill or the larger card.
fn current_overlay_logical_size(window: &tauri::webview::WebviewWindow) -> (f64, f64) {
    let scale = window.scale_factor().unwrap_or(1.0);
    match window.inner_size() {
        Ok(size) if size.width > 0 && size.height > 0 => {
            (size.width as f64 / scale, size.height as f64 / scale)
        }
        _ => (OVERLAY_WIDTH, OVERLAY_HEIGHT),
    }
}

/// Creates the recording overlay window and keeps it hidden by default
#[cfg(not(target_os = "macos"))]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    // On Linux (Wayland), monitor detection often fails, but we don't need exact coordinates
    // for Layer Shell as we use anchors. On other platforms, we require a monitor.
    #[cfg(not(target_os = "linux"))]
    {
        let position = calculate_overlay_position(app_handle);
        if position.is_none() {
            debug!("Failed to determine overlay position, not creating overlay window");
            return;
        }
    }

    // Position starts unset — update_overlay_position() sets the correct
    // LogicalPosition before the overlay is shown.
    let mut builder = WebviewWindowBuilder::new(
        app_handle,
        "recording_overlay",
        tauri::WebviewUrl::App("src/overlay/index.html".into()),
    )
    // Must match every other window's args (see WEBVIEW2_BROWSER_ARGS).
    // Windows/WebView2 only; no-op on macOS/Linux.
    .additional_browser_args(crate::WEBVIEW2_BROWSER_ARGS)
    .title("Recording")
    .resizable(false)
    .inner_size(
        OVERLAY_WIDTH * overlay_text_scale(),
        OVERLAY_HEIGHT * overlay_text_scale(),
    )
    .shadow(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .accept_first_mouse(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .focused(false)
    .visible(false);

    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    #[allow(unused_variables)]
    match builder.build() {
        Ok(window) => {
            // The visible pill is much smaller than this window (see
            // OVERLAY_WIDTH), so most of the overlay is transparent padding that
            // still sat in front of the user's app and swallowed every click that
            // landed on it. Worse, a click that reached the overlay *activated*
            // it, which moved keyboard focus off the field being dictated into —
            // and the synthetic Ctrl+V then pasted the transcript into the
            // overlay's own webview, where it went nowhere. So the window is
            // click-through by default; `show_overlay_state_with_notice` gives
            // the live card the pointer only where it cannot take focus (see
            // `live_card_takes_pointer`), and `finish_recording_overlay` gives it
            // back everywhere once the paste has happened.
            let _ = window.set_ignore_cursor_events(true);

            // Belt and braces on Windows: WS_EX_NOACTIVATE means even a click
            // that does reach the overlay cannot take the foreground, so the
            // paste target survives regardless of where the pointer is. Safe to
            // set here because the window is built hidden and unfocused — the
            // documented "cannot unfocus after set_focusable(false)" trap only
            // applies to a window that already holds focus.
            #[cfg(target_os = "windows")]
            let _ = window.set_focusable(false);

            #[cfg(target_os = "linux")]
            {
                // Try to initialize GTK layer shell, ignore errors if compositor doesn't support it
                if init_gtk_layer_shell(&window) {
                    debug!("GTK layer shell initialized for overlay window");
                } else {
                    debug!("GTK layer shell not available, falling back to regular window");
                    // No layer surface is keeping us above other windows, so fall
                    // back to the GTK keep-above hint (effective on X11/Xorg and
                    // XWayland; ignored on GNOME Wayland). Re-asserted on each show.
                    force_overlay_keep_above(&window);
                }
            }

            debug!("Recording overlay window created successfully (hidden)");
        }
        Err(e) => {
            debug!("Failed to create recording overlay window: {}", e);
        }
    }
}

/// Creates the recording overlay panel and keeps it hidden by default (macOS)
#[cfg(target_os = "macos")]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    if let Some((x, y)) = calculate_overlay_position(app_handle) {
        // PanelBuilder creates a Tauri window then converts it to NSPanel.
        // The window remains registered, so get_webview_window() still works.
        match PanelBuilder::<_, RecordingOverlayPanel>::new(app_handle, "recording_overlay")
            .url(WebviewUrl::App("src/overlay/index.html".into()))
            .title("Recording")
            .position(tauri::Position::Logical(tauri::LogicalPosition { x, y }))
            .level(PanelLevel::Status)
            .size(tauri::Size::Logical(tauri::LogicalSize {
                width: OVERLAY_WIDTH,
                height: OVERLAY_HEIGHT,
            }))
            .has_shadow(false)
            .transparent(true)
            .no_activate(true)
            .corner_radius(0.0)
            .with_window(|w| w.decorations(false).transparent(true))
            .collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary(),
            )
            .build()
        {
            Ok(panel) => {
                let _ = panel.hide();
            }
            Err(e) => {
                log::error!("Failed to create recording overlay panel: {}", e);
            }
        }
    }
}

/// Whether the currently selected transcription model supports native
/// live-streaming. Read-only capability lookup (via the model catalog) used to
/// resolve `OverlayStyle::Auto` into Live vs Minimal, for both the recording
/// overlay and the assistant. Returns false if the model info isn't available.
pub fn selected_model_supports_live(app: &AppHandle) -> bool {
    let settings = settings::get_settings(app);
    // In cloud mode there is no local model to ask, and the answer comes from
    // whether the selected provider has a realtime endpoint. Checked first
    // because `selected_model` may still name a local model the user last used
    // — or nothing at all, on an install that never downloaded one.
    if crate::stt_cloud::cloud_stt_active(&settings) {
        return crate::stt_cloud::cloud_stt_streaming_active(&settings);
    }
    let selected = settings.selected_model;
    app.try_state::<std::sync::Arc<crate::managers::model::ModelManager>>()
        .and_then(|mm| mm.get_model_info(&selected))
        .map(|info| info.supports_streaming)
        .unwrap_or(false)
}

fn show_overlay_state(app_handle: &AppHandle, state: &str) {
    show_overlay_state_with_notice(app_handle, state, None);
}

/// Show the overlay in `state`. Returns the lifetime it started, or `None` when
/// the overlay is switched off and nothing was shown.
fn show_overlay_state_with_notice(
    app_handle: &AppHandle,
    state: &str,
    notice: Option<String>,
) -> Option<u64> {
    let epoch = OVERLAY_LIFECYCLE.advance();
    // Check if overlay should be shown based on position setting
    let settings = settings::get_settings(app_handle);

    // Resolve the chosen overlay style (Auto follows the model: Live when the
    // selected model supports live streaming, else Minimal). `None` — from the
    // style OR the legacy position setting — means "show nothing".
    let supports_live = selected_model_supports_live(app_handle);
    let style = settings::resolve_overlay_style(settings.overlay_style, supports_live);
    if style == settings::OverlayStyle::None || settings.overlay_position == OverlayPosition::None {
        return None;
    }

    // Live → the enlarged readable card (running committed + tentative
    // transcript); Minimal → the compact pill. The card is only meaningful
    // while streaming actually produces text, but it degrades gracefully to a
    // waveform + state label for batch models, so it's safe to show on Live.
    // A recovery offer is always the compact pill: it has one line to say and
    // one thing to click, and a card-sized window taking the pointer would sit
    // in front of far more of the user's app than it needs to.
    let recovery = is_recovery_state(state);
    let streaming_window = style == settings::OverlayStyle::Live && !recovery;
    OVERLAY_STREAMING.store(streaming_window, Ordering::SeqCst);

    let (width, height) = if recovery {
        recovery_overlay_size(state, notice.is_some())
    } else if streaming_window {
        (OVERLAY_STREAM_WIDTH, OVERLAY_STREAM_HEIGHT)
    } else if matches!(state, "generating" | "vision" | "notice") {
        (OVERLAY_LABEL_WIDTH, OVERLAY_LABEL_HEIGHT)
    } else {
        (OVERLAY_WIDTH, OVERLAY_HEIGHT)
    };
    // Read per show, so a text-size change applies to the next dictation.
    let (width, height) = scale_overlay_size((width, height), overlay_text_scale());

    // Size the overlay for the current mode, then re-center for that exact
    // size, BEFORE showing it — so it never flashes at the wrong size/position.
    let mut placed_on = None;
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        // Already on screen means this is a state change mid-dictation
        // (recording → transcribing → cleanup). It re-centres for the new size
        // on the display the overlay is *on*; if the cursor has since moved to
        // another display, the follower moves it there once the cursor settles.
        // Snapping it across on every state change would move it while the user
        // is sweeping past, which is what the dwell exists to prevent.
        let on_screen = overlay_window.is_visible().unwrap_or(false);
        let _ = overlay_window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
        let monitor = if on_screen {
            overlay_window.current_monitor().ok().flatten()
        } else {
            None
        };
        placed_on = place_overlay_on(app_handle, &overlay_window, width, height, monitor);
    }

    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        // The compact pill has nothing to act on, so it never intercepts the
        // pointer. The live card takes it wherever that cannot cost the paste
        // target its focus; elsewhere it waits for `finish_recording_overlay`.
        // A recovery pill exists to be clicked, and is only ever offered where
        // that is safe (see `show_recovery_overlay`).
        let interactive = (streaming_window || recovery) && live_card_takes_pointer();
        let _ = overlay_window.set_ignore_cursor_events(!interactive);
        let _ = overlay_window.show();

        // On Windows, aggressively re-assert "topmost" in the native Z-order after showing
        #[cfg(target_os = "windows")]
        force_overlay_topmost(&overlay_window);

        // …and keep asserting it for as long as the overlay is up, because
        // another process can take the position back at any point during a
        // recording.
        #[cfg(target_os = "windows")]
        start_overlay_topmost_guard(app_handle);

        // Follow the cursor to whichever display the user is working on.
        start_overlay_follow(app_handle, placed_on, (width, height));

        // On Linux, re-assert the keep-above hint after showing (for the
        // non-layer-shell fallback on X11/Xorg). No-op under layer shell and
        // under GNOME Wayland; cheap, and mirrors the Windows re-assert above.
        #[cfg(target_os = "linux")]
        force_overlay_keep_above(&overlay_window);

        let _ = overlay_window.emit(
            "show-overlay",
            ShowOverlayPayload {
                state: state.to_string(),
                streaming_window,
                interactive,
                notice,
                epoch,
            },
        );
        return Some(epoch);
    }
    None
}

/// Shows the recording overlay window with fade-in animation
pub fn show_recording_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "recording");
}

/// Shows the transcribing overlay window
pub fn show_transcribing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "transcribing");
}

/// Shows the processing overlay window
pub fn show_processing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "processing");
}

/// Shows the overlay in the "generating" state (Flow is writing).
pub fn show_generating_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "generating");
}

/// Shows the overlay in the "vision" state (Flow is looking at the screen).
pub fn show_vision_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "vision");
}

/// Shows a brief text notice on the overlay (e.g. a Flow error), then hides
/// it after a short delay. `notice_key` is the i18n suffix under
/// `overlay.notices.*` in the webview.
pub fn show_overlay_notice(app_handle: &AppHandle, notice_key: &str) {
    show_overlay_state_with_notice(app_handle, "notice", Some(notice_key.to_string()));
    let epoch = OVERLAY_LIFECYCLE.current();
    let app = app_handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2600));
        // Only hide if no newer overlay state replaced the notice meanwhile.
        if OVERLAY_LIFECYCLE.current() == epoch {
            hide_recording_overlay(&app);
        }
    });
}

/// Offer a dismissed or failed dictation back on the pill: "Dismissed · Undo"
/// or "Transcription failed · Try again". `state` is `"dismissed"` or
/// `"failed"`; `notice` replaces the pill's words with an `overlay.notices.*`
/// explanation.
///
/// `on_shown` receives the overlay lifetime the pill was shown in, before
/// anything can click it, so the caller can key its offer to exactly this pill.
/// The pill waits [`RECOVERY_LINGER`] (longer while the pointer is on it), then
/// fades out and the offer expires; replaced by anything else first, the offer
/// is withdrawn.
///
/// Returns false, showing nothing, where a clickable overlay could take
/// keyboard focus from the app being dictated into (see
/// [`live_card_takes_pointer`]) or when the overlay is switched off. The
/// dictation is still in History either way.
pub fn show_recovery_overlay(
    app: &AppHandle,
    state: &str,
    notice: Option<&'static str>,
    on_shown: impl FnOnce(u64),
) -> bool {
    if !live_card_takes_pointer() {
        return false;
    }
    set_overlay_hovered(false);
    let Some(epoch) = show_overlay_state_with_notice(app, state, notice.map(str::to_string)) else {
        return false;
    };
    let Some(window) = app.get_webview_window("recording_overlay") else {
        return false;
    };
    on_shown(epoch);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let ready = OVERLAY_LIFECYCLE
            .wait_for_dismissal(
                epoch,
                RECOVERY_LINGER,
                std::time::Duration::from_millis(OVERLAY_FADE_MS),
                |fading| {
                    let _ = window.emit(
                        if fading {
                            "fade-overlay"
                        } else {
                            "restore-overlay"
                        },
                        epoch,
                    );
                },
            )
            .await;
        if !ready {
            // Something else took the overlay: an Undo that is now running, or
            // a new recording. Either way this pill's offer is over.
            crate::dictation_recovery::withdraw(epoch);
            return;
        }
        crate::dictation_recovery::expire(epoch);
        let app_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            if OVERLAY_LIFECYCLE.is_current(epoch) {
                hide_recording_overlay(&app_main);
            }
        });
    });
    true
}

/// A click reached a recovery pill whose offer is already gone (it expired a
/// moment earlier, or History recovered the same dictation). Take the pill
/// down rather than leave a button that does nothing.
pub fn dismiss_recovery_overlay(app: &AppHandle, epoch: u64) {
    if OVERLAY_LIFECYCLE.is_current(epoch) {
        hide_recording_overlay(app);
    }
}

/// Updates the overlay window position based on current settings, re-centering
/// for the overlay's current size (compact pill or larger live card).
pub fn update_overlay_position(app_handle: &AppHandle) {
    let (width, height) = app_handle
        .get_webview_window("recording_overlay")
        .map(|w| current_overlay_logical_size(&w))
        .unwrap_or((OVERLAY_WIDTH, OVERLAY_HEIGHT));
    update_overlay_position_sized(app_handle, width, height);
}

/// Positions the overlay window centered for a window of the given logical
/// size. Shared by the compact pill and the larger live-transcription card so
/// both re-center correctly on the monitor under the cursor.
fn update_overlay_position_sized(app_handle: &AppHandle, width: f64, height: f64) {
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        let _ = place_overlay_on(app_handle, &overlay_window, width, height, None);
    }
}

/// Centres the overlay on `monitor` for the given logical size, or on the
/// monitor under the cursor when `monitor` is `None`. Returns the monitor it
/// was placed on, which is where the cursor follower starts from.
fn place_overlay_on(
    app_handle: &AppHandle,
    overlay_window: &tauri::webview::WebviewWindow,
    width: f64,
    height: f64,
    monitor: Option<tauri::Monitor>,
) -> Option<tauri::Monitor> {
    #[cfg(target_os = "linux")]
    {
        update_gtk_layer_shell_anchors(overlay_window);
    }

    let monitor = monitor.or_else(|| get_monitor_with_cursor(app_handle))?;
    let position = settings::get_settings(app_handle).overlay_position;
    set_overlay_placement(
        overlay_window,
        placement_for(&monitor, width, height, position),
    );
    Some(monitor)
}

/// A successful dictation leaves its final (including cleaned-up) text available
/// to copy. Compact mode keeps its immediate dismissal and original footprint.
pub fn finish_recording_overlay(app: &AppHandle, text: &str, notice: Option<&str>) {
    if !OVERLAY_STREAMING.load(Ordering::SeqCst) || text.trim().is_empty() {
        if let Some(notice) = notice {
            show_overlay_notice(app, notice);
        } else {
            if !text.trim().is_empty() {
                // Let the compact progress bar finish during the existing hide
                // transition. Pasting has already happened; no extra wait is
                // added. An empty/cancelled recording never signals success.
                if let Some(window) = app.get_webview_window("recording_overlay") {
                    let _ = window.emit(
                        "finish-overlay",
                        serde_json::json!({
                            "epoch": OVERLAY_LIFECYCLE.current(), "text": ""
                        }),
                    );
                }
            }
            hide_recording_overlay(app);
        }
        return;
    }
    let epoch = OVERLAY_LIFECYCLE.advance();
    let Some(window) = app.get_webview_window("recording_overlay") else {
        return;
    };
    // The paste has happened, so the card can take the pointer on every platform
    // now — including an X11 window, which had to stay click-through while a
    // click could still have moved focus off the paste target. Its hover also
    // holds the linger open.
    let _ = window.set_ignore_cursor_events(false);
    let _ = window.emit(
        "finish-overlay",
        serde_json::json!({ "epoch": epoch, "text": text, "notice": notice }),
    );
    let linger = settings::get_settings(app).overlay_linger.duration();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let ready = OVERLAY_LIFECYCLE
            .wait_for_dismissal(
                epoch,
                linger,
                std::time::Duration::from_millis(OVERLAY_FADE_MS),
                |fading| {
                    let _ = window.emit(
                        if fading {
                            "fade-overlay"
                        } else {
                            "restore-overlay"
                        },
                        epoch,
                    );
                },
            )
            .await;
        if ready {
            let _ = app.run_on_main_thread(move || {
                if OVERLAY_LIFECYCLE.is_current(epoch) {
                    #[cfg(target_os = "windows")]
                    stop_overlay_topmost_guard();
                    stop_overlay_follow();
                    let _ = window.emit("hide-overlay", ());
                    let _ = window.set_ignore_cursor_events(true);
                    let _ = window.hide();
                    set_overlay_hovered(false);
                }
            });
        }
    });
}

/// Cancellation and empty/error outcomes never leave a completed card behind.
pub fn hide_recording_overlay(app_handle: &AppHandle) {
    let epoch = OVERLAY_LIFECYCLE.advance();
    set_overlay_hovered(false);
    // Retire the watcher before the window goes down, so a tick already queued on
    // the main thread cannot put a deliberately hidden overlay back on screen.
    #[cfg(target_os = "windows")]
    stop_overlay_topmost_guard();
    // Same for the cursor follower: a hop must not move (or, on some platforms,
    // re-map) a window that is on its way down.
    stop_overlay_follow();
    if let Some(window) = app_handle.get_webview_window("recording_overlay") {
        let _ = window.emit("hide-overlay", ());
        let app = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(240)).await;
            let _ = app.run_on_main_thread(move || {
                if OVERLAY_LIFECYCLE.current() == epoch {
                    let _ = window.set_ignore_cursor_events(true);
                    let _ = window.hide();
                }
            });
        });
    }
}

pub fn emit_levels(app_handle: &AppHandle, levels: &Vec<f32>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Throttle to ~30 FPS. Ported from Handy #1279: every `emit` becomes an
    // `evaluate_script` call in each listening WebView, and wry's
    // `evaluate_script` has an upstream memory leak (tauri-apps/wry#1489; the
    // WebView2/Windows equivalent is tauri#12724) that accumulates per call and
    // is never released. The audio visualizer fired this ~94x/sec — and
    // previously TWICE per frame via a redundant second broadcast — so over a
    // long session WebView memory grew into the gigabytes and eventually caused
    // swap thrashing / OOM (the "memory climbs, disk 100%, PC freezes" reports).
    // ~30 FPS keeps the meter smooth while cutting emit volume by roughly 6x.
    const EMIT_THROTTLE_MS: u64 = 33;
    static LAST_EMIT_MS: AtomicU64 = AtomicU64::new(0);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if now.saturating_sub(LAST_EMIT_MS.load(Ordering::Relaxed)) < EMIT_THROTTLE_MS {
        return;
    }
    LAST_EMIT_MS.store(now, Ordering::Relaxed);

    // A single global broadcast reaches every window that registered a
    // `mic-level` listener (currently the recording overlay AND the assistant
    // panel) and, thanks to Tauri's listener filtering, does not evaluate script
    // in windows without one. This replaces the old pair of `.emit()` calls that
    // delivered the event to the overlay twice per frame.
    let _ = app_handle.emit("mic-level", levels);
}

#[cfg(test)]
mod text_scale_tests {
    use super::{clamp_text_scale, scale_overlay_size, OVERLAY_HEIGHT, OVERLAY_WIDTH};

    #[test]
    fn the_overlay_grows_with_the_windows_text_size() {
        assert_eq!(
            scale_overlay_size((OVERLAY_WIDTH, OVERLAY_HEIGHT), 1.0),
            (OVERLAY_WIDTH, OVERLAY_HEIGHT)
        );
        assert_eq!(scale_overlay_size((400.0, 120.0), 1.5), (600.0, 180.0));
    }

    #[test]
    fn a_corrupt_text_scale_is_clamped_to_what_windows_allows() {
        assert_eq!(clamp_text_scale(0.5), 1.0);
        assert_eq!(clamp_text_scale(1.25), 1.25);
        assert_eq!(clamp_text_scale(9.0), 2.25);
        assert_eq!(clamp_text_scale(f64::NAN), 1.0);
    }
}
