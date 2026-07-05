//! Shared shortcut event handling logic
//!
//! This module contains the common logic for handling shortcut events,
//! used by both the Tauri and handy-keys implementations.

use log::warn;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use crate::settings::get_settings;
use crate::transcription_coordinator::{is_transcribe_binding, recording_mode, LOCK_SUFFIX};
use crate::TranscriptionCoordinator;

/// Handle a shortcut event from either implementation.
///
/// This function contains the shared logic for:
/// - Looking up the action in ACTION_MAP
/// - Handling the cancel binding (only fires when recording)
/// - Routing transcribe/assistant bindings to the coordinator, resolving the
///   recording mode (push-to-talk hold vs hands-free lock) from the setting and
///   whether the fired shortcut is the Shift "lock" variant
///
/// # Arguments
/// * `app` - The Tauri app handle
/// * `binding_id` - The ID of the binding (e.g., "transcribe", "cancel")
/// * `hotkey_string` - The string representation of the hotkey
/// * `is_pressed` - Whether this is a key press (true) or release (false)
pub fn handle_shortcut_event(
    app: &AppHandle,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
) {
    // Recording shortcuts have an auto-derived Shift "lock" variant whose
    // binding id carries a `.lock` suffix (e.g. "transcribe.lock"). Strip it to
    // recover the real action id, and remember it was the hands-free variant.
    let (base_id, is_lock_variant) = match binding_id.strip_suffix(LOCK_SUFFIX) {
        Some(base) => (base, true),
        None => (binding_id, false),
    };

    // Transcribe/assistant bindings are handled by the coordinator. The base
    // shortcut uses the default mode (push-to-talk hold by default); tapping the
    // lock key on top converts a hold to hands-free mid-recording.
    if is_transcribe_binding(base_id) {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            // The assistant always starts as push-to-talk (hold): press and
            // hold to record, release to send. It only goes hands-free
            // (press-again-to-stop) when the user presses the lock shortcut
            // (Shift by default) mid-recording — so it never opens in a
            // "locked" state on its own. Dictation still follows the global
            // push-to-talk setting.
            let mode = if base_id == "assistant" && !is_lock_variant {
                crate::transcription_coordinator::RecordingMode::Hold
            } else {
                recording_mode(get_settings(app).push_to_talk, is_lock_variant)
            };
            coordinator.send_input(base_id, hotkey_string, is_pressed, mode);
        } else {
            warn!("TranscriptionCoordinator is not initialized");
        }
        return;
    }

    let Some(action) = ACTION_MAP.get(base_id) else {
        warn!(
            "No action defined in ACTION_MAP for shortcut ID '{}'. Shortcut: '{}', Pressed: {}",
            base_id, hotkey_string, is_pressed
        );
        return;
    };

    // Cancel binding: fires while recording OR while the assistant is
    // generating an answer, so Esc can stop a reply mid-stream — not only a
    // recording. Only on key-press.
    if base_id == "cancel" {
        let audio_manager = app.state::<Arc<AudioRecordingManager>>();
        let assistant_busy = app
            .try_state::<crate::assistant::AssistantConversation>()
            .map_or(false, |c| c.is_busy());
        if is_pressed && (audio_manager.is_recording() || assistant_busy) {
            action.start(app, base_id, hotkey_string);
        }
        return;
    }

    // Remaining bindings (e.g. "test") use simple start/stop on press/release.
    if is_pressed {
        action.start(app, base_id, hotkey_string);
    } else {
        action.stop(app, base_id, hotkey_string);
    }
}
