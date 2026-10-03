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
use crate::transcription_coordinator::{is_transcribe_binding, recording_mode};
use crate::TranscriptionCoordinator;

/// Handle a shortcut event from either implementation.
///
/// This function contains the shared logic for:
/// - Looking up the action in ACTION_MAP
/// - Handling the cancel binding (only fires when recording)
/// - Routing transcribe/assistant bindings to the coordinator, resolving the
///   recording mode (push-to-talk hold vs tap to toggle) from the setting
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
    let base_id = binding_id;

    // The assistant is switched off: its hotkeys do nothing. They are also
    // unregistered at the OS level when the setting changes, so this is the
    // belt-and-braces path for a shortcut that was already in flight.
    if crate::assistant::is_assistant_binding(base_id) && !get_settings(app).assistant_enabled {
        return;
    }

    // Transcribe/assistant bindings are handled by the coordinator.
    if is_transcribe_binding(base_id) {
        // The assistant's own shortcut is a quick ask, which takes the window
        // over, so it ends a call. Dictation does not: it holds the call's
        // microphone while it records (see `voice_conversation::DictationTracker`)
        // and the call carries on afterwards. Hanging up here used to throw the
        // call's whole conversation away just to type a sentence somewhere else.
        if is_pressed
            && crate::assistant::is_assistant_binding(base_id)
            && crate::voice_conversation::is_active(app)
        {
            crate::voice_conversation::end(app);
        }
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            // Every recording shortcut — dictation, dictation + post-processing,
            // and the assistant — follows the single Push-to-talk setting:
            //   • Push-to-talk ON  → hold the shortcut to record, release to stop.
            //   • Push-to-talk OFF → tap once to start, tap again to stop.
            let mode = recording_mode(get_settings(app).push_to_talk);
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
        if is_pressed {
            // Esc during a call: stop the reply if there is one, hang up if
            // there isn't — unless a dictation is running beside the call, in
            // which case Esc is the dictation's and the call is left alone.
            //
            // It used to hang up unconditionally, which is the worst of both.
            // Mid-answer it threw away the thing the user was listening to, and
            // either way it ended the session while leaving the window on
            // screen — so the panel re-rendered as the quick-ask card and the
            // call had silently died behind it. A hang-up now goes through
            // `hide_assistant_panel` (which ends the session itself), so the
            // surface always leaves with the call.
            if crate::voice_conversation::esc_hangs_up(
                crate::voice_conversation::is_active(app),
                assistant_busy,
                crate::voice_conversation::dictation_in_flight(),
            ) {
                crate::assistant::hide_assistant_panel(app);
            }
            if audio_manager.is_recording() || assistant_busy || flow_busy {
                action.start(app, base_id, hotkey_string);
            }
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

/// A modifier-only shortcut that fired a moment ago was withdrawn by the
/// hotkey engine: another key followed, so the modifiers were the start of a
/// different shortcut (`Ctrl+Win+→` to switch desktops while dictation is
/// `Ctrl+Win`). Throw away the recording that press started.
///
/// Only recording shortcuts can be withdrawn this way in practice — every other
/// action ships on a combo with a main key, which the engine never withdraws —
/// so anything else has nothing to undo.
pub fn handle_shortcut_cancelled(app: &AppHandle, binding_id: &str) {
    if !is_transcribe_binding(binding_id) {
        return;
    }
    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.notify_abort(binding_id);
    }
}
