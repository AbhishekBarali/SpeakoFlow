import { invoke } from "@tauri-apps/api/core";

/*
 * Meetings command surface.
 *
 * Commands go through `invoke` rather than the generated `commands.*` wrappers,
 * and the types below are hand-mirrored from the Rust structs. Same reason as
 * `src/reminder/ReminderPopup.tsx` and `useVoiceConversation`: `src/bindings.ts`
 * is only regenerated while the app runs in dev, so a fresh checkout would fail
 * to type-check anything that depended on it existing.
 *
 * Argument keys are camelCase — Tauri converts them to the Rust parameter's
 * snake_case, which is exactly what the generated bindings do (`{ modelId }` →
 * `model_id`).
 *
 * Payload field names stay snake_case because they come straight off serde.
 */

/** Which audio stream a segment was captured on. Mirrors `SpeakerSource`.
 *
 *  This is the whole feature's speaker attribution and it is a hardware fact,
 *  not a guess: the microphone is the user, loopback is everyone else. Nothing
 *  in the UI should ever merge the two into one anonymous stream. */
export type SpeakerSource = "mic" | "system";

/** Mirrors `MeetingStatus`. */
export type MeetingStatus =
  | "recording"
  | "processing"
  | "complete"
  | "interrupted";

/** Wire ids of `NotesTemplate` (serde `rename_all = "snake_case"`). */
export type NotesTemplateId =
  | "general"
  | "standup"
  | "one_on_one"
  | "interview"
  | "action_items";

/** Every template this build ships, in the order the picker shows them. */
export const NOTES_TEMPLATES: readonly NotesTemplateId[] = [
  "general",
  "standup",
  "one_on_one",
  "interview",
  "action_items",
] as const;

export const DEFAULT_NOTES_TEMPLATE: NotesTemplateId = "general";

/** Mirrors `Meeting`. `duration_secs()` is a Rust method, not a field, so the
 *  UI derives length from the two timestamps itself. */
export interface Meeting {
  id: number;
  title: string;
  /** Epoch seconds. */
  started_at: number;
  /** Epoch seconds, or null while still recording. */
  ended_at: number | null;
  status: MeetingStatus;
  mic_file: string | null;
  system_file: string | null;
  my_notes: string;
  notes: string | null;
  notes_template: string | null;
  language: string | null;
  diarized: boolean;
  segment_count: number;
}

/** Mirrors `MeetingSegment`. */
export interface MeetingSegment {
  id: number;
  meeting_id: number;
  source: SpeakerSource;
  speaker_key: string | null;
  start_ms: number;
  end_ms: number;
  text: string;
  /** Diarization confidence in [-1, 1], when a pass has run. */
  confidence: number | null;
}

/** Mirrors `MeetingSpeaker`. */
export interface MeetingSpeaker {
  speaker_key: string;
  display_name: string;
  is_me: boolean;
}

/** Mirrors `PaginatedMeetings`. */
export interface PaginatedMeetings {
  meetings: Meeting[];
  has_more: boolean;
}

/** Mirrors `PaginatedSegments`. */
export interface PaginatedSegments {
  segments: MeetingSegment[];
  has_more: boolean;
  total: number;
}

/** Mirrors `MeetingState`, the payload of the `meeting-state` event. */
export interface MeetingState {
  meeting_id: number | null;
  status: MeetingStatus;
  paused: boolean;
  /** False means the transcript will contain only the user's own side. */
  system_audio: boolean;
  system_audio_error: string | null;
  elapsed_ms: number;
}

/** Mirrors `SegmentEvent`, the payload of the `meeting-segment` event. */
export interface SegmentEvent {
  meeting_id: number;
  source: SpeakerSource;
  speaker_key: string;
  start_ms: number;
  end_ms: number;
  text: string;
}

/** Mirrors `GeneratedNotes`. */
export interface GeneratedNotes {
  notes: string;
  template_id: string;
  windows: number;
  /** Windows whose summary call failed. Surfaced, never hidden: notes built
   *  from 6 of 9 windows have a hole the user is entitled to know about. */
  skipped_windows: number;
}

/** Progress of the automatic notes job that runs when a call ends.
 *
 *  Mirrors `NotesProgress`, an internally-tagged enum on `stage`. An event rather
 *  than a promise because nobody is awaiting it — the user has finished their call
 *  and may have closed the window; the UI fills in whenever it is looking. */
export type NotesProgress =
  | { stage: "started"; meeting_id: number }
  | {
      stage: "finished";
      meeting_id: number;
      notes: string;
      skipped_windows: number;
    }
  | { stage: "failed"; meeting_id: number; error: string };

/** Progress of an automatic post-call notes job. */
export const MEETING_NOTES_PROGRESS_EVENT = "meeting-notes-progress";

/** Mirrors `SystemAudioStatus`. */
export interface SystemAudioStatus {
  supported: boolean;
  help: string | null;
}

/** Mirrors `MeetingLevels`, the payload of the `meeting-level` event.
 *
 *  Two levels rather than one, because the second is the point: a meeting that
 *  captures the user but not the other participants is the most common way this
 *  feature fails, and it fails silently. A meter that stays flat on the system
 *  side says so during the call instead of after it. */
export interface MeetingLevels {
  mic: number;
  system: number;
}

/** A newly transcribed segment, live. */
export const MEETING_SEGMENT_EVENT = "meeting-segment";
/** A change in recording state. */
export const MEETING_STATE_EVENT = "meeting-state";
/** The meeting list changed in a way a list view cannot infer on its own. */
export const MEETINGS_UPDATED_EVENT = "meetings-updated";
/** Live input levels for both streams, throttled to ~15/s in Rust. */
export const MEETING_LEVEL_EVENT = "meeting-level";
/** The pill's mode changed from outside the pill's own webview. */
export const MEETING_PILL_MODE_EVENT = "meeting-pill-mode";

/** Segments per request. Matches `store::SEGMENT_PAGE_SIZE`. */
export const SEGMENT_PAGE_SIZE = 200;
/** Meetings per request. Matches `MEETING_PAGE_SIZE` in `commands/meetings.rs`. */
export const MEETING_PAGE_SIZE = 30;

/* ─────────────────────────────── recording ─────────────────────────────── */

/** Start recording. The title is supplied here because the default is
 *  localised — an English default in Rust would escape i18next. */
export const startMeeting = (
  title: string,
  language?: string | null,
): Promise<number> =>
  invoke<number>("start_meeting", { title, language: language ?? null });

export const stopMeeting = (): Promise<number> =>
  invoke<number>("stop_meeting");

export const setMeetingPaused = (paused: boolean): Promise<null> =>
  invoke<null>("set_meeting_paused", { paused });

export const getMeetingState = (): Promise<MeetingState> =>
  invoke<MeetingState>("get_meeting_state");

export const getSystemAudioStatus = (): Promise<SystemAudioStatus> =>
  invoke<SystemAudioStatus>("get_system_audio_status");

/* ──────────────────────────────── reading ──────────────────────────────── */

export const listMeetings = (
  limit: number,
  offset: number,
): Promise<PaginatedMeetings> =>
  invoke<PaginatedMeetings>("list_meetings", { limit, offset });

/** `null` is success, not a failure: a detail view opened from a stale list
 *  should render "gone" rather than an error. */
export const getMeeting = (meetingId: number): Promise<Meeting | null> =>
  invoke<Meeting | null>("get_meeting", { meetingId });

export const getMeetingSegments = (
  meetingId: number,
  limit: number,
  offset: number,
): Promise<PaginatedSegments> =>
  invoke<PaginatedSegments>("get_meeting_segments", {
    meetingId,
    limit,
    offset,
  });

export const getMeetingSpeakers = (
  meetingId: number,
): Promise<MeetingSpeaker[]> =>
  invoke<MeetingSpeaker[]>("get_meeting_speakers", { meetingId });

/* ──────────────────────────────── editing ──────────────────────────────── */

export const renameMeeting = (
  meetingId: number,
  title: string,
): Promise<null> => invoke<null>("rename_meeting", { meetingId, title });

export const renameMeetingSpeaker = (
  meetingId: number,
  speakerKey: string,
  displayName: string,
): Promise<null> =>
  invoke<null>("rename_meeting_speaker", {
    meetingId,
    speakerKey,
    displayName,
  });

export const setMeetingMyNotes = (
  meetingId: number,
  notes: string,
): Promise<null> => invoke<null>("set_meeting_my_notes", { meetingId, notes });

/* ──────────────────────────── notes / deleting ─────────────────────────── */

export const generateMeetingNotes = (
  meetingId: number,
  template: NotesTemplateId,
): Promise<GeneratedNotes> =>
  invoke<GeneratedNotes>("generate_meeting_notes", { meetingId, template });

export const deleteMeeting = (meetingId: number): Promise<null> =>
  invoke<null>("delete_meeting", { meetingId });

/* ─────────────────────────── the call offer ─────────────────────────── */

/** A call appears to be in progress. Payload is the app name when one could be
 *  read — the detector works without it, so `null` must render too. */
export const MEETING_CALL_OFFER_EVENT = "meeting-call-detected";

/** Mirrors `CallOffer`, the payload of `meeting-call-detected`.
 *
 *  `active` is explicit rather than implied by a null app name: reading the process
 *  name is allowed to fail and detection does not depend on it, so "an offer with no
 *  name" is normal and must not be confused with "no offer". */
export interface CallOffer {
  active: boolean;
  app: string | null;
}

/** Mirrors `CallDetectionStatus`. */
export interface CallDetectionStatus {
  /** False on macOS and Linux, where the question cannot currently be asked. Hide
   *  the switch rather than offer one that does nothing. */
  supported: boolean;
  enabled: boolean;
}

/** The user said no. Silences this call permanently and the next one for a while. */
export const dismissCallOffer = (): Promise<null> =>
  invoke<null>("dismiss_call_offer");

/** The user said yes. Stops further offers without arming the dismissal cooldown;
 *  the caller then starts the recording itself, so the title stays localised. */
export const acceptCallOffer = (): Promise<null> =>
  invoke<null>("accept_call_offer");

export const getCallDetectionStatus = (): Promise<CallDetectionStatus> =>
  invoke<CallDetectionStatus>("get_call_detection_status");

export const setMeetingAutoDetect = (enabled: boolean): Promise<null> =>
  invoke<null>("set_meeting_auto_detect", { enabled });

/* ───────────────────────────── diarization ───────────────────────────── */

/** Mirrors `DiarizationStatus`. */
export interface DiarizationStatus {
  installed: boolean;
  download_mb: number;
}

/** Mirrors `DiarizationOutcome`, an internally-tagged enum on `outcome`.
 *
 *  `one_speaker` is deliberately distinct from "one speaker found": no labels are
 *  written, because renaming a lone remote voice to "Speaker 1" would tell the user
 *  the app had told several voices apart. */
export type DiarizationOutcome =
  | { outcome: "labelled"; speakers: number; segments: number }
  | { outcome: "one_speaker" }
  | { outcome: "skipped"; reason: DiarizationSkipReason };

export type DiarizationSkipReason =
  | "model_not_installed"
  | "no_system_audio"
  | "audio_unreadable"
  | "too_few_segments"
  | "already_diarized"
  | "model_unusable";

/** Progress of a diarization pass. */
export const MEETING_DIARIZATION_PROGRESS_EVENT =
  "meeting-diarization-progress";

export interface DiarizationProgress {
  meeting_id: number;
  done: number;
  total: number;
}

export const getDiarizationStatus = (): Promise<DiarizationStatus> =>
  invoke<DiarizationStatus>("get_diarization_status");

export const downloadDiarizationModel = (): Promise<null> =>
  invoke<null>("download_diarization_model");

/** Identify who said what on the far side of one recorded meeting. */
export const diarizeMeeting = (
  meetingId: number,
): Promise<DiarizationOutcome> =>
  invoke<DiarizationOutcome>("diarize_meeting", { meetingId });

/* ──────────────────────── ask about a meeting ─────────────────────────── */

/** One message in a meeting's question thread. Mirrors `llm_client::ChatMessage`
 *  (`images` is always empty here — a transcript question carries no pictures). */
export interface MeetingChatMessage {
  role: string;
  content: string;
  images: string[];
}

/** A streamed content delta. */
export const MEETING_CHAT_TOKEN_EVENT = "meeting-chat-token";
/** The authoritative full thread after any change. Render from this plus a
 *  transient stream buffer, so a duplicate listener cannot duplicate a message. */
export const MEETING_CHAT_MESSAGES_EVENT = "meeting-chat-messages";
/** Whether a reply is in flight. */
export const MEETING_CHAT_STATE_EVENT = "meeting-chat-state";
/** Something went wrong, with a sentence to show. */
export const MEETING_CHAT_ERROR_EVENT = "meeting-chat-error";

/** Ask a question about one meeting. Resolves with the finished answer; the
 *  streaming half arrives on `meeting-chat-token`. */
export const askAboutMeeting = (
  meetingId: number,
  question: string,
): Promise<string> =>
  invoke<string>("ask_about_meeting", { meetingId, question });

/** The thread for a meeting. Switching meetings clears it, so this is also how
 *  the UI learns the thread is empty for a newly opened meeting. */
export const getMeetingChat = (
  meetingId: number,
): Promise<MeetingChatMessage[]> =>
  invoke<MeetingChatMessage[]>("get_meeting_chat", { meetingId });

export const clearMeetingChat = (): Promise<null> =>
  invoke<null>("clear_meeting_chat");

/** Stop a streaming reply. What already arrived is kept. */
export const cancelMeetingChat = (): Promise<null> =>
  invoke<null>("cancel_meeting_chat");

/* ────────────────────────────── the pill ──────────────────────────────── */

/** Report the height the pill's content needs so the window can take it.
 *
 *  Called from a `ResizeObserver`, so it fires often; Rust ignores a height it
 *  already has, which is what stops a resize from triggering the measurement
 *  that triggered it. */
export const fitMeetingPill = (height: number): Promise<null> =>
  invoke<null>("fit_meeting_pill", { height });

/** Expand the pill into the transcript card, or collapse it back.
 *
 *  **Blur the ask input before collapsing.** On Windows the collapsed pill is
 *  made unfocusable (`WS_EX_NOACTIVATE`) so it can never steal the caret from
 *  the app the user is working in — and the platform will not remove focusability
 *  from a window that currently holds focus. A pill left focusable is a pill that
 *  takes the caret on the next click. */
export const setMeetingPillExpanded = (expanded: boolean): Promise<null> =>
  invoke<null>("set_meeting_pill_expanded", { expanded });

export const getMeetingPillExpanded = (): Promise<boolean> =>
  invoke<boolean>("get_meeting_pill_expanded");
