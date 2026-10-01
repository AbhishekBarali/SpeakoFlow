use crate::TranscriptionCoordinator;
#[cfg(unix)]
use log::debug;
use log::warn;
use tauri::{AppHandle, Manager};

#[cfg(target_os = "macos")]
use signal_hook::consts::SIGUSR1;
#[cfg(unix)]
use signal_hook::consts::SIGUSR2;
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
    // Same rule as the hotkey path (see `shortcut::handler`): an assistant
    // trigger must not hang up a live voice conversation that is already
    // listening — it surfaces the panel instead. Dictation leaves the call up
    // too: it holds the call's microphone while it records.
    if crate::voice_conversation::is_active(app)
        && crate::assistant::is_assistant_binding(binding_id)
    {
        crate::assistant::show_assistant_panel(app);
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

/// Listen for Unix signals that remotely toggle transcription.
///
/// SIGUSR2 toggles plain transcription on every Unix platform. SIGUSR1
/// (transcription with post-processing) is only handled on macOS: on Linux,
/// WebKitGTK's JavaScriptCore garbage collector sends SIGUSR1 to its own
/// threads to suspend them, so handling it started or stopped a recording on
/// its own every few minutes (Handy #1660). Linux users trigger that binding
/// with `--toggle-post-process` instead. Backport of Handy #1824.
///
/// Registration failure is logged rather than fatal: the signals are a
/// convenience for scripts, and the app must still start without them.
#[cfg(unix)]
pub fn setup_signal_handler(app_handle: AppHandle) {
    #[cfg(target_os = "macos")]
    let registered = Signals::new([SIGUSR1, SIGUSR2]);
    #[cfg(not(target_os = "macos"))]
    let registered = Signals::new([SIGUSR2]);
    let mut signals = match registered {
        Ok(signals) => signals,
        Err(e) => {
            warn!("Could not register transcription signal handlers: {e}");
            return;
        }
    };
    #[cfg(target_os = "macos")]
    debug!("Signal handlers registered (SIGUSR1, SIGUSR2)");
    #[cfg(not(target_os = "macos"))]
    debug!("Signal handler registered (SIGUSR2; SIGUSR1 is left to WebKitGTK)");
    thread::spawn(move || {
        for sig in signals.forever() {
            let (binding_id, signal_name) = match sig {
                #[cfg(target_os = "macos")]
                SIGUSR1 => ("transcribe_with_post_process", "SIGUSR1"),
                SIGUSR2 => ("transcribe", "SIGUSR2"),
                _ => continue,
            };
            debug!("Received {signal_name}");
            send_transcription_input(&app_handle, binding_id, signal_name);
        }
    });
}
