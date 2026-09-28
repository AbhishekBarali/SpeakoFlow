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

/// How long a superseded utterance stays eligible to be carried into the next
/// turn. Long enough to cover a pause plus the transcription that follows it,
/// short enough that a sentence abandoned when the user muted and walked away
/// does not reappear in front of whatever they say when they come back.
const CARRY_FORWARD_WINDOW: std::time::Duration = std::time::Duration::from_secs(45);
/// Upper bound on carried-over speech, so a run of superseded utterances cannot
/// grow one question without limit.
const MAX_CARRIED_CHARS: usize = 4_000;

#[derive(Default)]
struct Session {
    serial: u32,
    ticket: Option<VoiceTicket>,
    submitted: Option<VoiceTicket>,
    /// Speech that was transcribed for a turn which no longer exists, because
    /// the user carried on talking before it could be asked. It is still
    /// something they said, so it waits here for the next turn instead of being
    /// dropped.
    carried: Vec<String>,
    carried_at: Option<std::time::Instant>,
    /// Whether this call reads its replies aloud. The call bar's speaker switch;
    /// off turns the call into voice-in, text-out without touching the
    /// microphone. Seeded from `assistant_tts_enabled` when the call starts.
    speaker_on: bool,
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

    /// Open a new turn in `session`, which makes everything still in flight for
    /// the previous one stale: its transcript is carried, its reply and its
    /// audio are dropped. Speech onset, a typed message, New chat and opening a
    /// saved conversation all start here.
    fn advance_turn(&self, session: u32) -> Result<VoiceTicket, String> {
        let mut s = self
            .session
            .lock()
            .map_err(|_| "Voice session lock unavailable")?;
        let ticket = s
            .ticket
            .as_mut()
            .filter(|t| t.session == session)
            .ok_or("Voice session ended")?;
        ticket.turn = ticket.turn.wrapping_add(1);
        Ok(*ticket)
    }

    /// Whether the live call speaks its replies. `false` when no call is live.
    pub fn speaker_on(&self) -> bool {
        self.session
            .lock()
            .map(|s| s.ticket.is_some() && s.speaker_on)
            .unwrap_or(false)
    }

    /// Flip the speaker switch for `session`. Returns whether it applied.
    fn set_speaker(&self, session: u32, on: bool) -> bool {
        self.session
            .lock()
            .map(|mut s| {
                if s.ticket.is_none_or(|t| t.session != session) {
                    return false;
                }
                s.speaker_on = on;
                true
            })
            .unwrap_or(false)
    }

    /// Hold a transcript whose turn was superseded, so the next turn asks about
    /// it too.
    ///
    /// A pause longer than the pace setting ends the utterance and starts a
    /// turn, so "my name is Abhishek — <pause> — can you help me with this?" is
    /// two utterances. Resuming speech supersedes the first one's turn, and
    /// transcription almost always loses that race (it is still running when the
    /// user's next word arrives, and on a cloud engine or a cold local model it
    /// is not close). Dropping the result there is what made the assistant
    /// answer only the last thing said before it stopped listening.
    fn carry(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if let Ok(mut session) = self.session.lock() {
            // Only speech from the live session, and only while it is still
            // plausibly part of what the user is saying now.
            if session.ticket.is_none() {
                return;
            }
            if session
                .carried_at
                .is_some_and(|at| at.elapsed() > CARRY_FORWARD_WINDOW)
            {
                session.carried.clear();
            }
            let held: usize = session.carried.iter().map(String::len).sum();
            if held + text.len() > MAX_CARRIED_CHARS {
                // Keep the newest speech: it is the part still being finished.
                while !session.carried.is_empty()
                    && session.carried.iter().map(String::len).sum::<usize>() + text.len()
                        > MAX_CARRIED_CHARS
                {
                    session.carried.remove(0);
                }
            }
            session.carried.push(text.to_string());
            session.carried_at = Some(std::time::Instant::now());
        }
    }

    /// Fold any recent carried speech in front of `text` for the turn that is
    /// about to run, and forget it either way.
    fn with_carried(&self, text: &str) -> String {
        let carried = self
            .session
            .lock()
            .map(|mut session| {
                let fresh = session
                    .carried_at
                    .is_some_and(|at| at.elapsed() <= CARRY_FORWARD_WINDOW);
                session.carried_at = None;
                let carried = std::mem::take(&mut session.carried);
                if fresh {
                    carried
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();
        if carried.is_empty() {
            return text.trim().to_string();
        }
        // Spoken fragments of one thought: join them the way they were said.
        let mut joined = carried.join(" ");
        joined.push(' ');
        joined.push_str(text.trim());
        joined.trim().to_string()
    }

    fn clear_carried(&self) {
        if let Ok(mut session) = self.session.lock() {
            session.carried.clear();
            session.carried_at = None;
        }
    }
}

pub fn is_current(app: &AppHandle, ticket: VoiceTicket) -> bool {
    app.state::<VoiceConversation>().is_current(ticket)
}

pub fn is_active(app: &AppHandle) -> bool {
    app.try_state::<VoiceConversation>()
        .is_some_and(|voice| voice.is_active())
}

/// Whether the live call reads its replies aloud (the call bar's speaker
/// switch). `false` when there is no call.
pub fn speaker_on(app: &AppHandle) -> bool {
    app.try_state::<VoiceConversation>()
        .is_some_and(|voice| voice.speaker_on())
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
        voice.clear_carried();
        // Hanging up while the assistant is talking cuts the answer off just as
        // a barge-in does, so the record has to say so too — otherwise the next
        // turn assumes the user heard a reply they only heard the start of.
        if !app.state::<AssistantConversation>().is_busy() {
            mark_last_reply_interrupted(app);
        }
        app.state::<AssistantConversation>().request_cancel();
        crate::tts::stop_all(app);
        let _ = app.emit("assistant-conversation-ended", ticket.session);
        // Hand the window back to the chat panel's size lane: the pill or the
        // Live overlay when collapsed, the remembered chat size when expanded.
        assistant::leave_conversation_size(app);
        // A finished call is a finished conversation: learn from it the same way
        // closing the panel on a typed chat does. Closing the panel *during* a
        // call deliberately defers to here, so this is the only place a spoken
        // conversation can reach memory.
        assistant::distill_conversation_if_ended(app);
        // And then the call takes its transcript with it.
        //
        // Without this, pressing End was the shortest route back to the bug this
        // separation exists to kill. `end_session` cleared the ticket and left the
        // window up, so the panel re-rendered as the quick-ask card fed from the
        // same message list — the last thing said on the call appeared in the card
        // the instant you hung up, and because the window was still visible the
        // next quick ask counted as a follow-up and carried the whole call with
        // it. Only the X escaped, because that hides the window.
        //
        // Distillation runs first, so memory still learns from the call.
        assistant::reset_conversation_for_new_exchange(app);
    }
}

/// Note on the last recorded answer that the user may not have heard all of it.
///
/// Generation can finish before playback, so the full text is worth keeping
/// readable — but a later turn must not treat it as delivered. Returns whether
/// anything changed, so callers only pay for an emit + save when it did.
fn mark_last_reply_interrupted(app: &AppHandle) -> bool {
    let marked = app
        .state::<AssistantConversation>()
        .messages
        .lock()
        .map(|mut history| match history.last_mut() {
            Some(last)
                if last.role == "assistant" && !last.content.ends_with(INTERRUPTED_MARKER) =>
            {
                last.content.push_str(&format!("\n{}", INTERRUPTED_MARKER));
                true
            }
            _ => false,
        })
        .unwrap_or(false);
    if marked {
        assistant::emit_conversation(app);
        assistant::persist_assistant_session(app);
    }
    marked
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
    // A call speaks every reply whether or not spoken replies are switched on,
    // so a voice engine that cannot synthesize has to be caught here. Left to
    // the turn, it fails once per utterance with a generic provider error and no
    // reachable explanation — the panel's error banner is hidden behind the orb.
    if let Some(blocker) = crate::tts::voice_engine_blocker(&settings) {
        return Err(blocker);
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
        s.carried.clear();
        s.carried_at = None;
        s.speaker_on = settings.assistant_tts_enabled;
        ticket
    };
    // A call is its own conversation, so it starts from nothing.
    //
    // This is the other half of the quick-ask separation (see
    // `assistant::should_reset_quick_ask`): the two features shared one message
    // list, so a call opened after a few quick asks began with those asks as its
    // history and answered its first utterance as though it were mid-thread. Same
    // single cause, opposite direction.
    //
    // Deliberately *after* the ticket is claimed, which is the last point this
    // function can still fail. Run any earlier and the "a conversation is already
    // active" rejection would wipe the live call's history on its way out — and
    // the frontend fires an unawaited `end()` immediately before `start()`, so
    // that race is reachable rather than hypothetical.
    assistant::reset_conversation_for_new_exchange(&app);
    // Voice has its own window size (see `assistant::enter_conversation_size`).
    // Do this with the ticket in hand so the orb view is never laid out inside
    // a window still sized for the message list.
    assistant::enter_conversation_size(&app);
    crate::tts::stop_all(&app);
    app.state::<Arc<TranscriptionManager>>()
        .initiate_model_load();
    // A native voice loads in ~0.3 s (Kitten) to ~1.2 s (Kokoro) and pays ONNX
    // Runtime's first-run cost on its first sentence; do both while the user is
    // still speaking.
    crate::native_tts::prewarm(&settings);
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
///
/// `async` on purpose: a synchronous command runs on the main thread, and this
/// one can write the whole conversation to disk. Doing that at the instant the
/// user starts talking stalled the event loop — a visible hitch in the orb and
/// in window dragging on any conversation long enough to matter.
#[tauri::command]
#[specta::specta]
pub async fn assistant_conversation_interrupt(
    app: AppHandle,
    session: u32,
    interrupted_reply: bool,
) -> Result<VoiceTicket, String> {
    let voice = app.state::<VoiceConversation>();
    let ticket = voice.advance_turn(session)?;
    app.state::<AssistantConversation>().request_cancel();
    crate::tts::stop_all(&app);
    // Generation can finish before playback. Keep that full answer readable,
    // but tell the next turn it cannot assume the listener heard all of it.
    if interrupted_reply && !app.state::<AssistantConversation>().is_busy() {
        mark_last_reply_interrupted(&app);
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
    log::debug!(
        "Voice conversation STT: {}ms",
        started.elapsed().as_millis()
    );
    // Room noise and VAD misfires reach a local engine as `[BLANK_AUDIO]` or a
    // bracketed annotation rather than as an empty string, so without this a
    // cough spends a whole generation — and, on the built-in engine, a cold
    // model load first — answering something nobody said.
    if crate::audio_toolkit::is_speechless_transcription(&text) {
        log::debug!("Voice utterance had no speech ({text:?}); nothing to ask");
        return Ok(());
    }
    if !voice.is_current(ticket) {
        // Superseded before it could be asked. Keep the words for the next turn.
        voice.carry(&text);
        return Ok(());
    }
    match wait_for_turn_slot(&app, &voice, ticket).await {
        TurnSlot::Ready => {}
        TurnSlot::Superseded => {
            voice.carry(&text);
            return Ok(());
        }
        TurnSlot::TimedOut => {
            voice.carry(&text);
            return Err(STILL_STOPPING.into());
        }
    }
    assistant::run_conversation_turn(app.clone(), voice.with_carried(&text), ticket).await;
    Ok(())
}

const STILL_STOPPING: &str = "The previous reply is still stopping. Please try again.";

/// How long a new turn waits for a cancelled one to let go of the shared busy
/// guard. A cancelled model or tool future normally unwinds in milliseconds.
const TURN_SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

enum TurnSlot {
    Ready,
    /// A newer turn was opened while this one waited.
    Superseded,
    TimedOut,
}

/// Wait for the previous turn to release the assistant, rechecking the ticket
/// on every wake so a superseded turn never starts.
async fn wait_for_turn_slot(
    app: &AppHandle,
    voice: &VoiceConversation,
    ticket: VoiceTicket,
) -> TurnSlot {
    let deadline = tokio::time::Instant::now() + TURN_SLOT_WAIT;
    while app.state::<AssistantConversation>().is_busy() {
        if !voice.is_current(ticket) {
            return TurnSlot::Superseded;
        }
        if tokio::time::Instant::now() >= deadline {
            return TurnSlot::TimedOut;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    if voice.is_current(ticket) {
        TurnSlot::Ready
    } else {
        TurnSlot::Superseded
    }
}

/// Longest message the call bar will send. Generous for anything typed; the
/// bound exists so a paste of a whole document cannot become one turn by
/// accident.
const MAX_TYPED_CHARS: usize = 8_000;

/// A typed message in a live call.
///
/// Same turn discipline as speech: the frontend opens the turn with
/// `assistant_conversation_interrupt` first, so a reply in flight is cut off
/// exactly as if the user had started talking, and the reply to this message is
/// spoken or not by the same speaker switch. Nothing is carried in front of it —
/// a sentence abandoned mid-air is not part of something the user chose to type.
#[tauri::command]
#[specta::specta]
pub async fn assistant_conversation_text(
    app: AppHandle,
    session: u32,
    turn: u32,
    text: String,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Type a message first".into());
    }
    if text.chars().count() > MAX_TYPED_CHARS {
        return Err(format!(
            "That message is too long to send in a call ({MAX_TYPED_CHARS} characters at most)."
        ));
    }
    let ticket = VoiceTicket { session, turn };
    let voice = app.state::<VoiceConversation>();
    let _processing = voice.processing.lock().await;
    if !voice.claim(ticket) {
        return Ok(());
    }
    voice.clear_carried();
    match wait_for_turn_slot(&app, &voice, ticket).await {
        TurnSlot::Ready => {}
        TurnSlot::Superseded => return Ok(()),
        TurnSlot::TimedOut => return Err(STILL_STOPPING.into()),
    }
    assistant::run_conversation_turn(app.clone(), text, ticket).await;
    Ok(())
}

/// Put a live call between conversations: stop what it is saying and thinking,
/// and make everything still in flight for the old conversation stale, so a
/// late transcript or reply cannot land in the new one.
async fn settle_call_for_switch(app: &AppHandle, session: u32) -> Result<(), String> {
    let voice = app.state::<VoiceConversation>();
    voice.advance_turn(session)?;
    voice.clear_carried();
    app.state::<AssistantConversation>().request_cancel();
    crate::tts::stop_all(app);
    let deadline = tokio::time::Instant::now() + TURN_SLOT_WAIT;
    while app.state::<AssistantConversation>().is_busy() {
        if tokio::time::Instant::now() >= deadline {
            return Err(STILL_STOPPING.into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Ok(())
}

/// Start a new conversation without hanging up (the call bar's New chat).
///
/// The one that just finished is saved and distilled like any other ended
/// conversation; the microphone, the speaker switch and the window stay as
/// they are.
#[tauri::command]
#[specta::specta]
pub async fn assistant_conversation_new(app: AppHandle, session: u32) -> Result<(), String> {
    settle_call_for_switch(&app, session).await?;
    assistant::reset_conversation_for_new_exchange(&app);
    Ok(())
}

/// Continue a saved conversation inside the live call.
///
/// This is what makes history reachable from a call at all. The only way back
/// to an old conversation used to be History → Continue, which hung the call up
/// first and then reopened the thread as a quick-ask card showing one exchange.
/// Here the thread replaces the call's conversation in place, the next thing
/// said is appended to it, and later turns keep updating the same History row.
#[tauri::command]
#[specta::specta]
pub async fn assistant_conversation_load(
    app: AppHandle,
    session: u32,
    id: i64,
) -> Result<(), String> {
    let history = app
        .try_state::<Arc<crate::managers::history::HistoryManager>>()
        .ok_or("History unavailable")?
        .inner()
        .clone();
    let entry = history
        .get_assistant_session(id)
        .map_err(|e| format!("Couldn't load the conversation: {e}"))?
        .ok_or("That conversation no longer exists.")?;
    settle_call_for_switch(&app, session).await?;
    assistant::adopt_saved_conversation(&app, entry.id, entry.messages);
    Ok(())
}

/// Continue a saved conversation inside the live call from one chosen message, as
/// a new branch (History → "Continue from here"). The original row is never
/// modified: the branch has no History id, so its next turn saves a new one.
#[tauri::command]
#[specta::specta]
pub async fn assistant_conversation_branch(
    app: AppHandle,
    session: u32,
    id: i64,
    message_index: usize,
) -> Result<(), String> {
    let history = app
        .try_state::<Arc<crate::managers::history::HistoryManager>>()
        .ok_or("History unavailable")?
        .inner()
        .clone();
    let entry = history
        .get_assistant_session(id)
        .map_err(|e| format!("Couldn't load the conversation: {e}"))?
        .ok_or("That conversation no longer exists.")?;
    let branched = assistant::branch_messages(entry.messages, message_index);
    if branched.is_empty() {
        return Err("There's nothing to continue from there.".into());
    }
    settle_call_for_switch(&app, session).await?;
    assistant::adopt_branch(&app, branched);
    Ok(())
}

/// The call bar's speaker switch.
///
/// Off stops what is being read out right now but lets the reply finish as
/// text, and later replies are not synthesized at all. The microphone is not
/// touched: that is the mute button's job, and the two used to be tangled into
/// one "sound off" switch that closed the microphone as well.
#[tauri::command]
#[specta::specta]
pub fn assistant_conversation_set_speaker(
    app: AppHandle,
    session: u32,
    on: bool,
) -> Result<(), String> {
    let voice = app.state::<VoiceConversation>();
    if !voice.set_speaker(session, on) {
        return Err("Voice session ended".into());
    }
    if !on {
        crate::tts::stop_all(&app);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_session(turn: u32) -> VoiceConversation {
        let voice = VoiceConversation::default();
        voice.session.lock().unwrap().ticket = Some(VoiceTicket { session: 1, turn });
        voice
    }

    /// The bug this exists for: a pause longer than the pace setting splits one
    /// thought into two utterances, and resuming speech supersedes the first
    /// one's turn. Its transcript used to be dropped, so the assistant answered
    /// only the words spoken after the pause.
    #[test]
    fn speech_superseded_before_its_turn_is_carried_into_the_next_one() {
        let voice = live_session(1);
        voice.carry("my name is Abhishek");
        voice.session.lock().unwrap().ticket = Some(VoiceTicket {
            session: 1,
            turn: 2,
        });
        assert_eq!(
            voice.with_carried("can you help me with this?"),
            "my name is Abhishek can you help me with this?"
        );
        // Taken once: the next turn starts from what was actually said then.
        assert_eq!(voice.with_carried("and this too"), "and this too");
    }

    #[test]
    fn several_superseded_fragments_are_asked_in_the_order_they_were_spoken() {
        let voice = live_session(1);
        voice.carry("first");
        voice.carry("second");
        assert_eq!(voice.with_carried("third"), "first second third");
    }

    /// Muting or walking away leaves an abandoned fragment behind. It must not
    /// reappear in front of whatever is said when the user comes back.
    #[test]
    fn stale_speech_is_not_carried_into_a_later_question() {
        let voice = live_session(1);
        voice.carry("something from ages ago");
        voice.session.lock().unwrap().carried_at = Some(
            std::time::Instant::now() - (CARRY_FORWARD_WINDOW + std::time::Duration::from_secs(1)),
        );
        assert_eq!(voice.with_carried("what time is it?"), "what time is it?");
    }

    #[test]
    fn carried_speech_is_bounded_and_belongs_to_a_live_session() {
        let voice = live_session(1);
        let chunk = "x".repeat(1_000);
        for _ in 0..10 {
            voice.carry(&chunk);
        }
        let held: usize = voice
            .session
            .lock()
            .unwrap()
            .carried
            .iter()
            .map(String::len)
            .sum();
        assert!(held <= MAX_CARRIED_CHARS, "carried {held} chars");

        // Nothing is retained once the session is over.
        voice.clear_carried();
        voice.session.lock().unwrap().ticket = None;
        voice.carry("after the call ended");
        assert!(voice.session.lock().unwrap().carried.is_empty());
    }

    /// Opening a turn (speech, a typed message, New chat, opening a saved
    /// conversation) must make the previous turn's results unacceptable, and must
    /// refuse a session that is not the live one.
    #[test]
    fn a_new_turn_makes_the_previous_one_stale() {
        let voice = live_session(4);
        let old = VoiceTicket {
            session: 1,
            turn: 4,
        };
        assert!(voice.is_current(old));
        let next = voice.advance_turn(1).unwrap();
        assert_eq!(next.turn, 5);
        assert!(!voice.is_current(old));
        assert!(voice.is_current(next));
        assert!(voice.advance_turn(2).is_err(), "another session's call");
        voice.session.lock().unwrap().ticket = None;
        assert!(voice.advance_turn(1).is_err(), "no call at all");
    }

    /// The speaker switch belongs to the live session. It must read as off
    /// once the call is over, so a later quick ask can never be spoken because
    /// a call that ended had its speaker on.
    #[test]
    fn the_speaker_switch_belongs_to_the_live_call() {
        let voice = live_session(1);
        assert!(!voice.speaker_on(), "seeded by start, not by default");
        assert!(voice.set_speaker(1, true));
        assert!(voice.speaker_on());
        assert!(!voice.set_speaker(2, false), "another session's switch");
        assert!(voice.speaker_on());
        assert!(voice.set_speaker(1, false));
        assert!(!voice.speaker_on());
        assert!(voice.set_speaker(1, true));
        voice.session.lock().unwrap().ticket = None;
        assert!(!voice.speaker_on(), "no call, no speaker");
        assert!(!voice.set_speaker(1, true));
    }

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
