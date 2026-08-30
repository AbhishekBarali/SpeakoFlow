use crate::huggingface::{self, HfModelSummary, HfRepoFiles};
use crate::managers::model::{ModelInfo, ModelManager, DOWNLOAD_CANCELLED_ERROR};
use crate::managers::transcription::{ModelStateEvent, TranscriptionManager};
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
#[specta::specta]
pub async fn get_available_models(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<Vec<ModelInfo>, String> {
    Ok(model_manager.get_available_models())
}

#[tauri::command]
#[specta::specta]
pub async fn get_model_info(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<Option<ModelInfo>, String> {
    Ok(model_manager.get_model_info(&model_id))
}

#[tauri::command]
#[specta::specta]
pub async fn download_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<(), String> {
    let result = model_manager.download_model(&model_id).await;

    if let Err(ref error) = result {
        if error.to_string() != DOWNLOAD_CANCELLED_ERROR {
            let _ = app_handle.emit(
                "model-download-failed",
                serde_json::json!({ "model_id": &model_id, "error": error.to_string() }),
            );
        }
    }

    result.map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    model_id: String,
) -> Result<(), String> {
    // Never leave the built-in assistant pointing at a deleted local model.
    // Users must switch its selection first; this also protects custom models
    // whose catalog record would otherwise disappear entirely.
    let settings = get_settings(&app_handle);
    if settings
        .assistant_models
        .get(crate::settings::BUILTIN_POST_PROCESS_PROVIDER_ID)
        .is_some_and(|active_id| active_id == &model_id)
    {
        return Err(
            "Switch the built-in assistant to another model before deleting this one.".to_string(),
        );
    }
    // The same protection for AI cleanup, which has its own slot pointing at the
    // same built-in engine. Without this the two are asymmetric: deleting the
    // cleanup model from the assistant's catalog (or the Models page, where it is
    // just one more language model) succeeded and left cleanup pointing at a file
    // that no longer exists — which shows up later as dictation silently pasting
    // the raw transcript, a symptom with no visible connection to the deletion.
    if settings
        .post_process_models
        .get(crate::settings::BUILTIN_POST_PROCESS_PROVIDER_ID)
        .is_some_and(|active_id| active_id == &model_id)
    {
        return Err("Switch AI cleanup to another model before deleting this one.".to_string());
    }

    // If deleting the active transcription model, unload it and clear the setting.
    if settings.selected_model == model_id {
        transcription_manager
            .unload_model()
            .map_err(|e| format!("Failed to unload model: {}", e))?;

        let mut settings = get_settings(&app_handle);
        settings.selected_model = String::new();
        write_settings(&app_handle, settings);
    }

    model_manager
        .delete_model(&model_id)
        .map_err(|e| e.to_string())
}

/// Shared logic for switching the active model, used by both the Tauri command
/// and the tray menu handler.
///
/// Validates the model, updates the persisted setting, and loads the model
/// unless the unload timeout is set to "Immediately" (in which case the model
/// will be loaded on-demand during the next transcription).
pub fn switch_active_model(app: &AppHandle, model_id: &str) -> Result<(), String> {
    let model_manager = app.state::<Arc<ModelManager>>();
    let transcription_manager = app.state::<Arc<TranscriptionManager>>();

    // Atomically claim the loading slot — prevents concurrent model loads
    // from tray double-clicks or overlapping commands. The guard resets the
    // flag on drop (including early returns, errors, and panics).
    let _loading_guard = transcription_manager
        .try_start_loading()
        .ok_or_else(|| "Model load already in progress".to_string())?;

    // Check if model exists and is available
    let model_info = model_manager
        .get_model_info(model_id)
        .ok_or_else(|| format!("Model not found: {}", model_id))?;

    // Only transcription models can be the active recording model. LLM/TTS
    // models share the catalog but are managed independently.
    if !model_info.engine_type.is_transcription() {
        return Err(format!(
            "Model '{}' is not a transcription model and cannot be set as the active model",
            model_id
        ));
    }

    if !model_info.is_downloaded {
        return Err(format!("Model not downloaded: {}", model_id));
    }

    let settings = get_settings(app);
    let unload_timeout = settings.model_unload_timeout;
    let old_model = settings.selected_model.clone();

    // Persist the new selection early so the frontend sees the correct model
    // when it reacts to events emitted by load_model.
    let mut settings = settings;
    settings.selected_model = model_id.to_string();

    // Reset language to auto if the new model doesn't support the currently selected language.
    // This prevents stale language settings from causing errors (e.g. Canary receiving zh-Hans)
    // and stops downstream processing (e.g. OpenCC) from running on an irrelevant language.
    if settings.selected_language != "auto"
        && !model_info.supported_languages.is_empty()
        && !model_info
            .supported_languages
            .contains(&settings.selected_language)
    {
        log::info!(
            "Resetting language from '{}' to 'auto' (not supported by {})",
            settings.selected_language,
            model_id
        );
        settings.selected_language = "auto".to_string();
    }

    write_settings(app, settings);

    // Skip eager loading if unload is set to "Immediately" — the model
    // will be loaded on-demand during the next transcription.
    if unload_timeout == ModelUnloadTimeout::Immediately {
        // Notify frontend — load_model won't be called so no events
        // would otherwise be emitted.
        let _ = app.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "selection_changed".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: Some(model_info.name.clone()),
                error: None,
            },
        );
        log::info!(
            "Model selection changed to {} (not loading — unload set to Immediately).",
            model_id
        );
        return Ok(());
    }

    // Load the model. On failure, revert the persisted selection.
    if let Err(e) = transcription_manager.load_model(model_id) {
        let mut settings = get_settings(app);
        settings.selected_model = old_model;
        write_settings(app, settings);
        return Err(e.to_string());
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_active_model(
    app_handle: AppHandle,
    _model_manager: State<'_, Arc<ModelManager>>,
    _transcription_manager: State<'_, Arc<TranscriptionManager>>,
    model_id: String,
) -> Result<(), String> {
    switch_active_model(&app_handle, &model_id)
}

#[tauri::command]
#[specta::specta]
pub async fn get_current_model(app_handle: AppHandle) -> Result<String, String> {
    let settings = get_settings(&app_handle);
    Ok(settings.selected_model)
}

#[tauri::command]
#[specta::specta]
pub async fn get_transcription_model_status(
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
) -> Result<Option<String>, String> {
    Ok(transcription_manager.get_current_model())
}

#[tauri::command]
#[specta::specta]
pub async fn is_model_loading(
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
) -> Result<bool, String> {
    // Check if transcription manager has a loaded model
    let current_model = transcription_manager.get_current_model();
    Ok(current_model.is_none())
}

#[tauri::command]
#[specta::specta]
pub async fn has_any_models_available(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<bool, String> {
    let models = model_manager.get_available_models();
    // Onboarding/first-run is about transcription models only; LLM and TTS
    // entries (e.g. always-available Kokoro) must not count here.
    Ok(models
        .iter()
        .any(|m| m.is_downloaded && m.engine_type.is_transcription()))
}

#[tauri::command]
#[specta::specta]
pub async fn has_any_models_or_downloads(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<bool, String> {
    let models = model_manager.get_available_models();
    // Return true if any transcription models are downloaded
    Ok(models
        .iter()
        .any(|m| m.is_downloaded && m.engine_type.is_transcription()))
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_download(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<(), String> {
    model_manager
        .cancel_download(&model_id)
        .map_err(|e| e.to_string())
}

/// Search the Hugging Face Hub for GGUF language-model repositories.
///
/// An empty query returns the most-downloaded GGUF repos as a default browse
/// list. Used by the "Add custom model" flow in the Models tab.
#[tauri::command]
#[specta::specta]
pub async fn search_huggingface_models(query: String) -> Result<Vec<HfModelSummary>, String> {
    huggingface::search_gguf_models(&query).await
}

/// List the downloadable `.gguf` files (model weights and vision projectors)
/// in a Hugging Face repo, with sizes, so the user can pick a quantization.
#[tauri::command]
#[specta::specta]
pub async fn list_huggingface_gguf_files(repo_id: String) -> Result<HfRepoFiles, String> {
    huggingface::list_repo_gguf_files(&repo_id).await
}

/// Register a user-chosen GGUF model from the Hub as a custom local LLM and
/// persist it. The frontend then downloads it via the normal `download_model`
/// command. `mmproj_filename` (optional) is the repo's vision projector.
#[tauri::command]
#[specta::specta]
pub async fn add_custom_llm_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    repo_id: String,
    filename: String,
    size_mb: u64,
    mmproj_filename: Option<String>,
) -> Result<ModelInfo, String> {
    let info = model_manager
        .add_custom_llm_model(&repo_id, &filename, size_mb, mmproj_filename)
        .map_err(|e| e.to_string())?;

    // Refresh any open Models views so the new entry appears immediately.
    let _ = app_handle.emit("model-state-changed", ());

    Ok(info)
}

// ---------------------------------------------------------------------
// Models the user already has on disk
// ---------------------------------------------------------------------

/// One file that couldn't be registered, and why.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LocalModelFailure {
    pub path: String,
    /// User-facing explanation, e.g. that the file is a vision projector rather
    /// than a model in its own right.
    pub message: String,
}

/// Result of registering a batch of picked files.
///
/// Deliberately not a plain `Result`: the picker is multi-select, and one
/// unusable file among five shouldn't discard the other four. The UI reports
/// what landed and what didn't.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LocalModelImport {
    pub added: Vec<ModelInfo>,
    pub failed: Vec<LocalModelFailure>,
}

/// Register models the user already has on disk, from paths they picked.
///
/// Nothing is copied or moved — each entry points at the file where it lives.
#[tauri::command]
#[specta::specta]
pub async fn add_local_models(
    model_manager: State<'_, Arc<ModelManager>>,
    paths: Vec<String>,
) -> Result<LocalModelImport, String> {
    if paths.is_empty() {
        return Err("No files were selected".to_string());
    }

    let manager = model_manager.inner().clone();
    // Reading a GGUF header per file is blocking I/O, and a batch of them off an
    // external drive is slow enough to matter, so keep it off the async runtime.
    tauri::async_runtime::spawn_blocking(move || {
        let mut added = Vec::new();
        let mut failed = Vec::new();
        for path in paths {
            match manager.add_local_model_file(&path) {
                Ok(info) => added.push(info),
                Err(e) => failed.push(LocalModelFailure {
                    path,
                    message: e.to_string(),
                }),
            }
        }
        LocalModelImport { added, failed }
    })
    .await
    .map_err(|e| format!("Failed to add local models: {}", e))
}

/// Link a folder of existing models and scan it. Returns how many were found.
#[tauri::command]
#[specta::specta]
pub async fn add_model_folder(
    model_manager: State<'_, Arc<ModelManager>>,
    path: String,
) -> Result<u32, String> {
    let manager = model_manager.inner().clone();
    // A linked folder can be large; scanning it blocks.
    tauri::async_runtime::spawn_blocking(move || manager.add_model_folder(&path))
        .await
        .map_err(|e| format!("Failed to scan folder: {}", e))?
        .map(|found| found as u32)
        .map_err(|e| e.to_string())
}

/// Unlink a folder. Its models leave the catalog; the files stay where they are.
#[tauri::command]
#[specta::specta]
pub async fn remove_model_folder(
    model_manager: State<'_, Arc<ModelManager>>,
    path: String,
) -> Result<(), String> {
    let manager = model_manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.remove_model_folder(&path))
        .await
        .map_err(|e| format!("Failed to unlink folder: {}", e))?
        .map_err(|e| e.to_string())
}

/// The currently linked model folders, in the order they were added.
#[tauri::command]
#[specta::specta]
pub async fn get_model_folders(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<Vec<String>, String> {
    Ok(model_manager.model_folders())
}

/// Re-scan every linked folder and re-check every registered local file.
///
/// The manual counterpart to the scan that runs at startup, for when the user
/// has just added a model to a linked folder or reconnected a drive. Returns the
/// total number of local models now registered.
#[tauri::command]
#[specta::specta]
pub async fn rescan_local_models(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<u32, String> {
    let manager = model_manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.refresh_local_models())
        .await
        .map_err(|e| format!("Failed to rescan: {}", e))?
        .map(|count| count as u32)
        .map_err(|e| e.to_string())
}
