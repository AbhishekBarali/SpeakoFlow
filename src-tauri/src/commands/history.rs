use crate::actions::process_transcription_output;
use crate::managers::{
    history::{HistoryManager, PaginatedAssistantHistory, PaginatedHistory},
    transcription::TranscriptionManager,
};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
#[specta::specta]
pub async fn get_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    history_manager
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_history_entry_saved(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_audio_file_path(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    file_name: String,
) -> Result<String, String> {
    let path = history_manager.get_audio_file_path(&file_name);
    path.to_str()
        .ok_or_else(|| "Invalid file path".to_string())
        .map(|s| s.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_history_entry(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_transcription(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    id: i64,
) -> Result<(), String> {
    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {} not found", id))?;

    let audio_path = history_manager.get_audio_file_path(&entry.file_name);
    let samples = crate::audio_toolkit::read_wav_samples(&audio_path)
        .map_err(|e| format!("Failed to load audio: {}", e))?;

    if samples.is_empty() {
        return Err("Recording has no audio samples".to_string());
    }

    transcription_manager.initiate_model_load();

    let tm = Arc::clone(&transcription_manager);
    let transcription = tauri::async_runtime::spawn_blocking(move || tm.transcribe(samples))
        .await
        .map_err(|e| format!("Transcription task panicked: {}", e))?
        .map_err(|e| e.to_string())?;

    if transcription.is_empty() {
        return Err("Recording contains no speech".to_string());
    }

    let is_flow_entry =
        entry.post_process_prompt.as_deref() == Some(crate::flow::FLOW_HISTORY_MARKER);
    let (post_processed_text, post_process_prompt) = if is_flow_entry {
        // Re-running speech recognition should repair only the transcript. A
        // Flow output is a completed generated artifact; keep it and its marker
        // instead of silently converting the row into ordinary dictation.
        (
            entry.post_processed_text.clone(),
            entry.post_process_prompt.clone(),
        )
    } else {
        let processed =
            process_transcription_output(&app, &transcription, entry.post_process_requested).await;
        (processed.post_processed_text, processed.post_process_prompt)
    };

    history_manager
        .update_transcription(id, transcription, post_processed_text, post_process_prompt)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Apply the stored policy and tell the frontend how many recordings went away.
///
/// Callers write the setting first; this runs after it is safely on disk.
///
/// Pruning deliberately cannot fail this command. It used to: the settings were
/// written first and a cleanup error returned `Err`, at which point the frontend
/// rolled its optimistic value *back* while disk kept the new one — the UI and the
/// stored setting then disagreed until the next refresh, which is the "it says
/// saved but it isn't" symptom. A prune that fails is logged and retried by the
/// next sweep; the user's choice is already safely stored.
fn prune_and_notify(app: &AppHandle, history_manager: &HistoryManager) -> usize {
    let deleted = match history_manager.cleanup_old_entries() {
        Ok(deleted) => deleted,
        Err(e) => {
            log::error!("History retention cleanup failed: {}", e);
            0
        }
    };

    if deleted > 0 {
        // The listener refetches the first page, which discards everything the
        // user has scrolled into view — so only signal when rows really went away.
        if let Err(e) = app.emit("history-retention-applied", ()) {
            log::error!("Failed to emit history-retention-applied: {}", e);
        }
    }

    deleted
}

#[tauri::command]
#[specta::specta]
pub async fn update_history_limit(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    limit: usize,
) -> Result<usize, String> {
    // A limit of 0 deletes every unstarred recording. That is never what someone
    // means while clearing an input field, and it is unrecoverable.
    let allowed = crate::settings::MIN_HISTORY_LIMIT..=crate::settings::MAX_HISTORY_LIMIT;
    if !allowed.contains(&limit) {
        return Err(format!(
            "Recording limit must be between {} and {}",
            crate::settings::MIN_HISTORY_LIMIT,
            crate::settings::MAX_HISTORY_LIMIT
        ));
    }

    let mut settings = crate::settings::get_settings(&app);
    settings.history_limit = limit;
    crate::settings::write_settings(&app, settings);

    Ok(prune_and_notify(&app, &history_manager))
}

/// Takes the enum, not a string.
///
/// The previous signature was `period: String` with a hand-written match, which
/// is how the wire-format mismatch stayed invisible: `bindings.ts` advertised
/// `"days_3"` (specta) while this match only accepted `"days3"` (serde), so the
/// generated types described a call the backend would have rejected. With the
/// typed parameter, serde owns the mapping and specta generates the same
/// literals it accepts.
#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_period(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: crate::settings::RecordingRetentionPeriod,
) -> Result<usize, String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.recording_retention_period = period;
    crate::settings::write_settings(&app, settings);

    Ok(prune_and_notify(&app, &history_manager))
}

/// Length of the custom "keep for N days" policy.
#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_days(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    days: u32,
) -> Result<usize, String> {
    let allowed = crate::settings::MIN_RECORDING_RETENTION_DAYS
        ..=crate::settings::MAX_RECORDING_RETENTION_DAYS;
    if !allowed.contains(&days) {
        return Err(format!(
            "Retention window must be between {} and {} days",
            crate::settings::MIN_RECORDING_RETENTION_DAYS,
            crate::settings::MAX_RECORDING_RETENTION_DAYS
        ));
    }

    let mut settings = crate::settings::get_settings(&app);
    settings.recording_retention_days = days;
    crate::settings::write_settings(&app, settings);

    Ok(prune_and_notify(&app, &history_manager))
}

/// How many recordings a prospective policy would delete, changing nothing.
///
/// Retention deletes the audio file along with the row, so the History panel asks
/// this before applying a stricter policy and makes the user confirm the number.
#[tauri::command]
#[specta::specta]
pub async fn preview_recording_retention(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: crate::settings::RecordingRetentionPeriod,
    limit: usize,
    days: u32,
) -> Result<usize, String> {
    let plan =
        crate::managers::history::resolve_retention_plan(period, limit, days, chrono::Utc::now());
    history_manager
        .count_pending_deletions(plan)
        .map_err(|e| e.to_string())
}

/// Re-apply the stored policy right now.
///
/// The History panel calls this when it opens. A time-based policy is otherwise
/// only enforced at launch and after a new recording is saved, so entries that
/// crossed the boundary while the app sat idle stayed listed — "the history is
/// visible regardless of the retention period".
#[tauri::command]
#[specta::specta]
pub async fn enforce_recording_retention(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
) -> Result<usize, String> {
    let deleted = history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    if deleted > 0 {
        if let Err(e) = app.emit("history-retention-applied", ()) {
            log::error!("Failed to emit history-retention-applied: {}", e);
        }
    }

    Ok(deleted)
}

/// Page through saved assistant conversations (newest first).
#[tauri::command]
#[specta::specta]
pub async fn get_assistant_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedAssistantHistory, String> {
    history_manager
        .get_assistant_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a single saved assistant conversation.
#[tauri::command]
#[specta::specta]
pub async fn delete_assistant_history_entry(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .delete_assistant_session(id)
        .map_err(|e| e.to_string())?;

    // If this was the conversation currently open in the panel, detach it so
    // the next turn re-saves instead of updating the deleted row.
    if let Some(conversation) = app.try_state::<crate::assistant::AssistantConversation>() {
        conversation.forget_session_if(id);
    }

    let _ = app.emit("assistant-history-updated", ());
    Ok(())
}
