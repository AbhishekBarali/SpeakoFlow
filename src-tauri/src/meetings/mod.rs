//! Meetings: recording, transcribing and summarizing a call.
//!
//! This is a separate feature from dictation, not a longer dictation. The
//! differences that actually shape the code:
//!
//! * **Two audio sources, kept apart.** Dictation hears one person. A meeting
//!   hears the user (microphone) and everyone else (system audio), and those
//!   stay separate all the way into the database. See
//!   [`SpeakerSource`] — that enum is the cheapest and most reliable speaker
//!   attribution available, and mixing the streams would destroy it.
//! * **Duration.** A dictation is seconds; a meeting is hours. Anything that
//!   accumulates in memory per utterance has to be bounded, and the transcript
//!   has to be readable from the database in pages rather than all at once.
//! * **The output is a document, not a paste.** Nothing is typed into another
//!   app. The result is a stored transcript plus generated notes the user edits.
//!
//! Storage lives in its own `meetings.db` rather than joining `history.db`.
//! A dictation history is small, hot, and written on every transcription; a
//! meeting store is large, cold, and written in bursts. Keeping them apart means
//! a meetings migration can never put a user's dictation history at risk, and a
//! long meeting's writes cannot contend with the `save_entry` path that runs on
//! every dictation.

pub mod call_detect;
pub mod chat;
pub mod chunker;
pub mod diarize;
pub mod pill;
pub mod retrieve;
pub mod session;
pub mod store;
pub mod summarize;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Which audio stream a transcript segment came from.
///
/// This is the load-bearing type of the whole feature. It records a **fact about
/// the hardware**, not a guess: microphone samples are the user, loopback
/// samples are the remote side of the call. Speaker diarization is statistical
/// and can be wrong; this cannot. It is why the capture layer refuses to mix the
/// two streams (see `audio_toolkit::audio::loopback`).
///
/// The practical consequence: even with diarization disabled, unavailable, or
/// performing badly, a meeting transcript is still correctly split into "you"
/// and "them", which is the distinction that matters most when the notes assign
/// action items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerSource {
    /// The user's microphone.
    Mic,
    /// System audio — the other participants.
    System,
}

impl SpeakerSource {
    /// Stable string form for the database. Written explicitly rather than via
    /// serde so a rename in the API cannot silently rewrite stored rows.
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::System => "system",
        }
    }

    /// Parse a stored value. Unknown values resolve to [`Self::System`] rather
    /// than failing the read: a segment attributed to the wrong side is a
    /// cosmetic problem, but refusing to load a two-hour transcript because one
    /// row is malformed is a data-loss problem.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "mic" => Self::Mic,
            _ => Self::System,
        }
    }

    /// The speaker key used before diarization has run (or when it never will).
    ///
    /// The microphone side gets a single stable key because it is known to be
    /// one person. The system side gets one placeholder key covering everyone,
    /// which diarization later replaces with per-speaker keys.
    pub fn default_speaker_key(self) -> &'static str {
        match self {
            Self::Mic => "me",
            Self::System => "them",
        }
    }
}

/// How far along a meeting is.
///
/// [`MeetingStatus::Interrupted`] exists because the app can be killed
/// mid-meeting. A meeting left in [`MeetingStatus::Recording`] when the process
/// dies is not recording any more, and presenting it as though it were is how a
/// user loses an hour of audio they believe is still being captured. Startup
/// reconciles this (see [`store::MeetingStore::reconcile_interrupted`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum MeetingStatus {
    /// Audio is being captured right now.
    Recording,
    /// Capture finished; transcription, diarization or notes still running.
    Processing,
    /// Everything finished.
    Complete,
    /// The app stopped while this meeting was recording. Whatever was already
    /// transcribed is intact and readable.
    Interrupted,
}

impl MeetingStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Processing => "processing",
            Self::Complete => "complete",
            Self::Interrupted => "interrupted",
        }
    }

    /// Unknown values resolve to [`Self::Complete`] so an unrecognised status
    /// from a newer build still lists and opens rather than looking broken.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "recording" => Self::Recording,
            "processing" => Self::Processing,
            "interrupted" => Self::Interrupted,
            _ => Self::Complete,
        }
    }
}

/// A meeting record without its transcript.
///
/// The transcript is deliberately absent: an hour-long meeting is thousands of
/// segments, and the list view needs none of them. Segments are fetched per
/// meeting, in pages, via [`store::MeetingStore::segments`].
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Meeting {
    pub id: i64,
    pub title: String,
    /// Epoch seconds when recording started.
    pub started_at: i64,
    /// Epoch seconds when recording stopped; `None` while still recording.
    pub ended_at: Option<i64>,
    pub status: MeetingStatus,
    /// Recorded microphone audio, relative to the meetings audio directory.
    pub mic_file: Option<String>,
    /// Recorded system audio, relative to the meetings audio directory.
    pub system_file: Option<String>,
    /// The user's own notes, typed during or after the meeting. Merged with the
    /// transcript when generating notes — the thing that makes the output about
    /// what the user cared about rather than a flat summary.
    pub my_notes: String,
    /// Generated notes (markdown). `None` until notes are produced.
    pub notes: Option<String>,
    /// Which template produced [`Self::notes`].
    pub notes_template: Option<String>,
    /// Language hint used for transcription.
    pub language: Option<String>,
    /// Whether a diarization pass has completed for this meeting.
    pub diarized: bool,
    /// Number of transcript segments, so the list view can show length without
    /// loading the transcript.
    pub segment_count: i64,
}

impl Meeting {
    /// Recorded length in seconds, or `None` while still recording.
    pub fn duration_secs(&self) -> Option<i64> {
        self.ended_at.map(|end| (end - self.started_at).max(0))
    }
}

/// One utterance in a meeting transcript.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingSegment {
    pub id: i64,
    pub meeting_id: i64,
    /// Which stream this came from. Never inferred.
    pub source: SpeakerSource,
    /// Speaker key: `"me"`, `"them"`, or a diarization cluster such as
    /// `"spk_1"`. Never `NULL` in practice — the source always supplies a
    /// default — but nullable in the schema so a future pass can clear it.
    pub speaker_key: Option<String>,
    /// Milliseconds from the start of the meeting.
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// Diarization confidence in `[-1, 1]`, when available. Low values should be
    /// shown de-emphasised rather than asserted: a confidently wrong speaker
    /// label is worse than a visibly uncertain one.
    pub confidence: Option<f32>,
}

/// A segment about to be written. Separate from [`MeetingSegment`] because the
/// row id does not exist yet.
#[derive(Clone, Debug)]
pub struct NewSegment {
    pub source: SpeakerSource,
    pub speaker_key: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub confidence: Option<f32>,
}

/// A named speaker within one meeting.
///
/// Speaker names are per-meeting, not global. Recognising the same voice across
/// meetings requires storing a voice fingerprint, which is a privacy decision
/// that deserves its own opt-in rather than arriving as a side effect of
/// labelling one transcript.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingSpeaker {
    pub speaker_key: String,
    pub display_name: String,
    /// True for the key representing the local user.
    pub is_me: bool,
}

/// A page of meetings.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedMeetings {
    pub meetings: Vec<Meeting>,
    pub has_more: bool,
}

/// A page of transcript segments.
///
/// Pagination is not premature optimisation here. Meetily shipped without it and
/// had to retrofit both SQL paging and list virtualisation; an hour of speech is
/// on the order of a thousand segments and loading them all costs both the query
/// and the DOM.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedSegments {
    pub segments: Vec<MeetingSegment>,
    pub has_more: bool,
    /// Total segments for this meeting, so a virtualised list can size itself.
    pub total: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_db_round_trip() {
        for source in [SpeakerSource::Mic, SpeakerSource::System] {
            assert_eq!(SpeakerSource::from_db_str(source.as_db_str()), source);
        }
    }

    /// A malformed stored value must not fail the read. Losing one segment's
    /// attribution is survivable; refusing to open the meeting is not.
    #[test]
    fn unknown_source_falls_back_to_system() {
        assert_eq!(SpeakerSource::from_db_str("wat"), SpeakerSource::System);
    }

    #[test]
    fn status_db_round_trip() {
        for status in [
            MeetingStatus::Recording,
            MeetingStatus::Processing,
            MeetingStatus::Complete,
            MeetingStatus::Interrupted,
        ] {
            assert_eq!(MeetingStatus::from_db_str(status.as_db_str()), status);
        }
    }

    #[test]
    fn unknown_status_falls_back_to_complete() {
        assert_eq!(
            MeetingStatus::from_db_str("future"),
            MeetingStatus::Complete
        );
    }

    #[test]
    fn default_speaker_keys_are_distinct() {
        assert_ne!(
            SpeakerSource::Mic.default_speaker_key(),
            SpeakerSource::System.default_speaker_key()
        );
    }

    #[test]
    fn duration_is_none_while_recording() {
        let meeting = meeting_fixture(1_000, None);
        assert_eq!(meeting.duration_secs(), None);
    }

    #[test]
    fn duration_counts_elapsed_seconds() {
        let meeting = meeting_fixture(1_000, Some(1_090));
        assert_eq!(meeting.duration_secs(), Some(90));
    }

    /// A clock that jumped backwards mid-meeting (NTP correction, laptop resume)
    /// must not produce a negative duration that renders as "-3s".
    #[test]
    fn duration_never_goes_negative() {
        let meeting = meeting_fixture(1_000, Some(900));
        assert_eq!(meeting.duration_secs(), Some(0));
    }

    fn meeting_fixture(started_at: i64, ended_at: Option<i64>) -> Meeting {
        Meeting {
            id: 1,
            title: "Test".into(),
            started_at,
            ended_at,
            status: MeetingStatus::Complete,
            mic_file: None,
            system_file: None,
            my_notes: String::new(),
            notes: None,
            notes_template: None,
            language: None,
            diarized: false,
            segment_count: 0,
        }
    }
}
