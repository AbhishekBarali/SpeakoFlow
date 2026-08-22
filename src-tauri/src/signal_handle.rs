use crate::TranscriptionCoordinator;
#[cfg(unix)]
use log::debug;
use log::warn;
use tauri::{AppHandle, Manager};

#[cfg(unix)]
use signal_hook::consts::{SIGUSR1, SIGUSR2};
#[cfg(unix)]
use signal_hook::iterator::Signals;
#[cfg(unix)]
use std::thread;

/// Send a transcription input to the coordinator.
/// Used by signal handlers, CLI flags, and any other external trigger.
pub fn send_transcription_input(app: &AppHandle, binding_id: &str, source: &str) {
    // External triggers reach the coordinator directly, so they skip the gate in
    // `shortcut::handler`. Without this, `--toggle-assistant` would still record
    // and run a whole assistant turn — model, screen capture, spoken reply —
    // with the assistant switched off and no window to show any of it in.
    if crate::assistant::is_assistant_binding(binding_id)
        && !crate::settings::get_settings(app).assistant_enabled
    {
        warn!("Ignoring '{binding_id}' from {source}: the assistant is switched off");
        return;
    }
    if let Some(c) = app.try_state::<TranscriptionCoordinator>() {
        // External triggers can't "hold", so they always run hands-free (lock).
        c.send_input(
            binding_id,
            source,
            true,
            crate::transcription_coordinator::RecordingMode::Lock,
        );
    } else {
        warn!("TranscriptionCoordinator not initialized");
    }
}

#[cfg(unix)]
pub fn setup_signal_handler(app_handle: AppHandle, mut signals: Signals) {
    debug!("Signal handlers registered (SIGUSR1, SIGUSR2)");
    thread::spawn(move || {
        for sig in signals.forever() {
            let (binding_id, signal_name) = match sig {
                SIGUSR1 => ("transcribe_with_post_process", "SIGUSR1"),
                SIGUSR2 => ("transcribe", "SIGUSR2"),
                _ => continue,
            };
            debug!("Received {signal_name}");
            send_transcription_input(&app_handle, binding_id, signal_name);
        }
    });
}
