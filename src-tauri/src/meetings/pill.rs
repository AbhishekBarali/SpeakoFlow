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
//! while the app runs. The build happens **inline on the calling thread**:
//! dispatching a webview build
//! to the main thread from inside a Tauri command's call stack deadlocks WebView2
//! on Windows. Everything *after* the build — show, hide, size, position,
//! focusability — is queued onto the main thread, because those are ordinary
//! window operations and the callers are audio and shortcut threads.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

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

/// The collapsed pill: a dot, a small waveform and the clock — the same object
/// as the dictation overlay's compact pill, and about as small. The window is
/// exactly the pill, with no transparent frame around it, so there is no
/// invisible margin sitting over the user's work and eating clicks. Hovering
/// swaps the waveform for pause and stop *inside* the same width, which is what
/// lets the controls appear without the window resizing under the pointer.
const PILL_WIDTH_COLLAPSED: f64 = 152.0;
/// The call offer is a sentence and two buttons; it needs more room than the
/// recording pill.
const PILL_WIDTH_OFFER: f64 = 304.0;
const PILL_WIDTH_EXPANDED: f64 = 420.0;

/// Used only until the webview reports its measured height.
const COLLAPSED_FALLBACK_HEIGHT: f64 = 36.0;
const EXPANDED_FALLBACK_HEIGHT: f64 = 420.0;

/// Floor, so a measurement that arrives mid-render cannot collapse the window to
/// nothing and leave the user with no way to click it.
const MIN_HEIGHT: f64 = 30.0;

/// Ceiling on the expanded card, as a fraction of the display's height.
///
/// The clamp lives here rather than in CSS because the webview cannot see the
/// display: its own `100vh` is only ever the window it is already in, so a
/// transcript that keeps growing would keep growing the window.
const MAX_HEIGHT_FRACTION: f64 = 0.62;

/// Gap between the pill and the display edge it first docks against.
const EDGE_MARGIN: f64 = 16.0;

/// Kept clear at the top and bottom of the display: the macOS menu bar, the
/// Windows taskbar. Monitor bounds rather than `work_area()` for the reason
/// `overlay.rs` gives — the latter misreports negative-origin displays on macOS.
#[cfg(target_os = "macos")]
const TOP_CLEARANCE: f64 = 32.0;
#[cfg(not(target_os = "macos"))]
const TOP_CLEARANCE: f64 = 8.0;
#[cfg(target_os = "macos")]
const BOTTOM_CLEARANCE: f64 = 16.0;
#[cfg(not(target_os = "macos"))]
const BOTTOM_CLEARANCE: f64 = 48.0;

/// Where the pill was left, in the settings store, so the next meeting — and the
/// next launch — puts it back there instead of at the default.
const POSITION_KEY: &str = "meeting_pill_position";

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

/// Whether the window is currently showing a call *offer* rather than a recording.
///
/// The two share one window, so "take the offer down" and "take the recorder down"
/// are the same call — which makes it essential to know which one is up. Withdrawing
/// an offer that has already been replaced by a live recording would hide the
/// indicator for a meeting that is still capturing, and that is the one state this
/// window exists to never show.
static OFFER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Which corner of the window stays fixed when it changes size.
///
/// Decided from where the *small* form sits and then kept while the card is
/// open, so the card grows out of the pill and folds back into the same corner.
/// Recomputing it from the card instead made a pill that opened downward come
/// back at the card's far end.
static CORNER: Mutex<Option<Corner>> = Mutex::new(None);

/// The shape the window currently has on screen, `None` until it is first
/// placed this run.
static APPLIED: Mutex<Option<PillShape>> = Mutex::new(None);

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
/// WebView2 on Windows. Call this from a `spawn_blocking` closure or a background
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

    // A recording supersedes any offer, so the window is no longer an offer. Without
    // this, a later "the call ended" would withdraw an offer that no longer exists
    // and hide the live recording indicator with it.
    OFFER_ACTIVE.store(false, Ordering::SeqCst);

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
        // Collapsed, like the geometry just applied (see `hide_pill`).
        let _ = app_main.emit_to(PILL_LABEL, PILL_MODE_EVENT, false);
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
        // Remembered before it goes, while the position is still the one the
        // user can see.
        save_position(&app_main, &window);
        // The webview keeps its own `expanded` state and only hears about a mode
        // change from `set_pill_expanded`. Collapsing here without telling it
        // left the next meeting's pill rendering the full card inside the
        // collapsed-size, unfocusable window: squeezed, clipped and untypeable.
        let _ = app_main.emit_to(PILL_LABEL, PILL_MODE_EVENT, false);
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

    OFFER_ACTIVE.store(true, Ordering::SeqCst);
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
    OFFER_ACTIVE.store(false, Ordering::SeqCst);
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

/// Take down an offer the user never answered, because the call it was about is
/// over.
///
/// The gap this closes: the detector's own end-of-call branch reset its internal
/// state and told nobody, and the only things that could remove an offer were the
/// user's own X, a meeting starting, and the setting being switched off. So a card
/// nobody touched stayed on screen, always on top, indefinitely — long after the
/// call it was asking about had finished. "After I'm done with the call it's just
/// annoying" is exactly this.
///
/// Guarded on the window actually showing an offer, because the offer and the live
/// recorder are the same window: unguarded, a call ending 20 seconds after the user
/// accepted would hide the indicator for the meeting still being recorded.
pub fn withdraw_call_offer(app: &AppHandle) {
    if !OFFER_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    debug!("The detected call ended with the offer unanswered; withdrawing it");
    hide_call_offer(app);
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
///
/// # Where it goes
///
/// It used to be recomputed from scratch on every call — bottom centre of
/// whichever display had the cursor — so every measurement the webview reported
/// snapped a pill the user had dragged aside straight back to the middle of the
/// screen. Now the window's own position is the input: the first placement in a
/// run uses the remembered spot (or the default), and every later one resizes
/// around a fixed corner of wherever the window is right now.
///
/// The default is the right edge, vertically centred, rather than the bottom
/// centre. Bottom centre is where the dictation overlay and the assistant's call
/// bar both live, and the meeting pill sat directly on top of the call's status
/// bubble — two always-on-top windows fighting for the same pixels.
fn apply_geometry(app: &AppHandle, window: &tauri::WebviewWindow) {
    let shape = current_shape();
    let width = shape.width();
    let applied = APPLIED.lock().ok().and_then(|slot| *slot);

    // Where the window is now, when it has been placed this run; otherwise where
    // it was left last time. Either answer brings the display it is on, so the
    // clamp and the position agree about which screen they mean.
    let placed = if applied.is_some() {
        live_rect(app, window)
    } else {
        saved_rect(app)
    };
    let (current, monitor) = match placed {
        Some((rect, monitor)) => (Some(rect), Some(monitor)),
        None => (
            None,
            crate::overlay::get_monitor_with_cursor(app)
                .or_else(|| window.current_monitor().ok().flatten()),
        ),
    };

    let Some(monitor) = monitor else {
        let height = resolve_height(shape == PillShape::Expanded, None);
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
        return;
    };
    let area = usable_area(&monitor);
    let height = resolve_height(shape == PillShape::Expanded, Some(area.h));

    let target = match current {
        Some(current) => {
            // Only a small form chooses the corner. The card keeps the one it
            // opened from, so collapsing lands the pill where it started.
            let stored = CORNER.lock().ok().and_then(|slot| *slot);
            let corner = match (applied, stored) {
                (Some(PillShape::Expanded), Some(corner)) => corner,
                _ => {
                    let corner = corner_for(current, area);
                    if let Ok(mut slot) = CORNER.lock() {
                        *slot = Some(corner);
                    }
                    corner
                }
            };
            resize_from_corner(current, corner, width, height, area)
        }
        None => default_rect(width, height, area),
    };

    let _ = window.set_size(tauri::LogicalSize::new(target.w, target.h));
    // Logical, never physical: tao converts a `PhysicalPosition` using the scale
    // factor of the monitor the window is *currently* on, which is the wrong one
    // whenever the window is moving between displays.
    let _ = window.set_position(tauri::LogicalPosition::new(target.x, target.y));
    if let Ok(mut slot) = APPLIED.lock() {
        *slot = Some(shape);
    }
}

/// Which form the window should take right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PillShape {
    Collapsed,
    Offer,
    Expanded,
}

impl PillShape {
    fn width(self) -> f64 {
        match self {
            Self::Collapsed => PILL_WIDTH_COLLAPSED,
            Self::Offer => PILL_WIDTH_OFFER,
            Self::Expanded => PILL_WIDTH_EXPANDED,
        }
    }
}

fn current_shape() -> PillShape {
    if PILL_EXPANDED.load(Ordering::SeqCst) {
        PillShape::Expanded
    } else if OFFER_ACTIVE.load(Ordering::SeqCst) {
        PillShape::Offer
    } else {
        PillShape::Collapsed
    }
}

/// A rectangle in one display's logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The corner that stays put when the window changes size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Corner {
    right: bool,
    bottom: bool,
}

/// The corner nearest the display edges the window sits against.
///
/// A pill in the right half keeps its right edge, so the card opens leftward
/// into the screen instead of off it; a pill in the lower half keeps its bottom
/// edge and the card opens upward. Pure, like the rest of the arithmetic here.
fn corner_for(rect: Rect, area: Rect) -> Corner {
    let (cx, cy) = rect.centre();
    let (ax, ay) = area.centre();
    Corner {
        right: cx >= ax,
        bottom: cy >= ay,
    }
}

/// Resize `current` to `w`x`h` around `corner`, then keep it on the display.
fn resize_from_corner(current: Rect, corner: Corner, w: f64, h: f64, area: Rect) -> Rect {
    let x = if corner.right {
        current.x + current.w - w
    } else {
        current.x
    };
    let y = if corner.bottom {
        current.y + current.h - h
    } else {
        current.y
    };
    clamp_into(Rect { x, y, w, h }, area)
}

/// First placement ever: docked to the right edge, vertically centred.
fn default_rect(w: f64, h: f64, area: Rect) -> Rect {
    clamp_into(
        Rect {
            x: area.x + area.w - w - EDGE_MARGIN,
            y: area.y + (area.h - h) / 2.0,
            w,
            h,
        },
        area,
    )
}

/// Keep a rectangle fully inside `area`, moving it as little as possible.
fn clamp_into(rect: Rect, area: Rect) -> Rect {
    let max_x = (area.x + area.w - rect.w).max(area.x);
    let max_y = (area.y + area.h - rect.h).max(area.y);
    Rect {
        x: rect.x.clamp(area.x, max_x),
        y: rect.y.clamp(area.y, max_y),
        ..rect
    }
}

/// A monitor's bounds in its own logical points, minus the system chrome.
fn usable_area(monitor: &tauri::Monitor) -> Rect {
    let scale = monitor.scale_factor();
    let x = monitor.position().x as f64 / scale;
    let y = monitor.position().y as f64 / scale;
    let w = monitor.size().width as f64 / scale;
    let h = monitor.size().height as f64 / scale;
    Rect {
        x,
        y: y + TOP_CLEARANCE,
        w,
        h: (h - TOP_CLEARANCE - BOTTOM_CLEARANCE).max(MIN_HEIGHT),
    }
}

/// The monitor whose physical bounds contain a physical point.
///
/// Hit-tested in physical pixels because logical coordinates of neighbouring
/// displays with different scale factors do not tile — the same trap
/// `overlay::get_monitor_with_cursor` documents.
fn monitor_at(app: &AppHandle, px: f64, py: f64) -> Option<tauri::Monitor> {
    app.available_monitors().ok()?.into_iter().find(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        px >= position.x as f64
            && px < position.x as f64 + size.width as f64
            && py >= position.y as f64
            && py < position.y as f64 + size.height as f64
    })
}

/// A physical rectangle, expressed in the logical points of the display that
/// holds its centre.
fn physical_to_logical(
    app: &AppHandle,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Option<(Rect, tauri::Monitor)> {
    let monitor = monitor_at(app, x + w / 2.0, y + h / 2.0)?;
    let scale = monitor.scale_factor();
    Some((
        Rect {
            x: x / scale,
            y: y / scale,
            w: w / scale,
            h: h / scale,
        },
        monitor,
    ))
}

/// Where the window is on screen right now.
fn live_rect(app: &AppHandle, window: &tauri::WebviewWindow) -> Option<(Rect, tauri::Monitor)> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    physical_to_logical(
        app,
        position.x as f64,
        position.y as f64,
        size.width as f64,
        size.height as f64,
    )
}

/// Where the pill was left at the end of an earlier meeting, if that spot is
/// still on a connected display.
fn saved_rect(app: &AppHandle) -> Option<(Rect, tauri::Monitor)> {
    let value = crate::settings::settings_file(app).get(POSITION_KEY)?;
    let read = |key: &str| value.get(key).and_then(serde_json::Value::as_f64);
    let (x, y, w, h) = (read("x")?, read("y")?, read("w")?, read("h")?);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    physical_to_logical(app, x, y, w, h)
}

/// Remember where the pill is, in physical pixels, as the collapsed pill.
///
/// The collapsed form is what gets stored even when the card is open, so the
/// next meeting starts from the pill the card grew out of rather than from the
/// card's top-left corner. Main thread only.
fn save_position(app: &AppHandle, window: &tauri::WebviewWindow) {
    if APPLIED.lock().ok().and_then(|slot| *slot).is_none() {
        return;
    }
    let Some((rect, monitor)) = live_rect(app, window) else {
        return;
    };
    let area = usable_area(&monitor);
    let pill = match CORNER.lock().ok().and_then(|slot| *slot) {
        Some(corner) if rect.w > PILL_WIDTH_COLLAPSED + 1.0 => resize_from_corner(
            rect,
            corner,
            PILL_WIDTH_COLLAPSED,
            COLLAPSED_FALLBACK_HEIGHT,
            area,
        ),
        _ => rect,
    };
    let scale = monitor.scale_factor();
    crate::settings::settings_file(app).set(
        POSITION_KEY,
        serde_json::json!({
            "x": pill.x * scale,
            "y": pill.y * scale,
            "w": pill.w * scale,
            "h": pill.h * scale,
        }),
    );
}

/// The height to use, given the mode and the display height to clamp against.
fn resolve_height(expanded: bool, display_height: Option<f64>) -> f64 {
    let fallback = if expanded {
        EXPANDED_FALLBACK_HEIGHT
    } else {
        COLLAPSED_FALLBACK_HEIGHT
    };
    let measured = match PILL_HEIGHT.load(Ordering::SeqCst) {
        0 => fallback,
        value => (value as f64).max(MIN_HEIGHT),
    };
    clamp_height(measured, expanded, display_height)
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
        // windows. Re-raise only when what covers us belongs to someone else.
        //
        // Unconditionally re-raising every tick was a fight with our own
        // windows: the assistant panel and the dictation overlay are topmost
        // too, and a meeting pill that jumped back above them 1.4 times a second
        // punched through an open assistant conversation wherever the two
        // overlapped. Those are windows the user opened on purpose, on top of
        // this one; leaving them there is the right stacking.
        if topmost_neighbour_is_ours(hwnd) {
            return;
        }
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

/// Whether the nearest visible window stacked above the pill is one of ours.
///
/// Bounded, because a z-order walk over a busy desktop is otherwise unbounded
/// work on the main thread every tick.
#[cfg(target_os = "windows")]
unsafe fn topmost_neighbour_is_ours(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowThreadProcessId, IsWindowVisible, GW_HWNDPREV,
    };

    let own_pid = GetCurrentProcessId();
    let mut current = hwnd;
    for _ in 0..64 {
        let Ok(above) = GetWindow(current, GW_HWNDPREV) else {
            // Nothing above us at all: already first.
            return true;
        };
        if above.is_invalid() {
            return true;
        }
        if IsWindowVisible(above).as_bool() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(above, Some(&mut pid));
            return pid == own_pid;
        }
        current = above;
    }
    false
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
        assert!(PILL_WIDTH_OFFER > PILL_WIDTH_COLLAPSED);
    }

    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1000.0,
    };

    /// The default dock is the right edge, clear of the bottom centre where the
    /// dictation overlay and the assistant's call bar live.
    #[test]
    fn the_default_pill_docks_to_the_right_edge() {
        let pill = default_rect(PILL_WIDTH_COLLAPSED, 36.0, AREA);
        assert_eq!(pill.x + pill.w, AREA.w - EDGE_MARGIN);
        let (_, cy) = pill.centre();
        assert!((cy - AREA.h / 2.0).abs() < 1.0, "vertically centred");
    }

    /// The card grows out of the pill's own corner and folds back into it, so a
    /// round trip lands the pill exactly where it started.
    #[test]
    fn expanding_and_collapsing_returns_the_pill_to_its_spot() {
        let pill = Rect {
            x: 1700.0,
            y: 700.0,
            w: PILL_WIDTH_COLLAPSED,
            h: 36.0,
        };
        let corner = corner_for(pill, AREA);
        assert_eq!(
            corner,
            Corner {
                right: true,
                bottom: true
            }
        );
        let card = resize_from_corner(pill, corner, PILL_WIDTH_EXPANDED, 400.0, AREA);
        assert_eq!(card.x + card.w, pill.x + pill.w, "right edges agree");
        assert_eq!(card.y + card.h, pill.y + pill.h, "bottom edges agree");
        let back = resize_from_corner(card, corner, PILL_WIDTH_COLLAPSED, 36.0, AREA);
        assert_eq!(back, pill);
    }

    /// A pill dragged to the top-left opens down and to the right, into the
    /// screen rather than off it.
    #[test]
    fn a_pill_in_the_top_left_opens_down_and_right() {
        let pill = Rect {
            x: 40.0,
            y: 60.0,
            w: PILL_WIDTH_COLLAPSED,
            h: 36.0,
        };
        let corner = corner_for(pill, AREA);
        let card = resize_from_corner(pill, corner, PILL_WIDTH_EXPANDED, 400.0, AREA);
        assert_eq!((card.x, card.y), (pill.x, pill.y));
    }

    /// A card that would overhang the display is moved back onto it.
    #[test]
    fn a_card_is_kept_on_the_display() {
        let pill = Rect {
            x: 1760.0,
            y: 20.0,
            w: PILL_WIDTH_COLLAPSED,
            h: 36.0,
        };
        let card = resize_from_corner(
            pill,
            Corner {
                right: false,
                bottom: true,
            },
            PILL_WIDTH_EXPANDED,
            400.0,
            AREA,
        );
        assert!(card.x + card.w <= AREA.w);
        assert!(card.y >= AREA.y);
    }
}
