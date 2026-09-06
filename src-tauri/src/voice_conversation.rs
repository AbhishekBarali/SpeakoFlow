//! Opt-in, panel-owned hands-free sessions. Session/turn tickets make late STT
//! results and synthesized audio harmless after interruption, mute, or End.
use crate::assistant::{self, AssistantConversation};
use crate::managers::transcription::TranscriptionManager;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// What the app knows that the model cannot infer from the transcript alone:
/// this turn is spoken aloud rather than displayed, and it can be cut off
/// mid-sentence. Everything else — tone, length, whether to ask a follow-up,
/// how to sound in a conversation — belongs to the profile the user chose and
/// to the model itself. Earlier versions dictated all of that here ("start with
/// a short, useful sentence", "no stock acknowledgments or filler", "ask at most
/// one follow-up question"), which overrode the user's own persona and made
/// capable models talk like a support script.
const VOICE_PROMPT: &str = "This conversation is spoken: your reply is read aloud, so write it as speech rather than as a document — no markdown, headings, code blocks, or bullet lists. The user hears you in real time and can start talking over you at any point; when they do, answer what they just said instead of restarting your previous answer. A previous reply of yours marked as interrupted may contain words they never heard.";

/// Voice changes the delivery medium, not the user's length preference: the
/// length dial (profile override, else the global setting) still decides how
/// long a reply is. Only on Default does voice add a hint, because an unbounded
/// essay is unusable when it arrives one sentence at a time through a speaker.
pub fn voice_prompt(length: crate::settings::AssistantResponseLength) -> String {
    if length == crate::settings::AssistantResponseLength::Default {
        format!("{VOICE_PROMPT} Spoken answers work best when they stay conversational in length — say what is needed and leave room for the user to reply, then go into as much detail as they ask for.")
    } else {
        VOICE_PROMPT.to_string()
    }
}
pub const INTERRUPTED_MARKER: &str =
    "[Voice reply interrupted; some of this answer may not have been heard.]";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct VoiceTicket {
    pub session: u32,
    pub turn: u32,
}

#[derive(Default)]
struct Session {
    serial: u32,
    ticket: Option<VoiceTicket>,
    submitted: Option<VoiceTicket>,
}

#[derive(Default)]
pub struct VoiceConversation {
    session: Mutex<Session>,
    // The STT engine is shared with dictation. Never queue multiple voice turns
    // inside its blocking inference lock; discard superseded work first.
    processing: tokio::sync::Mutex<()>,
}

impl VoiceConversation {
    /// Each detected utterance can be submitted once. A retransmitted IPC body
    /// must not append the same user message twice after the first turn ends.
    fn claim(&self, ticket: VoiceTicket) -> bool {
        self.session
            .lock()
            .map(|mut s| {
                if s.ticket != Some(ticket) || s.submitted == Some(ticket) {
                    return false;
                }
                s.submitted = Some(ticket);
                true
            })
            .unwrap_or(false)
    }
    pub fn is_active(&self) -> bool {
        self.session
            .lock()
            .map(|s| s.ticket.is_some())
            .unwrap_or(false)
    }

    pub fn is_current(&self, ticket: VoiceTicket) -> bool {
        self.session
            .lock()
            .map(|s| s.ticket == Some(ticket))
            .unwrap_or(false)
    }
}

pub fn is_current(app: &AppHandle, ticket: VoiceTicket) -> bool {
    app.state::<VoiceConversation>().is_current(ticket)
}

pub fn is_active(app: &AppHandle) -> bool {
    app.try_state::<VoiceConversation>()
        .is_some_and(|voice| voice.is_active())
}

/// End on an explicit stop or destruction, independently of panel visibility.
pub fn end(app: &AppHandle) {
    end_session(app, None);
}

fn end_session(app: &AppHandle, expected_session: Option<u32>) {
    let Some(voice) = app.try_state::<VoiceConversation>() else {
        return;
    };
    let ended = voice.session.lock().ok().and_then(|mut s| {
        if expected_session.is_some_and(|id| s.ticket.is_none_or(|t| t.session != id)) {
            return None;
        }
        s.ticket.take()
    });
    if let Some(ticket) = ended {
        app.state::<AssistantConversation>().request_cancel();
        crate::tts::stop_all(app);
        let _ = app.emit("assistant-conversation-ended", ticket.session);
        // Hand the window back to the chat panel's size lane: the pill or the
        // Live overlay when collapsed, the remembered chat size when expanded.
        assistant::leave_conversation_size(app);
    }
}

#[tauri::command]
#[specta::specta]
pub async fn assistant_conversation_start(app: AppHandle) -> Result<VoiceTicket, String> {
    let settings = crate::settings::get_settings(&app);
    if !settings.assistant_enabled {
        return Err("The assistant is switched off".into());
    }
    if settings.active_character_is_cat() {
        return Err("Choose a conversational profile to start voice conversation".into());
    }
    let provider = settings
        .active_assistant_provider()
        .ok_or("Choose an assistant provider in Settings")?;
    if settings
        .assistant_models
        .get(&provider.id)
        .is_none_or(|m| m.trim().is_empty())
    {
        return Err("Choose an assistant model in Settings".into());
    }
    if app
        .state::<Arc<crate::managers::audio::AudioRecordingManager>>()
        .is_recording()
    {
        return Err("Finish the current recording first".into());
    }
    if app.state::<AssistantConversation>().is_busy() {
        return Err("Stop the current reply before starting conversation".into());
    }
    let voice = app.state::<VoiceConversation>();
    let ticket = {
        let mut s = voice
            .session
            .lock()
            .map_err(|_| "Voice session lock unavailable")?;
        if s.ticket.is_some() {
            return Err("A voice conversation is already active".into());
        }
        s.serial = s.serial.wrapping_add(1);
        let ticket = VoiceTicket {
            session: s.serial,
            turn: 0,
        };
        s.ticket = Some(ticket);
        ticket
    };
    // Voice has its own window size (see `assistant::enter_conversation_size`).
    // Do this with the ticket in hand so the orb view is never laid out inside
    // a window still sized for the message list.
    assistant::enter_conversation_size(&app);
    crate::tts::stop_all(&app);
    app.state::<Arc<TranscriptionManager>>()
        .initiate_model_load();
    if provider.id == "builtin" {
        if let Some(model) = settings.assistant_models.get(&provider.id) {
            crate::actions::prewarm_assistant_llm(&app, model.clone());
        }
    }
    Ok(ticket)
}

#[tauri::command]
#[specta::specta]
pub fn assistant_conversation_end(app: AppHandle, session: u32) {
    end_session(&app, Some(session));
}

/// Switch between the orb view and the larger transcript-reading form. Window
/// size and view layout are one state, so whichever control grows the window is
/// also the control that shrinks it back.
#[tauri::command]
#[specta::specta]
pub fn assistant_conversation_set_expanded(app: AppHandle, expanded: bool) {
    assistant::set_conversation_expanded(&app, expanded);
}

/// Speech onset (also used by mute): invalidate pending inference and audio
/// before the new utterance finishes. Returns the new turn's cancellation key.
#[tauri::command]
#[specta::specta]
pub fn assistant_conversation_interrupt(
    app: AppHandle,
    session: u32,
    interrupted_reply: bool,
) -> Result<VoiceTicket, String> {
    let voice = app.state::<VoiceConversation>();
    let ticket = {
        let mut s = voice
            .session
            .lock()
            .map_err(|_| "Voice session lock unavailable")?;
        let ticket = s
            .ticket
            .as_mut()
            .filter(|t| t.session == session)
            .ok_or("Voice session ended")?;
        ticket.turn = ticket.turn.wrapping_add(1);
        *ticket
    };
    app.state::<AssistantConversation>().request_cancel();
    crate::tts::stop_all(&app);
    // Generation can finish before playback. Keep that full answer readable,
    // but tell the next turn it cannot assume the listener heard all of it.
    if interrupted_reply && !app.state::<AssistantConversation>().is_busy() {
        if let Ok(mut history) = app.state::<AssistantConversation>().messages.lock() {
            if let Some(last) = history.last_mut().filter(|m| m.role == "assistant") {
                if !last.content.ends_with(INTERRUPTED_MARKER) {
                    last.content.push_str(&format!("\n{}", INTERRUPTED_MARKER));
                }
            }
        }
        assistant::emit_conversation(&app);
        assistant::persist_assistant_session(&app);
    }
    Ok(ticket)
}

/// Raw little-endian mono f32 PCM at 16 kHz. Binary IPC avoids expanding every
/// sample into JSON. The microphone/VAD stays local until a complete utterance.
#[tauri::command]
pub async fn assistant_conversation_audio(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let header = |name: &str| -> Result<u32, String> {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("Missing {name}"))
    };
    let ticket = VoiceTicket {
        session: header("x-voice-session")?,
        turn: header("x-voice-turn")?,
    };
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected PCM audio".into());
    };
    if bytes.len() < 1600 * 4 || bytes.len() > 16_000 * 65 * 4 || bytes.len() % 4 != 0 {
        return Err("Voice utterance must be between 0.1 and 65 seconds".into());
    }
    let audio: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    if audio.iter().any(|s| !s.is_finite() || s.abs() > 1.0) {
        return Err("Invalid PCM sample".into());
    }
    let voice = app.state::<VoiceConversation>();
    let _processing = voice.processing.lock().await;
    if !voice.claim(ticket) {
        return Ok(());
    }
    let tm = app.state::<Arc<TranscriptionManager>>().inner().clone();
    let started = std::time::Instant::now();
    let text = tauri::async_runtime::spawn_blocking(move || tm.transcribe(audio))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if !voice.is_current(ticket) || text.trim().is_empty() {
        return Ok(());
    }
    log::debug!(
        "Voice conversation STT: {}ms",
        started.elapsed().as_millis()
    );
    // Let a cancelled tool/model future release the shared turn guard. The
    // ticket is rechecked on every wake; stale audio never becomes a new turn.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.state::<AssistantConversation>().is_busy() {
        if !voice.is_current(ticket) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("The previous reply is still stopping. Please try again.".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assistant::run_conversation_turn(app.clone(), text, ticket).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_style_does_not_override_an_explicit_response_length() {
        use crate::settings::AssistantResponseLength;
        assert!(voice_prompt(AssistantResponseLength::Default).contains("conversational in length"));
        for length in [
            AssistantResponseLength::Short,
            AssistantResponseLength::Medium,
            AssistantResponseLength::Long,
        ] {
            let prompt = voice_prompt(length);
            assert!(!prompt.contains("conversational in length"));
            assert!(prompt.contains("read aloud"));
        }
    }

    /// The prompt carries only what the app knows — spoken medium, barge-in —
    /// and leaves persona, tone and pacing to the user's profile and the model.
    #[test]
    fn voice_prompt_does_not_dictate_conversational_style() {
        use crate::settings::AssistantResponseLength;
        for length in [
            AssistantResponseLength::Default,
            AssistantResponseLength::Short,
            AssistantResponseLength::Medium,
            AssistantResponseLength::Long,
        ] {
            let prompt = voice_prompt(length).to_lowercase();
            for banned in [
                "one to three",
                "stock acknowledgment",
                "filler",
                "at most one follow-up",
                "start with a short",
                "yield the floor",
            ] {
                assert!(
                    !prompt.contains(banned),
                    "voice prompt should not prescribe style: {banned}"
                );
            }
            // What it must still say: this is speech, and it can be cut off.
            assert!(prompt.contains("read aloud"));
            assert!(prompt.contains("no markdown"));
            assert!(prompt.contains("interrupted"));
        }
    }
    #[test]
    fn old_audio_cannot_cross_turn_or_session_boundaries() {
        let voice = VoiceConversation::default();
        let first = VoiceTicket {
            session: 1,
            turn: 1,
        };
        voice.session.lock().unwrap().ticket = Some(first);
        assert!(voice.is_current(first));
        voice.session.lock().unwrap().ticket = Some(VoiceTicket {
            session: 1,
            turn: 2,
        });
        assert!(!voice.is_current(first));
        voice.session.lock().unwrap().ticket = None;
        assert!(!voice.is_active());
        voice.session.lock().unwrap().ticket = Some(VoiceTicket {
            session: 2,
            turn: 1,
        });
        assert!(!voice.is_current(first));
    }

    #[test]
    fn duplicate_audio_is_dropped_but_the_next_utterance_is_accepted() {
        let voice = VoiceConversation::default();
        let first = VoiceTicket {
            session: 1,
            turn: 1,
        };
        voice.session.lock().unwrap().ticket = Some(first);
        assert!(voice.claim(first));
        assert!(!voice.claim(first));
        let next = VoiceTicket {
            session: 1,
            turn: 2,
        };
        voice.session.lock().unwrap().ticket = Some(next);
        assert!(!voice.claim(first));
        assert!(voice.claim(next));
    }
}
