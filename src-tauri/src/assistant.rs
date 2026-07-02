//! Assistant mode: voice question → local STT → LLM → streaming answer in a
//! floating always-on-top panel window.
//!
//! Conversation state lives in memory (cleared on app restart or via the
//! panel's clear button). Requests are built cache-friendly: byte-identical
//! system prompt first, then append-only history, newest user message last.

use crate::llm_client::{self, ChatMessage};
use crate::settings::get_settings;
use crate::web_search;
use log::{debug, error, warn};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};
use tauri_plugin_store::StoreExt;
use tokio::sync::Notify;

pub const PANEL_LABEL: &str = "assistant_panel";
const PANEL_MARGIN: f64 = 24.0;
const PANEL_POSITION_KEY: &str = "assistant_panel_position";

/// Collapsed "pill" mode: a small transparent window in which the chip floats
/// and hugs its content, like the STT recording overlay (128×40 there).
const PILL_WIDTH: f64 = 240.0;
const PILL_HEIGHT: f64 = 44.0;

/// The one expanded panel size. There are no size presets; the window stays
/// user-resizable, and a manual resize is remembered for the session (below)
/// so collapse → expand round-trips keep it.
const PANEL_WIDTH: f64 = 400.0;
const PANEL_HEIGHT: f64 = 560.0;

/// Session memory of the last expanded size (logical px), so collapsing to the
/// pill and expanding again restores a manual resize. 0 = never resized — use
/// the default. Not persisted: a fresh app start uses the standard size.
static EXPANDED_W: AtomicU32 = AtomicU32::new(0);
static EXPANDED_H: AtomicU32 = AtomicU32::new(0);

fn expanded_size() -> (f64, f64) {
    let w = EXPANDED_W.load(Ordering::SeqCst);
    let h = EXPANDED_H.load(Ordering::SeqCst);
    if w == 0 || h == 0 {
        (PANEL_WIDTH, PANEL_HEIGHT)
    } else {
        (w as f64, h as f64)
    }
}

/// Whether the panel is currently collapsed to the pill. Starts collapsed so
/// the assistant first appears as the small pill rather than the full panel;
/// expanding (or collapsing) updates it for the rest of the session.
static PILL_MODE: AtomicBool = AtomicBool::new(true);

/// Sticky "attach the screen to assistant turns" flag, set by the panel's
/// camera toggle. Persists across turns until the user turns it off (the
/// pill/panel show a camera badge the whole time it's armed).
static SCREEN_ARMED: AtomicBool = AtomicBool::new(false);

pub fn set_screen_armed(app: &AppHandle, armed: bool) {
    SCREEN_ARMED.store(armed, Ordering::SeqCst);
    let _ = app.emit("assistant-screen-armed", armed);
}

/// Whether screen vision is currently armed. Sticky: reading does NOT clear it.
pub fn screen_armed() -> bool {
    SCREEN_ARMED.load(Ordering::SeqCst)
}

/// Appended to the stored user message when a screenshot was sent with it.
/// The panel strips it for display and shows a chip instead; on later turns
/// it tells the model a screenshot accompanied that message.
pub const SCREENSHOT_MARKER: &str = "[screenshot attached]";

/// Appended (one per image) when the user attached images to the message.
/// Stripped for display like the screenshot marker — keep in sync with
/// AssistantPanel.tsx.
pub const IMAGE_MARKER: &str = "[image attached]";

/// Prefix for per-file attachment markers: `[file attached: name.ext]`.
/// Keep in sync with AssistantPanel.tsx.
pub const FILE_MARKER_PREFIX: &str = "[file attached:";

/// A text-like file attached to a turn as context (content extracted in the
/// webview or by `assistant_read_file`).
#[derive(Clone, serde::Deserialize, serde::Serialize, specta::Type)]
pub struct FileAttachment {
    pub name: String,
    pub content: String,
}

/// Frozen full-screen capture waiting for the user to pick a region in the
/// snip overlay. Captured BEFORE the overlay opens, so the dimmer (and any
/// on-screen assistant window churn) can never photobomb the crop.
pub static PENDING_SNIP: Mutex<Option<image::DynamicImage>> = Mutex::new(None);

/// Attachments staged in the panel (chips above the input) and mirrored here
/// so VOICE turns include them too — the pill/hotkey path runs entirely in
/// Rust and can't see the webview's React state.
static PENDING_ATTACHMENTS: Mutex<(Vec<String>, Vec<FileAttachment>)> =
    Mutex::new((Vec::new(), Vec::new()));

/// Mirror the panel's staged attachments (called on every add/remove).
pub fn set_pending_attachments(images: Vec<String>, files: Vec<FileAttachment>) {
    if let Ok(mut pending) = PENDING_ATTACHMENTS.lock() {
        *pending = (images, files);
    }
}

/// Take (and clear) the staged attachments for a turn that consumes them.
/// Tells the panel so its chips clear as well.
pub fn take_pending_attachments(app: &AppHandle) -> (Vec<String>, Vec<FileAttachment>) {
    let taken = PENDING_ATTACHMENTS
        .lock()
        .map(|mut p| std::mem::take(&mut *p))
        .unwrap_or_default();
    if !taken.0.is_empty() || !taken.1.is_empty() {
        let _ = app.emit("assistant-attachments-consumed", ());
    }
    taken
}

/// One-shot "route the current dictation to the assistant" flag, set by the
/// STT overlay's Ask-Assistant button just before it commits the recording.
/// Cleared on every dictation start so a stale click can never redirect a
/// later, unrelated dictation.
static TRANSCRIBE_REDIRECT: AtomicBool = AtomicBool::new(false);

pub fn set_transcribe_redirect() {
    TRANSCRIBE_REDIRECT.store(true, Ordering::SeqCst);
}

pub fn clear_transcribe_redirect() {
    TRANSCRIBE_REDIRECT.store(false, Ordering::SeqCst);
}

pub fn take_transcribe_redirect() -> bool {
    TRANSCRIBE_REDIRECT.swap(false, Ordering::SeqCst)
}

/// Phrases that signal the user is asking about what's on their screen.
/// When the screenshot toggle is on, these auto-attach a capture even on the
/// normal assistant hotkey.
pub fn wants_screen_context(text: &str) -> bool {
    let lower = text.to_lowercase();
    const PATTERNS: [&str; 14] = [
        "my screen",
        "the screen",
        "on screen",
        "my display",
        "the display",
        "my monitor",
        "what do you see",
        "what are you seeing",
        "can you see",
        "what am i looking at",
        "look at this",
        "looking at",
        "this error",
        "this page",
    ];
    PATTERNS.iter().any(|p| lower.contains(p))
}

/// In-memory conversation history, managed as Tauri state.
pub struct AssistantConversation {
    pub messages: Mutex<Vec<ChatMessage>>,
    /// Guards against duplicate concurrent turns (double-fired hotkeys etc).
    busy: AtomicBool,
    /// Notified when the user presses Stop, to cancel an in-flight turn.
    cancel: Arc<Notify>,
    /// Sticky cancel flag for the current turn. `Notify::notify_waiters` only
    /// wakes waiters registered *at that instant*, so a Stop pressed outside the
    /// streaming `select!` (e.g. while a web search is running, or in the race
    /// between the stream finishing and TTS starting) would otherwise be lost.
    /// This flag is set alongside the notify and checked at each turn stage.
    cancelled: AtomicBool,
    /// Row id of the conversation as persisted in the history database, or
    /// `None` before the first save and after the conversation is cleared.
    /// Lets each turn update the same row instead of creating duplicates.
    session_id: Mutex<Option<i64>>,
}

impl AssistantConversation {
    pub fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            busy: AtomicBool::new(false),
            cancel: Arc::new(Notify::new()),
            cancelled: AtomicBool::new(false),
            session_id: Mutex::new(None),
        }
    }

    /// Mark the start of a new turn: clears any leftover cancel signal so a
    /// Stop from a previous turn can never suppress this one.
    pub fn begin_turn(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    /// Whether the current turn has been cancelled by the user.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Cancel the current assistant turn (if any). Safe to call when idle.
    /// Sets the sticky flag *and* wakes the streaming select.
    pub fn request_cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.cancel.notify_waiters();
    }

    /// Forget the persisted-session pointer so the next turn starts a brand
    /// new history row. Called when the conversation is cleared.
    pub fn reset_session(&self) {
        if let Ok(mut id) = self.session_id.lock() {
            *id = None;
        }
    }

    /// Drop the session pointer only if it matches `id` — used when a
    /// conversation is deleted from the History view while still active, so
    /// the next turn re-saves instead of updating a now-deleted row.
    pub fn forget_session_if(&self, id: i64) {
        if let Ok(mut current) = self.session_id.lock() {
            if *current == Some(id) {
                *current = None;
            }
        }
    }
}

#[derive(Clone, Serialize)]
struct AssistantStatePayload {
    state: String,
}

/// Structured error payload: `code` is a stable identifier the panel maps to a
/// short localized message (with a pill-sized variant); `detail` carries the
/// raw provider/OS text for the expanded view and unknown-code fallback.
#[derive(Clone, Serialize)]
struct AssistantErrorPayload {
    code: String,
    detail: String,
}

/// Emit a user-facing assistant error. Codes the panel understands:
/// `no_provider`, `no_model`, `engine_start`, `provider`, `vision_unsupported`,
/// `screenshot_too_large`, `screen_capture`, `transcription`, `tts`,
/// `mic_denied`, `mic_unavailable`.
pub fn emit_error(app: &AppHandle, code: &str, detail: String) {
    let _ = app.emit(
        "assistant-error",
        AssistantErrorPayload {
            code: code.to_string(),
            detail,
        },
    );
}

/// Emit a non-blocking notice (the turn keeps going). The panel shows it as a
/// quiet transient line rather than an error bubble. Codes:
/// `web_search_failed`.
pub fn emit_notice(app: &AppHandle, code: &str) {
    let _ = app.emit("assistant-notice", code.to_string());
}

/// Emit a pipeline state update to the panel:
/// "listening" | "transcribing" | "searching" | "thinking" | "speaking" | "idle"
pub fn emit_state(app: &AppHandle, state: &str) {
    let _ = app.emit(
        "assistant-state",
        AssistantStatePayload {
            state: state.to_string(),
        },
    );
}

/// Emit the full conversation snapshot. The panel renders exclusively from
/// these snapshots (plus a transient streaming buffer), which makes the UI
/// idempotent: duplicate listeners or replayed events can never duplicate
/// messages.
pub fn emit_conversation(app: &AppHandle) {
    let snapshot = {
        let conversation = app.state::<AssistantConversation>();
        let history = conversation.messages.lock().unwrap();
        history.clone()
    };
    let _ = app.emit("assistant-conversation", snapshot);
}

/// Persist the current conversation to the history database so it shows up in
/// the History view (and survives the panel window being recreated). Upserts
/// against the session's row: creates one on the first turn, updates it on
/// every turn after. Best-effort — a storage failure must never break a chat.
///
/// Emits a lightweight `assistant-history-updated` event afterward so the
/// (separate) main window's History view can refresh.
pub fn persist_assistant_session(app: &AppHandle) {
    let Some(hm) = app.try_state::<Arc<crate::managers::history::HistoryManager>>() else {
        return;
    };

    let conversation = app.state::<AssistantConversation>();
    let messages = match conversation.messages.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };
    if messages.is_empty() {
        return;
    }

    let mut session_id = match conversation.session_id.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };

    let saved = match *session_id {
        Some(id) => match hm.update_assistant_session(id, &messages) {
            Ok(Some(entry)) => Some(entry),
            // Row vanished (deleted in the UI) — start a fresh one.
            Ok(None) => hm.create_assistant_session(&messages).ok(),
            Err(e) => {
                error!("Failed to update assistant session {}: {}", id, e);
                None
            }
        },
        None => match hm.create_assistant_session(&messages) {
            Ok(entry) => Some(entry),
            Err(e) => {
                error!("Failed to create assistant session: {}", e);
                None
            }
        },
    };

    if let Some(entry) = saved {
        *session_id = Some(entry.id);
    }
    drop(session_id);

    let _ = app.emit("assistant-history-updated", ());
}

// ---------------------------------------------------------------------------
// Panel window management
// ---------------------------------------------------------------------------

/// Force the panel topmost via Win32; Tauri's always_on_top flag can be
/// overridden by other topmost windows (same trick as the recording overlay).
#[cfg(target_os = "windows")]
fn force_panel_topmost(window: &tauri::webview::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    let window_clone = window.clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(hwnd) = window_clone.hwnd() {
            unsafe {
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

fn saved_position(app: &AppHandle) -> Option<(f64, f64)> {
    let store = app
        .store(crate::portable::store_path(
            crate::settings::SETTINGS_STORE_PATH,
        ))
        .ok()?;
    let value = store.get(PANEL_POSITION_KEY)?;
    let x = value.get("x")?.as_f64()?;
    let y = value.get("y")?.as_f64()?;
    Some((x, y))
}

fn save_position(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        if let (Ok(pos), Ok(monitor)) = (window.outer_position(), window.current_monitor()) {
            let scale = monitor.map(|m| m.scale_factor()).unwrap_or(1.0);
            if let Ok(store) = app.store(crate::portable::store_path(
                crate::settings::SETTINGS_STORE_PATH,
            )) {
                store.set(
                    PANEL_POSITION_KEY,
                    serde_json::json!({
                        "x": pos.x as f64 / scale,
                        "y": pos.y as f64 / scale,
                    }),
                );
            }
        }
    }
}

/// Default position: bottom-right of the primary monitor (logical coords).
fn default_position(app: &AppHandle) -> (f64, f64) {
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let scale = monitor.scale_factor();
        let mw = monitor.size().width as f64 / scale;
        let mh = monitor.size().height as f64 / scale;
        let mx = monitor.position().x as f64 / scale;
        let my = monitor.position().y as f64 / scale;
        (
            mx + mw - PANEL_WIDTH - PANEL_MARGIN,
            my + mh - PANEL_HEIGHT - PANEL_MARGIN - 40.0, // keep clear of taskbar
        )
    } else {
        (100.0, 100.0)
    }
}

/// Create the assistant panel window, hidden by default. Called once at setup.
pub fn create_assistant_panel(app: &AppHandle) {
    let (x, y) = saved_position(app).unwrap_or_else(|| default_position(app));
    // Build at whichever size matches the current mode (pill by default) so the
    // first show doesn't briefly flash the large panel before collapsing.
    let (init_w, init_h) = if PILL_MODE.load(Ordering::SeqCst) {
        (PILL_WIDTH, PILL_HEIGHT)
    } else {
        expanded_size()
    };

    let mut builder = WebviewWindowBuilder::new(
        app,
        PANEL_LABEL,
        tauri::WebviewUrl::App("src/assistant/index.html".into()),
    )
    .title("Assistant")
    .inner_size(init_w, init_h)
    .min_inner_size(PILL_WIDTH, PILL_HEIGHT)
    .position(x, y)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false);

    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    match builder.build() {
        Ok(window) => {
            // Persist position while the user drags the panel around.
            let app_handle = app.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Moved(_)) {
                    save_position(&app_handle);
                }
            });
            debug!("Assistant panel window created (hidden)");
        }
        Err(e) => error!("Failed to create assistant panel window: {}", e),
    }
}

pub fn show_assistant_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        // Keep the webview's layout in sync with the actual window before
        // showing. A reloaded webview resets its React state to "expanded", so
        // without this it can render the full panel inside the pill-sized
        // window (showing only the header bar). Re-assert the pill size when
        // collapsed, and always tell the webview which mode to render.
        let collapsed = PILL_MODE.load(Ordering::SeqCst);
        if collapsed {
            let _ = window.set_size(tauri::LogicalSize::new(PILL_WIDTH, PILL_HEIGHT));
        }
        let _ = app.emit("assistant-collapsed", collapsed);

        let _ = window.show();
        #[cfg(target_os = "windows")]
        force_panel_topmost(&window);
        let _ = app.emit("assistant-panel-shown", ());
    }
}

/// Whether the assistant panel is currently collapsed to the pill. Lets the
/// webview initialise its layout correctly after a (re)load.
pub fn is_panel_collapsed() -> bool {
    PILL_MODE.load(Ordering::SeqCst)
}

pub fn hide_assistant_panel(app: &AppHandle) {
    save_position(app);
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let _ = window.hide();
    }
}

pub fn toggle_assistant_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        match window.is_visible() {
            Ok(true) => hide_assistant_panel(app),
            _ => show_assistant_panel(app),
        }
    }
}

/// Collapse the panel to a small pill, or restore it to the expanded size.
/// Collapsing remembers the current expanded size for the session so a manual
/// resize survives the round-trip. The window's BOTTOM-LEFT corner stays
/// anchored through both transitions (the pill usually sits near the bottom of
/// the screen, so expanding grows upward instead of pushing below the screen
/// edge), and the result is clamped onto the current monitor.
pub fn set_panel_collapsed(app: &AppHandle, collapsed: bool) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let scale = window
            .current_monitor()
            .ok()
            .flatten()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        let old_pos = window.outer_position().ok();
        let old_size = window.inner_size().ok();

        if collapsed && !PILL_MODE.load(Ordering::SeqCst) {
            if let Some(size) = old_size {
                let w = (size.width as f64 / scale).round() as u32;
                let h = (size.height as f64 / scale).round() as u32;
                // Ignore degenerate sizes (e.g. already pill-sized) so a stray
                // double-collapse can't shrink the remembered panel.
                if w > PILL_WIDTH as u32 && h > PILL_HEIGHT as u32 {
                    EXPANDED_W.store(w, Ordering::SeqCst);
                    EXPANDED_H.store(h, Ordering::SeqCst);
                }
            }
        }
        PILL_MODE.store(collapsed, Ordering::SeqCst);

        let (new_w, new_h) = if collapsed {
            (PILL_WIDTH, PILL_HEIGHT)
        } else {
            expanded_size()
        };
        let _ = window.set_size(tauri::LogicalSize::new(new_w, new_h));

        // Bottom-left anchor + on-screen clamp.
        if let (Some(pos), Some(size)) = (old_pos, old_size) {
            let old_x = pos.x as f64 / scale;
            let old_y = pos.y as f64 / scale;
            let old_h = size.height as f64 / scale;
            let mut new_x = old_x;
            let mut new_y = old_y + old_h - new_h;
            if let Ok(Some(monitor)) = window.current_monitor() {
                let mx = monitor.position().x as f64 / scale;
                let my = monitor.position().y as f64 / scale;
                let mw = monitor.size().width as f64 / scale;
                let mh = monitor.size().height as f64 / scale;
                new_x = new_x.clamp(mx + 8.0, (mx + mw - new_w - 8.0).max(mx + 8.0));
                new_y = new_y.clamp(my + 8.0, (my + mh - new_h - 8.0).max(my + 8.0));
            }
            let _ = window.set_position(tauri::LogicalPosition::new(new_x, new_y));
        }

        let _ = app.emit("assistant-collapsed", collapsed);
    } else {
        PILL_MODE.store(collapsed, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Region snip overlay
// ---------------------------------------------------------------------------

pub const SNIP_LABEL: &str = "snip_overlay";

/// Open the region-snip overlay for a frame that was just captured: store it
/// in PENDING_SNIP, then cover the cursor's monitor with a transparent
/// selection window. Called from an async command (worker thread) — building
/// a webview inline on the main thread inside a command deadlocks WebView2 on
/// Windows, so this must NOT be dispatched to the main thread.
pub fn open_snip_overlay(app: &AppHandle, frame: image::DynamicImage) -> Result<(), String> {
    if app.get_webview_window(SNIP_LABEL).is_some() {
        return Ok(()); // already snipping
    }

    if let Ok(mut pending) = PENDING_SNIP.lock() {
        *pending = Some(frame);
    }

    // Cover the monitor the cursor is on (physical coords); primary otherwise.
    let cursor = crate::screenshot::cursor_position();
    let monitor = match cursor {
        Some((x, y)) => app
            .monitor_from_point(x as f64, y as f64)
            .ok()
            .flatten()
            .or_else(|| app.primary_monitor().ok().flatten()),
        None => app.primary_monitor().ok().flatten(),
    }
    .ok_or_else(|| "No monitor available for region snip".to_string())?;

    let window = WebviewWindowBuilder::new(
        app,
        SNIP_LABEL,
        tauri::WebviewUrl::App("src/assistant/snip.html".into()),
    )
    .title("Snip")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .focused(true)
    .visible(true)
    .build()
    .map_err(|e| format!("Couldn't open the snip overlay: {}", e))?;

    // Position/size in PHYSICAL pixels so the overlay covers the monitor
    // exactly regardless of DPI scale.
    let _ = window.set_position(tauri::PhysicalPosition::new(
        monitor.position().x,
        monitor.position().y,
    ));
    let _ = window.set_size(tauri::PhysicalSize::new(
        monitor.size().width,
        monitor.size().height,
    ));
    let _ = window.set_focus();
    #[cfg(target_os = "windows")]
    force_panel_topmost(&window);
    Ok(())
}

/// Close the snip overlay and, when a rectangle was chosen, crop it from the
/// frozen frame and hand it to the panel as a pending image attachment via the
/// `assistant-region-captured` event. `rect` is in the overlay's logical px.
pub fn finish_region_snip(app: &AppHandle, scale: f64, rect: Option<(f64, f64, f64, f64)>) {
    if let Some(window) = app.get_webview_window(SNIP_LABEL) {
        let _ = window.close();
    }
    let frame = PENDING_SNIP.lock().ok().and_then(|mut p| p.take());
    let Some(rect) = rect else {
        return; // cancelled
    };
    let Some(frame) = frame else {
        emit_error(app, "screen_capture", "No captured frame for snip".into());
        return;
    };

    let (x, y, w, h) = rect;
    let to_px = |v: f64| -> u32 { (v * scale).round().max(0.0) as u32 };
    if w * scale < 4.0 || h * scale < 4.0 {
        return; // a stray click, not a selection
    }

    let settings = get_settings(app);
    let profile = settings
        .active_assistant_provider()
        .map(|p| crate::screenshot::CaptureProfile::for_base_url(&p.base_url))
        .unwrap_or(crate::screenshot::CaptureProfile::Generous);

    match crate::screenshot::encode_region_data_url(
        &frame,
        profile,
        to_px(x),
        to_px(y),
        to_px(w),
        to_px(h),
    ) {
        Ok(data_url) => {
            let _ = app.emit("assistant-region-captured", data_url);
        }
        Err(e) => emit_error(app, "screen_capture", e),
    }
}

// ---------------------------------------------------------------------------
// Assistant pipeline
// ---------------------------------------------------------------------------

/// Run a voice-initiated assistant turn on a finished transcription: decide
/// whether to attach the screen (sticky arm or "what's on my screen" phrasing),
/// capture it, pick up any attachments staged in the panel, and run the turn.
/// Shared by the assistant hotkey/pill path and the STT overlay's
/// Ask-Assistant redirect.
pub async fn run_voice_turn(app: AppHandle, transcription: String) {
    let settings = get_settings(&app);
    let wants_screen = screen_armed() || wants_screen_context(&transcription);
    let screenshot = if wants_screen && settings.assistant_screenshot_enabled {
        // Tiny body only for Azure; loopback (built-in/local engine) gets a
        // balanced image, cloud gets the sharp one.
        let profile = settings
            .active_assistant_provider()
            .map(|p| crate::screenshot::CaptureProfile::for_base_url(&p.base_url))
            .unwrap_or(crate::screenshot::CaptureProfile::Generous);
        let captured = tauri::async_runtime::spawn_blocking(move || {
            crate::screenshot::capture_screen_data_url(profile)
        })
        .await;
        match captured {
            Ok(Ok(data_url)) => Some(data_url),
            Ok(Err(e)) => {
                // Don't silently send a text-only request when the user asked
                // about their screen.
                error!("Screen capture failed: {}", e);
                emit_error(&app, "screen_capture", e);
                emit_state(&app, "idle");
                return;
            }
            Err(e) => {
                error!("Screen capture task failed: {}", e);
                emit_error(&app, "screen_capture", e.to_string());
                emit_state(&app, "idle");
                return;
            }
        }
    } else {
        None
    };
    let (images, files) = take_pending_attachments(&app);
    run_assistant_turn(app, transcription, screenshot, images, files).await;
}

/// Resets the busy flag when a turn finishes, on every exit path.
struct BusyReset(AppHandle);

impl Drop for BusyReset {
    fn drop(&mut self) {
        self.0
            .state::<AssistantConversation>()
            .busy
            .store(false, Ordering::SeqCst);
    }
}

/// Detect provider errors that mean "the selected model can't accept images".
/// Covers the bundled llama.cpp engine / LM Studio / Ollama ("image input is
/// not supported", "mmproj") as well as OpenAI / Azure / other OpenAI-compatible
/// gateways that reject image content. Only consulted on screenshot turns, so
/// matching image/vision keywords broadly is safe.
fn is_vision_unsupported_error(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("image input is not supported")
        || e.contains("mmproj")
        || e.contains("does not support image")
        || e.contains("not support image")
        || e.contains("support image input")
        || e.contains("image_url")
        || e.contains("multimodal")
        || (e.contains("vision") && (e.contains("not") || e.contains("unsupported")))
}

/// A clear, actionable message for when a screenshot was sent to a model that
/// can't see images. The built-in provider gets a tailored hint because its
/// vision models work as soon as the multimodal projector is installed, so the
/// problem there is a missing component rather than an incapable model.
fn vision_unsupported_message(provider_id: &str, model: &str) -> String {
    if provider_id == "builtin" && crate::managers::model::mmproj_for(model).is_some() {
        format!(
            "The built-in model '{}' supports vision, but its image component isn't installed yet. Re-download it from the model manager to enable screen vision, or ask again without a screenshot.",
            model
        )
    } else {
        format!(
            "The selected model '{}' doesn't support vision — it can't read screenshots. Pick a vision-capable model in Settings → Assistant (e.g. gpt-4o-mini, gpt-4.1-mini, gemini-flash, claude, or a multimodal local model), or ask again without a screenshot.",
            model
        )
    }
}

/// Fixed clause appended to the system prompt so the model knows a live clock
/// is being supplied (and to use it for "today/now/latest" questions). It's
/// byte-identical every turn — the changing timestamp goes in the user message
/// instead — so it doesn't disturb provider-side prompt caching.
const TIME_AWARENESS_NOTE: &str = "The user's current local date and time is provided at the top of their message. Treat it as the present moment for any time-related question (today, now, this week, how long ago, latest, etc.) — never guess the date from your training data.";

/// The current local date/time line prepended to each request's user message.
/// LLMs have no clock, so this is the production-standard way to keep "what's
/// the date / what time is it / how many days until X" answers correct: inject
/// the real time fresh on every turn, with an explicit UTC offset so the model
/// can reason across time zones. (Kept in the user message rather than the
/// cached system prefix so it never invalidates prompt caching.)
fn current_datetime_line() -> String {
    let now = chrono::Local::now();
    // e.g. "Current date and time: Saturday, June 20, 2026, 2:34 PM (UTC+05:30)."
    now.format("Current date and time: %A, %B %-d, %Y, %-I:%M %p (UTC%:z).")
        .to_string()
}

/// Build a short transcript of the most recent conversation turns to give the
/// search planner context, so follow-up questions ("what about its price?")
/// can be resolved into self-contained queries. Excludes the just-recorded
/// current message and the screenshot marker, and bounds each line so the
/// planner prompt stays small.
fn recent_context_for_planner(app: &AppHandle) -> String {
    let conversation = app.state::<AssistantConversation>();
    let history = conversation.messages.lock().unwrap();
    let mut lines: Vec<String> = Vec::new();
    // Skip the latest user message (just pushed above); take the few before it.
    for message in history.iter().rev().skip(1).take(4) {
        let role = if message.role == "assistant" {
            "Assistant"
        } else {
            "User"
        };
        // Collapse whitespace and strip attachment markers, then bound length.
        let text: String = message
            .content
            .replace(SCREENSHOT_MARKER, "")
            .replace(IMAGE_MARKER, "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }
        let text: String = text.chars().take(300).collect();
        lines.push(format!("{}: {}", role, text));
    }
    lines.reverse();
    lines.join("\n")
}

/// Run one assistant turn: record the user message, stream the LLM answer to
/// the panel via events, and append the reply to the conversation history.
///
/// `screenshot` is an optional `data:image/...;base64,` URL captured from the
/// user's screen, `images` are user-attached pictures (same format), and
/// `files` are text-like attachments whose content is inlined as context.
/// Visuals are sent to the model only for this turn (the history keeps text
/// markers instead, so images never burn tokens twice).
///
/// Events emitted:
/// - `assistant-conversation` (Vec<ChatMessage>): full snapshot after change
/// - `assistant-token` (String): each streamed content delta
/// - `assistant-tts` (String): short spoken summary (only when TTS enabled)
/// - `assistant-error` ({code, detail}): structured error description
pub async fn run_assistant_turn(
    app: AppHandle,
    user_text: String,
    screenshot: Option<String>,
    images: Vec<String>,
    files: Vec<FileAttachment>,
) {
    let user_text = user_text.trim().to_string();
    if user_text.is_empty() {
        emit_state(&app, "idle");
        return;
    }
    // Whether any picture rides along this turn (screen capture or attachment).
    let has_visual = screenshot.is_some() || !images.is_empty();

    // Re-entrancy guard: a double-fired hotkey or repeated Enter must never
    // start a second concurrent turn (this caused duplicated messages).
    {
        let conversation = app.state::<AssistantConversation>();
        if conversation.busy.swap(true, Ordering::SeqCst) {
            debug!("Assistant turn already in progress; ignoring duplicate trigger");
            return;
        }
    }
    let _busy = BusyReset(app.clone());

    // Fresh turn: clear any leftover cancel signal from a previous Stop.
    app.state::<AssistantConversation>().begin_turn();

    let settings = get_settings(&app);

    let Some(provider) = settings.active_assistant_provider().cloned() else {
        emit_error(
            &app,
            "no_provider",
            "No assistant provider configured. Pick one in Settings → Assistant.".to_string(),
        );
        emit_state(&app, "idle");
        return;
    };

    let model = settings
        .assistant_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        emit_error(
            &app,
            "no_model",
            format!(
                "No model configured for provider '{}'. Set one in Settings → Assistant.",
                provider.label
            ),
        );
        emit_state(&app, "idle");
        return;
    }

    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    // Record the user message (text markers instead of raw image/file data)
    // and show it in the panel immediately — before any web search runs, so the
    // bubble appears right away while results are being fetched.
    {
        let conversation = app.state::<AssistantConversation>();
        let mut history = conversation.messages.lock().unwrap();
        let mut stored = user_text.clone();
        for file in &files {
            stored.push_str(&format!("\n{} {}]", FILE_MARKER_PREFIX, file.name));
        }
        for _ in &images {
            stored.push_str(&format!("\n{}", IMAGE_MARKER));
        }
        if screenshot.is_some() {
            stored.push_str(&format!("\n{}", SCREENSHOT_MARKER));
        }
        history.push(ChatMessage {
            role: "user".to_string(),
            content: stored,
        });
    }
    emit_conversation(&app);
    // Save right after the user message so the question is preserved even if
    // the model (or the search) errors out before replying.
    persist_assistant_session(&app);

    // Optional web search. Runs only when enabled and this isn't a visual
    // turn (those are about the screen/images, not the web). A capable model
    // plans the search — deciding whether one is actually needed and rewriting
    // the (often messy, transcribed) request into clean queries — and then we
    // fetch real page content to ground the answer. Any failure or timeout
    // degrades gracefully: we answer without web context rather than breaking
    // the turn. The cheap `should_search` pre-gate skips obvious non-search
    // turns so we don't spend a planner round-trip on chit-chat, code, or math.
    let web_context: Option<String> = if settings.assistant_web_search_enabled
        && !has_visual
        && web_search::should_search(&user_text)
    {
        // Race every search stage against a Stop press: a slow search must never
        // trap the user in the "searching" state with no way out.
        let cancel = app.state::<AssistantConversation>().cancel.clone();

        // Stage 1 — decide whether to search and craft queries. Cloud/custom
        // models always use the LLM planner (the smart decider). The built-in
        // local model uses the fast keyword heuristic by default, or the same
        // planner when the user enabled "smart" local search — in which case we
        // load its engine first so the planning call isn't cold. `None` means
        // skip (cancelled).
        let use_planner = if provider.id == "builtin" {
            settings.assistant_local_search_smart && {
                let manager = app.state::<Arc<crate::managers::local_llm::LocalLlmManager>>();
                match manager.ensure_running(&model).await {
                    Ok(_) => true,
                    Err(e) => {
                        warn!(
                            "Built-in engine couldn't start for planning ({}); using heuristic",
                            e
                        );
                        false
                    }
                }
            }
        } else {
            true
        };

        let mut plan_opt: Option<web_search::SearchPlan> = if use_planner {
            // The planner is a quick LLM call; show "thinking" while it decides
            // (we only switch to "searching" once a search actually runs).
            emit_state(&app, "thinking");
            let recent = recent_context_for_planner(&app);
            let planned = tokio::select! {
                r = web_search::plan_search(
                    &provider,
                    api_key.clone(),
                    &model,
                    provider.supports_structured_output,
                    &recent,
                    &user_text,
                ) => Some(r),
                _ = cancel.notified() => None,
            };
            match planned {
                Some(Ok(plan)) => Some(plan),
                Some(Err(e)) => {
                    warn!(
                        "Search planner failed ({}); falling back to a signal heuristic",
                        e
                    );
                    Some(web_search::SearchPlan::heuristic(&user_text))
                }
                None => None, // cancelled during planning
            }
        } else {
            Some(web_search::SearchPlan::heuristic(&user_text))
        };

        // Force a search even when the planner judged one unnecessary, in two
        // cases: (1) the user explicitly asked ("search the web for …"), or
        // (2) the question clearly needs current/external facts — a role holder,
        // price, score, weather, a recent year, etc. Capable models are often
        // over-confident and answer "who is the current …" questions straight
        // from stale training data instead of searching; this deterministic
        // guard is the fail-safe for that (the `should_search` pre-gate has
        // already screened out chit-chat, code and math before we get here).
        if let Some(plan) = plan_opt.as_mut() {
            if !plan.needs_search
                && (web_search::is_explicit_search_request(&user_text)
                    || web_search::looks_time_sensitive(&user_text))
            {
                plan.needs_search = true;
                if plan.queries.is_empty() {
                    plan.queries
                        .push(user_text.trim().chars().take(480).collect());
                }
            }
        }

        // Stage 2 — retrieve, only when the decision calls for it. The
        // "searching" state is emitted here (not earlier) so the panel never
        // shows "Searching the web…" on a turn that decides NOT to search.
        match plan_opt {
            Some(plan) if plan.needs_search && !plan.queries.is_empty() => {
                emit_state(&app, "searching");
                let search_result = tokio::select! {
                    r = web_search::search_with_plan(&settings, &plan) => Some(r),
                    _ = cancel.notified() => None,
                };
                match search_result {
                    Some(results) if !results.is_empty() => {
                        debug!(
                            "Web search returned {} results across {} queries",
                            results.len(),
                            plan.queries.len()
                        );
                        let budget =
                            web_search::context_budget_for(settings.assistant_search_depth);
                        Some(web_search::format_results_for_prompt(&results, budget))
                    }
                    Some(_) => {
                        debug!("Web search returned no results; answering without web context");
                        // §4: never silent — tell the user the answer proceeds
                        // without web results (covers failures and no-hits).
                        emit_notice(&app, "web_search_failed");
                        None
                    }
                    None => None, // cancelled during search
                }
            }
            _ => None,
        }
    } else {
        None
    };

    // If the user pressed Stop during the search (or anytime up to here), abort
    // before spending a model call. The question stays in the panel/history.
    if app.state::<AssistantConversation>().is_cancelled() {
        debug!("Assistant turn cancelled before generation");
        crate::tts::stop_remote();
        emit_state(&app, "idle");
        return;
    }

    // Build the request: stable system prompt → history → new user msg.
    // (Cache-friendly: the prefix only ever grows by appending.) History is
    // capped (newest first wins) so request bodies stay small — critical for
    // Azure, whose parser rejects oversized payloads. Visual turns get a much
    // tighter cap: the image(s) already dominate the body budget.
    let (max_history_messages, max_history_chars) = if has_visual {
        (
            (settings.assistant_max_history_messages as usize).min(4),
            6_000usize,
        )
    } else {
        (
            settings.assistant_max_history_messages as usize,
            24_000usize,
        )
    };
    let mut messages: Vec<Value> = Vec::new();
    let system_content = {
        let mut content = settings.assistant_system_prompt.clone();
        // Always note that a live date/time accompanies the user's message, so
        // time-relative answers are correct. Fixed text → cache-safe.
        if !content.trim().is_empty() {
            content.push_str("\n\n");
        }
        content.push_str(TIME_AWARENESS_NOTE);
        // When web search is enabled, tell the model it HAS that capability on
        // every turn — even ones where the app didn't auto-search — so it never
        // denies having internet access and can offer to look things up. Stable
        // text → cache-safe.
        if settings.assistant_web_search_enabled {
            content.push_str("\n\n");
            content.push_str(web_search::WEB_SEARCH_CAPABILITY_NOTE);
        }
        // Append the user's response-length preference (if any) so a single
        // system prompt covers both display and spoken output.
        if let Some(directive) = settings.assistant_response_length.directive() {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(directive);
        }
        // On web-search turns, tell the model to ground its answer in the
        // results that are prepended to the user's message below — and, crucially,
        // to treat them as its OWN findings (never "the results you sent"). The
        // directive adapts to TTS: speech-friendly prose when the reply is spoken,
        // richer Markdown (tables/bullets) when it's only read on screen.
        if web_context.is_some() {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&web_search::web_search_system_directive(
                settings.assistant_tts_enabled,
            ));
        }
        content
    };
    messages.push(json!({
        "role": "system",
        "content": system_content,
    }));
    {
        let conversation = app.state::<AssistantConversation>();
        let history = conversation.messages.lock().unwrap();
        let mut kept: Vec<&ChatMessage> = Vec::new();
        let mut chars = 0usize;
        // Skip the just-recorded user message (pushed above); it's appended
        // explicitly below with the proper request content.
        for message in history.iter().rev().skip(1).take(max_history_messages) {
            chars += message.content.len();
            if chars > max_history_chars && !kept.is_empty() {
                break;
            }
            kept.push(message);
        }
        for message in kept.into_iter().rev() {
            messages.push(json!({"role": message.role, "content": message.content}));
        }
    }
    // Per-turn context prepended to the user's message for the request only
    // (never stored in history): the live date/time always, web results when
    // present, and the content of any attached files. Keeping this in the user
    // message rather than the cached system prefix means the timestamp stays
    // fresh every turn without invalidating provider-side prompt caching.
    let mut preamble = current_datetime_line();
    if let Some(ctx) = &web_context {
        preamble.push_str("\n\n");
        preamble.push_str(ctx);
    }
    // Inline attached files as clearly-delimited context blocks, individually
    // and collectively bounded so a huge file can't blow the request budget.
    const FILE_CHAR_CAP: usize = 20_000;
    const FILES_TOTAL_CAP: usize = 40_000;
    let mut files_budget = FILES_TOTAL_CAP;
    for file in &files {
        let take = file.content.len().min(FILE_CHAR_CAP).min(files_budget);
        if take == 0 {
            break;
        }
        let content: String = file.content.chars().take(take).collect();
        files_budget = files_budget.saturating_sub(content.len());
        let truncated = content.len() < file.content.len();
        preamble.push_str(&format!(
            "\n\nAttached file: {}{}\n---\n{}\n---",
            file.name,
            if truncated { " (truncated)" } else { "" },
            content
        ));
    }
    let user_content = format!("{}\n\n{}", preamble, user_text);

    // Visuals: the screen capture (if any) first, then attached images, capped
    // so a pile of attachments can't produce an oversized request.
    const MAX_VISUALS: usize = 4;
    let mut visuals: Vec<&String> = Vec::new();
    if let Some(data_url) = &screenshot {
        visuals.push(data_url);
    }
    for image in images.iter() {
        if visuals.len() >= MAX_VISUALS {
            break;
        }
        visuals.push(image);
    }

    if visuals.is_empty() {
        messages.push(json!({"role": "user", "content": user_content}));
    } else {
        let mut parts: Vec<Value> = vec![json!({"type": "text", "text": user_content})];
        for url in &visuals {
            parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
        }
        messages.push(json!({"role": "user", "content": parts}));
    }

    emit_state(&app, "thinking");

    // The built-in provider is backed by the bundled llama.cpp engine. Ensure
    // it is running and serving the selected model before streaming. The user
    // message is already shown and the panel shows "thinking" during load.
    // Built-in provider: ensure the engine is running, then hold an activity
    // guard across the streamed turn so the idle watcher won't unload it
    // mid-generation.
    let _llm_activity_guard = if provider.id == "builtin" {
        let manager = app.state::<Arc<crate::managers::local_llm::LocalLlmManager>>();
        if let Err(e) = manager.ensure_running(&model).await {
            emit_error(&app, "engine_start", e.to_string());
            emit_state(&app, "idle");
            return;
        }
        Some(manager.begin_request())
    } else {
        None
    };

    debug!(
        "Assistant turn: provider '{}', model '{}', {} messages, visuals: {}, files: {}",
        provider.id,
        model,
        messages.len(),
        visuals.len(),
        files.len()
    );

    let cancel = {
        let conversation = app.state::<AssistantConversation>();
        conversation.cancel.clone()
    };

    // Accumulate streamed tokens so a cancelled turn can keep the partial reply.
    let partial = Arc::new(Mutex::new(String::new()));
    let partial_cb = partial.clone();
    let app_for_tokens = app.clone();
    let stream_fut =
        llm_client::send_chat_stream(&provider, api_key.clone(), &model, messages, move |token| {
            if let Ok(mut buf) = partial_cb.lock() {
                buf.push_str(token);
            }
            let _ = app_for_tokens.emit("assistant-token", token.to_string());
        });
    tokio::pin!(stream_fut);

    // Race the stream against a Stop request. notify_waiters wakes this select.
    let outcome = tokio::select! {
        result = &mut stream_fut => Some(result),
        _ = cancel.notified() => None,
    };

    // Whether a spoken reply is starting. When it is, the turn ends in a
    // "speaking" UI state rather than idle, so the panel/pill doesn't flash its
    // idle "Assistant" affordance in the gap before audio begins.
    let mut speaking = false;
    match outcome {
        None => {
            // User pressed Stop. Silence any spoken summary already playing and
            // keep whatever text was generated so far (like a cancelled chat).
            crate::tts::stop_remote();
            let partial_text = partial
                .lock()
                .map(|b| b.trim().to_string())
                .unwrap_or_default();
            if !partial_text.is_empty() {
                let conversation = app.state::<AssistantConversation>();
                let mut history = conversation.messages.lock().unwrap();
                history.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: partial_text,
                });
            }
            emit_conversation(&app);
            persist_assistant_session(&app);
            debug!("Assistant turn cancelled by user");
        }
        Some(Ok(full_text)) => {
            {
                let conversation = app.state::<AssistantConversation>();
                let mut history = conversation.messages.lock().unwrap();
                history.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: full_text.clone(),
                });
            }
            emit_conversation(&app);
            persist_assistant_session(&app);

            // A Stop that lands in the tiny gap between the stream finishing and
            // here must still suppress the spoken reply.
            if app.state::<AssistantConversation>().is_cancelled() {
                crate::tts::stop_remote();
            } else if settings.assistant_tts_enabled {
                spawn_tts_speak(&app, &settings, full_text);
                speaking = true;
            }
        }
        Some(Err(e)) => {
            error!("Assistant request failed: {}", e);
            if e.contains("Unterminated string") && has_visual {
                emit_error(&app, "screenshot_too_large", e);
            } else if has_visual && is_vision_unsupported_error(&e) {
                emit_error(
                    &app,
                    "vision_unsupported",
                    vision_unsupported_message(&provider.id, &model),
                );
            } else {
                emit_error(&app, "provider", e);
            }
        }
    }

    // When a spoken reply is starting, hand the UI a dedicated "speaking" state
    // instead of dropping straight to idle — otherwise the panel/pill flashes
    // its idle "Assistant" affordance in the gap before audio begins. The panel
    // flips itself back to idle once playback ends (it owns the local engine).
    emit_state(&app, if speaking { "speaking" } else { "idle" });
}

/// Speak the assistant's reply aloud via the configured TTS engine.
/// Fire-and-forget. Response length is controlled by the user's
/// `assistant_response_length` setting (injected into the system prompt), so no
/// separate summary is generated — we speak the reply directly.
fn spawn_tts_speak(app: &AppHandle, settings: &crate::settings::AppSettings, full_text: String) {
    // The full reply is spoken verbatim, so strip Markdown, code blocks, links
    // and emojis first — otherwise the engine reads symbols and code aloud. The
    // on-screen reply is unaffected; this only cleans the spoken copy.
    let text = crate::tts::sanitize_for_speech(&full_text);
    if text.trim().is_empty() {
        return;
    }
    // Capture the playback epoch *now*, at turn completion — NOT inside the
    // spawned task. A Stop pressed in the window between completion and the task
    // actually running bumps the epoch; capturing it here lets us detect that
    // and skip the stale reply ("the new one waiting"). Capturing inside the
    // task would read the already-bumped value and play anyway.
    let epoch = crate::tts::current_epoch();
    let app = app.clone();
    let settings = settings.clone();

    tauri::async_runtime::spawn(async move {
        // Superseded by a Stop (or TTS disable) before we got here? Don't speak.
        // This covers Kokoro too, which otherwise has no epoch gate of its own.
        if crate::tts::current_epoch() != epoch {
            debug!("TTS superseded before playback; skipping");
            return;
        }
        if settings.assistant_tts_engine == "kokoro" {
            // Local engine lives in the panel webview (kokoro-js); the webview
            // hook ignores it when TTS is disabled.
            let _ = app.emit("assistant-tts", text);
        } else {
            crate::tts::speak_remote_epoch(&app, &settings, text, epoch).await;
        }
    });
}

// ---------------------------------------------------------------------------
// Conversation quality-of-life: regenerate / continue / summarize
// ---------------------------------------------------------------------------

/// Strip attachment markers from a stored user message, leaving the text the
/// user actually typed/said.
fn strip_markers(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let t = line.trim();
            t != SCREENSHOT_MARKER
                && t != IMAGE_MARKER
                && !(t.starts_with(FILE_MARKER_PREFIX) && t.ends_with(']'))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Regenerate the latest answer: drop the last assistant message and the user
/// message that produced it, then re-run the turn with the same text. Visual
/// attachments aren't stored (only markers), so a regenerated turn is
/// text-only — the message text still tells the model what was asked.
pub async fn regenerate_last(app: AppHandle) {
    let text = {
        let conversation = app.state::<AssistantConversation>();
        let mut history = conversation.messages.lock().unwrap();
        if matches!(history.last(), Some(m) if m.role == "assistant") {
            history.pop();
        }
        match history.last() {
            Some(m) if m.role == "user" => {
                let t = strip_markers(&m.content);
                history.pop();
                Some(t)
            }
            _ => None,
        }
    };
    emit_conversation(&app);
    match text {
        Some(t) if !t.is_empty() => {
            run_assistant_turn(app, t, None, Vec::new(), Vec::new()).await;
        }
        _ => emit_state(&app, "idle"),
    }
}

/// What a meta-turn does with its streamed result.
pub enum MetaTurn {
    /// Extend the last assistant message in place.
    Continue,
    /// Replace the whole conversation with a compact summary, so a long chat
    /// stays coherent within the model's context budget.
    Summarize,
}

/// Run a "meta" turn over the existing conversation: same provider/stream/
/// cancel machinery as a normal turn, but the instruction is never stored in
/// history — only its effect is (an extended last answer, or a summary that
/// replaces the transcript).
pub async fn run_meta_turn(app: AppHandle, kind: MetaTurn) {
    {
        let conversation = app.state::<AssistantConversation>();
        if conversation.busy.swap(true, Ordering::SeqCst) {
            debug!("Assistant busy; ignoring meta turn");
            return;
        }
        // Nothing to do on an empty conversation.
        let history = conversation.messages.lock().unwrap();
        if history.is_empty() {
            drop(history);
            conversation.busy.store(false, Ordering::SeqCst);
            return;
        }
    }
    let _busy = BusyReset(app.clone());
    app.state::<AssistantConversation>().begin_turn();

    let settings = get_settings(&app);
    let Some(provider) = settings.active_assistant_provider().cloned() else {
        emit_error(
            &app,
            "no_provider",
            "No assistant provider configured. Pick one in Settings → Assistant.".to_string(),
        );
        emit_state(&app, "idle");
        return;
    };
    let model = settings
        .assistant_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        emit_error(
            &app,
            "no_model",
            format!(
                "No model configured for provider '{}'. Set one in Settings → Assistant.",
                provider.label
            ),
        );
        emit_state(&app, "idle");
        return;
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    let instruction = match kind {
        MetaTurn::Continue => {
            "Continue your previous answer exactly where it stopped. Do not repeat or rephrase anything you already said — just carry on."
        }
        MetaTurn::Summarize => {
            "Summarize our entire conversation so far into a compact brief that can replace it: preserve key facts, decisions, names, numbers, code worth keeping, and open questions. Be faithful and dense. Use Markdown."
        }
    };

    // System prompt + capped history (including the last answer) + instruction.
    let mut messages: Vec<Value> = Vec::new();
    let mut system_content = settings.assistant_system_prompt.clone();
    if !system_content.trim().is_empty() {
        system_content.push_str("\n\n");
    }
    system_content.push_str(TIME_AWARENESS_NOTE);
    messages.push(json!({"role": "system", "content": system_content}));
    {
        let conversation = app.state::<AssistantConversation>();
        let history = conversation.messages.lock().unwrap();
        let max_messages = (settings.assistant_max_history_messages as usize).max(8);
        let mut kept: Vec<&ChatMessage> = Vec::new();
        let mut chars = 0usize;
        for message in history.iter().rev().take(max_messages) {
            chars += message.content.len();
            if chars > 24_000 && !kept.is_empty() {
                break;
            }
            kept.push(message);
        }
        for message in kept.into_iter().rev() {
            messages.push(json!({"role": message.role, "content": message.content}));
        }
    }
    messages.push(json!({
        "role": "user",
        "content": format!("{}\n\n{}", current_datetime_line(), instruction),
    }));

    emit_state(&app, "thinking");

    let _llm_activity_guard = if provider.id == "builtin" {
        let manager = app.state::<Arc<crate::managers::local_llm::LocalLlmManager>>();
        if let Err(e) = manager.ensure_running(&model).await {
            emit_error(&app, "engine_start", e.to_string());
            emit_state(&app, "idle");
            return;
        }
        Some(manager.begin_request())
    } else {
        None
    };

    let cancel = app.state::<AssistantConversation>().cancel.clone();
    let partial = Arc::new(Mutex::new(String::new()));
    let partial_cb = partial.clone();
    let app_for_tokens = app.clone();
    let stream_fut =
        llm_client::send_chat_stream(&provider, api_key, &model, messages, move |token| {
            if let Ok(mut buf) = partial_cb.lock() {
                buf.push_str(token);
            }
            let _ = app_for_tokens.emit("assistant-token", token.to_string());
        });
    tokio::pin!(stream_fut);

    let outcome = tokio::select! {
        result = &mut stream_fut => Some(result),
        _ = cancel.notified() => None,
    };

    let mut speaking = false;
    match outcome {
        None => {
            // Cancelled. For Continue, keep the partial extension (it's real
            // content); for Summarize, keep the original transcript untouched.
            crate::tts::stop_remote();
            if matches!(kind, MetaTurn::Continue) {
                let partial_text = partial
                    .lock()
                    .map(|b| b.trim().to_string())
                    .unwrap_or_default();
                if !partial_text.is_empty() {
                    apply_continuation(&app, &partial_text);
                    persist_assistant_session(&app);
                }
            }
            emit_conversation(&app);
            debug!("Assistant meta turn cancelled by user");
        }
        Some(Ok(full_text)) => {
            let text = full_text.trim().to_string();
            if !text.is_empty() {
                match kind {
                    MetaTurn::Continue => apply_continuation(&app, &text),
                    MetaTurn::Summarize => {
                        let conversation = app.state::<AssistantConversation>();
                        let mut history = conversation.messages.lock().unwrap();
                        history.clear();
                        history.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: text.clone(),
                        });
                    }
                }
                persist_assistant_session(&app);
            }
            emit_conversation(&app);

            // Speak only continuations — a summary is a housekeeping artifact.
            if matches!(kind, MetaTurn::Continue)
                && !app.state::<AssistantConversation>().is_cancelled()
                && settings.assistant_tts_enabled
            {
                spawn_tts_speak(&app, &settings, text);
                speaking = true;
            }
        }
        Some(Err(e)) => {
            error!("Assistant meta turn failed: {}", e);
            emit_error(&app, "provider", e);
        }
    }

    emit_state(&app, if speaking { "speaking" } else { "idle" });
}

/// Append continuation text to the last assistant message (or start one if the
/// conversation somehow ends on a user message).
fn apply_continuation(app: &AppHandle, text: &str) {
    let conversation = app.state::<AssistantConversation>();
    let mut history = conversation.messages.lock().unwrap();
    match history.last_mut() {
        Some(last) if last.role == "assistant" => {
            last.content.push_str("\n\n");
            last.content.push_str(text);
        }
        _ => history.push(ChatMessage {
            role: "assistant".to_string(),
            content: text.to_string(),
        }),
    }
}
