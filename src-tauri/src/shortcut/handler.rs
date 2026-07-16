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
            // Every recording shortcut — dictation, dictation + post-processing,
            // and the assistant — follows the single Push-to-talk setting:
            //   • Push-to-talk ON  → hold the shortcut to record, release to stop.
            //   • Push-to-talk OFF → tap once to start, tap again to stop.
            // Escape cancels. There is no separate tap-to-lock; this is the
            // simple Handy-style model.
            let mode = recording_mode(get_settings(app).push_to_talk, is_lock_variant);
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

    // Cancel binding: fires while recording, while the assistant is generating
    // an answer, OR while Flow is starting/generating, so Esc can stop every
    // long-running voice operation after recording ends. Only on key-press.
    if base_id == "cancel" {
        let audio_manager = app.state::<Arc<AudioRecordingManager>>();
        let assistant_busy = app
            .try_state::<crate::assistant::AssistantConversation>()
            .map_or(false, |c| c.is_busy());
        let flow_busy = crate::flow::is_generation_active();
        if is_pressed && (audio_manager.is_recording() || assistant_busy || flow_busy) {
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
