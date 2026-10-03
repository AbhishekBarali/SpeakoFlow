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

/// Display wrapper for user content (transcripts, model output, reminder text,
/// search queries) in log lines.
///
/// Development builds print the text so it can be debugged; release builds
/// print only its length. The log file defaults to Debug level and lives on
/// disk, so without this every dictation was written there in full — which the
/// privacy promise ("your voice never leaves your device", nothing kept that you
/// can't see) does not cover. Backport of Handy 258899a2, extended to every log
/// site that carried user content. Not for secrets such as API keys, which must
/// never be logged in any build.
pub struct Redacted<'a>(pub &'a str);

impl std::fmt::Display for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if cfg!(debug_assertions) {
            f.write_str(self.0)
        } else {
            write!(f, "[redacted, {} chars]", self.0.chars().count())
        }
    }
}

impl std::fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if cfg!(debug_assertions) {
            write!(f, "{:?}", self.0)
        } else {
            std::fmt::Display::fmt(self, f)
        }
    }
}

/// Shorthand for [`Redacted`].
pub fn redact_text(text: &str) -> Redacted<'_> {
    Redacted(text)
}

/// Centralized cancellation function that can be called from anywhere in the app.
/// Handles cancelling both recording and transcription operations and updates UI state.
pub fn cancel_current_operation(app: &AppHandle) {
    cancel_operation(app, CancelOrigin::User);
}

/// Throw away a recording whose shortcut press turned out to be the first keys
/// of some other application's shortcut (`Ctrl+Alt+↑` while the ask key is
/// `Ctrl+Alt`). Called by the coordinator itself, which has already returned
/// to idle — so, unlike a cancel, it is not told about it again: that queued
/// notice would land after the shortcut the press gave way to had started its
/// own recording, and reset it. Nothing is offered back either; the user never
/// meant to record.
pub fn discard_aborted_recording(app: &AppHandle) {
    cancel_operation(app, CancelOrigin::AbortedPress);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CancelOrigin {
    /// Esc, the pill's stop button, `--cancel`, a recovery after a panic.
    User,
    /// A hotkey press withdrawn by the hotkey engine.
    AbortedPress,
}

fn cancel_operation(app: &AppHandle, origin: CancelOrigin) {
    info!("Initiating operation cancellation...");

    // Unregister the cancel shortcut asynchronously
    shortcut::unregister_cancel_shortcut(app);

    // Cancel any ongoing recording. What it captured comes back, so a
    // dictation cancelled by mistake can be offered back below instead of lost.
    let audio_manager = app.state::<Arc<AudioRecordingManager>>();
    // Decided before anything is torn down. A cancel during a dictation that is
    // running beside a call is aimed at the dictation: the call's reply, its
    // voice and its state are left exactly as they are.
    let spare_call = crate::voice_conversation::cancel_spares_call(
        crate::voice_conversation::is_active(app),
        crate::voice_conversation::dictation_in_flight(),
    );
    let cancelled = audio_manager.cancel_recording();
    let recording_was_active = cancelled.is_some();
    if recording_was_active {
        // Hands the microphone back to a call this dictation was holding.
        crate::voice_conversation::dictation_cancelled(app);
    }

    // Cancel any in-flight Flow generation and ensure a cancelled recording's
    // live-transcript watcher cannot leak into the next recording mode.
    crate::flow::cancel_generation();
    crate::flow::stop_prewarm_watch();

    // Drop any screen frame grabbed at the start of a voice question (Immediate
    // vision timing) so a cancelled capture never rides along with a later turn.
    crate::assistant::clear_agent_capture();

    // Same reasoning for a captured text selection: a recording the user
    // abandoned must not donate its selection to the next question they ask. That
    // failure would be silent and would put text they never offered in front of a
    // model, so it is cleared on the way out rather than relied on to expire.
    crate::selection::clear_pending();

    // Whether this cancellation belongs to the assistant: either a turn is in
    // flight, or the recording being cancelled was routed to the assistant.
    // Read before `request_cancel()` below, while the turn still reports busy.
    let assistant_owns_cancel = !spare_call
        && (app
            .try_state::<crate::assistant::AssistantConversation>()
            .map(|conversation| conversation.is_busy())
            .unwrap_or(false)
            || crate::assistant::is_transcribe_redirected());

    // Abort any in-flight assistant turn (streaming LLM answer) and silence a
    // spoken reply that's playing or about to play, so cancel (Esc / the pill's
    // stop button) stops a reply mid-generation — not only a recording. All of
    // these are no-ops when the assistant is idle.
    if !spare_call {
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
        // after a cancel even though the recording and turn have actually
        // stopped — the "I pressed cancel and nothing happened" bug. Safe and
        // idempotent when the panel is hidden or already idle.
        crate::assistant::emit_state(app, "idle");
    }
    // The compact voice overlay is transient, so cancelling an assistant turn
    // dismisses it. Cancelling a plain dictation leaves it alone: `Esc` during
    // dictation shouldn't close the assistant, and `hide_assistant_panel` ends
    // the conversation for memory distillation.
    if assistant_owns_cancel {
        crate::assistant::dismiss_voice_overlay(app);
    }

    // A dictation cancelled mid-recording is kept (History, marked dismissed)
    // and offered back on the pill for a few seconds, which replaces the hide.
    let offered = origin == CancelOrigin::User
        && cancelled
            .map(|recording| crate::actions::keep_cancelled_recording(app, recording))
            .unwrap_or(false);

    // Update tray icon and hide overlay
    change_tray_icon(app, crate::tray::TrayIconState::Idle);
    if !offered {
        hide_recording_overlay(app);
        // Nothing will be pasted, so the window this recording was aimed at
        // stops being a restore target. Keeping it would mean a later paste
        // could hand the foreground to a window the user has since abandoned.
        // An Undo on the pill is the one paste still meant for it; that offer
        // forgets it when it expires (`dictation_recovery::expire`).
        crate::input::forget_paste_target();
    }

    // Unload model if immediate unload is enabled
    let tm = app.state::<Arc<TranscriptionManager>>();
    // Cancel any active live/streaming transcription worker so it releases the
    // leased model engine. No-op when live transcription isn't active.
    tm.cancel_stream();
    tm.maybe_unload_immediately("cancellation");

    // Notify coordinator so it can keep lifecycle state coherent. An aborted
    // press comes from the coordinator, which is already idle.
    if origin == CancelOrigin::User {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            coordinator.notify_cancel(recording_was_active);
        }
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

/// Check if running on the GNOME desktop environment.
/// `XDG_CURRENT_DESKTOP` can be colon-separated (e.g. "ubuntu:GNOME").
#[cfg(target_os = "linux")]
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("GNOME"))
        .unwrap_or(false)
}

/// Check if running on GNOME with Wayland.
///
/// True even when `main.rs` has moved *this process* onto XWayland for the
/// overlay (`GDK_BACKEND=x11`): that only changes how our own windows are
/// drawn. The session is still Wayland (`WAYLAND_DISPLAY` stays set) and the
/// apps being pasted into are still native Wayland clients.
#[cfg(target_os = "linux")]
pub fn is_gnome_wayland() -> bool {
    is_wayland() && is_gnome()
}
