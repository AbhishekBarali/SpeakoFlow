//! Assistant mode: voice question → local STT → LLM → streaming answer in a
//! floating always-on-top panel window.
//!
//! Conversation state lives in memory (cleared on app restart or via the
//! panel's clear button). Requests are built cache-friendly: byte-identical
//! system prompt first, then append-only history, newest user message last.

use crate::llm_client::{self, ChatMessage};
use crate::settings::get_settings;
use log::{debug, error};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};
use tauri_plugin_store::StoreExt;

pub const PANEL_LABEL: &str = "assistant_panel";
const PANEL_MARGIN: f64 = 24.0;
const PANEL_POSITION_KEY: &str = "assistant_panel_position";

/// Collapsed "pill" mode dimensions (small floating button bar).
const PILL_WIDTH: f64 = 232.0;
const PILL_HEIGHT: f64 = 64.0;

/// Logical size for each panel size preset.
pub fn panel_size_for(preset: &str) -> (f64, f64) {
    match preset {
        "compact" => (340.0, 440.0),
        "large" => (560.0, 720.0),
        _ => (420.0, 560.0),
    }
}

/// Whether the panel is currently collapsed to the pill.
static PILL_MODE: AtomicBool = AtomicBool::new(false);

/// One-shot "attach a screenshot to the next turn" flag, set by the panel's
/// camera button. Applies to BOTH typed and voice turns.
static SCREEN_ARMED: AtomicBool = AtomicBool::new(false);

pub fn set_screen_armed(app: &AppHandle, armed: bool) {
    SCREEN_ARMED.store(armed, Ordering::SeqCst);
    let _ = app.emit("assistant-screen-armed", armed);
}

/// Consume the armed flag (returns true at most once per arm).
pub fn take_screen_armed(app: &AppHandle) -> bool {
    let armed = SCREEN_ARMED.swap(false, Ordering::SeqCst);
    if armed {
        let _ = app.emit("assistant-screen-armed", false);
    }
    armed
}

/// Appended to the stored user message when a screenshot was sent with it.
/// The panel strips it for display and shows a chip instead; on later turns
/// it tells the model a screenshot accompanied that message.
pub const SCREENSHOT_MARKER: &str = "[screenshot attached]";

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
}

impl AssistantConversation {
    pub fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            busy: AtomicBool::new(false),
        }
    }
}

#[derive(Clone, Serialize)]
struct AssistantStatePayload {
    state: String,
}

/// Emit a pipeline state update to the panel:
/// "listening" | "transcribing" | "thinking" | "idle"
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
    let settings = get_settings(app);
    let (panel_w, panel_h) = panel_size_for(&settings.assistant_panel_size);
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let scale = monitor.scale_factor();
        let mw = monitor.size().width as f64 / scale;
        let mh = monitor.size().height as f64 / scale;
        let mx = monitor.position().x as f64 / scale;
        let my = monitor.position().y as f64 / scale;
        (
            mx + mw - panel_w - PANEL_MARGIN,
            my + mh - panel_h - PANEL_MARGIN - 40.0, // keep clear of taskbar
        )
    } else {
        (100.0, 100.0)
    }
}

/// Create the assistant panel window, hidden by default. Called once at setup.
pub fn create_assistant_panel(app: &AppHandle) {
    let (x, y) = saved_position(app).unwrap_or_else(|| default_position(app));
    let settings = get_settings(app);
    let (panel_w, panel_h) = panel_size_for(&settings.assistant_panel_size);

    let mut builder = WebviewWindowBuilder::new(
        app,
        PANEL_LABEL,
        tauri::WebviewUrl::App("src/assistant/index.html".into()),
    )
    .title("Assistant")
    .inner_size(panel_w, panel_h)
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
        let _ = window.show();
        #[cfg(target_os = "windows")]
        force_panel_topmost(&window);
        let _ = app.emit("assistant-panel-shown", ());
    }
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

/// Apply the configured size preset to the panel window (no-op in pill mode).
pub fn apply_panel_size(app: &AppHandle) {
    if PILL_MODE.load(Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let settings = get_settings(app);
        let (w, h) = panel_size_for(&settings.assistant_panel_size);
        let _ = window.set_size(tauri::LogicalSize::new(w, h));
    }
}

/// Collapse the panel to a small pill, or restore it to its configured size.
pub fn set_panel_collapsed(app: &AppHandle, collapsed: bool) {
    PILL_MODE.store(collapsed, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        if collapsed {
            let _ = window.set_size(tauri::LogicalSize::new(PILL_WIDTH, PILL_HEIGHT));
        } else {
            let settings = get_settings(app);
            let (w, h) = panel_size_for(&settings.assistant_panel_size);
            let _ = window.set_size(tauri::LogicalSize::new(w, h));
        }
        let _ = app.emit("assistant-collapsed", collapsed);
    }
}

// ---------------------------------------------------------------------------
// Assistant pipeline
// ---------------------------------------------------------------------------

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

/// Run one assistant turn: record the user message, stream the LLM answer to
/// the panel via events, and append the reply to the conversation history.
///
/// `screenshot` is an optional `data:image/...;base64,` URL captured from the
/// user's screen; it is sent to the model only for this turn (the history
/// keeps a text marker instead, so images never burn tokens twice).
///
/// Events emitted:
/// - `assistant-conversation` (Vec<ChatMessage>): full snapshot after change
/// - `assistant-token` (String): each streamed content delta
/// - `assistant-tts` (String): short spoken summary (only when TTS enabled)
/// - `assistant-error` (String): error description
pub async fn run_assistant_turn(app: AppHandle, user_text: String, screenshot: Option<String>) {
    let user_text = user_text.trim().to_string();
    if user_text.is_empty() {
        emit_state(&app, "idle");
        return;
    }

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

    let settings = get_settings(&app);

    let Some(provider) = settings.active_assistant_provider().cloned() else {
        let _ = app.emit(
            "assistant-error",
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
        let _ = app.emit(
            "assistant-error",
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

    // Build the request: stable system prompt → history → new user msg.
    // (Cache-friendly: the prefix only ever grows by appending.) History is
    // capped (newest first wins) so request bodies stay small — critical for
    // Azure, whose parser rejects oversized payloads. Screenshot turns get a
    // much tighter cap: the image already dominates the body budget.
    let (max_history_messages, max_history_chars) = if screenshot.is_some() {
        (4usize, 6_000usize)
    } else {
        (12usize, 24_000usize)
    };
    let mut messages: Vec<Value> = Vec::new();
    messages.push(json!({
        "role": "system",
        "content": settings.assistant_system_prompt,
    }));
    {
        let conversation = app.state::<AssistantConversation>();
        let history = conversation.messages.lock().unwrap();
        let mut kept: Vec<&ChatMessage> = Vec::new();
        let mut chars = 0usize;
        for message in history.iter().rev().take(max_history_messages) {
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
    match &screenshot {
        Some(data_url) => messages.push(json!({
            "role": "user",
            "content": [
                {"type": "text", "text": user_text},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        })),
        None => messages.push(json!({"role": "user", "content": user_text})),
    }

    // Record the user message (text marker instead of raw image data) and
    // show it in the panel immediately.
    {
        let conversation = app.state::<AssistantConversation>();
        let mut history = conversation.messages.lock().unwrap();
        let stored = if screenshot.is_some() {
            format!("{}\n\n{}", user_text, SCREENSHOT_MARKER)
        } else {
            user_text.clone()
        };
        history.push(ChatMessage {
            role: "user".to_string(),
            content: stored,
        });
    }
    emit_conversation(&app);
    emit_state(&app, "thinking");

    debug!(
        "Assistant turn: provider '{}', model '{}', {} messages, screenshot: {}",
        provider.id,
        model,
        messages.len(),
        screenshot.is_some()
    );

    let app_for_tokens = app.clone();
    let result =
        llm_client::send_chat_stream(&provider, api_key.clone(), &model, messages, move |token| {
            let _ = app_for_tokens.emit("assistant-token", token.to_string());
        })
        .await;

    match result {
        Ok(full_text) => {
            {
                let conversation = app.state::<AssistantConversation>();
                let mut history = conversation.messages.lock().unwrap();
                history.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: full_text.clone(),
                });
            }
            emit_conversation(&app);

            if settings.assistant_tts_enabled {
                spawn_tts_summary(&app, &settings, &provider, api_key, &model, full_text);
            }
        }
        Err(e) => {
            error!("Assistant request failed: {}", e);
            let message = if e.contains("Unterminated string") && screenshot.is_some() {
                "The request was cut off by the provider — the screenshot made it too large for this endpoint. It will be compressed harder next time; please try again.".to_string()
            } else if screenshot.is_some() {
                format!(
                    "{}\n\nNote: a screenshot was attached — make sure the selected model supports image input (e.g. gpt-4o-mini, gpt-4.1-mini, gemini-flash, claude, llava).",
                    e
                )
            } else {
                e
            };
            let _ = app.emit("assistant-error", message);
        }
    }

    emit_state(&app, "idle");
}

/// Ask the model for a brief spoken summary of its reply and route it to the
/// configured TTS engine. Fire-and-forget.
fn spawn_tts_summary(
    app: &AppHandle,
    settings: &crate::settings::AppSettings,
    provider: &crate::settings::PostProcessProvider,
    api_key: String,
    model: &str,
    full_text: String,
) {
    let app = app.clone();
    let provider = provider.clone();
    let model = model.to_string();
    let tts_prompt = settings.assistant_tts_prompt.clone();
    let settings = settings.clone();

    tauri::async_runtime::spawn(async move {
        match llm_client::send_chat_completion_with_schema(
            &provider,
            api_key,
            &model,
            full_text,
            Some(tts_prompt),
            None,
            None,
            None,
        )
        .await
        {
            Ok(Some(summary)) if !summary.trim().is_empty() => {
                let summary = summary.trim().to_string();
                debug!(
                    "TTS summary ({}): {}",
                    settings.assistant_tts_engine, summary
                );
                if settings.assistant_tts_engine == "kokoro" {
                    // Local engine lives in the panel webview (kokoro-js).
                    let _ = app.emit("assistant-tts", summary);
                } else {
                    crate::tts::speak_remote(&app, &settings, summary).await;
                }
            }
            Ok(_) => debug!("TTS summary request returned no content"),
            Err(e) => debug!("TTS summary request failed: {}", e),
        }
    });
}
