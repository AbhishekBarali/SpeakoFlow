//! Tauri commands for the local personal-memory feature (Settings → Memory).
//!
//! Everything here reads/writes `settings.assistant_memory` (and the related
//! toggles) and lives entirely on-device. Mutations emit
//! `assistant-settings-changed` so the panel and settings window refresh.

use crate::assistant::AssistantConversation;
use crate::memory;
use crate::settings::{
    get_settings, write_settings, MemoryConfidence, MemoryDetail, MemoryNote, UserMemory,
};
use tauri::{AppHandle, Emitter, Manager};

/// Notify open webviews (settings window + panel) that settings changed.
fn emit_settings_changed(app: &AppHandle) {
    let _ = app.emit("assistant-settings-changed", ());
}

/// Turn the personal-memory feature on or off. Off by default.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_memory_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_memory_enabled = enabled;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Set how much memory is injected per turn (the token-budget dial).
#[tauri::command]
#[specta::specta]
pub fn set_assistant_memory_detail(app: AppHandle, detail: MemoryDetail) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_memory_detail = detail;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Toggle incognito: when on, this conversation is neither remembered nor
/// personalized from memory.
#[tauri::command]
#[specta::specta]
pub fn set_assistant_memory_incognito(app: AppHandle, incognito: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_memory_incognito = incognito;
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Replace the always-on "About You" summary (user-edited in Settings).
#[tauri::command]
#[specta::specta]
pub fn set_assistant_memory_about_you(app: AppHandle, text: String) -> Result<(), String> {
    let trimmed = text.trim();
    if memory::is_sensitive(trimmed) {
        return Err(
            "That text looks like it contains a secret or an instruction — memory can't store it."
                .to_string(),
        );
    }
    let mut settings = get_settings(&app);
    settings.assistant_memory.about_you = trimmed.chars().take(600).collect();
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Add a user-authored note (explicit → high confidence). Returns the new note.
#[tauri::command]
#[specta::specta]
pub fn add_assistant_memory_note(app: AppHandle, text: String) -> Result<MemoryNote, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Write something to remember first.".to_string());
    }
    if memory::is_sensitive(trimmed) {
        return Err(
            "That looks like a secret or an instruction — memory can't store it.".to_string(),
        );
    }
    let note = MemoryNote {
        id: memory::new_note_id(),
        text: trimmed.chars().take(240).collect(),
        updated: memory::today_iso(),
        confidence: MemoryConfidence::High,
        source: "user".to_string(),
    };
    let mut settings = get_settings(&app);
    settings.assistant_memory.notes.push(note.clone());
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(note)
}

/// Edit an existing note's text (keeps it user-owned; bumps its date).
#[tauri::command]
#[specta::specta]
pub fn update_assistant_memory_note(
    app: AppHandle,
    id: String,
    text: String,
) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("A note can't be empty — delete it instead.".to_string());
    }
    if memory::is_sensitive(trimmed) {
        return Err(
            "That looks like a secret or an instruction — memory can't store it.".to_string(),
        );
    }
    let mut settings = get_settings(&app);
    let Some(note) = settings
        .assistant_memory
        .notes
        .iter_mut()
        .find(|n| n.id == id)
    else {
        return Err("That note no longer exists.".to_string());
    };
    note.text = trimmed.chars().take(240).collect();
    note.updated = memory::today_iso();
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Delete a single note by id.
#[tauri::command]
#[specta::specta]
pub fn delete_assistant_memory_note(app: AppHandle, id: String) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_memory.notes.retain(|n| n.id != id);
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Wipe the entire personal memory (summary + all notes). Does not change the
/// enabled toggle.
#[tauri::command]
#[specta::specta]
pub fn clear_assistant_memory(app: AppHandle) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.assistant_memory = UserMemory::default();
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(())
}

/// Export the whole memory to a JSON file on disk (path chosen via the UI's
/// save dialog). Your data, in a portable, human-readable file.
#[tauri::command]
#[specta::specta]
pub fn export_assistant_memory(app: AppHandle, path: String) -> Result<(), String> {
    let settings = get_settings(&app);
    let json =
        serde_json::to_string_pretty(&settings.assistant_memory).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Couldn't write file: {}", e))?;
    Ok(())
}

/// Import memory from a JSON file (path chosen via the UI's file dialog),
/// replacing the current memory. Sensitive/oversized entries are filtered out
/// on the way in.
#[tauri::command]
#[specta::specta]
pub fn import_assistant_memory(app: AppHandle, path: String) -> Result<UserMemory, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("Couldn't read file: {}", e))?;
    let mut imported: UserMemory = serde_json::from_slice(&bytes)
        .map_err(|e| format!("That file isn't a valid memory export: {}", e))?;

    // Sanitize: drop sensitive summary, filter sensitive/empty notes, backfill
    // ids/dates so the store stays well-formed.
    if memory::is_sensitive(&imported.about_you) {
        imported.about_you = String::new();
    } else {
        imported.about_you = imported.about_you.trim().chars().take(600).collect();
    }
    imported.notes.retain(|n| {
        let t = n.text.trim();
        !t.is_empty() && !memory::is_sensitive(t)
    });
    for note in imported.notes.iter_mut() {
        if note.id.trim().is_empty() {
            note.id = memory::new_note_id();
        }
        if note.updated.trim().is_empty() {
            note.updated = memory::today_iso();
        }
        note.text = note.text.trim().chars().take(240).collect();
    }

    let mut settings = get_settings(&app);
    settings.assistant_memory = imported.clone();
    write_settings(&app, settings);
    emit_settings_changed(&app);
    Ok(imported)
}

/// Distill memory from the CURRENT conversation right now (the "Update memory
/// from this chat" button). Runs the offline extraction pass immediately so the
/// user can see it work without waiting for the conversation to end.
#[tauri::command]
#[specta::specta]
pub async fn assistant_distill_memory_now(app: AppHandle) -> Result<(), String> {
    let settings = get_settings(&app);
    if !settings.assistant_memory_enabled {
        return Err("Turn on memory first.".to_string());
    }
    if settings.assistant_memory_incognito {
        return Err("This conversation is incognito — turn that off to remember it.".to_string());
    }

    let messages = {
        let conversation = app.state::<AssistantConversation>();
        let guard = conversation
            .messages
            .lock()
            .map_err(|e| format!("Conversation lock poisoned: {}", e))?;
        guard.clone()
    };
    if memory::user_turn_count(&messages) < 2 {
        return Err("Have a short conversation first, then I can learn from it.".to_string());
    }

    memory::distill_and_store(app.clone(), messages).await;
    // Mark this length as distilled so closing the panel later won't redo it.
    app.state::<AssistantConversation>()
        .mark_distilled_current();
    Ok(())
}
