//! Tauri commands for the assistant panel and assistant settings.

use crate::assistant::{self, AssistantConversation};
use crate::llm_client::ChatMessage;
use crate::settings::{get_settings, write_settings};
use tauri::{AppHandle, Manager};

/// Send a typed message to the assistant (keyboard alternative to voice).
#[tauri::command]
#[specta::specta]
pub async fn assistant_send_text(app: AppHandle, text: String) -> Result<(), String> {
    assistant::run_assistant_turn(app, text, None).await;
    Ok(())
}

/// Send a typed message with a screenshot of the current screen attached.
#[tauri::command]
#[specta::specta]
pub async fn assistant_send_text_with_screen(app: AppHandle, text: String) -> Result<(), String> {
    let settings = get_settings(&app);
    let screenshot = if settings.assistant_screenshot_enabled {
        // Tiny body only for Azure's gateway; loopback (built-in/local engine)
        // gets a balanced image, cloud gets the sharp one.
        let profile = settings
            .active_assistant_provider()
            .map(|p| crate::screenshot::CaptureProfile::for_base_url(&p.base_url))
            .unwrap_or(crate::screenshot::CaptureProfile::Generous);
        match tauri::async_runtime::spawn_blocking(move || {
            crate::screenshot::capture_screen_data_url(profile)
        })
        .await
        {
            Ok(Ok(url)) => Some(url),
            Ok(Err(e)) => {
                use tauri::Emitter;
                let _ = app.emit("assistant-error", format!("Screen capture failed: {}", e));
                None
            }
            Err(e) => {
                use tauri::Emitter;
                let _ = app.emit("assistant-error", format!("Screen capture failed: {}", e));
                None
            }
        }
    } else {
        None
    };
    assistant::run_assistant_turn(app, text, screenshot).await;
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

#[tauri::command]
#[specta::specta]
pub fn assistant_clear_conversation(app: AppHandle) -> Result<(), String> {
    let conversation = app.state::<AssistantConversation>();
    conversation
        .messages
        .lock()
        .map_err(|e| format!("Conversation lock poisoned: {}", e))?
        .clear();
    // Detach from the saved row so the next turn starts a new conversation in
    // history rather than appending to the one the user just cleared.
    conversation.reset_session();
    assistant::emit_conversation(&app);
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
    if !settings
        .post_process_providers
        .iter()
        .any(|p| p.id == provider_id)
    {
        return Err(format!("Unknown provider: {}", provider_id));
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
pub fn set_assistant_screenshot_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_screenshot_enabled = enabled;
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
pub fn set_assistant_panel_opacity(app: AppHandle, opacity: f64) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_panel_opacity = opacity.clamp(0.5, 1.0);
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
pub fn set_assistant_accent(app: AppHandle, accent: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_accent = accent;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_engine(app: AppHandle, engine: String) -> Result<(), String> {
    if !matches!(
        engine.as_str(),
        "kokoro" | "openai" | "elevenlabs" | "azure"
    ) {
        return Err(format!("Unknown TTS engine: {}", engine));
    }
    // Switching engine mid-playback should stop the current clip.
    crate::tts::stop_remote();
    let mut settings = get_settings(&app);
    settings.assistant_tts_engine = engine;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_base_url(app: AppHandle, base_url: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_tts_base_url = base_url;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_api_key(app: AppHandle, api_key: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_tts_api_key = crate::settings::SecretString(api_key);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_model(app: AppHandle, model: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_tts_model = model;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_tts_remote_voice(app: AppHandle, voice: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
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
pub fn set_assistant_panel_size(app: AppHandle, size: String) -> Result<(), String> {
    if !matches!(size.as_str(), "compact" | "standard" | "large") {
        return Err(format!("Unknown panel size: {}", size));
    }
    let mut settings = get_settings(&app);
    settings.assistant_panel_size = size;
    write_settings(&app, settings);
    assistant::apply_panel_size(&app);
    emit_settings_changed(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_assistant_panel_theme(app: AppHandle, theme: String) -> Result<(), String> {
    if !matches!(theme.as_str(), "auto" | "light" | "dark") {
        return Err(format!("Unknown panel theme: {}", theme));
    }
    let mut settings = get_settings(&app);
    settings.assistant_panel_theme = theme;
    write_settings(&app, settings);
    emit_settings_changed(&app);
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

/// Arm (or disarm) a screenshot for the NEXT assistant turn — typed or
/// voice. One-shot: consumed by the next turn.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_screen_armed(app: AppHandle, armed: bool) -> Result<(), String> {
    assistant::set_screen_armed(&app, armed);
    Ok(())
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

/// Choose the search backend: "duckduckgo" (free, no key), "firecrawl", or
/// "brave".
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_provider(app: AppHandle, provider: String) -> Result<(), String> {
    if !matches!(provider.as_str(), "duckduckgo" | "firecrawl" | "brave") {
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

/// Set the daily Firecrawl credit budget for web search (0 = unlimited). A
/// safety rail so a session can't silently drain the user's Firecrawl plan; a
/// rolling per-minute request cap guards against runaway loops regardless.
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

/// Toggle fetching full page content for the top results (Firecrawl only).
/// Full content makes answers far more accurate; turning it off relies on short
/// snippets and saves Firecrawl credits.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_fetch_content(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_web_search_fetch_content = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Store the API key for a keyed search provider ("firecrawl" or "brave").
#[tauri::command]
#[specta::specta]
pub fn set_assistant_web_search_api_key(
    app: AppHandle,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    if !matches!(provider.as_str(), "firecrawl" | "brave") {
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
