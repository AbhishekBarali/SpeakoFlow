//! Tauri commands for the meetings feature.
//!
//! Thin by design: every decision that could be wrong lives in
//! [`crate::meetings`] — the store owns SQL and file ownership, the recorder owns
//! the capture state machine, `summarize` owns the map-reduce. What is left here
//! is argument validation, moving blocking work off the async runtime, and the
//! two things a command is uniquely responsible for:
//!
//! * **Unlinking audio after a delete.** `MeetingStore::delete_meeting` commits
//!   the row removal and hands back the paths, deliberately without touching the
//!   filesystem, so a failed commit can never leave a meeting that opens to
//!   nothing. The caller — here — is what actually unlinks.
//! * **Blocking calls go through `spawn_blocking`.** Starting a meeting opens
//!   audio devices, and stopping one joins the transcription worker, which first
//!   drains every queued chunk through Whisper. That is seconds to minutes of
//!   CPU work, and running it on the async runtime would stall the whole IPC
//!   layer — including the very UI waiting for the stop to finish.
//!
//! Note what is *not* here: nothing in this module ever combines the microphone
//! and system-audio sides. Segments arrive tagged with the
//! [`SpeakerSource`](crate::meetings::SpeakerSource) they were captured on, and
//! that tag is the speaker attribution. Presentation joins the two streams by
//! timestamp for reading; storage and this API keep them distinct.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::meetings::session::{MeetingRecorder, MeetingState};
use crate::meetings::store::{MeetingStore, SEGMENT_PAGE_SIZE};
use crate::meetings::summarize::{
    generate_meeting_notes as run_notes_job, GeneratedNotes, NotesTemplate,
};
use crate::meetings::{Meeting, MeetingSpeaker, PaginatedMeetings, PaginatedSegments};

/// Emitted whenever the meeting list changes in a way a list view cannot infer
/// from its own action — a delete, a rename, or notes landing on a row. The
/// meetings panel refetches the current page on this.
///
/// Mirrors `history-retention-applied`: a plain string event rather than a
/// `tauri_specta::Event`, because the payload is empty and the listener is a
/// single panel.
pub const MEETINGS_UPDATED_EVENT: &str = "meetings-updated";

/// Default page size for the meeting list.
///
/// Meetings are long and few — tens per month, not thousands like dictation
/// history — so one screenful plus scroll headroom is the right first page.
const MEETING_PAGE_SIZE: u32 = 30;

/// Upper bound on any page request.
///
/// A page size is a UI concern, but an unbounded one from the frontend turns
/// into "load a two-hour transcript in one query", which is exactly what the
/// pagination exists to prevent.
const MAX_PAGE_SIZE: u32 = 500;

/// Whether this machine can capture the other participants at all.
///
/// Two fields rather than one bool because "no" has a cause the user can act on
/// exactly once: on macOS the missing piece is a permission that is *separate*
/// from screen recording, and without it the OS records silence and never
/// prompts. Surfacing the reason is the difference between a fixable setup step
/// and a feature that appears broken.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct SystemAudioStatus {
    /// True when a loopback source exists on this platform and build.
    pub supported: bool,
    /// Platform-specific guidance when capture needs the user to grant
    /// something. `None` when there is nothing for them to do.
    pub help: Option<String>,
}

/// Clamp a caller-supplied page size to something a query can serve.
fn page_size(requested: Option<u32>, default: u32) -> u32 {
    requested
        .unwrap_or(default)
        // 0 would silently return an empty page forever, which reads as "the
        // meeting has no transcript" rather than as a bad argument.
        .clamp(1, MAX_PAGE_SIZE)
}

/// Reject a blank title/name before it reaches the database.
///
/// Trimmed rather than rejected outright where a fallback exists; an empty
/// string is only ever an accident, and storing one produces a row the user
/// cannot identify in a list.
fn require_text(value: &str, what: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{what} cannot be empty."));
    }
    Ok(trimmed.to_string())
}

/* ─────────────────────────────── recording ─────────────────────────────── */

/// Start recording a meeting and return its id.
///
/// `title` is supplied by the frontend so the default is localised (invariant 7
/// of the meetings plan — an English default written here would escape
/// i18next). A blank title falls back to the local start time, which carries no
/// language at all, rather than to an English word.
///
/// `language` defaults to the app's configured transcription language, so the
/// stored hint matches what the engine was actually asked to do instead of
/// being null for every meeting.
///
/// Runs on the blocking pool: this opens the microphone and the loopback
/// device, and device enumeration is not fast on any platform.
#[tauri::command]
#[specta::specta]
pub async fn start_meeting(
    app: AppHandle,
    recorder: State<'_, Arc<MeetingRecorder>>,
    title: Option<String>,
    language: Option<String>,
) -> Result<i64, String> {
    let title = title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());

    let language = language
        .filter(|l| !l.trim().is_empty())
        .or_else(|| {
            let configured = crate::settings::get_settings(&app).selected_language;
            (!configured.trim().is_empty()).then_some(configured)
        })
        .map(|l| l.trim().to_string());

    let recorder = Arc::clone(&recorder);
    let pill_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let meeting_id = recorder
            .start(&title, language.as_deref())
            .map_err(|e| e.to_string())?;
        // Deliberately inside the blocking closure, not in the async body above.
        // `ensure_pill_window` builds the webview inline on the calling thread,
        // and dispatching a webview build to the main thread from inside a
        // command's call stack deadlocks WebView2 on Windows — the same reason
        // `assistant::open_snip_overlay` builds inline. A blocking-pool thread is
        // the safe place for it.
        //
        // After the recorder has already started, so a failure to put the pill on
        // screen costs an indicator rather than the recording.
        crate::meetings::pill::show_pill(&pill_app);
        Ok::<i64, String>(meeting_id)
    })
    .await
    .map_err(|e| format!("Starting the meeting panicked: {e}"))?
}

/// Stop the active recording and return the meeting id.
///
/// Marks the meeting complete once `stop()` returns. That is correct rather than
/// premature: `stop()` flushes both chunkers and *joins* the transcription
/// worker, so by the time it answers, every captured chunk has been transcribed
/// and written. Leaving the row in `processing` would instead mean the next
/// launch reconciled it to `interrupted` — telling the user a meeting they
/// finished cleanly had crashed, purely because they never asked for notes.
///
/// Runs on the blocking pool because that worker join can take minutes on a long
/// meeting with a queue behind it.
#[tauri::command]
#[specta::specta]
pub async fn stop_meeting(
    app: AppHandle,
    recorder: State<'_, Arc<MeetingRecorder>>,
) -> Result<i64, String> {
    let recorder = Arc::clone(&recorder);
    let pill_app = app.clone();
    let meeting_id = tauri::async_runtime::spawn_blocking(move || {
        // Before `stop()`, which joins the transcription worker and can take
        // minutes on a long meeting. Leaving the pill up through that drain would
        // show a live clock and a level meter for a recording that has already
        // ended — the one state the pill exists to never show.
        crate::meetings::pill::hide_pill(&pill_app);

        let meeting_id = recorder.stop().map_err(|e| e.to_string())?;
        // Failing to flip the status must not fail a stop that already captured
        // and stored everything; the audio and transcript are safe either way.
        if let Err(e) = recorder.store().complete_meeting(meeting_id) {
            log::error!("Meeting {meeting_id} stopped but could not be marked complete: {e}");
        }
        Ok::<i64, String>(meeting_id)
    })
    .await
    .map_err(|e| format!("Stopping the meeting panicked: {e}"))??;

    let _ = app.emit(MEETINGS_UPDATED_EVENT, ());

    // The call is over, so summarize it. Not awaited: this is a sequence of model
    // calls that can take minutes on a long meeting, and the user has finished
    // their call and wants their transcript now. Progress arrives on
    // `meeting-notes-progress`.
    //
    // This is the whole difference between the old Summary tab and this one. It
    // used to require three decisions — open the tab, pick a template, press
    // Generate — to reach the thing every user wants every time, and the template
    // choice had to be made before reading the transcript it applies to. Now the
    // default runs on its own and choosing a template becomes a re-generate.
    crate::meetings::summarize::spawn_notes_job(&app, meeting_id);

    Ok(meeting_id)
}

/// Pause or resume the active recording.
///
/// Paused frames are dropped, not buffered, so this genuinely stops capturing
/// rather than deferring it — which is what a user pausing a meeting means.
#[tauri::command]
#[specta::specta]
pub async fn set_meeting_paused(
    _app: AppHandle,
    recorder: State<'_, Arc<MeetingRecorder>>,
    paused: bool,
) -> Result<(), String> {
    recorder.set_paused(paused).map_err(|e| e.to_string())
}

/// Current recording state, for a panel that just mounted.
///
/// The live path is the `meeting-state` event; this is how a window opened
/// mid-meeting learns where things stand without waiting for the next change.
#[tauri::command]
#[specta::specta]
pub async fn get_meeting_state(
    _app: AppHandle,
    recorder: State<'_, Arc<MeetingRecorder>>,
) -> Result<MeetingState, String> {
    Ok(recorder.state())
}

/// Whether the other participants can be captured on this machine.
///
/// Asked before recording starts, so the UI can warn that a meeting would
/// capture only the user's own side — which is worth knowing *before* the call,
/// not after it.
#[tauri::command]
#[specta::specta]
pub async fn get_system_audio_status(_app: AppHandle) -> Result<SystemAudioStatus, String> {
    let supported = crate::audio_toolkit::loopback_supported();
    Ok(SystemAudioStatus {
        supported,
        // The macOS permission is the only case where "supported" is true and
        // capture can still yield pure silence, so it is the only case worth
        // pre-emptively explaining.
        help: if cfg!(target_os = "macos") {
            Some(crate::audio_toolkit::MACOS_PERMISSION_HELP.to_string())
        } else {
            None
        },
    })
}

/* ──────────────────────────────── reading ──────────────────────────────── */

/// Page through meetings, newest first.
#[tauri::command]
#[specta::specta]
pub async fn list_meetings(
    _app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<PaginatedMeetings, String> {
    let store = Arc::clone(&store);
    let limit = page_size(limit, MEETING_PAGE_SIZE);
    let offset = offset.unwrap_or(0);

    tauri::async_runtime::spawn_blocking(move || {
        store
            .list_meetings(limit, offset)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Listing meetings panicked: {e}"))?
}

/// One meeting by id. `None` means it was deleted, which is not an error — a
/// detail view opened from a stale list should show "gone", not a failure.
#[tauri::command]
#[specta::specta]
pub async fn get_meeting(
    _app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: i64,
) -> Result<Option<Meeting>, String> {
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        store.get_meeting(meeting_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Reading the meeting panicked: {e}"))?
}

/// A page of transcript segments, oldest first.
///
/// Paginated because an hour of speech is on the order of a thousand segments:
/// the response carries `total` so a virtualised list can size its scrollbar
/// without having fetched the rest.
#[tauri::command]
#[specta::specta]
pub async fn get_meeting_segments(
    _app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: i64,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<PaginatedSegments, String> {
    let store = Arc::clone(&store);
    let limit = page_size(limit, SEGMENT_PAGE_SIZE);
    let offset = offset.unwrap_or(0);

    tauri::async_runtime::spawn_blocking(move || {
        store
            .segments(meeting_id, limit, offset)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Reading the transcript panicked: {e}"))?
}

/// Speakers known for a meeting, local user first.
///
/// Always non-empty for a meeting that started: the two channel-derived
/// speakers are seeded at creation, so the UI has something to render and
/// rename even for a meeting that produced no transcript.
#[tauri::command]
#[specta::specta]
pub async fn get_meeting_speakers(
    _app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: i64,
) -> Result<Vec<MeetingSpeaker>, String> {
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        store.speakers(meeting_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Reading meeting speakers panicked: {e}"))?
}

/* ──────────────────────────────── editing ──────────────────────────────── */

/// Rename a meeting.
#[tauri::command]
#[specta::specta]
pub async fn rename_meeting(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: i64,
    title: String,
) -> Result<(), String> {
    let title = require_text(&title, "A meeting title")?;
    let store = Arc::clone(&store);

    tauri::async_runtime::spawn_blocking(move || {
        store
            .rename_meeting(meeting_id, &title)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Renaming the meeting panicked: {e}"))??;

    let _ = app.emit(MEETINGS_UPDATED_EVENT, ());
    Ok(())
}

/// Give a speaker a display name for this meeting only.
///
/// Per-meeting on purpose: recognising the same voice across meetings needs a
/// stored voice fingerprint, which is a privacy decision that deserves its own
/// opt-in rather than arriving as a side effect of labelling one transcript.
#[tauri::command]
#[specta::specta]
pub async fn rename_meeting_speaker(
    _app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: i64,
    speaker_key: String,
    display_name: String,
) -> Result<(), String> {
    let speaker_key = require_text(&speaker_key, "A speaker key")?;
    let display_name = require_text(&display_name, "A speaker name")?;
    let store = Arc::clone(&store);

    tauri::async_runtime::spawn_blocking(move || {
        store
            .set_speaker_name(meeting_id, &speaker_key, &display_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Renaming the speaker panicked: {e}"))?
}

/// Save the user's own notes.
///
/// Blank is legitimate here — clearing your notes is an edit, not a mistake —
/// so unlike a title this is stored as given after trimming.
#[tauri::command]
#[specta::specta]
pub async fn set_meeting_my_notes(
    _app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: i64,
    notes: String,
) -> Result<(), String> {
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        store
            .set_my_notes(meeting_id, notes.trim_end())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Saving your notes panicked: {e}"))?
}

/* ──────────────────────────────── notes ────────────────────────────────── */

/// Generate notes for a meeting and store them.
///
/// The template is the typed enum rather than a string, for the reason
/// `update_recording_retention_period` documents: with a hand-written string
/// match, specta advertised one spelling in `bindings.ts` while serde accepted
/// another, and the mismatch was invisible until a call failed. `None` means the
/// default template.
///
/// Deliberately awaited rather than fired off in the background. This is a
/// sequence of LLM calls, all `.await`, so it never occupies a worker thread —
/// and the frontend gets one promise that either resolves with notes or rejects
/// with the reason, instead of having to correlate a completion event with the
/// request that caused it. Progress, if the UI wants it, belongs in a later
/// event rather than in a changed return shape.
///
/// `skipped_windows` in the result is not decoration: a two-hour meeting is nine
/// calls and one provider hiccup must not discard the eight that worked, so the
/// job continues and reports the hole rather than hiding it.
#[tauri::command]
#[specta::specta]
pub async fn generate_meeting_notes(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: i64,
    template: Option<NotesTemplate>,
) -> Result<GeneratedNotes, String> {
    let store = Arc::clone(&store);
    let notes = run_notes_job(
        &app,
        store.as_ref(),
        meeting_id,
        template.unwrap_or_default(),
    )
    .await?;

    // Notes generation is the last step of a meeting; a row that was left in
    // `processing` (a recording interrupted mid-flight, then re-transcribed) is
    // finished once it has notes.
    if let Err(e) = store.complete_meeting(meeting_id) {
        log::error!("Meeting {meeting_id} has notes but could not be marked complete: {e}");
    }

    let _ = app.emit(MEETINGS_UPDATED_EVENT, ());
    Ok(notes)
}

/* ─────────────────────────── the call offer ─────────────────────────── */

/// The user declined to record the detected call.
///
/// Two separate effects, and both are needed. This call is never offered again, with
/// no dependence on a clock — the answer to "record this call?" does not expire while
/// the call is still running. And it starts a cooldown covering the *next* call,
/// because the usual reason a call ends and restarts within the hour is a dropped
/// connection or a rejoin: the same conversation the user just declined.
#[tauri::command]
#[specta::specta]
pub async fn dismiss_call_offer(app: AppHandle) -> Result<(), String> {
    if let Some(watcher) = app.try_state::<Arc<crate::meetings::call_detect::CallWatcher>>() {
        watcher.dismiss();
    }
    crate::meetings::pill::hide_call_offer(&app);
    Ok(())
}

/// The user accepted. Tells the watcher to stop offering **without** arming the
/// dismissal cooldown, since the answer was yes.
///
/// Starting the recording is the frontend's own next call, not this one's job: the
/// title has to be composed there so its default is localised.
#[tauri::command]
#[specta::specta]
pub async fn accept_call_offer(app: AppHandle) -> Result<(), String> {
    if let Some(watcher) = app.try_state::<Arc<crate::meetings::call_detect::CallWatcher>>() {
        watcher.accepted();
    }
    Ok(())
}

/// Whether call detection can work here, and whether it is switched on.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct CallDetectionStatus {
    /// False on macOS and Linux, where the question cannot currently be asked. The
    /// UI hides the switch rather than offering one that does nothing.
    pub supported: bool,
    pub enabled: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn get_call_detection_status(app: AppHandle) -> Result<CallDetectionStatus, String> {
    Ok(CallDetectionStatus {
        supported: crate::meetings::call_detect::detection_supported(),
        enabled: crate::settings::get_settings(&app).meeting_auto_detect,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn set_meeting_auto_detect(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.meeting_auto_detect = enabled;
    crate::settings::write_settings(&app, settings);
    // Turning it off should also take down an offer already on screen, rather than
    // leaving a card the setting says cannot appear.
    if !enabled {
        crate::meetings::pill::hide_call_offer(&app);
    }
    Ok(())
}

/* ───────────────────────────── diarization ───────────────────────────── */

/// Whether per-speaker labelling is available on this machine.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct DiarizationStatus {
    /// The speaker-embedding model is on disk.
    pub installed: bool,
    /// Download size, so the UI can promise it before starting.
    pub download_mb: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn get_diarization_status(app: AppHandle) -> Result<DiarizationStatus, String> {
    Ok(DiarizationStatus {
        installed: crate::meetings::diarize::model::is_installed(&app),
        download_mb: crate::meetings::diarize::model::MODEL_SIZE_MB,
    })
}

/// Download the speaker-embedding model.
///
/// A separate step from recording on purpose: a first meeting must not stall behind
/// a download nobody asked for, and a user who only ever talks to one person does
/// not need it at all.
#[tauri::command]
#[specta::specta]
pub async fn download_diarization_model(app: AppHandle) -> Result<(), String> {
    crate::meetings::diarize::model::download(&app)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Identify the speakers in one recorded meeting.
///
/// Normally runs automatically when a call ends. This is the explicit path: for a
/// meeting recorded before the model was installed, and for a retry.
///
/// Runs on the blocking pool — it is ONNX inference over every voiced window of the
/// recording, minutes of CPU on a long meeting.
#[tauri::command]
#[specta::specta]
pub async fn diarize_meeting(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorder>>,
    meeting_id: i64,
) -> Result<crate::meetings::diarize::DiarizationOutcome, String> {
    // The WAV is still being written and has no finalised header, so it cannot be
    // decoded yet.
    if recorder.state().meeting_id == Some(meeting_id) {
        return Err("Stop the recording before identifying speakers.".to_string());
    }

    let store = Arc::clone(&store);
    let job_app = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        crate::meetings::diarize::diarize_meeting(
            &job_app,
            store.as_ref(),
            meeting_id,
            &crate::meetings::diarize::DiarizeConfig::default(),
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|e| format!("Identifying speakers panicked: {e}"))??;

    let _ = app.emit(MEETINGS_UPDATED_EVENT, ());
    Ok(outcome)
}

/* ────────────────────────── ask about a meeting ────────────────────────── */

/// Ask a question about one meeting and stream the answer.
///
/// Awaited rather than fired into the background, so the caller gets one promise
/// that either resolves with the finished answer or rejects with a sentence to
/// show. The streaming half arrives out of band on `meeting-chat-token`, with an
/// authoritative full-thread snapshot on `meeting-chat-messages` at the end — the
/// same contract the assistant panel renders from, and what makes duplicate
/// listeners unable to duplicate a message.
#[tauri::command]
#[specta::specta]
pub async fn ask_about_meeting(
    app: AppHandle,
    meeting_id: i64,
    question: String,
) -> Result<String, String> {
    crate::meetings::chat::ask(&app, meeting_id, question).await
}

/// The question-and-answer thread for a meeting.
///
/// Switching meetings clears it, so opening a different meeting's detail view
/// cannot show answers derived from another transcript.
#[tauri::command]
#[specta::specta]
pub async fn get_meeting_chat(
    chat: State<'_, Arc<crate::meetings::chat::MeetingChat>>,
    meeting_id: i64,
) -> Result<Vec<crate::llm_client::ChatMessage>, String> {
    Ok(chat.history_for(meeting_id))
}

#[tauri::command]
#[specta::specta]
pub async fn clear_meeting_chat(
    chat: State<'_, Arc<crate::meetings::chat::MeetingChat>>,
) -> Result<(), String> {
    chat.clear();
    Ok(())
}

/// Stop a reply that is still streaming. Whatever already arrived is kept —
/// dropping it would blank text the user is reading.
#[tauri::command]
#[specta::specta]
pub async fn cancel_meeting_chat(
    chat: State<'_, Arc<crate::meetings::chat::MeetingChat>>,
) -> Result<(), String> {
    chat.request_cancel();
    Ok(())
}

/* ─────────────────────────────── the pill ─────────────────────────────── */

/// Report the height the pill's content needs, so the window can take it.
///
/// An event would do as well, but a command keeps this beside the other meeting
/// calls the pill makes and costs nothing extra — the pill already `invoke`s.
/// Idempotent on the Rust side: the webview reports on every `ResizeObserver`
/// callback, and a resize that triggers another measurement would oscillate.
#[tauri::command]
#[specta::specta]
pub async fn fit_meeting_pill(app: AppHandle, height: f64) -> Result<(), String> {
    crate::meetings::pill::fit_pill(&app, height);
    Ok(())
}

/// Expand the pill into the transcript card, or collapse it back.
///
/// The webview must blur its ask input *before* calling this with `false`:
/// `set_focusable(false)` is not honoured for a window that currently holds
/// focus, and a pill left focusable steals the caret from whatever the user
/// types into next.
#[tauri::command]
#[specta::specta]
pub async fn set_meeting_pill_expanded(app: AppHandle, expanded: bool) -> Result<(), String> {
    crate::meetings::pill::set_pill_expanded(&app, expanded);
    Ok(())
}

/// Whether the pill is currently showing its expanded card.
///
/// Read from an atomic rather than from the window, so this is safe to call from
/// the recording path — `window.is_visible()` is a blocking round-trip to the
/// event loop.
#[tauri::command]
#[specta::specta]
pub async fn get_meeting_pill_expanded(_app: AppHandle) -> Result<bool, String> {
    Ok(crate::meetings::pill::is_expanded())
}

/* ──────────────────────────────── deleting ─────────────────────────────── */

/// Delete a meeting, its transcript, and its recorded audio.
///
/// The store commits the row deletion and returns the audio paths *without*
/// touching them; unlinking is this command's job and happens strictly after
/// the commit. Doing it the other way round is how a user ends up with a
/// meeting that lists fine and opens to nothing.
///
/// A failed unlink is logged and does not fail the command. The record of truth
/// is already gone: reporting failure would tell the user the meeting still
/// exists when it does not, and there is nothing they could do differently. The
/// residue is an orphaned WAV, which is a disk-space problem, not a data one.
#[tauri::command]
#[specta::specta]
pub async fn delete_meeting(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    recorder: State<'_, Arc<MeetingRecorder>>,
    meeting_id: i64,
) -> Result<(), String> {
    // Deleting the meeting currently being recorded would leave the capture
    // threads writing into files and a row that no longer exist.
    if recorder.state().meeting_id == Some(meeting_id) {
        return Err("Stop the recording before deleting this meeting.".to_string());
    }

    let store = Arc::clone(&store);
    let files = tauri::async_runtime::spawn_blocking(move || {
        store.delete_meeting(meeting_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Deleting the meeting panicked: {e}"))??;

    for path in files {
        match std::fs::remove_file(&path) {
            Ok(()) => log::debug!("Removed meeting audio {path:?}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("Could not remove meeting audio {path:?}: {e}"),
        }
    }

    let _ = app.emit(MEETINGS_UPDATED_EVENT, ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_page_size_becomes_one() {
        // A zero limit returns an empty page for every offset, which reads as an
        // empty transcript rather than as a bad argument.
        assert_eq!(page_size(Some(0), 30), 1);
    }

    #[test]
    fn a_missing_page_size_uses_the_default() {
        assert_eq!(page_size(None, 30), 30);
    }

    #[test]
    fn an_absurd_page_size_is_capped() {
        assert_eq!(page_size(Some(u32::MAX), 30), MAX_PAGE_SIZE);
    }

    #[test]
    fn blank_text_is_rejected() {
        assert!(require_text("   \n", "A meeting title").is_err());
    }

    #[test]
    fn text_is_stored_trimmed() {
        assert_eq!(
            require_text("  Standup  ", "A meeting title"),
            Ok("Standup".to_string())
        );
    }
}
