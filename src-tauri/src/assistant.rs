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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};
use tauri_plugin_store::StoreExt;
use tokio::sync::Notify;

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
        // Collapse whitespace and strip the screenshot marker, then bound length.
        let text: String = message
            .content
            .replace(SCREENSHOT_MARKER, "")
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

    // Fresh turn: clear any leftover cancel signal from a previous Stop.
    app.state::<AssistantConversation>().begin_turn();

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

    // Record the user message (text marker instead of raw image data) and show
    // it in the panel immediately — before any web search runs, so the bubble
    // appears right away while results are being fetched.
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
    // Save right after the user message so the question is preserved even if
    // the model (or the search) errors out before replying.
    persist_assistant_session(&app);

    // Optional web search. Runs only when enabled and this isn't a screenshot
    // turn (those are about the screen, not the web). A capable model plans the
    // search — deciding whether one is actually needed and rewriting the (often
    // messy, transcribed) request into clean queries — and then we fetch real
    // page content to ground the answer. Any failure or timeout degrades
    // gracefully: we answer without web context rather than breaking the turn.
    // The cheap `should_search` pre-gate skips obvious non-search turns so we
    // don't spend a planner round-trip on chit-chat, code, or math.
    let web_context: Option<String> = if settings.assistant_web_search_enabled
        && screenshot.is_none()
        && web_search::should_search(&user_text)
    {
        emit_state(&app, "searching");
        // Race every search stage against a Stop press: a slow search must never
        // trap the user in the "searching" state with no way out.
        let cancel = app.state::<AssistantConversation>().cancel.clone();

        // Stage 1 — plan. The built-in local model skips planning (it's small and
        // its engine may be cold) and searches the raw question; cloud/"reasonable"
        // models do the full decide-and-rewrite step. `None` here means "skip the
        // search": either cancelled, or the model judged a search unnecessary.
        let plan_opt: Option<web_search::SearchPlan> = if provider.id == "builtin" {
            Some(web_search::SearchPlan::raw(&user_text))
        } else {
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
                    warn!("Search planner failed ({}); searching the raw question", e);
                    Some(web_search::SearchPlan::raw(&user_text))
                }
                None => None, // cancelled during planning
            }
        };

        // Stage 2 — retrieve, when the plan calls for it.
        match plan_opt {
            Some(plan) if plan.needs_search && !plan.queries.is_empty() => {
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
                        Some(web_search::format_results_for_prompt(&results))
                    }
                    Some(_) => {
                        debug!("Web search returned no results; answering without web context");
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
    // Azure, whose parser rejects oversized payloads. Screenshot turns get a
    // much tighter cap: the image already dominates the body budget.
    let (max_history_messages, max_history_chars) = if screenshot.is_some() {
        // Screenshot turns keep a tight cap regardless of the user setting:
        // the image already dominates the payload budget.
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
        // Append the user's response-length preference (if any) so a single
        // system prompt covers both display and spoken output.
        if let Some(directive) = settings.assistant_response_length.directive() {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(directive);
        }
        // On web-search turns, tell the model to ground its answer in (and
        // cite) the results that are prepended to the user's message below.
        if web_context.is_some() {
            if !content.trim().is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(web_search::WEB_SEARCH_SYSTEM_DIRECTIVE);
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
    // (never stored in history): the live date/time always, plus web results
    // when present. Keeping this in the user message rather than the cached
    // system prefix means the timestamp stays fresh every turn without
    // invalidating provider-side prompt caching.
    let mut preamble = current_datetime_line();
    if let Some(ctx) = &web_context {
        preamble.push_str("\n\n");
        preamble.push_str(ctx);
    }
    let user_content = format!("{}\n\n{}", preamble, user_text);

    match &screenshot {
        Some(data_url) => messages.push(json!({
            "role": "user",
            "content": [
                {"type": "text", "text": user_content},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        })),
        None => {
            messages.push(json!({"role": "user", "content": user_content}));
        }
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
            let _ = app.emit(
                "assistant-error",
                format!("Built-in model couldn't start: {}", e),
            );
            emit_state(&app, "idle");
            return;
        }
        Some(manager.begin_request())
    } else {
        None
    };

    debug!(
        "Assistant turn: provider '{}', model '{}', {} messages, screenshot: {}",
        provider.id,
        model,
        messages.len(),
        screenshot.is_some()
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
            }
        }
        Some(Err(e)) => {
            error!("Assistant request failed: {}", e);
            let message = if e.contains("Unterminated string") && screenshot.is_some() {
                "The request was cut off by the provider — the screenshot made it too large for this endpoint. It will be compressed harder next time; please try again.".to_string()
            } else if screenshot.is_some() && is_vision_unsupported_error(&e) {
                vision_unsupported_message(&provider.id, &model)
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
