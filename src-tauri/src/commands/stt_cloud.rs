//! Tauri commands for the cloud speech-to-text settings section.
//!
//! Deliberately thin: the plain fields (which provider, which model, whether to
//! stream) are ordinary settings the frontend writes through the existing
//! `update_settings` path, so they need no commands here. What does need a
//! command is anything the frontend cannot do itself — talk to a provider, or
//! touch the OS keychain.

use tauri::{AppHandle, Manager};

use crate::settings::{
    get_settings, write_settings, CloudSttProvider, CloudSttReadiness, SttEngineMode,
};

/// The configured cloud transcription endpoints, for the provider picker.
#[tauri::command]
#[specta::specta]
pub fn get_cloud_stt_providers(app: AppHandle) -> Vec<CloudSttProvider> {
    get_settings(&app).cloud_stt_providers
}

/// Whether cloud transcription is ready, and if not, why.
#[tauri::command]
#[specta::specta]
pub fn get_cloud_stt_readiness(app: AppHandle) -> CloudSttReadiness {
    crate::stt_cloud::cloud_stt_readiness(&get_settings(&app))
}

/// Switch between the local engine and cloud transcription.
///
/// A command rather than a raw settings write because turning cloud *off* has a
/// side effect: the local model has not been loaded while the user was on cloud,
/// so it is asked to start loading now instead of on the first press of the
/// dictation key, where the wait would be visible.
#[tauri::command]
#[specta::specta]
pub fn set_stt_engine_mode(app: AppHandle, mode: SttEngineMode) {
    let mut settings = get_settings(&app);
    if settings.stt_engine_mode == mode {
        return;
    }
    settings.stt_engine_mode = mode;
    write_settings(&app, settings);

    if mode == SttEngineMode::Local {
        if let Some(manager) =
            app.try_state::<std::sync::Arc<crate::managers::transcription::TranscriptionManager>>()
        {
            manager.initiate_model_load();
        }
    }
}

/// Store a provider's API key. Goes through settings so the keychain write in
/// `write_settings` runs; the plaintext copy is blanked once the OS store
/// confirms it holds the value.
#[tauri::command]
#[specta::specta]
pub fn set_cloud_stt_api_key(app: AppHandle, provider_id: String, api_key: String) {
    let mut settings = get_settings(&app);
    settings
        .cloud_stt_api_keys
        .insert(provider_id, api_key.trim().to_string());
    write_settings(&app, settings);
}

/// Select the active cloud provider.
#[tauri::command]
#[specta::specta]
pub fn set_cloud_stt_provider(app: AppHandle, provider_id: String) {
    let mut settings = get_settings(&app);
    if !settings
        .cloud_stt_providers
        .iter()
        .any(|p| p.id == provider_id)
    {
        return;
    }
    settings.cloud_stt_provider_id = provider_id;
    write_settings(&app, settings);
}

/// Set the model for one provider.
///
/// Per-provider rather than a whole-map write so switching providers, or two
/// edits landing close together, cannot clobber the other provider's choice.
#[tauri::command]
#[specta::specta]
pub fn set_cloud_stt_model(app: AppHandle, provider_id: String, model: String) {
    let mut settings = get_settings(&app);
    settings
        .cloud_stt_models
        .insert(provider_id, model.trim().to_string());
    write_settings(&app, settings);
}

/// Set the endpoint override for one provider. An empty value clears the
/// override, restoring the shipped base URL.
#[tauri::command]
#[specta::specta]
pub fn set_cloud_stt_base_url(app: AppHandle, provider_id: String, base_url: String) {
    let mut settings = get_settings(&app);
    let trimmed = base_url.trim().to_string();
    if trimmed.is_empty() {
        settings.cloud_stt_base_urls.remove(&provider_id);
    } else {
        settings.cloud_stt_base_urls.insert(provider_id, trimmed);
    }
    write_settings(&app, settings);
}

/// Use the provider's realtime endpoint where it has one.
#[tauri::command]
#[specta::specta]
pub fn set_cloud_stt_streaming(app: AppHandle, enabled: bool) {
    let mut settings = get_settings(&app);
    settings.cloud_stt_streaming = enabled;
    write_settings(&app, settings);
}

/// Forward the user's custom words to the provider as biasing hints.
#[tauri::command]
#[specta::specta]
pub fn set_cloud_stt_send_custom_words(app: AppHandle, enabled: bool) {
    let mut settings = get_settings(&app);
    settings.cloud_stt_send_custom_words = enabled;
    write_settings(&app, settings);
}

/// Ask the provider to strip filler words server-side.
#[tauri::command]
#[specta::specta]
pub fn set_cloud_stt_no_verbatim(app: AppHandle, enabled: bool) {
    let mut settings = get_settings(&app);
    settings.cloud_stt_no_verbatim = enabled;
    write_settings(&app, settings);
}

/// Whether a key is on file for each provider.
///
/// The UI shows "saved" from this rather than rendering the key back into a
/// password field, so a stored secret is never round-tripped through the DOM and
/// a paste of a new key is unambiguous. This is a display convenience, not an
/// isolation boundary: `get_app_settings` returns the hydrated settings —
/// including every provider key map — to the webview, exactly as it already does
/// for the post-processing, web-search, and TTS keys.
#[tauri::command]
#[specta::specta]
pub fn get_cloud_stt_key_status(app: AppHandle) -> Vec<(String, bool)> {
    let settings = get_settings(&app);
    settings
        .cloud_stt_providers
        .iter()
        .map(|provider| {
            let has_key = settings
                .cloud_stt_api_keys
                .get(&provider.id)
                .map(|key| !key.trim().is_empty())
                .unwrap_or(false);
            (provider.id.clone(), has_key)
        })
        .collect()
}

/// Ask the selected provider for its transcription models.
#[tauri::command]
#[specta::specta]
pub async fn list_cloud_stt_models(app: AppHandle) -> Result<Vec<String>, String> {
    let settings = get_settings(&app);
    let cfg = crate::stt_cloud::resolve_cloud_stt(&settings).map_err(describe_unavailable)?;
    tauri::async_runtime::spawn_blocking(move || crate::stt_cloud::list_cloud_stt_models(&cfg))
        .await
        .map_err(|e| format!("Model listing task failed: {e}"))?
}

/// Send one second of audio to confirm the key, the endpoint, and the model id
/// all work — before the user finds out mid-dictation.
#[tauri::command]
#[specta::specta]
pub async fn test_cloud_stt(app: AppHandle) -> Result<String, String> {
    let settings = get_settings(&app);
    let cfg = crate::stt_cloud::resolve_cloud_stt(&settings).map_err(describe_unavailable)?;
    tauri::async_runtime::spawn_blocking(move || crate::stt_cloud::verify_cloud_stt(&cfg))
        .await
        .map_err(|e| format!("Connection test task failed: {e}"))?
}

/// Turn a resolution failure into something worth showing in the UI. The typed
/// reason still reaches the frontend through
/// [`get_cloud_stt_readiness`]; this is the message for an action the user just
/// took and expects an answer to.
fn describe_unavailable(error: crate::settings::CloudSttResolutionError) -> String {
    use crate::settings::CloudSttUnavailableReason as Reason;
    match error.reason {
        Reason::NotEnabled => "Cloud transcription is turned off".to_string(),
        Reason::SelectedProviderMissing => {
            format!(
                "Unknown provider: {}",
                error.provider_id.unwrap_or_else(|| "(none)".to_string())
            )
        }
        Reason::MissingApiKey => format!(
            "Add an API key for {} first",
            error
                .provider_label
                .unwrap_or_else(|| "this provider".to_string())
        ),
        Reason::NoModelConfigured => format!(
            "Choose a model for {} first",
            error
                .provider_label
                .unwrap_or_else(|| "this provider".to_string())
        ),
    }
}
