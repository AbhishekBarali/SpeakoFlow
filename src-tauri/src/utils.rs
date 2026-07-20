use crate::managers::audio::AudioRecordingManager;
use crate::managers::transcription::TranscriptionManager;
use crate::shortcut;
use crate::TranscriptionCoordinator;
use log::info;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

// Re-export all utility modules for easy access
// pub use crate::audio_feedback::*;
pub use crate::clipboard::*;
pub use crate::overlay::*;
pub use crate::tray::*;

/// Centralized cancellation function that can be called from anywhere in the app.
/// Handles cancelling both recording and transcription operations and updates UI state.
pub fn cancel_current_operation(app: &AppHandle) {
    info!("Initiating operation cancellation...");

    // Unregister the cancel shortcut asynchronously
    shortcut::unregister_cancel_shortcut(app);

    // Cancel any ongoing recording
    let audio_manager = app.state::<Arc<AudioRecordingManager>>();
    let recording_was_active = audio_manager.is_recording();
    audio_manager.cancel_recording();

    // Cancel any in-flight Flow generation and ensure a cancelled recording's
    // live-transcript watcher cannot leak into the next recording mode.
    crate::flow::cancel_generation();
    crate::flow::stop_prewarm_watch();

    // Drop any screen frame grabbed at the start of a voice question (Immediate
    // vision timing) so a cancelled capture never rides along with a later turn.
    crate::assistant::clear_immediate_capture();

    // Abort any in-flight assistant turn (streaming LLM answer) and silence a
    // spoken reply that's playing or about to play, so cancel (Esc / the pill's
    // stop button) stops a reply mid-generation — not only a recording. All of
    // these are no-ops when the assistant is idle.
    if let Some(conversation) = app.try_state::<crate::assistant::AssistantConversation>() {
        conversation.request_cancel();
    }
    crate::tts::stop_remote();
    {
        use tauri::Emitter;
        let _ = app.emit("assistant-tts-stop", ());
    }
    // Reset the assistant panel/pill to idle. The panel renders purely from
    // `assistant-state` events, so without this an in-progress capture
    // (listening / transcribing / thinking / speaking) stays visually stuck
    // after a cancel even though the recording and turn have actually stopped —
    // the "I pressed cancel and nothing happened" bug. Safe/idempotent when the
    // panel is hidden or already idle.
    crate::assistant::emit_state(app, "idle");

    // Update tray icon and hide overlay
    change_tray_icon(app, crate::tray::TrayIconState::Idle);
    hide_recording_overlay(app);

    // Unload model if immediate unload is enabled
    let tm = app.state::<Arc<TranscriptionManager>>();
    // Cancel any active live/streaming transcription worker so it releases the
    // leased model engine. No-op when live transcription isn't active.
    tm.cancel_stream();
    tm.maybe_unload_immediately("cancellation");

    // Notify coordinator so it can keep lifecycle state coherent.
    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.notify_cancel(recording_was_active);
    }

    info!("Operation cancellation completed - returned to idle state");
}

/// Check if using the Wayland display server protocol
#[cfg(target_os = "linux")]
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.to_lowercase() == "wayland")
            .unwrap_or(false)
}

/// Check if running on KDE Plasma desktop environment
#[cfg(target_os = "linux")]
pub fn is_kde_plasma() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("KDE"))
        .unwrap_or(false)
        || std::env::var("KDE_SESSION_VERSION").is_ok()
}

/// Check if running on KDE Plasma with Wayland
#[cfg(target_os = "linux")]
pub fn is_kde_wayland() -> bool {
    is_wayland() && is_kde_plasma()
}
