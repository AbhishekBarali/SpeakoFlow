//! Tauri commands for the assistant panel and assistant settings.

use crate::assistant::{self, AssistantConversation, FileAttachment};
use crate::llm_client::ChatMessage;
use crate::settings::{
    assistant_provider_is_supported, get_settings, write_settings, AssistantCharacter,
    AssistantScreenAccessMode,
};
use tauri::{AppHandle, Manager};

fn require_manual_screen_access(mode: AssistantScreenAccessMode) -> Result<(), String> {
    if assistant::manual_screen_access_allowed(mode) {
        Ok(())
    } else {
        Err("Manual screen capture is unavailable in the current screen access mode".to_string())
    }
}

fn legacy_screen_access_mode(enabled: bool) -> AssistantScreenAccessMode {
    if enabled {
        AssistantScreenAccessMode::Manual
    } else {
        AssistantScreenAccessMode::Off
    }
}

/// Send a typed message to the assistant (keyboard alternative to voice).
#[tauri::command]
#[specta::specta]
pub async fn assistant_send_text(app: AppHandle, text: String) -> Result<(), String> {
    assistant::run_assistant_turn(app, text, None, Vec::new(), Vec::new(), None).await;
    Ok(())
}

/// Send a typed message with everything the composer collected: attached
/// images (data URLs, already downscaled), text-like files, and — when screen
/// vision is armed — a fresh screenshot. A capture failure surfaces as an
/// error but doesn't sink the turn (it proceeds without the screen).
#[tauri::command]
#[specta::specta]
pub async fn assistant_send_composed(
    app: AppHandle,
    text: String,
    images: Vec<String>,
    files: Vec<FileAttachment>,
    include_screen: bool,
) -> Result<(), String> {
    let settings = get_settings(&app);
    let manual_screen_token = if include_screen {
        require_manual_screen_access(settings.assistant_screen_access_mode)?;
        Some(assistant::authorize_manual_screen_operation(&app)?)
    } else {
        None
    };
    let include_screen = assistant::manual_composed_capture_allowed(
        include_screen,
        settings.assistant_screen_access_mode,
        settings.active_character_is_cat(),
    );
    let screenshot = if include_screen {
        // Tiny body only for Azure's gateway; loopback (built-in/local engine)
        // gets a balanced image, cloud gets the sharp one.
        let profile = settings
            .active_assistant_provider()
            .map(|p| crate::screenshot::CaptureProfile::for_base_url(&p.base_url))
            .unwrap_or(crate::screenshot::CaptureProfile::Generous);
        // Capture the monitor the mouse cursor is on (falls back to primary),
        // so multi-monitor users get the screen they're actually working on.
        match tauri::async_runtime::spawn_blocking(move || {
            crate::screenshot::capture_screen_data_url_at(None, profile)
        })
        .await
        {
            Ok(Ok(url)) => Some(url),
            Ok(Err(e)) => {
                assistant::emit_error(&app, "screen_capture", e);
                None
            }
            Err(e) => {
                assistant::emit_error(&app, "screen_capture", e.to_string());
                None
            }
        }
    } else {
        None
    };
    let manual_screen_token = screenshot.as_ref().and(manual_screen_token);
    assistant::run_assistant_turn(app, text, screenshot, images, files, manual_screen_token).await;
    Ok(())
}

/// Read a text-like file (code, markdown, logs, csv…) for attachment as
/// assistant context. Rejects binaries and (for now) PDFs with a clear error.
#[tauri::command]
#[specta::specta]
pub fn assistant_read_file(path: String) -> Result<FileAttachment, String> {
    const MAX_BYTES: u64 = 5 * 1024 * 1024;
    const MAX_CHARS: usize = 20_000;

    let meta = std::fs::metadata(&path).map_err(|e| format!("Can't read file: {}", e))?;
    if meta.len() > MAX_BYTES {
        return Err("File is too large to attach (over 5 MB)".to_string());
    }
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let ext = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "pdf" {
        return Err(
            "PDF text extraction isn't supported yet — export it as text or markdown first."
                .to_string(),
        );
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("Can't read file: {}", e))?;
    // Reject obvious binaries: NUL bytes in the first chunk.
    if bytes.iter().take(4096).any(|&b| b == 0) {
        return Err(format!(
            "'{}' looks like a binary file — attach text, code, or an image instead.",
            name
        ));
    }
    let text = String::from_utf8_lossy(&bytes);
    let content: String = text.chars().take(MAX_CHARS).collect();
    Ok(FileAttachment { name, content })
}

/// Load an image file from disk as a provider-ready data URL (downscaled).
#[tauri::command]
#[specta::specta]
pub async fn assistant_read_image(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || crate::screenshot::image_file_to_data_url(&path))
        .await
        .map_err(|e| e.to_string())?
}

/// Start the draw-a-box region screenshot flow: freeze the screen (off the
/// main thread), then open the selection overlay on the cursor's monitor.
/// Async on purpose — async commands run on a worker thread, from which Tauri
/// can create windows safely; doing it inline on the main thread inside a
/// sync command deadlocks/crashes WebView2 on Windows.
#[tauri::command]
#[specta::specta]
pub async fn assistant_begin_region_snip(app: AppHandle) -> Result<(), String> {
    let authorization = assistant::begin_region_snip(&app)?;

    // Pick the monitor under the cursor from Tauri's monitor list (the same
    // multi-monitor-safe detector the recording overlay uses), capture THAT
    // monitor, then open the overlay over it. Capturing the chosen monitor
    // (rather than letting the capture pick its own) guarantees the frozen
    // frame and the selection overlay line up on multi-monitor setups.
    let monitor = crate::overlay::get_monitor_with_cursor(&app)
        .ok_or_else(|| "No monitor available for region snip".to_string())?;
    let center_x = monitor.position().x + (monitor.size().width as i32) / 2;
    let center_y = monitor.position().y + (monitor.size().height as i32) / 2;
    let frame = tauri::async_runtime::spawn_blocking(move || {
        crate::screenshot::capture_monitor_at(center_x, center_y)
    })
    .await
    .map_err(|e| e.to_string())??;
    assistant::open_snip_overlay(&app, authorization, frame, monitor)
}

/// Rectangle chosen in the snip overlay, in that window's logical pixels.
#[derive(serde::Deserialize, specta::Type)]
pub struct SnipRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Finish (or cancel, with `rect: None`) the region snip. Called by the snip
/// overlay webview; the cropped image reaches the panel via the
/// `assistant-region-captured` event.
#[tauri::command]
#[specta::specta]
pub fn assistant_finish_region_snip(app: AppHandle, rect: Option<SnipRect>) -> Result<(), String> {
    require_manual_screen_access(get_settings(&app).assistant_screen_access_mode)?;
    assistant::authorize_manual_screen_operation(&app)?;
    assistant::finish_region_snip(&app, rect.map(|r| (r.x, r.y, r.width, r.height)));
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn assistant_get_conversation(app: AppHandle) -> Result<Vec<ChatMessage>, String> {
    let conversation = app.state::<AssistantConversation>();
    let history = conversation
        .messages
        .lock()
        .map_err(|e| format!("Conversation lock poisoned: {}", e))?;
    Ok(history.clone())
}

/// Regenerate the latest answer (re-runs the last user message). The previous
/// variant stays saved in History under its old row — regenerating forks a new
/// one — so earlier answers remain reachable.
#[tauri::command]
#[specta::specta]
pub async fn assistant_regenerate(app: AppHandle) -> Result<(), String> {
    assistant::regenerate_last(app).await;
    Ok(())
}

/// Compact the conversation into a summary that replaces the transcript
/// (the panel's `/summarize` command).
#[tauri::command]
#[specta::specta]
pub async fn assistant_summarize(app: AppHandle) -> Result<(), String> {
    assistant::run_summarize_turn(app).await;
    Ok(())
}

/// Load a past conversation from History into the panel and open it, so the
/// user can continue where they left off. Future turns update that same row.
#[tauri::command]
#[specta::specta]
pub fn assistant_resume_session(app: AppHandle, id: i64) -> Result<(), String> {
    let conversation = app.state::<AssistantConversation>();
    if conversation.is_busy() {
        return Err("The assistant is answering right now — stop it first.".to_string());
    }
    let hm = app
        .try_state::<std::sync::Arc<crate::managers::history::HistoryManager>>()
        .ok_or_else(|| "History unavailable".to_string())?;
    let entry = hm
        .get_assistant_session(id)
        .map_err(|e| format!("Couldn't load the conversation: {}", e))?
        .ok_or_else(|| "That conversation no longer exists.".to_string())?;

    conversation.load_session(entry.id, entry.messages);
    assistant::emit_conversation(&app);
    // Open straight into the full panel — resuming is a reading/typing flow.
    assistant::set_panel_collapsed(&app, false);
    assistant::show_assistant_panel(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn assistant_clear_conversation(app: AppHandle) -> Result<(), String> {
    let conversation = app.state::<AssistantConversation>();
    // Learn from the conversation before wiping it — but only if there's new,
    // substantial content since the last pass. `take_distillable` enforces that
    // (and marks it), so clearing right after a close never double-distills.
    let snapshot = conversation.take_distillable();
    conversation
        .messages
        .lock()
        .map_err(|e| format!("Conversation lock poisoned: {}", e))?
        .clear();
    // Detach from the saved row and reset the distill marker for the fresh chat.
    conversation.reset_session();
    conversation.reset_distilled_marker();
    assistant::emit_conversation(&app);

    // Fire-and-forget distillation of the just-ended conversation, off the hot
    // path. `distill_and_store` re-checks the memory toggles before doing work.
    if let Some(messages) = snapshot {
        let app_for_memory = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::memory::distill_and_store(app_for_memory, messages).await;
        });
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn toggle_assistant_panel(app: AppHandle) -> Result<(), String> {
    assistant::toggle_assistant_panel(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn hide_assistant_panel(app: AppHandle) -> Result<(), String> {
    assistant::hide_assistant_panel(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_provider(app: AppHandle, provider_id: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    let provider = settings
        .post_process_providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| format!("Unknown provider: {}", provider_id))?;
    if !assistant_provider_is_supported(&provider.id) {
        return Err(format!(
            "Provider '{}' is not supported by the Assistant",
            provider.label
        ));
    }
    settings.assistant_provider_id = provider_id;
    write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_assistant_model_setting(
    app: AppHandle,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_models.insert(provider_id, model);
    write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_assistant_system_prompt_setting(
    app: AppHandle,
    prompt: String,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_system_prompt = prompt;
    write_settings(&app, settings);
    Ok(())
}

/// Notify the panel (a separate webview) that assistant settings changed.
fn emit_settings_changed(app: &AppHandle) {
    use tauri::Emitter;
    let _ = app.emit("assistant-settings-changed", ());
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_screen_access_mode(
    app: AppHandle,
    mode: AssistantScreenAccessMode,
) -> Result<(), String> {
    // Agent decides exposes a `capture_screen` tool to the model instead of
    // manual controls. The helper orders persistence and Manual token
    // invalidation atomically relative to arm, snip, Immediate, and
    // composed-screen authorization.
    assistant::apply_screen_access_mode(&app, mode)?;

    if mode != AssistantScreenAccessMode::Manual {
        assistant::emit_screen_armed(&app, false);
    }
    emit_settings_changed(&app);
    Ok(())
}

/// Compatibility command for older webviews/configuration callers. Enabling
/// always means Manual and can never preserve or enter Agent decides.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_screenshot_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    set_assistant_screen_access_mode(app, legacy_screen_access_mode(enabled))
}

/// Choose when a screen capture is taken for a voice turn: `Immediate` (the
/// moment you start asking) or `OnSend` (when the message actually sends).
#[tauri::command]
#[specta::specta]
pub fn set_assistant_vision_capture_timing(
    app: AppHandle,
    timing: crate::settings::VisionCaptureTiming,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_vision_capture_timing = timing;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    // Disabling should silence whatever is playing or being generated right
    // now, not just suppress future summaries.
    if !enabled {
        crate::tts::stop_remote();
    }
    let mut settings = get_settings(&app);
    settings.assistant_tts_enabled = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_voice(app: AppHandle, voice: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_tts_voice = voice;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_response_length(
    app: AppHandle,
    length: crate::settings::AssistantResponseLength,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_response_length = length;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_font_size(app: AppHandle, size: String) -> Result<(), String> {
    if !matches!(size.as_str(), "small" | "medium" | "large") {
        return Err(format!("Unknown font size: {}", size));
    }
    let mut settings = get_settings(&app);
    settings.assistant_font_size = size;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_engine(app: AppHandle, engine: String) -> Result<(), String> {
    if !matches!(
        engine.as_str(),
        "kokoro" | "openai" | "openrouter" | "elevenlabs" | "azure"
    ) {
        return Err(format!("Unknown TTS engine: {}", engine));
    }
    // Switching engine mid-playback should stop the current clip.
    crate::tts::stop_remote();
    let mut settings = get_settings(&app);
    settings.assistant_tts_engine = engine;
    // Load THIS engine's own saved endpoint / model / voice / key into the flat
    // fields (from the per-engine maps, falling back to the engine's defaults).
    // Each engine keeps its own settings, so switching no longer wipes them or
    // carries another engine's values (e.g. an OpenAI base URL / key) across.
    settings.sync_active_tts_fields();
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_base_url(app: AppHandle, base_url: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    let engine = settings.assistant_tts_engine.clone();
    settings
        .assistant_tts_base_urls
        .insert(engine, base_url.clone());
    settings.assistant_tts_base_url = base_url;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_api_key(app: AppHandle, api_key: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    let engine = settings.assistant_tts_engine.clone();
    settings
        .assistant_tts_api_keys
        .insert(engine, api_key.clone());
    settings.assistant_tts_api_key = crate::settings::SecretString(api_key);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_model(app: AppHandle, model: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    let engine = settings.assistant_tts_engine.clone();
    settings.assistant_tts_models.insert(engine, model.clone());
    settings.assistant_tts_model = model;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_remote_voice(app: AppHandle, voice: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    let engine = settings.assistant_tts_engine.clone();
    settings
        .assistant_tts_remote_voices
        .insert(engine, voice.clone());
    settings.assistant_tts_remote_voice = voice;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_kokoro_dtype(app: AppHandle, dtype: String) -> Result<(), String> {
    if !matches!(dtype.as_str(), "fp32" | "fp16" | "q8" | "q4" | "q4f16") {
        return Err(format!("Unknown Kokoro dtype: {}", dtype));
    }
    let mut settings = get_settings(&app);
    settings.assistant_tts_kokoro_dtype = dtype;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Playback speed multiplier for spoken summaries (0.25x–4x). Clamped to that
/// range so a stray manual entry can't request an unusable rate. The change
/// takes effect on the next spoken clip rather than interrupting the current
/// one.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_speed(app: AppHandle, speed: f64) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_tts_speed = speed.clamp(0.25, 4.0);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_panel_opacity(app: AppHandle, opacity: f64) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_panel_opacity = opacity.clamp(0.5, 1.0);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Set the expanded panel size preset ("compact", "standard", or "large") and
/// resize the live panel window to match when it's currently expanded.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_panel_size(app: AppHandle, size: String) -> Result<(), String> {
    if !matches!(size.as_str(), "compact" | "standard" | "large") {
        return Err(format!("Unknown panel size: {}", size));
    }
    let mut settings = get_settings(&app);
    settings.assistant_panel_size = size.clone();
    write_settings(&app, settings);
    assistant::apply_panel_size(&app, &size);
    emit_settings_changed(&app);
    Ok(())
}

/// Whether starting a dictation silences a still-playing assistant reply.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_stop_on_dictation(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_tts_stop_on_dictation = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Mirror the panel's staged attachment chips into the backend so voice turns
/// (pill mic / hotkey) send them too.
#[tauri::command]
#[specta::specta]
pub fn assistant_set_pending_attachments(
    images: Vec<String>,
    files: Vec<FileAttachment>,
) -> Result<(), String> {
    assistant::set_pending_attachments(images, files);
    Ok(())
}

/// Route the dictation currently being recorded to the assistant (the STT
/// overlay's Ask-Assistant button), then commit it like a normal finish.
#[tauri::command]
#[specta::specta]
pub fn redirect_transcription_to_assistant() -> Result<(), String> {
    assistant::set_transcribe_redirect();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_panel_collapsed(app: AppHandle, collapsed: bool) -> Result<(), String> {
    assistant::set_panel_collapsed(&app, collapsed);
    Ok(())
}

/// Current pill/expanded state of the assistant panel. The webview queries this
/// on mount so a fresh or reloaded panel renders the right layout instead of
/// showing the full panel header inside the collapsed pill window.
#[tauri::command]
#[specta::specta]
pub fn get_assistant_panel_collapsed() -> bool {
    assistant::is_panel_collapsed()
}

/// Arm or disarm sticky Manual screen capture. Disarming is always accepted for
/// cleanup; arming is rejected unless the persisted mode is Manual.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_screen_armed(app: AppHandle, armed: bool) -> Result<(), String> {
    assistant::set_screen_armed_for_current_mode(&app, armed)
}

/// Restore the session-only Manual arm after a panel webview reload.
#[tauri::command]
#[specta::specta]
pub fn get_assistant_screen_armed(app: AppHandle) -> bool {
    assistant::screen_armed_for_current_mode(&app)
}

/// Start/stop assistant voice recording programmatically (pill mic button).
/// Hands-free toggle: first call starts, second stops (a click can't "hold").
#[tauri::command]
#[specta::specta]
pub fn assistant_toggle_voice(app: AppHandle) -> Result<(), String> {
    let coordinator = app
        .try_state::<crate::TranscriptionCoordinator>()
        .ok_or_else(|| "Coordinator not initialized".to_string())?;
    coordinator.send_input(
        "assistant",
        "pill",
        true,
        crate::transcription_coordinator::RecordingMode::Lock,
    );
    Ok(())
}

/// Speak arbitrary text with the configured remote TTS engine (used by the
/// panel to test or replay summaries; the kokoro engine plays in-webview).
#[tauri::command]
#[specta::specta]
pub async fn assistant_speak(app: AppHandle, text: String) -> Result<(), String> {
    let settings = get_settings(&app);
    // Same cleanup the auto-summary path uses, so replayed/!test text never
    // reads out Markdown, code or emojis.
    let text = crate::tts::sanitize_for_speech(&text);
    if text.trim().is_empty() {
        return Ok(());
    }
    if settings.assistant_tts_engine == "kokoro" {
        use tauri::Emitter;
        let _ = app.emit("assistant-tts", text);
    } else {
        crate::tts::speak_remote(&app, &settings, text).await;
    }
    Ok(())
}

/// Synthesize and play a short sample with the configured remote TTS engine,
/// returning any error so the settings "Test voice" button can show it inline.
/// (The local kokoro engine is tested in-webview, not through this command.)
#[tauri::command]
#[specta::specta]
pub async fn assistant_test_tts(app: AppHandle, text: String) -> Result<(), String> {
    let settings = get_settings(&app);
    if settings.assistant_tts_engine == "kokoro" {
        return Err("Kokoro is tested locally in the browser, not via this command".to_string());
    }
    // Interrupt anything currently playing before the test clip.
    crate::tts::stop_remote();
    // The phrase is chosen at random on the frontend (a rotating set of fun
    // sample lines) so the spoken test matches the kokoro path. Fall back to a
    // sensible default if the caller passes nothing.
    let sample = if text.trim().is_empty() {
        "Hi! This is a test of SpeakoFlow's voice output.".to_string()
    } else {
        text
    };
    crate::tts::test_remote(&settings, sample).await
}

/// Fetch all available Azure Speech neural voices for the configured endpoint
/// and key, so the settings UI can offer a voice picker instead of guessing.
#[tauri::command]
#[specta::specta]
pub async fn assistant_list_azure_voices(
    app: AppHandle,
) -> Result<Vec<crate::tts::AzureVoice>, String> {
    let settings = get_settings(&app);
    crate::tts::list_azure_voices(&settings).await
}

/// List selectable voices for the currently-configured remote TTS engine
/// (OpenAI-compatible, ElevenLabs, or Azure), so the settings UI can offer a
/// searchable voice picker instead of a raw text field. Returns an error string
/// for inline display when the lookup fails (bad key, unreachable endpoint).
#[tauri::command]
#[specta::specta]
pub async fn assistant_list_tts_voices(
    app: AppHandle,
) -> Result<Vec<crate::tts::TtsVoice>, String> {
    let settings = get_settings(&app);
    crate::tts::list_tts_voices(&settings).await
}

/// List selectable models for the currently-configured remote TTS engine
/// (OpenAI-compatible `/models`, or ElevenLabs text-to-speech models). Azure and
/// Kokoro don't expose a model list and return an error the UI shows inline.
#[tauri::command]
#[specta::specta]
pub async fn assistant_list_tts_models(app: AppHandle) -> Result<Vec<String>, String> {
    let settings = get_settings(&app);
    crate::tts::list_tts_models(&settings).await
}

/// Stop the current assistant turn: cancels in-flight generation and silences
/// any spoken summary that is playing or about to play.
#[tauri::command]
#[specta::specta]
pub fn assistant_stop(app: AppHandle) -> Result<(), String> {
    crate::tts::stop_remote();
    use tauri::Emitter;
    // Tell the panel webview to stop local (Kokoro) playback too.
    let _ = app.emit("assistant-tts-stop", ());
    if let Some(conversation) = app.try_state::<assistant::AssistantConversation>() {
        conversation.request_cancel();
    }
    Ok(())
}

/// How many prior messages the model receives as conversation context.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_max_history_messages(app: AppHandle, count: u32) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_max_history_messages = count.min(200);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Toggle automatic conversation summarization: when on, long chats fold older
/// turns into a rolling summary instead of dropping them.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_auto_summarize(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_auto_summarize = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Web search
// ---------------------------------------------------------------------------

/// Enable or disable web search for the assistant. When enabled, a fast local
/// heuristic still decides per-question whether a search is actually run, so
/// casual chat stays instant.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_web_search_enabled = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Prefer the provider's OWN built-in web search (currently OpenRouter's
/// `:online`) over the app's search. Providers without native search always use
/// the app's search regardless of this flag.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_prefer_provider_web_search(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_prefer_provider_web_search = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Choose the search backend: "serper" (default), "brave", "tavily", "exa", or
/// "serpapi". All are snippet-only and use a single API key.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_provider(app: AppHandle, provider: String) -> Result<(), String> {
    if !matches!(
        provider.as_str(),
        "serper" | "brave" | "tavily" | "exa" | "serpapi"
    ) {
        return Err(format!("Unknown web search provider: {}", provider));
    }
    let mut settings = get_settings(&app);
    settings.assistant_web_search_provider = provider;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// How many results to feed the model (clamped to 1–10).
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_max_results(app: AppHandle, count: u32) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_web_search_max_results = count.clamp(1, 10);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Set how thorough web search is: "low" (fastest), "medium" (default), or
/// "high" (broadest single pass). This is the primary depth control.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_search_depth(
    app: AppHandle,
    depth: crate::settings::AssistantSearchDepth,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_search_depth = depth;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// DEPRECATED / no-op since web search became snippet-only (the Firecrawl
/// credit guard was removed). Still registered so existing bindings/settings
/// stay valid; it only writes the now-unused setting field.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_daily_credit_budget(
    app: AppHandle,
    budget: u32,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    // Clamp to a sane ceiling; 0 stays 0 (unlimited).
    settings.assistant_web_search_daily_credit_budget = budget.min(1_000_000);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Built-in local model only: toggle smart (LLM-planned) search decisions vs the
/// fast keyword heuristic. No effect on cloud/custom providers.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_local_search_smart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_local_search_smart = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// DEPRECATED / no-op since web search became snippet-only (page fetching was
/// removed). Still registered so existing bindings/settings stay valid; it only
/// writes the now-unused setting field.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_fetch_content(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_web_search_fetch_content = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Store the API key for a search provider ("serper", "brave", "tavily", "exa",
/// or "serpapi").
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_api_key(
    app: AppHandle,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    if !matches!(
        provider.as_str(),
        "serper" | "brave" | "tavily" | "exa" | "serpapi"
    ) {
        return Err(format!(
            "Provider '{}' does not use an API key for web search",
            provider
        ));
    }
    let mut settings = get_settings(&app);
    settings.web_search_api_keys.insert(provider, api_key);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Run a one-off web search with the current settings and return the results,
/// so the settings UI can offer a "Test search" button and surface any error
/// (missing key, rate limit) inline.
#[tauri::command]
#[specta::specta]
pub async fn assistant_test_web_search(
    app: AppHandle,
    query: String,
) -> Result<Vec<crate::web_search::SearchResult>, String> {
    let settings = get_settings(&app);
    crate::web_search::search(&settings, &query).await
}

// ---------------------------------------------------------------------------
// Characters (assistant personas)
// ---------------------------------------------------------------------------

/// Switch the active character. Errors if the id no longer exists.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_active_character(app: AppHandle, id: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    if !settings.assistant_characters.iter().any(|c| c.id == id) {
        return Err("That character no longer exists.".to_string());
    }
    settings.assistant_active_character_id = id;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Replace the whole character list. Add / edit / reorder / duplicate / delete
/// all funnel through here (like text replacements), which keeps the UI simple.
/// Enforces the invariants: the non-deletable `default` character must remain,
/// the list can't be empty, ids must be unique, and the active id must still
/// resolve. The `default` character's prompt is mirrored back into
/// `assistant_system_prompt` for backward compatibility.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_characters(
    app: AppHandle,
    characters: Vec<AssistantCharacter>,
) -> Result<(), String> {
    if characters.is_empty() {
        return Err("At least one character is required.".to_string());
    }
    if !characters.iter().any(|c| c.id == "default") {
        return Err("The default assistant character can't be removed.".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for c in &characters {
        if c.id.trim().is_empty() {
            return Err("A character is missing an id.".to_string());
        }
        if !seen.insert(c.id.clone()) {
            return Err(format!("Duplicate character id: {}", c.id));
        }
    }

    let mut settings = get_settings(&app);
    if !characters
        .iter()
        .any(|c| c.id == settings.assistant_active_character_id)
    {
        settings.assistant_active_character_id = "default".to_string();
    }
    // Keep the plain system prompt in sync with the default character so any
    // legacy reader (and the first-run migration seed) stays correct.
    if let Some(def) = characters.iter().find(|c| c.id == "default") {
        if !def.prompt.trim().is_empty() {
            settings.assistant_system_prompt = def.prompt.clone();
        }
    }
    settings.assistant_characters = characters;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Load an image file as a small avatar data URL (downscaled to 256px so it
/// stays compact inside the settings file).
#[tauri::command]
#[specta::specta]
pub async fn assistant_read_avatar(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::screenshot::image_file_to_avatar_data_url(&path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Import a character from a JSON file on disk (path chosen via the UI's file
/// dialog). The imported character always gets a fresh id and is never marked
/// built-in, so it can't clobber a built-in or the non-deletable default.
#[tauri::command]
#[specta::specta]
pub fn assistant_import_character(
    app: AppHandle,
    path: String,
) -> Result<AssistantCharacter, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("Couldn't read file: {}", e))?;
    let mut character: AssistantCharacter = serde_json::from_slice(&bytes)
        .map_err(|e| format!("That file isn't a valid character: {}", e))?;
    character.id = new_character_id();
    character.builtin = false;
    if character.name.trim().is_empty() {
        character.name = "Imported character".to_string();
    }
    let mut settings = get_settings(&app);
    settings.assistant_characters.push(character.clone());
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(character)
}

/// Export a single character to a JSON file on disk (path chosen via the UI's
/// save dialog).
#[tauri::command]
#[specta::specta]
pub fn assistant_export_character(app: AppHandle, id: String, path: String) -> Result<(), String> {
    let settings = get_settings(&app);
    let character = settings
        .assistant_characters
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "Character not found.".to_string())?;
    let json = serde_json::to_string_pretty(character).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Couldn't write file: {}", e))?;
    Ok(())
}

/// Reset a built-in persona to the version shipped with the app (its original
/// name, role, prompt, greeting, avatar, and reply length). Custom personas
/// have no shipped default, so this only works on built-in ids. This is the
/// "reload" for a built-in whose prompt/details you edited (or wiped) and want
/// back.
#[tauri::command]
#[specta::specta]
pub fn assistant_restore_builtin_character(
    app: AppHandle,
    id: String,
) -> Result<AssistantCharacter, String> {
    // Passing an empty prompt makes the canonical "default" persona fall back to
    // its shipped system prompt, so this restores the true factory version.
    let shipped = crate::settings::default_assistant_characters("");
    let canonical = shipped
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "That isn't a built-in persona.".to_string())?;

    let mut settings = get_settings(&app);
    let existing = settings
        .assistant_characters
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "That persona no longer exists.".to_string())?;
    *existing = canonical.clone();
    // Keep the plain system prompt in sync with the base assistant.
    if id == "default" {
        settings.assistant_system_prompt = canonical.prompt.clone();
    }
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(canonical)
}

/// Re-add any built-in personas the user deleted, leaving their custom personas
/// and their edits to still-present built-ins untouched. Returns how many were
/// restored (0 if none were missing).
#[tauri::command]
#[specta::specta]
pub fn assistant_restore_missing_builtins(app: AppHandle) -> Result<u32, String> {
    let shipped = crate::settings::default_assistant_characters("");
    let mut settings = get_settings(&app);
    let mut restored = 0u32;
    for canonical in shipped {
        if !settings
            .assistant_characters
            .iter()
            .any(|c| c.id == canonical.id)
        {
            settings.assistant_characters.push(canonical);
            restored += 1;
        }
    }
    if restored > 0 {
        write_settings(&app, settings);
        emit_settings_changed(&app);
    }
    Ok(restored)
}

/// A persona drafted by the model from a short description. Not persisted by
/// the backend — the UI shows it for review, then saves it via
/// `set_assistant_characters`.
#[derive(serde::Serialize, serde::Deserialize, specta::Type)]
pub struct GeneratedCharacter {
    pub name: String,
    pub prompt: String,
    pub greeting: String,
}

/// Draft a character from a short natural-language description using the
/// currently-configured assistant provider/model. Returns the drafted
/// name/prompt/greeting for the user to review and save.
#[tauri::command]
#[specta::specta]
pub async fn assistant_generate_character(
    app: AppHandle,
    description: String,
) -> Result<GeneratedCharacter, String> {
    let description = description.trim().to_string();
    if description.is_empty() {
        return Err("Describe the character you want first.".to_string());
    }

    let settings = get_settings(&app);
    let provider = settings
        .active_assistant_provider()
        .cloned()
        .ok_or_else(|| {
            "No assistant provider configured. Pick one in Settings → Assistant.".to_string()
        })?;
    let model = settings
        .assistant_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(format!(
            "No model configured for provider '{}'. Set one in Settings → Assistant.",
            provider.label
        ));
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    // The built-in local engine must be running before we can call it.
    if provider.id == "builtin" {
        let manager = app.state::<std::sync::Arc<crate::managers::local_llm::LocalLlmManager>>();
        manager
            .ensure_running(&model)
            .await
            .map_err(|e| e.to_string())?;
    }

    let system = "You design personas for a voice assistant. Given the user's description, invent a single character and respond with ONLY a JSON object (no prose, no markdown fences) with exactly these keys: \"name\" (a short display name, 2-24 characters), \"prompt\" (the system prompt for the persona, written in the second person — define its personality, tone, speaking style, and any quirks or constraints; it must stay genuinely helpful and must never be hateful, harassing, or target real people or protected groups), and \"greeting\" (a short in-character opening line, one sentence). Keep it tasteful and PG.".to_string();

    let schema = if provider.supports_structured_output {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "prompt": { "type": "string" },
                "greeting": { "type": "string" }
            },
            "required": ["name", "prompt", "greeting"],
            "additionalProperties": false
        }))
    } else {
        None
    };

    let raw = crate::llm_client::send_chat_completion_with_schema(
        &provider,
        api_key,
        &model,
        description,
        Some(system),
        schema,
        None,
        None,
    )
    .await?
    .ok_or_else(|| "The model returned no content.".to_string())?;

    parse_generated_character(&raw)
}

/// Parse the model's reply into a GeneratedCharacter, tolerating markdown
/// fences and surrounding prose by extracting the first `{...}` block.
fn parse_generated_character(raw: &str) -> Result<GeneratedCharacter, String> {
    #[derive(serde::Deserialize)]
    struct Draft {
        name: Option<String>,
        prompt: Option<String>,
        greeting: Option<String>,
    }

    let trimmed = raw.trim();
    let json_slice = match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(start), Some(end)) if end >= start => &trimmed[start..=end],
        _ => trimmed,
    };

    let draft: Draft = serde_json::from_str(json_slice)
        .map_err(|e| format!("Couldn't understand the model's reply: {}", e))?;

    let prompt = draft.prompt.unwrap_or_default().trim().to_string();
    if prompt.is_empty() {
        return Err("The model didn't return a persona prompt — try again.".to_string());
    }
    let name = draft.name.unwrap_or_default().trim().to_string();
    let name: String = if name.is_empty() {
        "New character".to_string()
    } else {
        name.chars().take(40).collect()
    };
    let greeting = draft.greeting.unwrap_or_default().trim().to_string();
    Ok(GeneratedCharacter {
        name,
        prompt,
        greeting,
    })
}

/// A reasonably-unique id for a new/imported character (avoids a uuid dep).
fn new_character_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("char-{}", nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_screenshot_setter_maps_only_to_off_or_manual() {
        assert_eq!(
            legacy_screen_access_mode(false),
            AssistantScreenAccessMode::Off
        );
        assert_eq!(
            legacy_screen_access_mode(true),
            AssistantScreenAccessMode::Manual
        );
    }

    #[test]
    fn direct_manual_screen_actions_reject_off_and_agent_modes() {
        assert!(require_manual_screen_access(AssistantScreenAccessMode::Manual).is_ok());
        assert!(require_manual_screen_access(AssistantScreenAccessMode::Off).is_err());
        assert!(require_manual_screen_access(AssistantScreenAccessMode::AgentDecides).is_err());
    }
}
