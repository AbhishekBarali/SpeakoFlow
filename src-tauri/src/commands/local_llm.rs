//! Tauri commands for the built-in local LLM engine.

use crate::managers::local_llm::{LocalLlmManager, LocalLlmStatus};
use std::sync::Arc;
use tauri::State;

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
