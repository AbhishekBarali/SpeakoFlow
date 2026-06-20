//! Tauri commands for the built-in local LLM engine.

use crate::managers::local_llm::{
    LocalLlmManager, LocalLlmStatus, MAX_CONTEXT_SIZE, MIN_CONTEXT_SIZE,
};
use crate::settings::{get_settings, write_settings};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

/// Current status of the built-in LLM engine (running, which model, whether an
/// engine binary is present, etc.).
#[tauri::command]
#[specta::specta]
pub async fn get_local_llm_status(
    local_llm: State<'_, Arc<LocalLlmManager>>,
) -> Result<LocalLlmStatus, String> {
    Ok(local_llm.status())
}

/// Start (or switch) the built-in engine to serve `model_id`. Resolves once the
/// engine is accepting requests.
#[tauri::command]
#[specta::specta]
pub async fn start_local_llm(
    local_llm: State<'_, Arc<LocalLlmManager>>,
    model_id: String,
) -> Result<(), String> {
    local_llm.ensure_running(&model_id).await
}

/// Stop the built-in engine, freeing its memory.
#[tauri::command]
#[specta::specta]
pub async fn stop_local_llm(local_llm: State<'_, Arc<LocalLlmManager>>) -> Result<(), String> {
    local_llm.stop();
    Ok(())
}

/// Set the context window (in tokens) for the built-in local LLM engine.
///
/// The value is clamped to a safe range and only read when the engine starts,
/// so any running engine is stopped here; the next assistant or post-processing
/// turn restarts it with the new size. External providers (Ollama / LM Studio /
/// cloud) are unaffected — they manage their own context.
#[tauri::command]
#[specta::specta]
pub fn set_local_llm_context_size(
    app: AppHandle,
    local_llm: State<'_, Arc<LocalLlmManager>>,
    size: u32,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.local_llm_context_size = size.clamp(MIN_CONTEXT_SIZE, MAX_CONTEXT_SIZE);
    write_settings(&app, settings);
    // The size is applied at engine launch, so stop any running engine; the
    // next turn restarts it with the new context window.
    local_llm.stop();
    // Mirror the assistant settings commands so the panel webview refreshes.
    let _ = app.emit("assistant-settings-changed", ());
    Ok(())
}
