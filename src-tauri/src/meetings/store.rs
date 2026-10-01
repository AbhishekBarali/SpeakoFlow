//! SQLite storage for meetings.
//!
//! Follows the conventions established by [`crate::managers::history`] —
//! `rusqlite` with `rusqlite_migration`, version tracked in the `user_version`
//! pragma — with three deliberate differences, each earned by a bug that
//! feature already paid for:
//!
//! 1. **Writes are batched.** A meeting produces a segment every few seconds for
//!    hours. `history.rs` committed once per row when pruning and hit
//!    `SQLITE_BUSY` against a concurrent insert. Segments are therefore appended
//!    in one transaction per batch, not one per segment.
//! 2. **Deletion unlinks audio only after the transaction commits**, and only
//!    for files no surviving row references. Deleting the file first and then
//!    failing the commit leaves a row pointing at nothing.
//! 3. **Reads are paginated.** An hour of speech is on the order of a thousand
//!    segments. Nothing loads a whole transcript except the summarizer and an
//!    explicit export.

use anyhow::{Context, Result};
use log::{debug, info, warn};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    Meeting, MeetingSegment, MeetingSpeaker, MeetingStatus, NewSegment, PaginatedMeetings,
    PaginatedSegments, SpeakerSource,
};

/// Schema history. Append only — never edit an applied migration.
///
/// Note what migration 1 already contains: `speaker_key` on segments and the
/// whole `meeting_speakers` table, both present before any diarization code
/// exists. Adding speaker support later would mean migrating a table that by
/// then holds every meeting a user has ever recorded, so the columns are here
/// from the first version even though the first version only ever writes the two
/// channel-derived keys into them.
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS meetings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            status TEXT NOT NULL DEFAULT 'recording',
            mic_file TEXT,
            system_file TEXT,
            my_notes TEXT NOT NULL DEFAULT '',
            notes TEXT,
            notes_template TEXT,
            language TEXT,
            diarized INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS meeting_segments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id INTEGER NOT NULL,
            source TEXT NOT NULL,
            speaker_key TEXT,
            start_ms INTEGER NOT NULL,
            end_ms INTEGER NOT NULL,
            text TEXT NOT NULL,
            confidence REAL,
            FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS meeting_speakers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id INTEGER NOT NULL,
            speaker_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
            UNIQUE (meeting_id, speaker_key)
        );

        CREATE INDEX IF NOT EXISTS idx_meeting_segments_lookup
            ON meeting_segments (meeting_id, start_ms);

        CREATE INDEX IF NOT EXISTS idx_meetings_started
            ON meetings (started_at DESC);",
    ),
    // Migration 2: full-text search over transcript text.
    //
    // Needed because "ask anything about this meeting" cannot send a two-hour
    // transcript to a model — and map-reduce over it, which is right for
    // generating notes once, is wrong for an interactive question: it is nine
    // sequential model calls per question, and by the time it answers it has
    // already summarized away the sentence being asked about. Retrieving the
    // handful of passages that mention the question's terms is both faster and a
    // better answer.
    //
    // `content=` makes this an **external-content** index: the text lives once in
    // `meeting_segments` and FTS5 stores only the inverted index, so a long
    // meeting does not pay for its transcript twice on disk. The cost of that
    // choice is that FTS5 no longer maintains itself, hence the three triggers.
    //
    // No `tokenize=` argument, so this uses the default `unicode61` tokenizer.
    // Deliberate: `porter` stems English and would mangle the code-switched
    // Hindi/English this app targets, and `trigram` would triple the index size
    // for substring matching nobody asked for.
    //
    // FTS5 is available with no Cargo change — `rusqlite`'s `bundled` feature
    // compiles the amalgamation with `-DSQLITE_ENABLE_FTS5`.
    M::up(
        "CREATE VIRTUAL TABLE IF NOT EXISTS meeting_segments_fts USING fts5(
            text,
            content='meeting_segments',
            content_rowid='id'
        );

        -- External content means FTS5 is not notified of writes to the base
        -- table, so every write path has to tell it. A missing trigger does not
        -- error; it silently produces a meeting whose transcript cannot be
        -- searched, which is why all three exist even though the app only ever
        -- inserts.
        CREATE TRIGGER IF NOT EXISTS meeting_segments_fts_insert
        AFTER INSERT ON meeting_segments BEGIN
            INSERT INTO meeting_segments_fts (rowid, text) VALUES (new.id, new.text);
        END;

        -- The 'delete' command writes a tombstone carrying the OLD text; FTS5
        -- needs the original tokens to remove them from the index, so `old.text`
        -- is not redundant here.
        CREATE TRIGGER IF NOT EXISTS meeting_segments_fts_delete
        AFTER DELETE ON meeting_segments BEGIN
            INSERT INTO meeting_segments_fts (meeting_segments_fts, rowid, text)
                VALUES ('delete', old.id, old.text);
        END;

        CREATE TRIGGER IF NOT EXISTS meeting_segments_fts_update
        AFTER UPDATE OF text ON meeting_segments BEGIN
            INSERT INTO meeting_segments_fts (meeting_segments_fts, rowid, text)
                VALUES ('delete', old.id, old.text);
            INSERT INTO meeting_segments_fts (rowid, text) VALUES (new.id, new.text);
        END;

        -- Backfill every meeting recorded before this migration. 'rebuild' reads
        -- the whole base table, which is the only way an external-content index
        -- can be populated retroactively, and is why this is a one-off in a
        -- migration rather than something the app does at launch.
        INSERT INTO meeting_segments_fts (meeting_segments_fts) VALUES ('rebuild');",
    ),
];

/// Page size for transcript reads. Chosen to be comfortably more than one
/// screenful so the virtualised list has lookahead, and small enough that the
/// first page of a three-hour meeting is immediate.
pub const SEGMENT_PAGE_SIZE: u32 = 200;

pub struct MeetingStore {
    db_path: PathBuf,
    audio_dir: PathBuf,
}

impl MeetingStore {
    /// Open (creating if needed) the meetings database and audio directory.
    pub fn new(app_handle: &tauri::AppHandle) -> Result<Self> {
        let app_data_dir = crate::portable::app_data_dir(app_handle)?;
        let audio_dir = app_data_dir.join("meetings");
        let db_path = app_data_dir.join("meetings.db");

        if !audio_dir.exists() {
            fs::create_dir_all(&audio_dir)
                .with_context(|| format!("Creating meetings audio dir {audio_dir:?}"))?;
        }

        let store = Self { db_path, audio_dir };
        store.init()?;
        Ok(store)
    }

    /// A store in a temp directory, for tests in other modules that need real
    /// SQLite and FTS behaviour rather than a mock of it.
    #[cfg(test)]
    pub(crate) fn temp_for_tests() -> (Self, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio_dir = dir.path().join("meetings");
        fs::create_dir_all(&audio_dir).expect("audio dir");
        let store = Self {
            db_path: dir.path().join("meetings.db"),
            audio_dir,
        };
        store.init().expect("migrations");
        (store, dir)
    }

    /// Where meeting audio lives. Recordings are referenced by file name in the
    /// database and resolved against this directory, so the whole app data
    /// folder stays movable (portable installs depend on that).
    pub fn audio_dir(&self) -> &Path {
        &self.audio_dir
    }

    /// Absolute path for a stored recording file name.
    pub fn audio_path(&self, file_name: &str) -> PathBuf {
        self.audio_dir.join(file_name)
    }

    fn init(&self) -> Result<()> {
        let mut conn = self.open()?;
        let migrations = Migrations::new(MIGRATIONS.to_vec());

        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid meetings migrations");

        let before: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        migrations.to_latest(&mut conn)?;
        let after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if after > before {
            info!("Meetings database migrated from version {before} to {after}");
        } else {
            debug!("Meetings database at version {after}");
        }
        Ok(())
    }

    /// Open a connection with the pragmas this workload needs.
    ///
    /// WAL matters here specifically: a meeting writes segments continuously
    /// while the UI reads the transcript to display it. Under the default
    /// rollback journal a reader blocks the writer, which would stall capture;
    /// under WAL they proceed concurrently.
    ///
    /// `foreign_keys` is on so deleting a meeting cascades to its segments and
    /// speakers. SQLite defaults it **off** per connection, so it must be set
    /// every time rather than once at creation.
    fn open(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("Opening meetings db at {:?}", self.db_path))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(conn)
    }

    /* ─────────────────────────── lifecycle ─────────────────────────── */

    /// Start a meeting and return its id.
    pub fn create_meeting(
        &self,
        title: &str,
        started_at: i64,
        language: Option<&str>,
    ) -> Result<i64> {
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO meetings (title, started_at, status, language)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                title,
                started_at,
                MeetingStatus::Recording.as_db_str(),
                language
            ],
        )?;
        let id = conn.last_insert_rowid();

        // Seed the two channel-derived speakers immediately. Doing it here
        // rather than lazily means the UI always has something to render and
        // rename, even for a meeting that produced no transcript at all.
        self.ensure_speaker_on(&conn, id, SpeakerSource::Mic.default_speaker_key(), "You")?;
        self.ensure_speaker_on(
            &conn,
            id,
            SpeakerSource::System.default_speaker_key(),
            "Others",
        )?;

        info!("Started meeting {id}: {}", crate::utils::redact_text(title));
        Ok(id)
    }

    /// Record that capture finished, moving the meeting to `Processing`.
    ///
    /// Separate from [`Self::complete_meeting`] because transcription, diarization
    /// and notes all run after the audio stops, and a user reopening the app
    /// during that window should see "processing", not a meeting that looks
    /// finished but has no notes.
    pub fn finish_capture(
        &self,
        meeting_id: i64,
        ended_at: i64,
        mic_file: Option<&str>,
        system_file: Option<&str>,
    ) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE meetings
                SET ended_at = ?2, status = ?3, mic_file = ?4, system_file = ?5
              WHERE id = ?1",
            params![
                meeting_id,
                ended_at,
                MeetingStatus::Processing.as_db_str(),
                mic_file,
                system_file
            ],
        )?;
        Ok(())
    }

    /// Mark everything done.
    ///
    /// Never on a meeting that is still recording: notes can be written mid-call,
    /// and marking that row complete took it out of `reconcile_interrupted`, so a
    /// crash later in the call left it with no end time, reading as live forever.
    /// The post-call job marks it complete once the recording has stopped.
    pub fn complete_meeting(&self, meeting_id: i64) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE meetings SET status = ?2 WHERE id = ?1 AND status <> ?3",
            params![
                meeting_id,
                MeetingStatus::Complete.as_db_str(),
                MeetingStatus::Recording.as_db_str()
            ],
        )?;
        Ok(())
    }

    /// Reconcile meetings the app was recording when it stopped existing.
    ///
    /// Call once at startup, before anything reads the meeting list. A row left
    /// in `recording` cannot be recording — this process just started — so
    /// presenting it as live would tell the user audio is being captured when it
    /// is not. Anything already transcribed stays readable; only the status
    /// changes, and `ended_at` is backfilled from the last segment so the
    /// duration reflects what was actually captured rather than showing blank.
    ///
    /// Returns the number of meetings reconciled.
    ///
    /// Also recovers the recordings of meetings that died before `stop` wrote
    /// their file names: the files are named after the meeting id, so they can be
    /// found, and their headers only need the lengths `hound` never got to write.
    /// Without this they were orphaned — never listed, never deleted, around
    /// 460 MB an hour.
    pub fn reconcile_interrupted(&self) -> Result<usize> {
        let conn = self.open()?;
        // Best effort: failing to recover a file must not stop the status fix,
        // which is what keeps a dead meeting from reading as live.
        if let Err(e) = self.recover_orphaned_audio(&conn) {
            warn!("Could not recover interrupted meeting audio: {e}");
        }

        let changed = conn.execute(
            "UPDATE meetings
                SET status = ?1,
                    ended_at = COALESCE(
                        ended_at,
                        (SELECT started_at + MAX(end_ms) / 1000
                           FROM meeting_segments
                          WHERE meeting_segments.meeting_id = meetings.id),
                        started_at
                    )
              WHERE status IN (?2, ?3)",
            params![
                MeetingStatus::Interrupted.as_db_str(),
                MeetingStatus::Recording.as_db_str(),
                MeetingStatus::Processing.as_db_str(),
            ],
        )?;

        if changed > 0 {
            warn!("Reconciled {changed} meeting(s) interrupted by an app restart");
        }
        Ok(changed)
    }

    /// Backfill `mic_file` / `system_file` for unfinished meetings whose
    /// recordings exist on disk, repairing each header first.
    ///
    /// `interrupted` is included so recordings orphaned by builds before this
    /// existed are recovered too. A file that is not a WAV this can repair is
    /// left unreferenced rather than handed to a decoder that will reject it.
    fn recover_orphaned_audio(&self, conn: &Connection) -> Result<()> {
        let candidates = {
            let mut stmt = conn.prepare(
                "SELECT id, mic_file IS NULL, system_file IS NULL
                   FROM meetings
                  WHERE status IN (?1, ?2, ?3)
                    AND (mic_file IS NULL OR system_file IS NULL)",
            )?;
            let rows = stmt
                .query_map(
                    params![
                        MeetingStatus::Recording.as_db_str(),
                        MeetingStatus::Processing.as_db_str(),
                        MeetingStatus::Interrupted.as_db_str(),
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, bool>(2)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };

        for (id, mic_missing, system_missing) in candidates {
            let wanted = [
                (mic_missing, "mic_file", format!("meeting_{id}_mic.wav")),
                (
                    system_missing,
                    "system_file",
                    format!("meeting_{id}_system.wav"),
                ),
            ];
            for (missing, column, file_name) in wanted {
                if !missing {
                    continue;
                }
                let path = self.audio_path(&file_name);
                if !path.exists() {
                    continue;
                }
                match repair_wav_header(&path) {
                    Ok(repaired) => {
                        // `column` is one of two literals above, never input.
                        conn.execute(
                            &format!("UPDATE meetings SET {column} = ?2 WHERE id = ?1"),
                            params![id, file_name],
                        )?;
                        info!(
                            "Recovered {file_name} for meeting {id}{}",
                            if repaired { " (header repaired)" } else { "" }
                        );
                    }
                    Err(e) => warn!("Could not recover {file_name} for meeting {id}: {e}"),
                }
            }
        }
        Ok(())
    }

    /* ─────────────────────────── segments ─────────────────────────── */

    /// Append a batch of transcript segments in a single transaction.
    ///
    /// One transaction for the batch, not one per segment: a meeting writes for
    /// hours alongside whatever else the app is doing, and per-row commits are
    /// how `history.rs` earned an intermittent `SQLITE_BUSY`.
    pub fn append_segments(&self, meeting_id: i64, segments: &[NewSegment]) -> Result<()> {
        if segments.is_empty() {
            return Ok(());
        }

        let mut conn = self.open()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO meeting_segments
                     (meeting_id, source, speaker_key, start_ms, end_ms, text, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for segment in segments {
                let speaker_key = segment
                    .speaker_key
                    .clone()
                    .unwrap_or_else(|| segment.source.default_speaker_key().to_string());
                stmt.execute(params![
                    meeting_id,
                    segment.source.as_db_str(),
                    speaker_key,
                    segment.start_ms,
                    segment.end_ms,
                    segment.text,
                    segment.confidence,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Read one page of a transcript, ordered by time.
    ///
    /// Ordering is `(start_ms, id)`, not `start_ms` alone. The two streams are
    /// transcribed independently and can produce segments with identical start
    /// times; without the tiebreak, SQLite is free to return them in a different
    /// order on each query and the transcript would visibly reshuffle as the
    /// user scrolls.
    pub fn segments(&self, meeting_id: i64, limit: u32, offset: u32) -> Result<PaginatedSegments> {
        let conn = self.open()?;

        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_segments WHERE meeting_id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )?;

        // Fetch one extra row to learn whether another page exists without a
        // second COUNT query.
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, source, speaker_key, start_ms, end_ms, text, confidence
               FROM meeting_segments
              WHERE meeting_id = ?1
              ORDER BY start_ms ASC, id ASC
              LIMIT ?2 OFFSET ?3",
        )?;
        let mut segments = stmt
            .query_map(
                params![meeting_id, limit as i64 + 1, offset],
                row_to_segment,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let has_more = segments.len() > limit as usize;
        segments.truncate(limit as usize);

        Ok(PaginatedSegments {
            segments,
            has_more,
            total,
        })
    }

    /// Every segment for a meeting, in order.
    ///
    /// For summarization and export only. Deliberately not what the UI calls.
    pub fn all_segments(&self, meeting_id: i64) -> Result<Vec<MeetingSegment>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, source, speaker_key, start_ms, end_ms, text, confidence
               FROM meeting_segments
              WHERE meeting_id = ?1
              ORDER BY start_ms ASC, id ASC",
        )?;
        let segments = stmt
            .query_map(params![meeting_id], row_to_segment)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(segments)
    }

    /// Segments whose text matches an FTS5 query, best-matching first.
    ///
    /// `match_query` must already be a **valid FTS5 expression** — see
    /// [`super::retrieve::fts_query`], which is what builds one. A raw user
    /// question passed straight through here is a SQL error, not an empty result:
    /// FTS5 treats bare punctuation and unbalanced quotes as syntax errors.
    ///
    /// Ranked by `bm25()` rather than by time, because the caller wants the
    /// passages most likely to answer a question, not the earliest ones. The
    /// caller re-sorts into meeting order once it has picked its set — a model
    /// reading passages out of order will happily infer a sequence of events that
    /// never happened.
    pub fn search_segments(
        &self,
        meeting_id: i64,
        match_query: &str,
        limit: u32,
    ) -> Result<Vec<MeetingSegment>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.meeting_id, s.source, s.speaker_key, s.start_ms, s.end_ms,
                    s.text, s.confidence
               FROM meeting_segments_fts AS f
               JOIN meeting_segments AS s ON s.id = f.rowid
              WHERE f.text MATCH ?1
                AND s.meeting_id = ?2
              ORDER BY bm25(meeting_segments_fts) ASC
              LIMIT ?3",
        )?;
        let segments = stmt
            .query_map(params![match_query, meeting_id, limit], row_to_segment)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(segments)
    }

    /// Segments around a set of row ids, so a retrieved hit arrives with context.
    ///
    /// A matched utterance on its own is frequently unanswerable — "yeah, let's do
    /// that" matches a question about a decision and contains none of it. This
    /// widens each hit by `context` segments on either side, in one query, and
    /// returns the union in meeting order.
    ///
    /// The window is computed over `(meeting_id, start_ms)` ordering rather than
    /// over ids, because the two streams interleave: consecutive ids can be two
    /// different speakers minutes apart.
    pub fn segments_around(
        &self,
        meeting_id: i64,
        ids: &[i64],
        context: u32,
    ) -> Result<Vec<MeetingSegment>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;

        // Rank every segment in the meeting by time, find the ranks of the hits,
        // then take everything within `context` ranks of any hit. Done in SQL so a
        // long meeting does not round-trip its whole transcript to Rust just to
        // pick forty rows out of it.
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "WITH ordered AS (
                 SELECT id, ROW_NUMBER() OVER (ORDER BY start_ms ASC, id ASC) AS rn
                   FROM meeting_segments
                  WHERE meeting_id = ?1
             ),
             hits AS (
                 SELECT rn FROM ordered WHERE id IN ({placeholders})
             )
             SELECT s.id, s.meeting_id, s.source, s.speaker_key, s.start_ms, s.end_ms,
                    s.text, s.confidence
               FROM ordered AS o
               JOIN meeting_segments AS s ON s.id = o.id
              WHERE EXISTS (
                    SELECT 1 FROM hits WHERE ABS(hits.rn - o.rn) <= ?{context_index}
              )
              ORDER BY s.start_ms ASC, s.id ASC",
            context_index = ids.len() + 2
        );

        let mut stmt = conn.prepare(&sql)?;
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(ids.len() + 2);
        bound.push(Box::new(meeting_id));
        for id in ids {
            bound.push(Box::new(*id));
        }
        bound.push(Box::new(context));
        let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|b| b.as_ref()).collect();

        let segments = stmt
            .query_map(refs.as_slice(), row_to_segment)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(segments)
    }

    /// System-side segments only, in time order.
    ///
    /// Narrower than [`Self::all_segments`] because diarization never reads the
    /// microphone side — that attribution is a hardware fact and clustering could
    /// only make it worse — and an hour of transcript is a thousand rows of text
    /// this pass has no use for.
    pub fn system_segments(&self, meeting_id: i64) -> Result<Vec<MeetingSegment>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, source, speaker_key, start_ms, end_ms, text, confidence
               FROM meeting_segments
              WHERE meeting_id = ?1 AND source = ?2
              ORDER BY start_ms ASC, id ASC",
        )?;
        let segments = stmt
            .query_map(
                params![meeting_id, SpeakerSource::System.as_db_str()],
                row_to_segment,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(segments)
    }

    /// Mark a meeting diarized without writing any span.
    ///
    /// Needed for the one-speaker outcome: there is nothing to relabel — renaming a
    /// lone remote voice to "Speaker 1" would imply others exist — but the flag has
    /// to be set or the pass re-runs on every open, spending a minute of CPU each
    /// time to reach the same conclusion.
    ///
    /// This cannot go through [`Self::apply_diarization`], which deliberately
    /// early-returns on an empty span list without touching the flag.
    pub fn mark_diarized(&self, meeting_id: i64) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE meetings SET diarized = 1 WHERE id = ?1",
            params![meeting_id],
        )?;
        Ok(())
    }

    /* ─────────────────────────── meetings ─────────────────────────── */

    /// List meetings, newest first.
    pub fn list_meetings(&self, limit: u32, offset: u32) -> Result<PaginatedMeetings> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.title, m.started_at, m.ended_at, m.status, m.mic_file,
                    m.system_file, m.my_notes, m.notes, m.notes_template, m.language,
                    m.diarized,
                    (SELECT COUNT(*) FROM meeting_segments s WHERE s.meeting_id = m.id)
               FROM meetings m
              ORDER BY m.started_at DESC
              LIMIT ?1 OFFSET ?2",
        )?;
        let mut meetings = stmt
            .query_map(params![limit as i64 + 1, offset], row_to_meeting)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let has_more = meetings.len() > limit as usize;
        meetings.truncate(limit as usize);

        Ok(PaginatedMeetings { meetings, has_more })
    }

    /// One meeting by id, or `None` if it does not exist.
    pub fn get_meeting(&self, meeting_id: i64) -> Result<Option<Meeting>> {
        let conn = self.open()?;
        let meeting = conn
            .query_row(
                "SELECT m.id, m.title, m.started_at, m.ended_at, m.status, m.mic_file,
                        m.system_file, m.my_notes, m.notes, m.notes_template, m.language,
                        m.diarized,
                        (SELECT COUNT(*) FROM meeting_segments s WHERE s.meeting_id = m.id)
                   FROM meetings m
                  WHERE m.id = ?1",
                params![meeting_id],
                row_to_meeting,
            )
            .optional()?;
        Ok(meeting)
    }

    pub fn rename_meeting(&self, meeting_id: i64, title: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE meetings SET title = ?2 WHERE id = ?1",
            params![meeting_id, title],
        )?;
        Ok(())
    }

    /// Save the user's own notes.
    pub fn set_my_notes(&self, meeting_id: i64, notes: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE meetings SET my_notes = ?2 WHERE id = ?1",
            params![meeting_id, notes],
        )?;
        Ok(())
    }

    /// Save generated notes and the template that produced them.
    pub fn set_notes(&self, meeting_id: i64, notes: &str, template: Option<&str>) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE meetings SET notes = ?2, notes_template = ?3 WHERE id = ?1",
            params![meeting_id, notes, template],
        )?;
        Ok(())
    }

    /// Replace the text of generated notes the user edited — today, ticking a
    /// next step off — leaving the template that wrote them on record.
    pub fn update_notes_text(&self, meeting_id: i64, notes: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE meetings SET notes = ?2 WHERE id = ?1 AND notes IS NOT NULL",
            params![meeting_id, notes],
        )?;
        Ok(())
    }

    /// Delete a meeting and return the audio files that are now safe to unlink.
    ///
    /// The caller unlinks **after** this returns, not before: the row is the
    /// record of truth, and removing files first then failing to commit leaves
    /// the user with a meeting that opens to nothing. Cascade deletes handle the
    /// segments and speakers.
    pub fn delete_meeting(&self, meeting_id: i64) -> Result<Vec<PathBuf>> {
        let mut conn = self.open()?;
        let tx = conn.transaction()?;

        let files: (Option<String>, Option<String>) = tx
            .query_row(
                "SELECT mic_file, system_file FROM meetings WHERE id = ?1",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or((None, None));

        tx.execute("DELETE FROM meetings WHERE id = ?1", params![meeting_id])?;
        tx.commit()?;

        let mut paths = Vec::new();
        for file in [files.0, files.1].into_iter().flatten() {
            // Guard against a stored name that escapes the audio directory. The
            // names are ours, but a hand-edited database should not be able to
            // turn "delete this meeting" into deleting an arbitrary file.
            if file.contains("..") || file.contains('/') || file.contains('\\') {
                warn!("Refusing to unlink suspicious meeting audio name: {file}");
                continue;
            }
            paths.push(self.audio_path(&file));
        }
        Ok(paths)
    }

    /* ─────────────────────────── speakers ─────────────────────────── */

    /// Speakers known for a meeting, with the local user first.
    pub fn speakers(&self, meeting_id: i64) -> Result<Vec<MeetingSpeaker>> {
        let conn = self.open()?;
        let me_key = SpeakerSource::Mic.default_speaker_key();
        let mut stmt = conn.prepare(
            "SELECT speaker_key, display_name
               FROM meeting_speakers
              WHERE meeting_id = ?1
              ORDER BY (speaker_key = ?2) DESC, speaker_key ASC",
        )?;
        let speakers = stmt
            .query_map(params![meeting_id, me_key], |row| {
                let speaker_key: String = row.get(0)?;
                Ok(MeetingSpeaker {
                    is_me: speaker_key == me_key,
                    speaker_key,
                    display_name: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(speakers)
    }

    /// Give a speaker a display name, creating the row if needed.
    pub fn set_speaker_name(
        &self,
        meeting_id: i64,
        speaker_key: &str,
        display_name: &str,
    ) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO meeting_speakers (meeting_id, speaker_key, display_name)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (meeting_id, speaker_key)
             DO UPDATE SET display_name = excluded.display_name",
            params![meeting_id, speaker_key, display_name],
        )?;
        Ok(())
    }

    fn ensure_speaker_on(
        &self,
        conn: &Connection,
        meeting_id: i64,
        speaker_key: &str,
        display_name: &str,
    ) -> Result<()> {
        // `DO NOTHING`, not `DO UPDATE`: seeding must never overwrite a name the
        // user has already typed.
        conn.execute(
            "INSERT INTO meeting_speakers (meeting_id, speaker_key, display_name)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (meeting_id, speaker_key) DO NOTHING",
            params![meeting_id, speaker_key, display_name],
        )?;
        Ok(())
    }

    /// Apply diarization results: rewrite `speaker_key` on system-side segments
    /// whose time range falls inside a labelled span.
    ///
    /// Microphone segments are never touched. Their attribution came from the
    /// hardware and is not something a clustering model is entitled to revise.
    ///
    /// `spans` must be sorted by start time. Matching is by midpoint containment
    /// rather than overlap, so a segment straddling a speaker change is assigned
    /// to whoever held the majority of it instead of being split or duplicated.
    pub fn apply_diarization(&self, meeting_id: i64, spans: &[DiarizedSpan]) -> Result<usize> {
        if spans.is_empty() {
            return Ok(0);
        }

        let mut conn = self.open()?;
        let tx = conn.transaction()?;
        let mut updated = 0usize;

        {
            // Collect the system-side segments first; assigning inside an open
            // query on the same table is not something to rely on.
            let mut stmt = tx.prepare(
                "SELECT id, start_ms, end_ms
                   FROM meeting_segments
                  WHERE meeting_id = ?1 AND source = ?2
                  ORDER BY start_ms ASC",
            )?;
            let rows = stmt
                .query_map(
                    params![meeting_id, SpeakerSource::System.as_db_str()],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let mut update = tx.prepare(
                "UPDATE meeting_segments SET speaker_key = ?2, confidence = ?3 WHERE id = ?1",
            )?;

            for (id, start_ms, end_ms) in rows {
                let midpoint = start_ms + (end_ms - start_ms) / 2;
                if let Some(span) = spans
                    .iter()
                    .find(|s| midpoint >= s.start_ms && midpoint < s.end_ms)
                {
                    update.execute(params![id, span.speaker_key, span.confidence])?;
                    updated += 1;
                }
            }

            // Register any speaker the spans introduced so the UI can rename it.
            let mut seen = std::collections::BTreeSet::new();
            for span in spans {
                if seen.insert(span.speaker_key.clone()) {
                    tx.execute(
                        "INSERT INTO meeting_speakers (meeting_id, speaker_key, display_name)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT (meeting_id, speaker_key) DO NOTHING",
                        params![meeting_id, &span.speaker_key, &span.default_label],
                    )?;
                }
            }

            tx.execute(
                "UPDATE meetings SET diarized = 1 WHERE id = ?1",
                params![meeting_id],
            )?;
        }

        tx.commit()?;
        debug!("Diarization relabelled {updated} segment(s) in meeting {meeting_id}");
        Ok(updated)
    }
}

/// One contiguous span of speech attributed to a speaker by diarization.
#[derive(Clone, Debug, PartialEq)]
pub struct DiarizedSpan {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Stable key, e.g. `"spk_0"`.
    pub speaker_key: String,
    /// Name to show until the user renames it, e.g. `"Speaker 1"`.
    pub default_label: String,
    /// Clustering confidence, if the backend produced one.
    pub confidence: Option<f32>,
}

fn row_to_segment(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSegment> {
    let source: String = row.get(2)?;
    Ok(MeetingSegment {
        id: row.get(0)?,
        meeting_id: row.get(1)?,
        source: SpeakerSource::from_db_str(&source),
        speaker_key: row.get(3)?,
        start_ms: row.get(4)?,
        end_ms: row.get(5)?,
        text: row.get(6)?,
        confidence: row.get(7)?,
    })
}

fn row_to_meeting(row: &rusqlite::Row<'_>) -> rusqlite::Result<Meeting> {
    let status: String = row.get(4)?;
    let diarized: i64 = row.get(11)?;
    Ok(Meeting {
        id: row.get(0)?,
        title: row.get(1)?,
        started_at: row.get(2)?,
        ended_at: row.get(3)?,
        status: MeetingStatus::from_db_str(&status),
        mic_file: row.get(5)?,
        system_file: row.get(6)?,
        my_notes: row.get(7)?,
        notes: row.get(8)?,
        notes_template: row.get(9)?,
        language: row.get(10)?,
        diarized: diarized != 0,
        segment_count: row.get(12)?,
    })
}

/// Make a recording whose writer never finalised readable again.
///
/// `hound` writes the RIFF and `data` lengths as zero up front and fills them in
/// on finalise, so a process that died mid-meeting leaves both at zero and every
/// decoder reads an empty file. The chunks are walked rather than assuming a
/// layout: for 32-bit float mono `hound` writes a 40-byte WAVEFORMATEXTENSIBLE
/// `fmt ` chunk, so its data starts at byte 68, not the 44 of a plain PCM file.
///
/// The data length becomes whatever follows the `data` header, rounded down to
/// whole sample frames — a write cut off mid-sample is trimmed. A header whose
/// `data` length is already non-zero and fits the file is left alone: that file
/// was finalised. Returns whether anything was rewritten; an error means this is
/// not a WAV that can be repaired.
fn repair_wav_header(path: &Path) -> std::io::Result<bool> {
    use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};

    fn invalid(what: &str) -> Error {
        Error::new(ErrorKind::InvalidData, what.to_string())
    }

    let mut file = fs::OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();

    let mut riff = [0u8; 12];
    file.read_exact(&mut riff)
        .map_err(|_| invalid("shorter than a RIFF header"))?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(invalid("not a RIFF/WAVE file"));
    }

    let mut block_align: u64 = 0;
    let mut pos: u64 = 12;
    loop {
        if pos + 8 > len {
            return Err(invalid("no data chunk"));
        }
        let mut header = [0u8; 8];
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut header)?;
        let size = u64::from(u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        let body = pos + 8;

        if &header[0..4] == b"fmt " {
            // nBlockAlign: bytes per sample frame, at offset 12 of the body.
            let mut fmt = [0u8; 14];
            if size < 14 || body + 14 > len {
                return Err(invalid("truncated fmt chunk"));
            }
            file.read_exact(&mut fmt)?;
            block_align = u64::from(u16::from_le_bytes([fmt[12], fmt[13]]));
        } else if &header[0..4] == b"data" {
            if block_align == 0 {
                return Err(invalid("data before a usable fmt chunk"));
            }
            let available = len - body;
            if size != 0 && size <= available {
                return Ok(false);
            }
            // Both length fields are u32; the RIFF one also covers the header.
            let limit = u64::from(u32::MAX) - (body - 8);
            let data_len = available.min(limit);
            let data_len = data_len - data_len % block_align;

            file.seek(SeekFrom::Start(4))?;
            file.write_all(&((body - 8 + data_len) as u32).to_le_bytes())?;
            file.seek(SeekFrom::Start(pos + 4))?;
            file.write_all(&(data_len as u32).to_le_bytes())?;
            // Trim a torn final sample, but never audio past the 4 GiB limit.
            if available - data_len < block_align {
                file.set_len(body + data_len)?;
            }
            file.sync_all()?;
            return Ok(true);
        }

        // Chunks are word-aligned: an odd-sized body is followed by a pad byte.
        pos = body + size + (size & 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a store backed by a temp directory, bypassing `tauri::AppHandle`
    /// so the storage layer is testable without an app instance.
    fn temp_store() -> (MeetingStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio_dir = dir.path().join("meetings");
        fs::create_dir_all(&audio_dir).expect("audio dir");
        let store = MeetingStore {
            db_path: dir.path().join("meetings.db"),
            audio_dir,
        };
        store.init().expect("migrations");
        (store, dir)
    }

    fn segment(source: SpeakerSource, start_ms: i64, end_ms: i64, text: &str) -> NewSegment {
        NewSegment {
            source,
            speaker_key: None,
            start_ms,
            end_ms,
            text: text.to_string(),
            confidence: None,
        }
    }

    #[test]
    fn migrations_apply_to_a_fresh_database() {
        let (store, _dir) = temp_store();
        let page = store.list_meetings(10, 0).expect("list");
        assert!(page.meetings.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn create_and_read_back_a_meeting() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, Some("en")).unwrap();

        let meeting = store.get_meeting(id).unwrap().expect("meeting exists");
        assert_eq!(meeting.title, "Standup");
        assert_eq!(meeting.status, MeetingStatus::Recording);
        assert_eq!(meeting.language.as_deref(), Some("en"));
        assert_eq!(meeting.segment_count, 0);
        assert!(!meeting.diarized);
    }

    /// The two channel speakers must exist from the moment a meeting is created,
    /// so a meeting that captured nothing still renders a speaker list.
    #[test]
    fn channel_speakers_are_seeded_on_creation() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 1_000, None).unwrap();

        let speakers = store.speakers(id).unwrap();
        assert_eq!(speakers.len(), 2);
        assert!(speakers[0].is_me, "the local user must sort first");
        assert_eq!(speakers[0].speaker_key, "me");
    }

    #[test]
    fn segments_default_their_speaker_key_from_the_source() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store
            .append_segments(
                id,
                &[
                    segment(SpeakerSource::Mic, 0, 1_000, "hello"),
                    segment(SpeakerSource::System, 1_000, 2_000, "hi there"),
                ],
            )
            .unwrap();

        let page = store.segments(id, 10, 0).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.segments[0].speaker_key.as_deref(), Some("me"));
        assert_eq!(page.segments[1].speaker_key.as_deref(), Some("them"));
    }

    #[test]
    fn segments_are_returned_in_time_order() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        // Inserted out of order on purpose.
        store
            .append_segments(
                id,
                &[
                    segment(SpeakerSource::System, 5_000, 6_000, "third"),
                    segment(SpeakerSource::Mic, 0, 1_000, "first"),
                    segment(SpeakerSource::Mic, 2_000, 3_000, "second"),
                ],
            )
            .unwrap();

        let texts: Vec<_> = store
            .all_segments(id)
            .unwrap()
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn segment_pagination_reports_more_pages() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Long", 0, None).unwrap();
        let batch: Vec<_> = (0..5)
            .map(|i| segment(SpeakerSource::Mic, i * 1_000, i * 1_000 + 900, "line"))
            .collect();
        store.append_segments(id, &batch).unwrap();

        let first = store.segments(id, 2, 0).unwrap();
        assert_eq!(first.segments.len(), 2);
        assert!(first.has_more);
        assert_eq!(first.total, 5);

        let last = store.segments(id, 2, 4).unwrap();
        assert_eq!(last.segments.len(), 1);
        assert!(!last.has_more);
    }

    #[test]
    fn empty_batch_is_a_no_op() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store.append_segments(id, &[]).unwrap();
        assert_eq!(store.segments(id, 10, 0).unwrap().total, 0);
    }

    #[test]
    fn finish_capture_moves_to_processing_and_records_files() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 1_000, None).unwrap();
        store
            .finish_capture(id, 1_600, Some("mic.wav"), Some("sys.wav"))
            .unwrap();

        let meeting = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Processing);
        assert_eq!(meeting.duration_secs(), Some(600));
        assert_eq!(meeting.mic_file.as_deref(), Some("mic.wav"));

        store.complete_meeting(id).unwrap();
        assert_eq!(
            store.get_meeting(id).unwrap().unwrap().status,
            MeetingStatus::Complete
        );
    }

    /// A meeting left mid-recording by a crash must not still claim to be
    /// recording, and must report the duration it actually captured.
    #[test]
    fn interrupted_meetings_are_reconciled_with_a_derived_end_time() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Crashed", 1_000, None).unwrap();
        store
            .append_segments(id, &[segment(SpeakerSource::Mic, 0, 45_000, "words")])
            .unwrap();

        assert_eq!(store.reconcile_interrupted().unwrap(), 1);

        let meeting = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Interrupted);
        assert_eq!(meeting.ended_at, Some(1_045));
    }

    /// With no transcript at all there is nothing to derive an end time from, so
    /// the meeting collapses to zero length rather than leaving `ended_at` null
    /// and rendering as still running.
    #[test]
    fn interrupted_meeting_without_segments_gets_zero_duration() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Empty", 2_000, None).unwrap();
        store.reconcile_interrupted().unwrap();

        let meeting = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(meeting.ended_at, Some(2_000));
        assert_eq!(meeting.duration_secs(), Some(0));
    }

    #[test]
    fn a_completed_meeting_is_not_reconciled() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Done", 1_000, None).unwrap();
        store.finish_capture(id, 1_100, None, None).unwrap();
        store.complete_meeting(id).unwrap();

        assert_eq!(store.reconcile_interrupted().unwrap(), 0);
        assert_eq!(
            store.get_meeting(id).unwrap().unwrap().status,
            MeetingStatus::Complete
        );
    }

    /// Notes written mid-call must not take a live meeting out of crash
    /// reconciliation.
    #[test]
    fn notes_during_a_recording_do_not_mark_it_complete() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Live", 1_000, None).unwrap();
        store.complete_meeting(id).unwrap();

        assert_eq!(
            store.get_meeting(id).unwrap().unwrap().status,
            MeetingStatus::Recording
        );
        assert_eq!(store.reconcile_interrupted().unwrap(), 1);
    }

    #[test]
    fn notes_and_titles_persist() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Untitled", 0, None).unwrap();

        store.rename_meeting(id, "Pricing debate").unwrap();
        store.set_my_notes(id, "ask about the discount").unwrap();
        store
            .set_notes(id, "## Summary\n- decided", Some("standup"))
            .unwrap();

        let meeting = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(meeting.title, "Pricing debate");
        assert_eq!(meeting.my_notes, "ask about the discount");
        assert_eq!(meeting.notes.as_deref(), Some("## Summary\n- decided"));
        assert_eq!(meeting.notes_template.as_deref(), Some("standup"));
    }

    #[test]
    fn renaming_a_speaker_survives_a_reread() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store.set_speaker_name(id, "them", "Priya").unwrap();

        let speakers = store.speakers(id).unwrap();
        let them = speakers.iter().find(|s| s.speaker_key == "them").unwrap();
        assert_eq!(them.display_name, "Priya");
    }

    /// Seeding runs on every meeting creation, but it must never clobber a name
    /// the user typed.
    #[test]
    fn seeding_does_not_overwrite_a_user_supplied_name() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store.set_speaker_name(id, "me", "Abhishek").unwrap();

        let conn = store.open().unwrap();
        store.ensure_speaker_on(&conn, id, "me", "You").unwrap();

        let speakers = store.speakers(id).unwrap();
        assert_eq!(speakers[0].display_name, "Abhishek");
    }

    #[test]
    fn deleting_a_meeting_cascades_and_reports_its_audio() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store
            .append_segments(id, &[segment(SpeakerSource::Mic, 0, 1_000, "hi")])
            .unwrap();
        store
            .finish_capture(id, 100, Some("mic.wav"), Some("sys.wav"))
            .unwrap();

        let files = store.delete_meeting(id).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("mic.wav"));

        assert!(store.get_meeting(id).unwrap().is_none());
        // Cascade must have taken the segments with it.
        assert_eq!(store.segments(id, 10, 0).unwrap().total, 0);
    }

    /// A hand-edited database must not be able to turn a meeting delete into an
    /// arbitrary file delete.
    #[test]
    fn delete_refuses_path_traversal_in_stored_file_names() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store
            .finish_capture(id, 100, Some("../../evil.wav"), Some("ok.wav"))
            .unwrap();

        let files = store.delete_meeting(id).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("ok.wav"));
    }

    #[test]
    fn diarization_relabels_only_the_system_stream() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store
            .append_segments(
                id,
                &[
                    segment(SpeakerSource::Mic, 0, 1_000, "mine"),
                    segment(SpeakerSource::System, 1_000, 2_000, "theirs a"),
                    segment(SpeakerSource::System, 3_000, 4_000, "theirs b"),
                ],
            )
            .unwrap();

        let spans = vec![
            DiarizedSpan {
                start_ms: 900,
                end_ms: 2_500,
                speaker_key: "spk_0".into(),
                default_label: "Speaker 1".into(),
                confidence: Some(0.8),
            },
            DiarizedSpan {
                start_ms: 2_500,
                end_ms: 5_000,
                speaker_key: "spk_1".into(),
                default_label: "Speaker 2".into(),
                confidence: Some(0.6),
            },
        ];
        assert_eq!(store.apply_diarization(id, &spans).unwrap(), 2);

        let segments = store.all_segments(id).unwrap();
        // The microphone segment is untouched: its attribution is a hardware
        // fact, not a clustering opinion.
        assert_eq!(segments[0].speaker_key.as_deref(), Some("me"));
        assert_eq!(segments[1].speaker_key.as_deref(), Some("spk_0"));
        assert_eq!(segments[2].speaker_key.as_deref(), Some("spk_1"));
        assert_eq!(segments[1].confidence, Some(0.8));

        assert!(store.get_meeting(id).unwrap().unwrap().diarized);

        // The new speakers must be renameable.
        let keys: Vec<_> = store
            .speakers(id)
            .unwrap()
            .into_iter()
            .map(|s| s.speaker_key)
            .collect();
        assert!(keys.contains(&"spk_0".to_string()));
        assert!(keys.contains(&"spk_1".to_string()));
    }

    /// A segment whose midpoint lands in no span keeps its channel-derived key
    /// rather than being blanked.
    #[test]
    fn diarization_leaves_unmatched_segments_alone() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        store
            .append_segments(
                id,
                &[segment(SpeakerSource::System, 10_000, 11_000, "later")],
            )
            .unwrap();

        let spans = vec![DiarizedSpan {
            start_ms: 0,
            end_ms: 1_000,
            speaker_key: "spk_0".into(),
            default_label: "Speaker 1".into(),
            confidence: None,
        }];
        assert_eq!(store.apply_diarization(id, &spans).unwrap(), 0);
        assert_eq!(
            store.all_segments(id).unwrap()[0].speaker_key.as_deref(),
            Some("them")
        );
    }

    #[test]
    fn empty_diarization_is_a_no_op() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Call", 0, None).unwrap();
        assert_eq!(store.apply_diarization(id, &[]).unwrap(), 0);
        assert!(!store.get_meeting(id).unwrap().unwrap().diarized);
    }

    #[test]
    fn meeting_list_paginates_newest_first() {
        let (store, _dir) = temp_store();
        store.create_meeting("older", 1_000, None).unwrap();
        store.create_meeting("newer", 2_000, None).unwrap();

        let page = store.list_meetings(1, 0).unwrap();
        assert_eq!(page.meetings.len(), 1);
        assert_eq!(page.meetings[0].title, "newer");
        assert!(page.has_more);
    }

    /* ─────────────────── full-text search over the transcript ─────────────── */

    /// The whole point of migration 2: text written through the ordinary insert
    /// path is searchable, without any explicit indexing step.
    #[test]
    fn appended_segments_become_searchable() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        store
            .append_segments(
                id,
                &[
                    segment(
                        SpeakerSource::Mic,
                        0,
                        1_000,
                        "we agreed on the pricing model",
                    ),
                    segment(
                        SpeakerSource::System,
                        1_000,
                        2_000,
                        "the invoice goes out Friday",
                    ),
                    segment(
                        SpeakerSource::Mic,
                        2_000,
                        3_000,
                        "nothing to do with either",
                    ),
                ],
            )
            .unwrap();

        let hits = store.search_segments(id, "\"pricing\"*", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("pricing model"));
    }

    /// `OR` of prefix terms is exactly what `retrieve::fts_query` emits, so this is
    /// the shape that actually runs in production.
    #[test]
    fn a_prefix_or_query_matches_several_segments() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        store
            .append_segments(
                id,
                &[
                    segment(SpeakerSource::Mic, 0, 1_000, "budgeting for next quarter"),
                    segment(SpeakerSource::System, 1_000, 2_000, "the budget is fixed"),
                    segment(SpeakerSource::Mic, 2_000, 3_000, "unrelated chatter"),
                ],
            )
            .unwrap();

        let hits = store
            .search_segments(id, "\"budget\"* OR \"quarter\"*", 10)
            .unwrap();
        assert_eq!(
            hits.len(),
            2,
            "a prefix term must match both 'budget' and 'budgeting'"
        );
    }

    /// A search must never reach into another meeting's transcript.
    #[test]
    fn search_is_scoped_to_one_meeting() {
        let (store, _dir) = temp_store();
        let first = store.create_meeting("One", 1_000, None).unwrap();
        let second = store.create_meeting("Two", 2_000, None).unwrap();
        store
            .append_segments(
                first,
                &[segment(
                    SpeakerSource::Mic,
                    0,
                    1_000,
                    "the secret pricing deal",
                )],
            )
            .unwrap();
        store
            .append_segments(
                second,
                &[segment(SpeakerSource::Mic, 0, 1_000, "pricing again")],
            )
            .unwrap();

        let hits = store.search_segments(second, "\"pricing\"*", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, second);
    }

    #[test]
    fn a_query_that_matches_nothing_returns_nothing() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        store
            .append_segments(id, &[segment(SpeakerSource::Mic, 0, 1_000, "hello there")])
            .unwrap();
        assert!(store
            .search_segments(id, "\"kubernetes\"*", 10)
            .unwrap()
            .is_empty());
    }

    /// Deleting a meeting cascades to its segments; the index has to follow or a
    /// later search returns rows whose base table entry is gone (and the JOIN then
    /// silently drops them, hiding the leak until the index is huge).
    #[test]
    fn deleting_a_meeting_removes_it_from_the_index() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        store
            .append_segments(
                id,
                &[segment(
                    SpeakerSource::Mic,
                    0,
                    1_000,
                    "distinctive phrase here",
                )],
            )
            .unwrap();
        assert_eq!(
            store
                .search_segments(id, "\"distinctive\"*", 10)
                .unwrap()
                .len(),
            1
        );

        store.delete_meeting(id).unwrap();
        assert!(store
            .search_segments(id, "\"distinctive\"*", 10)
            .unwrap()
            .is_empty());
    }

    /// A matched line is frequently unanswerable alone — "yeah, let's do that"
    /// matches a question about a decision and contains none of it.
    #[test]
    fn context_expansion_widens_a_hit_on_both_sides() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        let rows: Vec<NewSegment> = (0..9)
            .map(|i| {
                segment(
                    if i % 2 == 0 {
                        SpeakerSource::Mic
                    } else {
                        SpeakerSource::System
                    },
                    i * 1_000,
                    i * 1_000 + 900,
                    &format!("utterance number {i}"),
                )
            })
            .collect();
        store.append_segments(id, &rows).unwrap();

        let all = store.all_segments(id).unwrap();
        let middle = all[4].id;
        let widened = store.segments_around(id, &[middle], 2).unwrap();

        assert_eq!(widened.len(), 5, "two on each side plus the hit");
        assert!(widened.iter().any(|s| s.id == middle));
        // And in meeting order, because a model reading passages out of order
        // infers a sequence of events that never happened.
        let starts: Vec<i64> = widened.iter().map(|s| s.start_ms).collect();
        assert!(starts.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    /// A hit at the very start has no earlier context, and clamping must not drop
    /// the hit itself or duplicate rows.
    #[test]
    fn context_expansion_clamps_at_the_edges() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        store
            .append_segments(
                id,
                &[
                    segment(SpeakerSource::Mic, 0, 900, "first"),
                    segment(SpeakerSource::System, 1_000, 1_900, "second"),
                ],
            )
            .unwrap();

        let all = store.all_segments(id).unwrap();
        let widened = store.segments_around(id, &[all[0].id], 5).unwrap();
        assert_eq!(
            widened.len(),
            2,
            "the whole (short) meeting, with no repeats"
        );
    }

    /// Two nearby hits share context, and the union must not contain a row twice —
    /// a duplicated utterance reads to the model as something said twice.
    #[test]
    fn overlapping_context_windows_are_deduplicated() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        let rows: Vec<NewSegment> = (0..6)
            .map(|i| {
                segment(
                    SpeakerSource::Mic,
                    i * 1_000,
                    i * 1_000 + 900,
                    &format!("line {i}"),
                )
            })
            .collect();
        store.append_segments(id, &rows).unwrap();

        let all = store.all_segments(id).unwrap();
        let widened = store
            .segments_around(id, &[all[2].id, all[3].id], 2)
            .unwrap();

        let mut ids: Vec<i64> = widened.iter().map(|s| s.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "no segment may appear twice");
    }

    #[test]
    fn expanding_no_hits_yields_nothing() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Standup", 1_000, None).unwrap();
        assert!(store.segments_around(id, &[], 2).unwrap().is_empty());
    }

    #[test]
    fn missing_meeting_reads_as_none() {
        let (store, _dir) = temp_store();
        assert!(store.get_meeting(9_999).unwrap().is_none());
    }

    /* ───────────────── recordings left behind by a dead process ───────────── */

    /// The spec the recorder writes (see `session::open_wav`).
    fn meeting_spec() -> hound::WavSpec {
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        }
    }

    /// Write the file a recorder killed mid-meeting leaves: `hound`'s header with
    /// both lengths still zero, the samples, and half of one more sample.
    fn write_unfinalised_wav(path: &Path, samples: &[f32]) {
        let mut writer = hound::WavWriter::create(path, meeting_spec()).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();

        let mut bytes = fs::read(path).unwrap();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap();
        bytes[4..8].fill(0);
        bytes[data + 4..data + 8].fill(0);
        bytes.extend_from_slice(&[0x12, 0x34]);
        fs::write(path, bytes).unwrap();
    }

    fn read_samples(path: &Path) -> Vec<f32> {
        hound::WavReader::open(path)
            .unwrap()
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// The layout the repair walks rather than assumes: 32-bit float makes hound
    /// write WAVEFORMATEXTENSIBLE, so the data chunk is not at the usual 36.
    #[test]
    fn hound_puts_float_mono_data_after_an_extensible_fmt_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.wav");
        write_unfinalised_wav(&path, &[0.5]);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[16..20], &40u32.to_le_bytes());
        assert_eq!(bytes.windows(4).position(|w| w == b"data"), Some(60));
    }

    #[test]
    fn an_unfinalised_recording_is_readable_after_repair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meeting_1_mic.wav");
        let samples: Vec<f32> = (0..1_000).map(|i| (i as f32 / 1_000.0) - 0.5).collect();
        write_unfinalised_wav(&path, &samples);

        // Unrepaired, a decoder sees no audio at all.
        assert_eq!(hound::WavReader::open(&path).unwrap().len(), 0);

        assert!(repair_wav_header(&path).unwrap());
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec(), meeting_spec());
        assert_eq!(
            read_samples(&path),
            samples,
            "the torn tail sample is trimmed"
        );
    }

    #[test]
    fn a_finalised_recording_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("done.wav");
        let mut writer = hound::WavWriter::create(&path, meeting_spec()).unwrap();
        for sample in [0.1f32, -0.2, 0.3] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        let before = fs::read(&path).unwrap();

        assert!(!repair_wav_header(&path).unwrap());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_file_that_is_not_a_wav_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.wav");
        fs::write(&path, b"definitely not audio").unwrap();
        assert!(repair_wav_header(&path).is_err());
        fs::write(&path, b"RIF").unwrap();
        assert!(repair_wav_header(&path).is_err());
    }

    /// A meeting killed mid-recording never had its file names written. Startup
    /// finds the recordings by their deterministic names, repairs them, and puts
    /// them back on the row, so they can be played, diarized and deleted.
    #[test]
    fn reconciling_recovers_the_recordings_of_a_dead_meeting() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Crashed", 1_000, None).unwrap();
        let samples = vec![0.25f32; 320];
        write_unfinalised_wav(
            &store.audio_path(&format!("meeting_{id}_mic.wav")),
            &samples,
        );

        store.reconcile_interrupted().unwrap();

        let meeting = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Interrupted);
        let mic_file = meeting.mic_file.expect("the mic recording is recovered");
        assert_eq!(mic_file, format!("meeting_{id}_mic.wav"));
        assert_eq!(read_samples(&store.audio_path(&mic_file)), samples);
        // No system file on disk, so nothing to point at.
        assert_eq!(meeting.system_file, None);

        // And the recovered file now goes when the meeting does.
        assert_eq!(store.delete_meeting(id).unwrap().len(), 1);
    }

    /// Recordings orphaned by earlier builds, on rows already reconciled, are
    /// recovered on the next launch too.
    #[test]
    fn an_already_interrupted_meeting_gets_its_recording_back() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Old crash", 1_000, None).unwrap();
        store.reconcile_interrupted().unwrap();
        write_unfinalised_wav(
            &store.audio_path(&format!("meeting_{id}_system.wav")),
            &[0.5; 16],
        );

        store.reconcile_interrupted().unwrap();
        assert_eq!(
            store.get_meeting(id).unwrap().unwrap().system_file,
            Some(format!("meeting_{id}_system.wav"))
        );
    }

    #[test]
    fn an_unrepairable_file_is_not_put_on_the_row() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Crashed", 1_000, None).unwrap();
        fs::write(store.audio_path(&format!("meeting_{id}_mic.wav")), b"").unwrap();

        store.reconcile_interrupted().unwrap();
        let meeting = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Interrupted);
        assert_eq!(meeting.mic_file, None);
    }

    /// A finished meeting is none of reconciliation's business, even with a file
    /// of the right name next to it.
    #[test]
    fn a_completed_meeting_is_not_given_files() {
        let (store, _dir) = temp_store();
        let id = store.create_meeting("Done", 1_000, None).unwrap();
        store.finish_capture(id, 1_100, None, None).unwrap();
        store.complete_meeting(id).unwrap();
        write_unfinalised_wav(&store.audio_path(&format!("meeting_{id}_mic.wav")), &[0.5]);

        store.reconcile_interrupted().unwrap();
        assert_eq!(store.get_meeting(id).unwrap().unwrap().mic_file, None);
    }
}
