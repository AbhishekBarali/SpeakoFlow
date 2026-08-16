//! Tauri commands for managed coding-agent sessions (Settings → Agents).
//!
//! These are the same operations the assistant can reach by voice, exposed to
//! the UI. The one that only exists here is high-risk approval: a destructive
//! command is deliberately refused on the voice path, so the app has to provide
//! a way to confirm it deliberately, with the command visible on screen.

use crate::agents::{AgentManager, AgentSessionView, Delivery, StartRequest};
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
///
/// `agent` names which CLI to drive ("kiro", "claude", "codex", …) and defaults
/// to the configured or first installed one.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub fn agent_session_start(
    app: AppHandle,
    cwd: String,
    task: String,
    label: Option<String>,
    agent: Option<String>,
    create_if_missing: Option<bool>,
    auto_approve: Option<bool>,
    model: Option<String>,
    effort: Option<String>,
) -> Result<String, String> {
    manager(&app)?.start(
        &app,
        StartRequest {
            cwd: &cwd,
            prompt: &task,
            label,
            model,
            effort,
            agent: agent.as_deref(),
            create_if_missing: create_if_missing.unwrap_or(false),
            auto_approve: auto_approve.unwrap_or(false),
        },
    )
}

/// Which agents are installed on this machine, for the picker.
#[tauri::command]
#[specta::specta]
pub fn agent_available_agents() -> Vec<String> {
    crate::agents::installed_agent_labels()
}

/// Send a follow-up instruction to a live session.
#[tauri::command]
#[specta::specta]
pub fn agent_session_send(app: AppHandle, id: String, message: String) -> Result<String, String> {
    manager(&app)?.send(&app, &id, &message)
}

/// Send an instruction that interrupts whatever the session is doing.
///
/// The UI equivalent of saying "no, stop, do this instead". Queued delivery is
/// [`agent_session_send`]; this one cancels the running turn first.
#[tauri::command]
#[specta::specta]
pub fn agent_session_interrupt_with(
    app: AppHandle,
    id: String,
    message: String,
) -> Result<String, String> {
    manager(&app)?.steer(&app, &id, &message, Delivery::Interrupt)
}

/// Switch a session's mode (Kiro exposes its agents this way).
#[tauri::command]
#[specta::specta]
pub fn agent_session_set_mode(app: AppHandle, id: String, mode: String) -> Result<String, String> {
    manager(&app)?.set_mode(&app, &id, &mode)
}

/// The modes a session offers.
#[tauri::command]
#[specta::specta]
pub fn agent_session_modes(app: AppHandle, id: String) -> Result<String, String> {
    manager(&app)?.modes_block(&id)
}

/// Turn automatic approval of safe actions on or off for one session.
#[tauri::command]
#[specta::specta]
pub fn agent_session_set_auto_approve(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<String, String> {
    manager(&app)?.set_auto_approve(&app, &id, enabled)
}

/// Create a project folder under one of the allowed roots.
#[tauri::command]
#[specta::specta]
pub fn agent_create_project_folder(path: String) -> Result<String, String> {
    crate::agents::create_folder(&path).map(|p| p.to_string_lossy().to_string())
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
