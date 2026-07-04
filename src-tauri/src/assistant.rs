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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};
use tauri_plugin_store::StoreExt;
use tokio::sync::Notify;

pub const PANEL_LABEL: &str = "assistant_panel";
const PANEL_MARGIN: f64 = 24.0;
/// Where the PILL last sat (legacy key — keeps existing stored positions).
const PANEL_POSITION_KEY: &str = "assistant_panel_position";
/// Where the EXPANDED panel last sat. Each form remembers its own place, so
/// expanding never dumps the panel wherever the pill happened to be dragged.
const PANEL_POSITION_EXPANDED_KEY: &str = "assistant_panel_position_expanded";

/// Collapsed "pill" mode: a small transparent window in which the chip floats
/// and hugs its content, like the STT recording overlay (128×40 there).
const PILL_WIDTH: f64 = 240.0;
const PILL_HEIGHT: f64 = 44.0;

/// The default expanded panel size (the "standard" preset). The window stays
/// user-resizable; the size preset chosen in Panel Appearance settings picks
/// the base dimensions, and a manual resize is remembered for the session
/// (below) so collapse → expand round-trips keep it.
const PANEL_WIDTH: f64 = 420.0;
const PANEL_HEIGHT: f64 = 560.0;

/// Logical width/height for each panel-size preset. Unknown/legacy values fall
/// back to the "standard" default.
fn panel_preset_size(size: &str) -> (f64, f64) {
    match size {
        "compact" => (360.0, 460.0),
        "large" => (520.0, 680.0),
        _ => (PANEL_WIDTH, PANEL_HEIGHT),
    }
}

/// Session memory of the last expanded size (logical px), so collapsing to the
/// pill and expanding again restores a manual resize. 0 = never resized this
/// session — fall back to the user's size preset. Not persisted: a fresh app
/// start uses the preset from settings.
static EXPANDED_W: AtomicU32 = AtomicU32::new(0);
static EXPANDED_H: AtomicU32 = AtomicU32::new(0);

fn expanded_size(app: &AppHandle) -> (f64, f64) {
    let w = EXPANDED_W.load(Ordering::SeqCst);
    let h = EXPANDED_H.load(Ordering::SeqCst);
    if w == 0 || h == 0 {
        // No manual resize this session — use the chosen size preset.
        panel_preset_size(&get_settings(app).assistant_panel_size)
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

/// A screen capture taken *early* — at the moment a voice recording starts —
/// when the "Vision capture timing" setting is `Immediate`. Stashed here by
/// `AssistantAction::start` and consumed by `run_voice_turn`, so the frame
/// reflects what the user was looking at when they *began* the question rather
/// than whatever is on screen after they finish talking. Holds the full-res
/// model data URL. Cleared at the start of every voice recording so a
/// cancelled/stale capture can never leak into a later turn.
static PENDING_IMMEDIATE_CAPTURE: Mutex<Option<String>> = Mutex::new(None);

/// Store an immediate (recording-start) screen capture for the next voice turn.
pub fn stash_immediate_capture(data_url: String) {
    if let Ok(mut slot) = PENDING_IMMEDIATE_CAPTURE.lock() {
        *slot = Some(data_url);
    }
}

/// Clear any stashed immediate capture (start of a new recording / on cancel).
pub fn clear_immediate_capture() {
    if let Ok(mut slot) = PENDING_IMMEDIATE_CAPTURE.lock() {
        *slot = None;
    }
}

/// Take (and clear) the stashed immediate capture, if one was taken at the
/// start of this recording.
fn take_immediate_capture() -> Option<String> {
    PENDING_IMMEDIATE_CAPTURE
        .lock()
        .ok()
        .and_then(|mut s| s.take())
}

/// Build small display thumbnails (data URLs) for the visuals attached to a
/// turn — the screen capture first (if any), then user-attached images — so the
/// panel can show and hover-enlarge what was sent, and it persists in history.
/// The full-resolution copies still go to the model; only these compact
/// thumbnails are stored. Runs the JPEG work off the async runtime; a thumbnail
/// that fails to encode is skipped (display-only — it never blocks the turn).
async fn build_message_thumbnails(screenshot: Option<String>, images: Vec<String>) -> Vec<String> {
    if screenshot.is_none() && images.is_empty() {
        return Vec::new();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut thumbs = Vec::new();
        for src in screenshot.iter().chain(images.iter()) {
            match crate::screenshot::data_url_to_thumbnail(src) {
                Ok(thumb) => thumbs.push(thumb),
                Err(e) => warn!("Vision thumbnail generation failed: {}", e),
            }
        }
        thumbs
    })
    .await
    .unwrap_or_default()
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
pub static PENDING_SNIP: Mutex<Option<PendingSnip>> = Mutex::new(None);

/// A frozen frame awaiting a region crop, bundled with the LOGICAL (CSS-pixel)
/// size of the overlay drawn over it. The selection rectangle arrives in the
/// overlay's CSS pixels; the finish step maps it onto the frame's real pixels
/// using the ratio of these two sizes — which never trusts a reported scale
/// factor (that can be wrong, or silently default to 1.0, on a high-DPI display
/// and mis-crop), so the crop lands correctly at any display scale.
pub struct PendingSnip {
    pub frame: image::DynamicImage,
    pub logical_w: f64,
    pub logical_h: f64,
}

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

/// One-shot "deliver this dictation's transcript to the app's own UI as an
/// event, instead of pasting it into the focused OS window" flag. Set when an
/// in-app dictation (source `"in-app"`, e.g. the Create-with-AI persona
/// description box) starts, and consumed when that recording completes. This is
/// what makes an in-app mic button reliable: the transcript arrives in the
/// webview via the `dictation-transcript` event rather than through a synthetic
/// paste that depends on OS focus.
static DICTATE_TO_FIELD: AtomicBool = AtomicBool::new(false);

pub fn set_dictate_to_field() {
    DICTATE_TO_FIELD.store(true, Ordering::SeqCst);
}

pub fn clear_dictate_to_field() {
    DICTATE_TO_FIELD.store(false, Ordering::SeqCst);
}

pub fn take_dictate_to_field() -> bool {
    DICTATE_TO_FIELD.swap(false, Ordering::SeqCst)
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
    /// Number of messages that had been distilled into memory as of the last
    /// distillation pass. A dirty-guard so closing the panel only triggers a
    /// learn pass when the conversation has actually grown since last time.
    last_distilled_len: AtomicUsize,
}

impl AssistantConversation {
    pub fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            busy: AtomicBool::new(false),
            cancel: Arc::new(Notify::new()),
            cancelled: AtomicBool::new(false),
            session_id: Mutex::new(None),
            last_distilled_len: AtomicUsize::new(0),
        }
    }

    /// Whether a turn is currently in flight.
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
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

    /// Snapshot the conversation for distillation IF it has grown since the
    /// last pass and holds at least two user turns. Marks the new length as
    /// distilled so repeated closes don't re-run on unchanged content.
    pub fn take_distillable(&self) -> Option<Vec<ChatMessage>> {
        let history = self.messages.lock().ok()?;
        let len = history.len();
        let last = self.last_distilled_len.load(Ordering::SeqCst);
        let user_turns = history.iter().filter(|m| m.role == "user").count();
        if len > last && user_turns >= 2 {
            self.last_distilled_len.store(len, Ordering::SeqCst);
            Some(history.clone())
        } else {
            None
        }
    }

    /// Mark the current conversation length as already distilled (e.g. after a
    /// manual "Update memory" pass) so a later close won't redo the same work.
    pub fn mark_distilled_current(&self) {
        if let Ok(history) = self.messages.lock() {
            self.last_distilled_len
                .store(history.len(), Ordering::SeqCst);
        }
    }

    /// Forget the distilled marker (conversation cleared / new session loaded).
    pub fn reset_distilled_marker(&self) {
        self.last_distilled_len.store(0, Ordering::SeqCst);
    }

    /// Replace the in-memory conversation with a session loaded from History,
    /// pointing future persists at that row so resuming continues it.
    pub fn load_session(&self, id: i64, messages: Vec<ChatMessage>) {
        if let Ok(mut history) = self.messages.lock() {
            *history = messages;
        }
        if let Ok(mut session) = self.session_id.lock() {
            *session = Some(id);
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

/// Storage key for the current mode's position slot.
fn position_key() -> &'static str {
    if PILL_MODE.load(Ordering::SeqCst) {
        PANEL_POSITION_KEY
    } else {
        PANEL_POSITION_EXPANDED_KEY
    }
}

fn saved_position_for(app: &AppHandle, key: &str) -> Option<(f64, f64)> {
    let store = app
        .store(crate::portable::store_path(
            crate::settings::SETTINGS_STORE_PATH,
        ))
        .ok()?;
    let value = store.get(key)?;
    let x = value.get("x")?.as_f64()?;
    let y = value.get("y")?.as_f64()?;
    Some((x, y))
}

fn saved_position(app: &AppHandle) -> Option<(f64, f64)> {
    saved_position_for(app, PANEL_POSITION_KEY)
}

/// Persist the window's current position into the slot for the CURRENT mode
/// (pill or expanded), so each form remembers its own place.
fn save_position(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        if let (Ok(pos), Ok(monitor)) = (window.outer_position(), window.current_monitor()) {
            let scale = monitor.map(|m| m.scale_factor()).unwrap_or(1.0);
            if let Ok(store) = app.store(crate::portable::store_path(
                crate::settings::SETTINGS_STORE_PATH,
            )) {
                store.set(
                    position_key(),
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
        expanded_size(app)
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
    // Learn from the conversation when the panel is closed — the common way to
    // "end" a chat besides Clear (users often just close it when it gets long).
    // Guarded so it only runs when memory is on, the chat isn't incognito, and
    // there's genuinely new content since the last pass, so opening/closing the
    // panel repeatedly never spends a wasted model call.
    let settings = crate::settings::get_settings(app);
    if settings.assistant_memory_enabled && !settings.assistant_memory_incognito {
        if let Some(conversation) = app.try_state::<AssistantConversation>() {
            if let Some(messages) = conversation.take_distillable() {
                let app_for_memory = app.clone();
                tauri::async_runtime::spawn(async move {
                    crate::memory::distill_and_store(app_for_memory, messages).await;
                });
            }
        }
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
/// Each form remembers its own last position (separate slots), so expanding
/// brings the panel back where the PANEL last was — not wherever the pill was
/// dragged. First-ever expand falls back to growing upward from the pill
/// (bottom-left anchor). Everything is clamped onto the current monitor.
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

        // Remember the current form's position + (for the panel) its size.
        save_position(app);
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
            expanded_size(app)
        };
        let _ = window.set_size(tauri::LogicalSize::new(new_w, new_h));

        // Restore the target form's own remembered position; fall back to a
        // bottom-left anchor on the current spot (grows upward, never off the
        // bottom of the screen).
        let target_key = if collapsed {
            PANEL_POSITION_KEY
        } else {
            PANEL_POSITION_EXPANDED_KEY
        };
        let (mut new_x, mut new_y) = match saved_position_for(app, target_key) {
            Some(pos) => pos,
            None => match (old_pos, old_size) {
                (Some(pos), Some(size)) => {
                    let old_x = pos.x as f64 / scale;
                    let old_y = pos.y as f64 / scale;
                    let old_h = size.height as f64 / scale;
                    (old_x, old_y + old_h - new_h)
                }
                _ => default_position(app),
            },
        };
        if let Ok(Some(monitor)) = window.current_monitor() {
            let mx = monitor.position().x as f64 / scale;
            let my = monitor.position().y as f64 / scale;
            let mw = monitor.size().width as f64 / scale;
            let mh = monitor.size().height as f64 / scale;
            new_x = new_x.clamp(mx + 8.0, (mx + mw - new_w - 8.0).max(mx + 8.0));
            new_y = new_y.clamp(my + 8.0, (my + mh - new_h - 8.0).max(my + 8.0));
        }
        let _ = window.set_position(tauri::LogicalPosition::new(new_x, new_y));

        let _ = app.emit("assistant-collapsed", collapsed);
    } else {
        PILL_MODE.store(collapsed, Ordering::SeqCst);
    }
}

/// Apply a panel-size preset chosen in Panel Appearance settings. Remembers it
/// as the session size (overriding an earlier manual drag-resize so the choice
/// takes effect immediately and sticks across collapse/expand), and resizes the
/// live window when the panel is currently expanded and on screen. The pill is
/// unaffected.
pub fn apply_panel_size(app: &AppHandle, size: &str) {
    let (w, h) = panel_preset_size(size);
    EXPANDED_W.store(w as u32, Ordering::SeqCst);
    EXPANDED_H.store(h as u32, Ordering::SeqCst);

    // Only touch the window if the expanded panel is actually visible.
    if PILL_MODE.load(Ordering::SeqCst) {
        return;
    }
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }

    let _ = window.set_size(tauri::LogicalSize::new(w, h));

    // Keep the newly sized panel fully on its monitor (growing can push it past
    // the right/bottom edge).
    if let (Ok(pos), Ok(Some(monitor))) = (window.outer_position(), window.current_monitor()) {
        let scale = monitor.scale_factor();
        let mx = monitor.position().x as f64 / scale;
        let my = monitor.position().y as f64 / scale;
        let mw = monitor.size().width as f64 / scale;
        let mh = monitor.size().height as f64 / scale;
        let x = (pos.x as f64 / scale).clamp(mx + 8.0, (mx + mw - w - 8.0).max(mx + 8.0));
        let y = (pos.y as f64 / scale).clamp(my + 8.0, (my + mh - h - 8.0).max(my + 8.0));
        let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    }

    save_position(app);
}

// ---------------------------------------------------------------------------
// Region snip overlay
// ---------------------------------------------------------------------------

pub const SNIP_LABEL: &str = "snip_overlay";
/// Open the region-snip overlay for a frame that was just captured: store it
/// in PENDING_SNIP, then cover `monitor` with a transparent selection window.
/// Called from an async command (worker thread) — building a webview inline on
/// the main thread inside a command deadlocks WebView2 on Windows, so this must
/// NOT be dispatched to the main thread.
///
/// `monitor` is chosen by the caller (from Tauri's monitor list) and the frozen
/// `frame` is captured from that SAME monitor, so the overlay and the crop stay
/// aligned on multi-monitor setups.
pub fn open_snip_overlay(
    app: &AppHandle,
    frame: image::DynamicImage,
    monitor: tauri::Monitor,
) -> Result<(), String> {
    if app.get_webview_window(SNIP_LABEL).is_some() {
        return Ok(()); // already snipping
    }

    // Cover the chosen monitor using LOGICAL coordinates set at BUILD time.
    // Positioning/sizing AFTER build via PhysicalPosition/PhysicalSize is
    // unreliable across monitors: tao converts physical values using the scale
    // factor of the monitor the window is *currently* on (usually the primary),
    // so on a mixed-DPI / mixed-orientation multi-monitor setup the snip window
    // lands off-screen or zero-sized and "nothing happens". Building with the
    // target monitor's logical origin/size is exactly how the recording overlay
    // and the panel place themselves reliably (see overlay.rs).
    let scale = monitor.scale_factor();
    let logical_x = monitor.position().x as f64 / scale;
    let logical_y = monitor.position().y as f64 / scale;
    let logical_w = (monitor.size().width as f64 / scale).max(1.0);
    let logical_h = (monitor.size().height as f64 / scale).max(1.0);

    // Stash the frozen frame together with the overlay's logical size, so the
    // finish step can map the selection (measured in the overlay's CSS pixels)
    // straight onto the frame's real pixels without trusting a scale factor.
    if let Ok(mut pending) = PENDING_SNIP.lock() {
        *pending = Some(PendingSnip {
            frame,
            logical_w,
            logical_h,
        });
    }

    let mut builder = WebviewWindowBuilder::new(
        app,
        SNIP_LABEL,
        tauri::WebviewUrl::App("src/assistant/snip.html".into()),
    )
    .title("Snip")
    .inner_size(logical_w, logical_h)
    .position(logical_x, logical_y)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .accept_first_mouse(true)
    .focused(true)
    .visible(true);

    // Match the other windows' WebView2 user-data dir so portable builds don't
    // spin up a second cache (and so window creation stays consistent).
    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    let window = builder
        .build()
        .map_err(|e| format!("Couldn't open the snip overlay: {}", e))?;

    let _ = window.set_focus();
    #[cfg(target_os = "windows")]
    force_panel_topmost(&window);
    Ok(())
}

/// Close the snip overlay and, when a rectangle was chosen, crop it from the
/// frozen frame and hand it to the panel as a pending image attachment via the
/// `assistant-region-captured` event. `rect` is in the overlay's CSS pixels; it
/// is mapped onto the frame's real pixels using the ratio of the frame size to
/// the overlay's logical size (stored in [`PendingSnip`]) — robust to any
/// display scaling.
pub fn finish_region_snip(app: &AppHandle, rect: Option<(f64, f64, f64, f64)>) {
    if let Some(window) = app.get_webview_window(SNIP_LABEL) {
        // Destroy (not close): the app-wide `CloseRequested` handler calls
        // `prevent_close()` + `hide()` for every window, so `close()` would only
        // HIDE this overlay. A hidden snip window still satisfies the
        // `get_webview_window(SNIP_LABEL).is_some()` guard in
        // `open_snip_overlay`, so the next snip would silently no-op ("already
        // snipping") and the overlay would never reappear. `destroy()` tears the
        // window down for real so a fresh snip can open every time.
        let _ = window.destroy();
    }
    let pending = PENDING_SNIP.lock().ok().and_then(|mut p| p.take());
    let Some(rect) = rect else {
        return; // cancelled
    };
    let Some(PendingSnip {
        frame,
        logical_w,
        logical_h,
    }) = pending
    else {
        emit_error(app, "screen_capture", "No captured frame for snip".into());
        return;
    };

    // Map the selection from the overlay's CSS pixels onto the frame's real
    // pixels via the ratio of the two coordinate spaces. This never multiplies
    // by a reported scale factor (which can be wrong — or default to 1.0 — on a
    // high-DPI display and silently mis-crop), so it lands correctly at any
    // display scale.
    let (frame_w, frame_h) = (frame.width() as f64, frame.height() as f64);
    let sx = if logical_w > 0.0 {
        frame_w / logical_w
    } else {
        1.0
    };
    let sy = if logical_h > 0.0 {
        frame_h / logical_h
    } else {
        1.0
    };
    let (x, y, w, h) = rect;
    let to_px = |v: f64, s: f64| -> u32 { (v * s).round().max(0.0) as u32 };

    // Ignore a stray click: a selection under ~4 real pixels isn't a crop.
    if w * sx < 4.0 || h * sy < 4.0 {
        return;
    }

    let settings = get_settings(app);
    let profile = settings
        .active_assistant_provider()
        .map(|p| crate::screenshot::CaptureProfile::for_base_url(&p.base_url))
        .unwrap_or(crate::screenshot::CaptureProfile::Generous);

    match crate::screenshot::encode_region_data_url(
        &frame,
        profile,
        to_px(x, sx),
        to_px(y, sy),
        to_px(w, sx),
        to_px(h, sy),
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
    // The Cat character never looks at the screen (it just meows), so don't
    // capture one even if the toggle/phrasing would normally arm it.
    let wants_screen = !settings.active_character_is_cat()
        && (screen_armed() || wants_screen_context(&transcription));

    // An immediate (recording-start) capture may already be waiting — taken
    // when the "Vision capture timing" setting is Immediate and the camera was
    // armed. Take it regardless so it never lingers into a later turn; only use
    // it when this turn actually wants the screen.
    let immediate = take_immediate_capture();

    let screenshot = if wants_screen && settings.assistant_screenshot_enabled {
        // Reuse the frame grabbed when the user started speaking — but only when
        // vision is still armed (the same arm that triggered that early
        // capture). A turn that only wants the screen because of a "what's on my
        // screen" phrase always captures fresh below.
        if let Some(data_url) = immediate.filter(|_| screen_armed()) {
            Some(data_url)
        } else {
            // Capture now (On-send timing, or a "what's on my screen" phrase we
            // could only detect after transcription). Tiny body only for Azure;
            // loopback (built-in/local engine) gets a balanced image, cloud gets
            // the sharp one. Target the monitor the mouse cursor is on — with
            // multiple displays that's the screen the user is actually working
            // on (the panel rarely moves, the cursor follows attention).
            let profile = settings
                .active_assistant_provider()
                .map(|p| crate::screenshot::CaptureProfile::for_base_url(&p.base_url))
                .unwrap_or(crate::screenshot::CaptureProfile::Generous);
            let captured = tauri::async_runtime::spawn_blocking(move || {
                crate::screenshot::capture_screen_data_url_at(None, profile)
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
const TIME_AWARENESS_NOTE: &str = "The user's current local date and time is provided at the top of their message — treat it as the present moment. Use it to resolve relative time references (today, yesterday, tonight, last week, this month, this year, how long ago) into concrete dates, and when looking something up, build the query around the correct date. Never guess the current date from your training data; your training is frozen at a past cutoff, so anything that can change over time may be out of date.";

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

/// Build the stored form of a user message: the text plus one marker line per
/// attachment (files/images/screenshot). The panel strips these markers for
/// display and shows chips instead; on later turns they remind the model that
/// attachments accompanied the message. Shared by the normal and Cat turns.
fn compose_stored_user_message(
    user_text: &str,
    files: &[FileAttachment],
    images: &[String],
    has_screenshot: bool,
) -> String {
    let mut stored = user_text.to_string();
    for file in files {
        stored.push_str(&format!("\n{} {}]", FILE_MARKER_PREFIX, file.name));
    }
    for _ in images {
        stored.push_str(&format!("\n{}", IMAGE_MARKER));
    }
    if has_screenshot {
        stored.push_str(&format!("\n{}", SCREENSHOT_MARKER));
    }
    stored
}

/// A short, random string of meows for the "Cat" character — sometimes
/// capitalized, sometimes with a trailing "!" or two. Uses a tiny time-seeded
/// LCG so we don't pull in the `rand` crate just for a joke.
fn random_meow() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        | 1;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state >> 33
    };
    let count = 1 + (next() % 4) as usize; // 1..=4 meows
    let mut parts = Vec::with_capacity(count);
    for _ in 0..count {
        let mut token = if next() % 2 == 0 { "meow" } else { "Meow" }.to_string();
        for _ in 0..(next() % 3) {
            // 0..=2 exclamation marks
            token.push('!');
        }
        parts.push(token);
    }
    parts.join(" ")
}

/// Handle a turn for the joke "Cat" character: record the user's message, then
/// reply with random meows — no model call, no web search, no vision. Speaks
/// the meow aloud when TTS is on (because obviously it should).
fn run_cat_turn(
    app: &AppHandle,
    settings: &crate::settings::AppSettings,
    user_text: &str,
    files: &[FileAttachment],
    images: &[String],
    has_screenshot: bool,
    thumbnails: Vec<String>,
) {
    {
        let conversation = app.state::<AssistantConversation>();
        let mut history = conversation.messages.lock().unwrap();
        history.push(ChatMessage {
            role: "user".to_string(),
            content: compose_stored_user_message(user_text, files, images, has_screenshot),
            images: thumbnails,
        });
    }
    emit_conversation(app);
    persist_assistant_session(app);

    let reply = random_meow();
    {
        let conversation = app.state::<AssistantConversation>();
        let mut history = conversation.messages.lock().unwrap();
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: reply.clone(),
            images: Vec::new(),
        });
    }
    emit_conversation(app);
    persist_assistant_session(app);

    // Speak the meow when TTS is on — unless a Stop already landed in the gap.
    let mut speaking = false;
    if !app.state::<AssistantConversation>().is_cancelled() && settings.assistant_tts_enabled {
        spawn_tts_speak(app, settings, reply);
        speaking = true;
    }
    emit_state(app, if speaking { "speaking" } else { "idle" });
}

/// The `web_search` tool definition handed to tool-capable models. The model
/// decides whether to call it; when it does, we run the search and feed the
/// results back. The description mirrors the planner's guidance so search
/// behavior stays consistent across the two paths.
fn web_search_tool_def() -> Value {
    json!([{
        "type": "function",
        "function": {
            "name": "web_search",
            "description": "Search the live web for current or external facts — news, prices, weather, sports scores, schedules, product releases/versions, who currently holds a role, or any recent/niche fact your training data wouldn't reliably know. Returns titles and short snippets. Call this ONLY when the answer needs current or external information; for greetings, general knowledge, writing, coding, or math, answer directly without searching.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "A concise, keyword-rich search query, as you would type it into a search engine."
                    },
                    "freshness": {
                        "type": "string",
                        "enum": ["none", "day", "week", "month", "year"],
                        "description": "How recent results should be; use 'day'/'week' for breaking news, 'none' when recency doesn't matter."
                    },
                    "news": {
                        "type": "boolean",
                        "description": "True for current events / breaking news topics."
                    }
                },
                "required": ["query"]
            }
        }
    }])
}

/// Parse the JSON arguments of a `web_search` tool call into (query, freshness,
/// news). Tolerates missing/extra fields and malformed JSON (returns empties).
fn parse_web_search_args(raw: &str) -> (String, Option<String>, bool) {
    let v: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let query = v
        .get("query")
        .and_then(|q| q.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let freshness = v
        .get("freshness")
        .and_then(|f| f.as_str())
        .map(|s| s.to_string());
    let news = v.get("news").and_then(|n| n.as_bool()).unwrap_or(false);
    (query, freshness, news)
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

    // Build the small display thumbnails once (screen capture first, then
    // attached images), before branching. Stored on the user message so the
    // panel can show + hover-enlarge what was sent, and it persists in history.
    let thumbnails = build_message_thumbnails(screenshot.clone(), images.clone()).await;

    // The "Cat" character ignores the model entirely: no provider, no web
    // search, no vision — it just meows. Handle it up front so it works even
    // when no LLM provider/model is configured.
    if settings.active_character_is_cat() {
        run_cat_turn(
            &app,
            &settings,
            &user_text,
            &files,
            &images,
            screenshot.is_some(),
            thumbnails,
        );
        return;
    }

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

    // Providers that need a compact request body: Azure's gateway rejects
    // oversized JSON, and the local engine (built-in, or an Ollama/LM Studio
    // loopback server) runs a small context window. Used both to trim history
    // and — on visual turns — to shrink the web-search context so the combined
    // image + snippets payload stays within the provider's limits.
    let base_url_lc = provider.base_url.to_ascii_lowercase();
    let needs_small_body = provider.id == "builtin"
        || base_url_lc.contains("azure")
        || base_url_lc.contains("127.0.0.1")
        || base_url_lc.contains("localhost");

    // Decide how web search runs this turn (only when enabled and the cheap
    // local pre-gate says it's plausibly a search — chit-chat/code/math never
    // search):
    //   • OpenRouter (non-visual) → its native `:online` search (server-side).
    //   • Other cloud (non-visual) → tool calling: the model itself decides and
    //     calls our `web_search` tool inline (no separate planner round-trip).
    //   • Local engine, or ANY visual turn → the planner path: pre-search and
    //     inject the results as text (avoids sending tools alongside an image,
    //     and small local models handle tool calling poorly).
    let is_openrouter = provider.id == "openrouter" || base_url_lc.contains("openrouter");
    let is_local_engine = provider.id == "builtin"
        || base_url_lc.contains("127.0.0.1")
        || base_url_lc.contains("localhost");
    let web_wanted = settings.assistant_web_search_enabled && web_search::should_search(&user_text);
    let web_via_online =
        web_wanted && is_openrouter && !has_visual && settings.assistant_prefer_provider_web_search;
    let web_via_tools = web_wanted && !is_openrouter && !is_local_engine && !has_visual;
    let web_via_planner = web_wanted && !web_via_online && !web_via_tools;
    // OpenRouter's `:online` model suffix turns on its built-in web search
    // server-side; every other path uses the model name unchanged.
    let request_model = if web_via_online {
        format!("{}:online", model)
    } else {
        model.clone()
    };

    // Record the user message (text markers instead of raw image/file data)
    // and show it in the panel immediately — before any web search runs, so the
    // bubble appears right away while results are being fetched.
    {
        let conversation = app.state::<AssistantConversation>();
        let mut history = conversation.messages.lock().unwrap();
        history.push(ChatMessage {
            role: "user".to_string(),
            content: compose_stored_user_message(&user_text, &files, &images, screenshot.is_some()),
            images: thumbnails,
        });
    }
    emit_conversation(&app);
    // Save right after the user message so the question is preserved even if
    // the model (or the search) errors out before replying.
    persist_assistant_session(&app);

    // Optional web search. Runs whenever it's enabled and the request looks
    // like it needs current/external facts — including turns that also carry a
    // screenshot or image, so "what's the current price of this?" about a photo
    // works. A capable model plans the search (deciding whether one is actually
    // needed and rewriting the often-messy transcribed request into clean
    // queries), then we fetch real page content to ground the answer. Any
    // failure or timeout degrades gracefully: we answer without web context and
    // show a small notice rather than breaking the turn. The cheap
    // `should_search` pre-gate skips obvious non-search turns so we don't spend
    // a planner round-trip on chit-chat, code, or math. On visual turns the
    // web-context budget is trimmed below (the image already fills the body).
    let web_context: Option<String> = if web_via_planner {
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
                        let mut budget =
                            web_search::context_budget_for(settings.assistant_search_depth);
                        // On a visual turn the image already dominates the
                        // request body, so trim the web-context budget to keep
                        // the combined payload within limits — tightly for
                        // constrained providers (Azure's JSON-body cap, the
                        // local engine's small context), moderately for cloud.
                        if has_visual {
                            budget = budget.min(if needs_small_body { 3_000 } else { 8_000 });
                        }
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
    // (Cache-friendly: the prefix only ever grows by appending.)
    //
    // The visible "Messages History" setting is the source of truth for HOW
    // MANY past messages to send. A *secondary* character cap only guards
    // providers that genuinely need a small request body:
    //   • Azure — its gateway/parser rejects oversized JSON bodies.
    //   • The local engine (built-in, or an Ollama/LM Studio loopback server) —
    //     it runs a small context window, so a huge history would overflow it.
    // Cloud APIs (OpenAI, Anthropic, Groq, OpenRouter, …) have large context
    // windows, so they get exactly the history the user asked for — no hidden
    // token/char cap. Visual turns still trim tighter on the constrained
    // providers because the image already dominates their body budget; cloud
    // visual turns keep the user's full message count.
    let (max_history_messages, max_history_chars) = if needs_small_body {
        if has_visual {
            (
                (settings.assistant_max_history_messages as usize).min(4),
                6_000usize,
            )
        } else {
            (
                settings.assistant_max_history_messages as usize,
                24_000usize,
            )
        }
    } else {
        // Cloud API: honor the Messages History setting; don't secretly trim.
        (settings.assistant_max_history_messages as usize, usize::MAX)
    };
    let mut messages: Vec<Value> = Vec::new();
    let system_content = {
        let mut content = settings.effective_system_prompt();
        // Always note that a live date/time accompanies the user's message, so
        // time-relative answers are correct. Fixed text → cache-safe.
        if !content.trim().is_empty() {
            content.push_str("\n\n");
        }
        content.push_str(TIME_AWARENESS_NOTE);
        // Tell the model about its web capability. On the tool-calling path it
        // must actively CALL the `web_search` tool, so it gets a tool-oriented
        // instruction; on the other paths the app runs search for it, so it
        // gets the "results may be added for you" note. Either way it never
        // denies having internet access. Stable per-mode text → cache-safe.
        if web_via_tools {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(
                "IMPORTANT — your situation and tools:\n\
                 • Your training data has a fixed cutoff and is very likely OUT OF DATE for \
                 anything that changes over time: who currently holds an office, role, or title; \
                 prices; scores and standings; schedules; software versions; and recent events. \
                 The user's real current date is provided with their message — treat it as now.\n\
                 • You have a web_search tool that returns live results from the internet.\n\
                 How to use it:\n\
                 1. For ANY question about current, recent, or changeable facts, you MUST call \
                 web_search BEFORE answering. Do not answer from memory even if you feel certain — \
                 your memory can be years out of date and confidently wrong (e.g. who holds a \
                 political office).\n\
                 2. Base current-fact answers only on what the results actually say. When the \
                 results confirm something, state it plainly and confidently — do NOT hedge with \
                 \"appears to be\", \"suggests\", or \"as of my knowledge\".\n\
                 3. If you couldn't search or the results don't cover it, say you're not certain \
                 and offer to look it up — never guess a current fact.\n\
                 4. For timeless things (definitions, concepts, math, coding, writing, \
                 translation, general how-to), just answer directly without searching.\n\
                 Examples:\n\
                 • \"What's the capital of France?\" → answer directly (timeless).\n\
                 • \"Who is the current president / PM / CEO of X?\" → web_search FIRST, then answer.\n\
                 • \"Latest iPhone price\", \"who won yesterday's match\", \"weather today\" → web_search FIRST.\n\
                 • \"Explain how recursion works\" → answer directly (timeless).",
            );
        } else if settings.assistant_web_search_enabled {
            content.push_str("\n\n");
            content.push_str(web_search::WEB_SEARCH_CAPABILITY_NOTE);
        }
        // Append the response-length preference (if any) so a single system
        // prompt covers both display and spoken output. The active persona can
        // override the global setting, so this resolves per-persona first.
        if let Some(directive) = settings.effective_response_length().directive() {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(directive);
        }
        // Personal memory: append an advisory "About You" block (plus the notes
        // relevant to this message, within the detail budget) when memory is on
        // and the conversation isn't incognito. Placed late so the earlier
        // prompt prefix stays stable/cache-friendly; the block itself states
        // that it's advisory and never overrides the user's current message.
        if let Some(block) = crate::memory::build_memory_block(&settings, &user_text) {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&block);
        }
        // On web-search turns, tell the model to ground its answer in the
        // results (whether prepended to the user's message on the planner path,
        // or returned via the web_search tool) — and to treat them as its OWN
        // findings (never "the results you sent"). The directive adapts to TTS:
        // speech-friendly prose when spoken, richer Markdown when read on screen.
        if web_context.is_some() || web_via_tools {
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

    // Generation. On the tool-calling path the model may call our `web_search`
    // tool; we run it, feed the results back, and let it continue (up to a few
    // rounds). Otherwise it's a plain stream (which, for OpenRouter, may carry
    // the `:online` suffix so the search happens server-side). Both stream
    // tokens via `assistant-token` and resolve to the final answer text, then
    // flow through the shared outcome handling below.
    let outcome = if web_via_tools {
        let tools = web_search_tool_def();
        // Safety net (same intent as the planner path's force-search): capable
        // models are often over-confident and answer "who is the current …"
        // from stale training data instead of searching. For clearly
        // time-sensitive or explicitly-requested queries, FORCE the web_search
        // tool on the first round; otherwise let the model decide (auto).
        let force_search = web_search::is_explicit_search_request(&user_text)
            || web_search::looks_time_sensitive(&user_text);
        let partial_cb = partial.clone();
        let app_tokens = app.clone();
        let app_state = app.clone();
        let provider_c = provider.clone();
        let api_key_c = api_key.clone();
        let model_c = model.clone();
        let settings_c = settings.clone();
        let loop_fut = async move {
            let mut msgs = messages;
            let mut answer = String::new();
            // A small round cap: one search round covers almost every question;
            // the cap just prevents a pathological tool-call loop.
            for round in 0..3usize {
                // Force the tool only on the first round of a warranted search;
                // afterwards use "auto" so the model answers with the results
                // instead of being forced to search again (which would loop).
                let tool_choice = if round == 0 && force_search {
                    json!({ "type": "function", "function": { "name": "web_search" } })
                } else {
                    json!("auto")
                };
                let po = partial_cb.clone();
                let ao = app_tokens.clone();
                let round_out = llm_client::send_chat_stream_with_tools(
                    &provider_c,
                    api_key_c.clone(),
                    &model_c,
                    msgs.clone(),
                    tools.clone(),
                    tool_choice,
                    move |token| {
                        if let Ok(mut buf) = po.lock() {
                            buf.push_str(token);
                        }
                        let _ = ao.emit("assistant-token", token.to_string());
                    },
                )
                .await?;
                if round_out.tool_calls.is_empty() {
                    answer = round_out.text;
                    break;
                }
                // The model asked to search — reflect that in the panel.
                emit_state(&app_state, "searching");
                let tool_calls_json: Vec<Value> = round_out
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": { "name": tc.name, "arguments": tc.arguments }
                        })
                    })
                    .collect();
                msgs.push(json!({
                    "role": "assistant",
                    "content": round_out.text,
                    "tool_calls": tool_calls_json
                }));
                for tc in &round_out.tool_calls {
                    let (query, freshness, news) = parse_web_search_args(&tc.arguments);
                    let results = if tc.name == "web_search" && !query.is_empty() {
                        web_search::run_tool_search(&settings_c, &query, freshness.as_deref(), news)
                            .await
                    } else {
                        Vec::new()
                    };
                    let content = if results.is_empty() {
                        "No results found for that query.".to_string()
                    } else {
                        let budget =
                            web_search::context_budget_for(settings_c.assistant_search_depth);
                        web_search::format_results_for_prompt(&results, budget)
                    };
                    msgs.push(json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": content
                    }));
                }
                emit_state(&app_state, "thinking");
                answer = round_out.text;
            }
            Ok::<String, String>(answer)
        };
        tokio::pin!(loop_fut);
        tokio::select! {
            result = &mut loop_fut => Some(result),
            _ = cancel.notified() => None,
        }
    } else {
        let partial_cb = partial.clone();
        let app_for_tokens = app.clone();
        let stream_fut = llm_client::send_chat_stream(
            &provider,
            api_key.clone(),
            &request_model,
            messages,
            move |token| {
                if let Ok(mut buf) = partial_cb.lock() {
                    buf.push_str(token);
                }
                let _ = app_for_tokens.emit("assistant-token", token.to_string());
            },
        );
        tokio::pin!(stream_fut);
        // Race the stream against a Stop request. notify_waiters wakes this select.
        tokio::select! {
            result = &mut stream_fut => Some(result),
            _ = cancel.notified() => None,
        }
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
                    images: Vec::new(),
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
                    images: Vec::new(),
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
// Conversation quality-of-life: regenerate / summarize
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
/// message that produced it, then re-run the turn with the same text. The
/// conversation FORKS in History — the pre-regenerate transcript stays saved
/// in its old row and the new attempt gets a fresh row, so earlier variants
/// remain reachable (and resumable) from the History view.
///
/// Visual attachments aren't stored (only markers), so a regenerated turn is
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
    // Fork: keep the old variant's row intact; persist into a new one.
    app.state::<AssistantConversation>().reset_session();
    emit_conversation(&app);
    match text {
        Some(t) if !t.is_empty() => {
            run_assistant_turn(app, t, None, Vec::new(), Vec::new()).await;
        }
        _ => emit_state(&app, "idle"),
    }
}

/// Compact the conversation into a summary that replaces the transcript (the
/// panel's `/summarize` command): same provider/stream/cancel machinery as a
/// normal turn, but the instruction is never stored in history — only its
/// effect is. On cancel or error the original transcript stays untouched.
pub async fn run_summarize_turn(app: AppHandle) {
    {
        let conversation = app.state::<AssistantConversation>();
        if conversation.busy.swap(true, Ordering::SeqCst) {
            debug!("Assistant busy; ignoring summarize");
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

    let instruction = "Summarize our entire conversation so far into a compact brief that can replace it: preserve key facts, decisions, names, numbers, code worth keeping, and open questions. Be faithful and dense. Use Markdown.";

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

    match outcome {
        None => {
            // Cancelled — keep the original transcript untouched.
            crate::tts::stop_remote();
            emit_conversation(&app);
            debug!("Assistant summarize cancelled by user");
        }
        Some(Ok(full_text)) => {
            let text = full_text.trim().to_string();
            if !text.is_empty() {
                {
                    let conversation = app.state::<AssistantConversation>();
                    let mut history = conversation.messages.lock().unwrap();
                    history.clear();
                    history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: text,
                        images: Vec::new(),
                    });
                }
                persist_assistant_session(&app);
            }
            emit_conversation(&app);
        }
        Some(Err(e)) => {
            error!("Assistant summarize failed: {}", e);
            emit_error(&app, "provider", e);
        }
    }

    emit_state(&app, "idle");
}
