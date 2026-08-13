//! Tauri commands for managed coding-agent sessions (Settings → Agents).
//!
//! These are the same operations the assistant can reach by voice, exposed to
//! the UI. The one that only exists here is high-risk approval: a destructive
//! command is deliberately refused on the voice path, so the app has to provide
//! a way to confirm it deliberately, with the command visible on screen.

use crate::agents::{AgentManager, AgentSessionView};
use tauri::{AppHandle, Manager};

/// Fetch the manager, or explain why it is missing rather than panicking.
fn manager(app: &AppHandle) -> Result<tauri::State<'_, AgentManager>, String> {
    app.try_state::<AgentManager>()
        .ok_or_else(|| "The agent session manager is not initialized.".to_string())
}

/// Every session SpeakoFlow has started, in start order.
#[tauri::command]
#[specta::specta]
pub fn agent_sessions(app: AppHandle) -> Result<Vec<AgentSessionView>, String> {
    Ok(manager(&app)?.views())
}

/// Start a session in `cwd` with `task` as its first instruction.
#[tauri::command]
#[specta::specta]
pub fn agent_session_start(
    app: AppHandle,
    cwd: String,
    task: String,
    label: Option<String>,
) -> Result<String, String> {
    manager(&app)?.start(&app, &cwd, &task, label, None)
}

/// Send a follow-up instruction to a live session.
#[tauri::command]
#[specta::specta]
pub fn agent_session_send(app: AppHandle, id: String, message: String) -> Result<String, String> {
    manager(&app)?.send(&app, &id, &message)
}

/// Stop the current turn, leaving the session open for a new instruction.
#[tauri::command]
#[specta::specta]
pub fn agent_session_cancel(app: AppHandle, id: String) -> Result<String, String> {
    manager(&app)?.cancel(&app, &id)
}

/// Shut a session down for good.
#[tauri::command]
#[specta::specta]
pub fn agent_session_close(app: AppHandle, id: String) -> Result<String, String> {
    manager(&app)?.close(&app, &id)
}

/// Answer the permission prompt a session is blocked on.
///
/// `force` is what makes this different from the voice path: approving an action
/// classified as destructive requires an explicit on-screen confirmation, where
/// the user can read the exact command instead of trusting a transcript.
#[tauri::command]
#[specta::specta]
pub fn agent_session_answer_permission(
    app: AppHandle,
    id: String,
    allow: bool,
    force: bool,
) -> Result<String, String> {
    manager(&app)?.answer_permission(&app, &id, allow, force)
}

/// Open a session's working folder in the OS file manager.
#[tauri::command]
#[specta::specta]
pub fn agent_session_open_folder(app: AppHandle, id: String) -> Result<String, String> {
    manager(&app)?.open_folder(&app, &id)
}

/// Hand a session to a real terminal, resumed with its full history.
///
/// SpeakoFlow stops driving it at that point — two processes on one transcript
/// would race — so this is a deliberate handover, not a second view.
#[tauri::command]
#[specta::specta]
pub fn agent_session_resume_in_terminal(app: AppHandle, id: String) -> Result<String, String> {
    manager(&app)?.resume_in_terminal(&app, &id)
}
