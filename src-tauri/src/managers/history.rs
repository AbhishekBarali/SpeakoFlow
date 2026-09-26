use anyhow::{anyhow, Result};
use chrono::{DateTime, Days, Local, Months, NaiveDate, TimeZone, Utc};
use log::{debug, error, info};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use tauri_specta::Event;

use crate::llm_client::ChatMessage;
use crate::settings::RecordingRetentionPeriod;

/// Database migrations for transcription history.
/// Each migration is applied in order. The library tracks which migrations
/// have been applied using SQLite's user_version pragma.
///
/// Note: For users upgrading from tauri-plugin-sql, migrate_from_tauri_plugin_sql()
/// converts the old _sqlx_migrations table tracking to the user_version pragma,
/// ensuring migrations don't re-run on existing databases.
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );",
    ),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;"),
    // Assistant conversations live alongside transcriptions but in their own
    // table: one row per conversation session, with the messages stored as a
    // JSON array. `timestamp` is when the conversation started, `updated_at`
    // is the last turn — the History view sorts by the latter so an active
    // chat stays near the top.
    M::up(
        "CREATE TABLE IF NOT EXISTS assistant_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            title TEXT NOT NULL,
            messages TEXT NOT NULL
        );",
    ),
    // Lifetime dictation usage. Aggregated per local calendar day rather than
    // derived from `transcription_history`, because retention prunes that table
    // and these are lifetime numbers: deleting a recording must not make the
    // user look like they dictated less. `timed_words` counts only the words of
    // entries whose audio duration was known, so words-per-minute is computed
    // over the same entries as `audio_seconds`.
    M::up(
        "CREATE TABLE IF NOT EXISTS usage_daily (
            day TEXT PRIMARY KEY,
            dictations INTEGER NOT NULL DEFAULT 0,
            words INTEGER NOT NULL DEFAULT 0,
            timed_words INTEGER NOT NULL DEFAULT 0,
            audio_seconds REAL NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS usage_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    ),
];

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedHistory {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

/// A persisted assistant conversation. One row per session; `messages` is the
/// ordered turn-by-turn transcript (the same `{role, content}` shape the
/// assistant panel renders).
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct AssistantHistoryEntry {
    pub id: i64,
    /// When the conversation was first saved (seconds since epoch).
    pub timestamp: i64,
    /// When the most recent turn was added (seconds since epoch).
    pub updated_at: i64,
    /// Short label derived from the first user message.
    pub title: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedAssistantHistory {
    pub entries: Vec<AssistantHistoryEntry>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action")]
pub enum HistoryUpdatePayload {
    #[serde(rename = "added")]
    Added { entry: HistoryEntry },
    #[serde(rename = "updated")]
    Updated { entry: HistoryEntry },
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
    #[serde(rename = "toggled")]
    Toggled { id: i64 },
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct HistoryEntry {
    pub id: i64,
    pub file_name: String,
    pub timestamp: i64,
    pub saved: bool,
    pub title: String,
    pub transcription_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
    pub post_process_requested: bool,
}

pub struct HistoryManager {
    app_handle: AppHandle,
    recordings_dir: PathBuf,
    db_path: PathBuf,
}

/// What the retention settings mean, resolved away from both the database and the
/// clock. Keeping this separate is what makes the policy testable: every bug this
/// feature had was in the decision, not in the SQL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionPlan {
    /// Delete nothing, ever. "Forever" must stay forever.
    KeepAll,
    /// Keep at most this many unstarred recordings, newest first.
    ByCount(usize),
    /// Delete unstarred recordings older than this epoch second.
    OlderThan(i64),
}

/// Turn the persisted settings into a plan.
///
/// `now` is a parameter rather than a call to `Utc::now()` so the time-based
/// branches can be asserted against a fixed clock.
pub fn resolve_retention_plan(
    period: RecordingRetentionPeriod,
    limit: usize,
    custom_days: u32,
    now: DateTime<Utc>,
) -> RetentionPlan {
    match period {
        RecordingRetentionPeriod::Never => RetentionPlan::KeepAll,
        // A limit of 0 would delete every unstarred recording. The settings
        // command rejects it, but clamping here too means a store hand-edited or
        // carried over from an older build cannot wipe a history either.
        RecordingRetentionPeriod::PreserveLimit => {
            RetentionPlan::ByCount(limit.max(crate::settings::MIN_HISTORY_LIMIT))
        }
        RecordingRetentionPeriod::Days3 => RetentionPlan::OlderThan(cutoff_days(now, 3)),
        RecordingRetentionPeriod::Weeks2 => RetentionPlan::OlderThan(cutoff_days(now, 14)),
        // Calendar months, not 90 days. The old code subtracted 3 * 30 days and
        // said "approximate" in a comment, which silently deleted up to two days
        // early depending on the month.
        RecordingRetentionPeriod::Months3 => RetentionPlan::OlderThan(
            now.checked_sub_months(Months::new(3))
                .unwrap_or(now)
                .timestamp(),
        ),
        RecordingRetentionPeriod::CustomDays => {
            let days = custom_days.clamp(
                crate::settings::MIN_RECORDING_RETENTION_DAYS,
                crate::settings::MAX_RECORDING_RETENTION_DAYS,
            );
            RetentionPlan::OlderThan(cutoff_days(now, days.into()))
        }
    }
}

fn cutoff_days(now: DateTime<Utc>, days: i64) -> i64 {
    now.timestamp() - days * 24 * 60 * 60
}

/// One local calendar day of dictation usage, as shown in the recent-activity
/// chart.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct UsageDay {
    /// Local calendar date, `%Y-%m-%d`.
    pub day: String,
    pub dictations: i64,
    pub words: i64,
    pub audio_seconds: f64,
}

/// Lifetime dictation usage. These survive history deletion and retention
/// pruning; they are aggregated in `usage_daily`, not derived from rows.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct UsageStats {
    pub total_dictations: i64,
    pub total_words: i64,
    /// Words from dictations whose audio duration is known. Words per minute is
    /// `timed_words / (total_audio_seconds / 60)`, never `total_words / ...`,
    /// so untimed (backfilled) entries cannot inflate the rate.
    pub timed_words: i64,
    pub total_audio_seconds: f64,
    pub today_words: i64,
    pub today_dictations: i64,
    pub current_streak_days: u32,
    pub longest_streak_days: u32,
    pub active_days: u32,
    /// Active days within the last `USAGE_RECENT_WINDOW_DAYS` local days
    /// including today, ascending. Days without a dictation are omitted.
    pub recent_days: Vec<UsageDay>,
}

/// `usage_meta` key recording that existing history has been folded into
/// `usage_daily`, so the backfill runs exactly once per database.
const USAGE_BACKFILL_KEY: &str = "backfill_v1";

/// The backfill reads WAV headers only for this many of the newest contributing
/// rows. It bounds startup cost on a large history; older rows still count
/// words and dictations but are treated as untimed.
const USAGE_BACKFILL_TIMED_ROWS: usize = 2000;

/// Length of the `recent_days` window, including today. Fifty-three weeks, so
/// the Insights activity grid can fill a full year of week columns whatever
/// weekday today is. `usage_daily` holds one row per active day and is already
/// read in full for the lifetime totals, so the wider window costs nothing.
const USAGE_RECENT_WINDOW_DAYS: u64 = 371;

/// Count the words in a transcript: whitespace-separated tokens containing at
/// least one alphanumeric character, so a lone em dash or `...` is not a word.
pub fn count_words(text: &str) -> i64 {
    text.split_whitespace()
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .count() as i64
}

/// The local calendar date (`%Y-%m-%d`) of an epoch-second timestamp. Usage is
/// bucketed by the day the user experienced, not the UTC day.
pub fn local_day(ts_secs: i64) -> String {
    Local
        .timestamp_opt(ts_secs, 0)
        .earliest()
        .unwrap_or_else(Local::now)
        .format("%Y-%m-%d")
        .to_string()
}

/// Duration of a WAV file in seconds, read from its header only. `None` on any
/// error, a zero sample rate, or a zero-length file.
pub fn wav_duration_seconds(path: &Path) -> Option<f64> {
    let reader = hound::WavReader::open(path).ok()?;
    let sample_rate = reader.spec().sample_rate;
    let frames = reader.duration();
    if sample_rate == 0 || frames == 0 {
        return None;
    }
    Some(f64::from(frames) / f64::from(sample_rate))
}

/// `(current, longest)` streaks of consecutive active days.
///
/// The current streak ends today, or yesterday when today has no activity yet —
/// otherwise every streak would read as broken each morning until the first
/// dictation. Input may be unsorted and contain duplicates.
pub fn compute_streaks(active_days: &[NaiveDate], today: NaiveDate) -> (u32, u32) {
    let mut days = active_days.to_vec();
    days.sort_unstable();
    days.dedup();

    let mut longest = 0u32;
    let mut run = 0u32;
    let mut prev: Option<NaiveDate> = None;
    for &day in &days {
        run = match prev {
            Some(p) if p.succ_opt() == Some(day) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(day);
    }

    let is_active = |d: NaiveDate| days.binary_search(&d).is_ok();
    let mut cursor = if is_active(today) {
        Some(today)
    } else {
        today.pred_opt().filter(|&y| is_active(y))
    };
    let mut current = 0u32;
    while let Some(day) = cursor.filter(|&d| is_active(d)) {
        current += 1;
        cursor = day.pred_opt();
    }

    (current, longest)
}

/// Add deltas to one day's usage row, creating it if needed. Every column is
/// clamped at zero, so a correction can never drive a counter negative.
pub fn apply_usage_delta(
    conn: &Connection,
    day: &str,
    d_dictations: i64,
    d_words: i64,
    d_timed_words: i64,
    d_seconds: f64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO usage_daily (day, dictations, words, timed_words, audio_seconds)
         VALUES (?1, MAX(0, ?2), MAX(0, ?3), MAX(0, ?4), MAX(0.0, ?5))
         ON CONFLICT(day) DO UPDATE SET
             dictations = MAX(0, usage_daily.dictations + ?2),
             words = MAX(0, usage_daily.words + ?3),
             timed_words = MAX(0, usage_daily.timed_words + ?4),
             audio_seconds = MAX(0.0, usage_daily.audio_seconds + ?5)",
        params![day, d_dictations, d_words, d_timed_words, d_seconds],
    )?;
    Ok(())
}

/// The usage delta `(dictations, words, timed_words, seconds)` for a retry that
/// replaced a transcript of `old_words` words with one of `new_words`, or `None`
/// when nothing changes. `duration` is only called when a delta is needed.
fn retry_usage_delta(
    old_words: i64,
    new_words: i64,
    duration: impl FnOnce() -> Option<f64>,
) -> Option<(i64, i64, i64, f64)> {
    if old_words == new_words {
        return None;
    }
    let duration = duration();
    let timed = |words: i64| if duration.is_some() { words } else { 0 };
    let seconds = duration.unwrap_or(0.0);
    Some(match (old_words, new_words) {
        (0, n) => (1, n, timed(n), seconds),
        (o, 0) => (-1, -o, -timed(o), -seconds),
        (o, n) => (0, n - o, timed(n - o), 0.0),
    })
}

/// Fold existing history into `usage_daily`, once per database.
///
/// Guarded by the `backfill_v1` meta key and done in one transaction, so it is
/// idempotent and a crash midway leaves nothing half-counted. WAV durations are
/// looked up only for the newest [`USAGE_BACKFILL_TIMED_ROWS`] contributing
/// rows. Returns the number of rows counted (0 when already done).
fn backfill_usage(
    conn: &mut Connection,
    duration_for: impl Fn(&str) -> Option<f64>,
) -> Result<usize> {
    let tx = conn.transaction()?;
    let done: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM usage_meta WHERE key = ?1)",
        params![USAGE_BACKFILL_KEY],
        |row| row.get(0),
    )?;
    if done {
        return Ok(0);
    }

    // day -> (dictations, words, timed_words, seconds)
    let mut per_day: BTreeMap<String, (i64, i64, i64, f64)> = BTreeMap::new();
    let mut contributing = 0usize;
    {
        let mut stmt = tx.prepare(
            "SELECT timestamp, file_name, transcription_text
             FROM transcription_history ORDER BY id DESC",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let timestamp: i64 = row.get(0)?;
            let file_name: String = row.get(1)?;
            let text: String = row.get(2)?;
            let words = count_words(&text);
            if words == 0 {
                continue;
            }
            let duration = if contributing < USAGE_BACKFILL_TIMED_ROWS {
                duration_for(&file_name)
            } else {
                None
            };
            contributing += 1;

            let agg = per_day.entry(local_day(timestamp)).or_default();
            agg.0 += 1;
            agg.1 += words;
            if let Some(seconds) = duration {
                agg.2 += words;
                agg.3 += seconds;
            }
        }
    }

    for (day, (dictations, words, timed_words, seconds)) in &per_day {
        apply_usage_delta(&tx, day, *dictations, *words, *timed_words, *seconds)?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO usage_meta (key, value) VALUES (?1, ?2)",
        params![USAGE_BACKFILL_KEY, Utc::now().timestamp().to_string()],
    )?;
    tx.commit()?;
    Ok(contributing)
}

/// Read lifetime usage as of the local date `today`.
pub fn compute_usage_stats(conn: &Connection, today: NaiveDate) -> rusqlite::Result<UsageStats> {
    let window_start = today
        .checked_sub_days(Days::new(USAGE_RECENT_WINDOW_DAYS - 1))
        .unwrap_or(today);
    let today_key = today.format("%Y-%m-%d").to_string();

    let mut stats = UsageStats {
        total_dictations: 0,
        total_words: 0,
        timed_words: 0,
        total_audio_seconds: 0.0,
        today_words: 0,
        today_dictations: 0,
        current_streak_days: 0,
        longest_streak_days: 0,
        active_days: 0,
        recent_days: Vec::new(),
    };
    let mut active: Vec<NaiveDate> = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT day, dictations, words, timed_words, audio_seconds
         FROM usage_daily ORDER BY day ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, f64>(4)?,
        ))
    })?;
    for row in rows {
        let (day, dictations, words, timed_words, audio_seconds) = row?;
        stats.total_dictations += dictations;
        stats.total_words += words;
        stats.timed_words += timed_words;
        stats.total_audio_seconds += audio_seconds;

        // A row corrected down to zero dictations is not an active day.
        if dictations <= 0 {
            continue;
        }
        if day == today_key {
            stats.today_words = words;
            stats.today_dictations = dictations;
        }
        let Ok(date) = NaiveDate::parse_from_str(&day, "%Y-%m-%d") else {
            continue;
        };
        active.push(date);
        if date >= window_start && date <= today {
            stats.recent_days.push(UsageDay {
                day,
                dictations,
                words,
                audio_seconds,
            });
        }
    }

    stats.active_days = active.len() as u32;
    let (current, longest) = compute_streaks(&active, today);
    stats.current_streak_days = current;
    stats.longest_streak_days = longest;
    Ok(stats)
}

impl HistoryManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Create recordings directory in app data dir
        let app_data_dir = crate::portable::app_data_dir(app_handle)?;
        let recordings_dir = app_data_dir.join("recordings");
        let db_path = app_data_dir.join("history.db");

        // Ensure recordings directory exists
        if !recordings_dir.exists() {
            fs::create_dir_all(&recordings_dir)?;
            debug!("Created recordings directory: {:?}", recordings_dir);
        }

        let manager = Self {
            app_handle: app_handle.clone(),
            recordings_dir,
            db_path,
        };

        // Initialize database and run migrations synchronously
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing database at {:?}", self.db_path);

        let mut conn = Connection::open(&self.db_path)?;

        // Handle migration from tauri-plugin-sql to rusqlite_migration
        // tauri-plugin-sql used _sqlx_migrations table, rusqlite_migration uses user_version pragma
        self.migrate_from_tauri_plugin_sql(&conn)?;

        // Create migrations object and run to latest version
        let migrations = Migrations::new(MIGRATIONS.to_vec());

        // Validate migrations in debug builds
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid migrations");

        // Get current version before migration
        let version_before: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        debug!("Database version before migration: {}", version_before);

        // Apply any pending migrations
        migrations.to_latest(&mut conn)?;

        // Get version after migration
        let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if version_after > version_before {
            info!(
                "Database migrated from version {} to {}",
                version_before, version_after
            );
        } else {
            debug!("Database already at latest version {}", version_after);
        }

        // Fold pre-existing history into the lifetime usage tables, once. A
        // failure here costs the stats for old entries, never the app launch.
        let recordings_dir = self.recordings_dir.clone();
        match backfill_usage(&mut conn, |file_name| {
            wav_duration_seconds(&recordings_dir.join(file_name))
        }) {
            Ok(0) => {}
            Ok(counted) => info!("Backfilled usage stats from {} history entries", counted),
            Err(e) => error!("Usage stats backfill failed: {}", e),
        }

        Ok(())
    }

    /// Migrate from tauri-plugin-sql's migration tracking to rusqlite_migration's.
    /// tauri-plugin-sql used a _sqlx_migrations table, while rusqlite_migration uses
    /// SQLite's user_version pragma. This function checks if the old system was in use
    /// and sets the user_version accordingly so migrations don't re-run.
    fn migrate_from_tauri_plugin_sql(&self, conn: &Connection) -> Result<()> {
        // Check if the old _sqlx_migrations table exists
        let has_sqlx_migrations: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_sqlx_migrations {
            return Ok(());
        }

        // Check current user_version
        let current_version: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if current_version > 0 {
            // Already migrated to rusqlite_migration system
            return Ok(());
        }

        // Get the highest version from the old migrations table
        let old_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if old_version > 0 {
            info!(
                "Migrating from tauri-plugin-sql (version {}) to rusqlite_migration",
                old_version
            );

            // Set user_version to match the old migration state
            conn.pragma_update(None, "user_version", old_version)?;

            // Optionally drop the old migrations table (keeping it doesn't hurt)
            // conn.execute("DROP TABLE IF EXISTS _sqlx_migrations", [])?;

            info!(
                "Migration tracking converted: user_version set to {}",
                old_version
            );
        }

        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    fn map_history_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        Ok(HistoryEntry {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            timestamp: row.get("timestamp")?,
            saved: row.get("saved")?,
            title: row.get("title")?,
            transcription_text: row.get("transcription_text")?,
            post_processed_text: row.get("post_processed_text")?,
            post_process_prompt: row.get("post_process_prompt")?,
            post_process_requested: row.get("post_process_requested")?,
        })
    }

    pub fn recordings_dir(&self) -> &std::path::Path {
        &self.recordings_dir
    }

    /// Count a freshly saved entry toward lifetime usage. Never fails the save:
    /// usage is a nice-to-have, the transcript is not.
    fn record_new_entry_usage(
        &self,
        conn: &Connection,
        timestamp: i64,
        file_name: &str,
        transcription_text: &str,
    ) {
        // Raw transcript, not the post-processed text: WPM measures how fast
        // the user spoke, not how long the cleaned-up output is.
        let words = count_words(transcription_text);
        if words == 0 {
            return;
        }
        let duration = wav_duration_seconds(&self.recordings_dir.join(file_name));
        let timed_words = if duration.is_some() { words } else { 0 };
        if let Err(e) = apply_usage_delta(
            conn,
            &local_day(timestamp),
            1,
            words,
            timed_words,
            duration.unwrap_or(0.0),
        ) {
            error!("Failed to record usage stats: {}", e);
        }
    }

    /// Lifetime dictation usage as of the local date today.
    pub fn get_usage_stats(&self) -> Result<UsageStats> {
        let conn = self.get_connection()?;
        Ok(compute_usage_stats(&conn, Local::now().date_naive())?)
    }

    /// Save a new history entry to the database.
    /// The WAV file should already have been written to the recordings directory.
    pub fn save_entry(
        &self,
        file_name: String,
        transcription_text: String,
        post_process_requested: bool,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
    ) -> Result<HistoryEntry> {
        let timestamp = Utc::now().timestamp();
        let title = self.format_timestamp_title(timestamp);

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &file_name,
                timestamp,
                false,
                &title,
                &transcription_text,
                &post_processed_text,
                &post_process_prompt,
                post_process_requested,
            ],
        )?;

        // Capture the id before touching usage: the usage UPSERT is an insert
        // too and would move `last_insert_rowid`.
        let id = conn.last_insert_rowid();
        self.record_new_entry_usage(&conn, timestamp, &file_name, &transcription_text);

        let entry = HistoryEntry {
            id,
            file_name,
            timestamp,
            saved: false,
            title,
            transcription_text,
            post_processed_text,
            post_process_prompt,
            post_process_requested,
        };

        debug!("Saved history entry with id {}", entry.id);
        drop(conn);

        // Publish the new row immediately, then enforce retention on every
        // insert. Previously the configured limit was only applied at startup
        // or when the setting changed, so a running session could grow without
        // bound and the History panel drifted away from the selected count.
        if let Err(e) = (HistoryUpdatePayload::Added {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        match self.cleanup_old_entries() {
            Ok(pruned) if pruned > 0 => {
                // The frontend refetches its first page so rows removed by
                // retention disappear immediately instead of lingering until
                // the History section is reopened.
                if let Err(e) = self.app_handle.emit("history-retention-applied", ()) {
                    error!("Failed to emit retention update: {}", e);
                }
            }
            Ok(_) => {}
            Err(e) => error!("History retention cleanup failed after save: {}", e),
        }

        Ok(entry)
    }

    /// Update an existing history entry with new transcription results (used by retry).
    pub fn update_transcription(
        &self,
        id: i64,
        transcription_text: String,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
    ) -> Result<HistoryEntry> {
        let conn = self.get_connection()?;
        // The previous transcript, read before it is overwritten, so usage can
        // be corrected by the difference. A failure here only skips the usage
        // correction; the retry itself still goes through.
        let previous: Option<(String, i64, String)> = conn
            .query_row(
                "SELECT transcription_text, timestamp, file_name
                 FROM transcription_history WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .unwrap_or_else(|e| {
                error!("Failed to read previous transcript for usage stats: {}", e);
                None
            });
        let updated = conn.execute(
            "UPDATE transcription_history
             SET transcription_text = ?1,
                 post_processed_text = ?2,
                 post_process_prompt = ?3
             WHERE id = ?4",
            params![
                transcription_text,
                post_processed_text,
                post_process_prompt,
                id
            ],
        )?;

        if updated == 0 {
            return Err(anyhow!("History entry {} not found", id));
        }

        if let Some((old_text, original_timestamp, file_name)) = previous {
            let delta = retry_usage_delta(
                count_words(&old_text),
                count_words(&transcription_text),
                || wav_duration_seconds(&self.recordings_dir.join(&file_name)),
            );
            if let Some((d_dictations, d_words, d_timed, d_seconds)) = delta {
                if let Err(e) = apply_usage_delta(
                    &conn,
                    &local_day(original_timestamp),
                    d_dictations,
                    d_words,
                    d_timed,
                    d_seconds,
                ) {
                    error!("Failed to update usage stats after retry: {}", e);
                }
            }
        }

        let entry = conn
            .query_row(
                "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                 FROM transcription_history WHERE id = ?1",
                params![id],
                Self::map_history_entry,
            )?;

        debug!("Updated transcription for history entry {}", id);

        if let Err(e) = (HistoryUpdatePayload::Updated {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    /// Apply the active recording-retention policy and return the number of
    /// database rows removed. Starred rows are never included.
    pub fn cleanup_old_entries(&self) -> Result<usize> {
        self.apply_plan(self.active_plan())
    }

    /// Resolve the retention settings into a database-independent plan.
    ///
    /// One `get_settings` call, not three. Each one is a full store read plus a
    /// deserialize, the `ensure_*_defaults` migrations, a keychain hydration, and
    /// possibly a write-back — and this runs after every dictation.
    pub fn active_plan(&self) -> RetentionPlan {
        let settings = crate::settings::get_settings(&self.app_handle);
        resolve_retention_plan(
            settings.recording_retention_period,
            settings.history_limit,
            settings.recording_retention_days,
            Utc::now(),
        )
    }

    /// How many recordings a plan *would* delete, without deleting anything.
    ///
    /// This exists so the History UI can tell the user "this will delete 410
    /// recordings" before it happens. Retention is irreversible and deletes the
    /// WAV file along with the row, so an unannounced switch to a stricter policy
    /// is the most destructive thing this panel can do.
    pub fn count_pending_deletions(&self, plan: RetentionPlan) -> Result<usize> {
        let conn = self.get_connection()?;
        Ok(Self::pending_deletions(&conn, plan)?.len())
    }

    /// Delete everything the plan selects, in one transaction, then remove the
    /// WAV files whose last referencing row is gone.
    pub fn apply_plan(&self, plan: RetentionPlan) -> Result<usize> {
        if matches!(plan, RetentionPlan::KeepAll) {
            return Ok(0);
        }

        let mut conn = self.get_connection()?;
        let doomed = Self::pending_deletions(&conn, plan)?;
        if doomed.is_empty() {
            return Ok(0);
        }

        // One transaction for the whole prune. Previously each row was its own
        // transaction on a *second* connection opened while the selecting
        // connection was still alive, which meant N commits and N fsyncs for a
        // prune of N rows and left the delete racing the insert in `save_entry`.
        // A `SQLITE_BUSY` there failed the command *after* the new setting had
        // already been persisted, so the UI rolled its value back while disk kept
        // the new one.
        let deleted_count;
        let orphaned_files;
        {
            let tx = conn.transaction()?;
            let mut removed = 0usize;
            for (id, _) in &doomed {
                // `saved = 0` is re-checked here, not just in the select above.
                // The rows were chosen before the transaction opened, and
                // `toggle_saved_status` stars a row on its own connection, so a
                // star landing in that window would otherwise be deleted anyway —
                // permanently, WAV included. Starring must always win.
                removed += tx.execute(
                    "DELETE FROM transcription_history WHERE id = ?1 AND saved = 0",
                    params![id],
                )?;
            }

            // Older builds named WAVs with second-level timestamps, so two rows
            // could reference the same file. Only delete a file once no surviving
            // row points at it; otherwise pruning one row breaks playback for a
            // row that is still visible. Deduped because two doomed rows sharing a
            // name would both see zero references.
            let mut still_referenced =
                tx.prepare("SELECT COUNT(*) FROM transcription_history WHERE file_name = ?1")?;
            let mut files: Vec<String> = Vec::new();
            for (_, file_name) in &doomed {
                if files.iter().any(|seen| seen == file_name) {
                    continue;
                }
                let refs: i64 = still_referenced.query_row(params![file_name], |row| row.get(0))?;
                if refs == 0 {
                    files.push(file_name.clone());
                }
            }
            drop(still_referenced);

            deleted_count = removed;
            orphaned_files = files;
            tx.commit()?;
        }

        // Files are removed only after the rows are committed. A failed unlink
        // leaves a stray WAV, which is recoverable; unlinking first would leave a
        // visible row whose audio is gone.
        for file_name in orphaned_files {
            let file_path = self.recordings_dir.join(&file_name);
            if file_path.exists() {
                if let Err(e) = fs::remove_file(&file_path) {
                    error!("Failed to delete WAV file {}: {}", file_name, e);
                } else {
                    debug!("Deleted old WAV file: {}", file_name);
                }
            }
        }

        info!(
            "History retention removed {} recording(s) ({:?})",
            deleted_count, plan
        );

        Ok(deleted_count)
    }

    /// Rows the plan selects for deletion, newest-first ordering preserved.
    fn pending_deletions(conn: &Connection, plan: RetentionPlan) -> Result<Vec<(i64, String)>> {
        match plan {
            RetentionPlan::KeepAll => Ok(Vec::new()),
            RetentionPlan::ByCount(limit) => Self::entries_beyond_unsaved_limit(conn, limit),
            RetentionPlan::OlderThan(cutoff) => Self::unsaved_entries_before(conn, cutoff),
        }
    }

    fn entries_beyond_unsaved_limit(conn: &Connection, limit: usize) -> Result<Vec<(i64, String)>> {
        // `LIMIT -1 OFFSET ?` is SQLite's "skip the newest N, return the rest".
        // Doing the skip in SQL matters because this runs after every dictation
        // and on every preview; the previous version pulled every unstarred row
        // into a Vec just to drop the first N.
        let mut stmt = conn.prepare(
            "SELECT id, file_name
             FROM transcription_history
             WHERE saved = 0
             ORDER BY timestamp DESC, id DESC
             LIMIT -1 OFFSET ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn unsaved_entries_before(
        conn: &Connection,
        cutoff_timestamp: i64,
    ) -> Result<Vec<(i64, String)>> {
        let mut stmt = conn.prepare(
            "SELECT id, file_name
             FROM transcription_history
             WHERE saved = 0 AND timestamp < ?1",
        )?;
        let rows = stmt.query_map(params![cutoff_timestamp], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub async fn get_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(100));

        let mut entries: Vec<HistoryEntry> = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     WHERE id < ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                let result = stmt
                    .query_map(params![cursor_id, fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (None, Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     ORDER BY id DESC
                     LIMIT ?1",
                )?;
                let result = stmt
                    .query_map(params![fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (_, None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     ORDER BY id DESC",
                )?;
                let result = stmt
                    .query_map([], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedHistory { entries, has_more })
    }

    #[cfg(test)]
    fn get_latest_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    /// Get the latest entry with non-empty transcription text.
    pub fn get_latest_completed_entry(&self) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        Self::get_latest_completed_entry_with_conn(&conn)
    }

    fn get_latest_completed_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             WHERE transcription_text != ''
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    pub async fn toggle_saved_status(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get current saved status
        let current_saved: bool = conn.query_row(
            "SELECT saved FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get("saved"),
        )?;

        let new_saved = !current_saved;

        conn.execute(
            "UPDATE transcription_history SET saved = ?1 WHERE id = ?2",
            params![new_saved, id],
        )?;

        debug!("Toggled saved status for entry {}: {}", id, new_saved);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Toggled { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    pub fn get_audio_file_path(&self, file_name: &str) -> PathBuf {
        self.recordings_dir.join(file_name)
    }

    pub async fn get_entry_by_id(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             WHERE id = ?1",
        )?;

        let entry = stmt.query_row([id], Self::map_history_entry).optional()?;

        Ok(entry)
    }

    pub async fn delete_entry(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        let file_name = conn
            .query_row(
                "SELECT file_name FROM transcription_history WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        conn.execute(
            "DELETE FROM transcription_history WHERE id = ?1",
            params![id],
        )?;

        // Old rows may share a second-resolution filename. Keep the WAV while
        // any other row still references it.
        if let Some(file_name) = file_name {
            let remaining_references: i64 = conn.query_row(
                "SELECT COUNT(*) FROM transcription_history WHERE file_name = ?1",
                params![&file_name],
                |row| row.get(0),
            )?;
            if remaining_references == 0 {
                let file_path = self.get_audio_file_path(&file_name);
                if file_path.exists() {
                    if let Err(e) = fs::remove_file(&file_path) {
                        error!("Failed to delete audio file {}: {}", file_name, e);
                    }
                }
            }
        }

        debug!("Deleted history entry with id: {}", id);

        if let Err(e) = (HistoryUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    fn format_timestamp_title(&self, timestamp: i64) -> String {
        if let Some(utc_datetime) = DateTime::from_timestamp(timestamp, 0) {
            // Convert UTC to local timezone
            let local_datetime = utc_datetime.with_timezone(&Local);
            local_datetime.format("%B %e, %Y - %l:%M%p").to_string()
        } else {
            format!("Recording {}", timestamp)
        }
    }

    // -----------------------------------------------------------------------
    // Assistant conversation history
    // -----------------------------------------------------------------------

    /// Keep at most this many assistant conversations so the table can't grow
    /// without bound. Generous on purpose — conversations are small JSON rows
    /// and users expect their chat history to stick around.
    const ASSISTANT_SESSION_CAP: i64 = 500;

    fn map_assistant_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantHistoryEntry> {
        let messages_json: String = row.get("messages")?;
        // A malformed row shouldn't take down the whole list — fall back to an
        // empty transcript rather than erroring the query.
        let messages = serde_json::from_str::<Vec<ChatMessage>>(&messages_json).unwrap_or_default();
        Ok(AssistantHistoryEntry {
            id: row.get("id")?,
            timestamp: row.get("timestamp")?,
            updated_at: row.get("updated_at")?,
            title: row.get("title")?,
            messages,
        })
    }

    /// Derive a short, human-readable title from the first user message.
    fn derive_assistant_title(messages: &[ChatMessage]) -> String {
        let raw = messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        // Stored user messages may carry the screenshot marker; drop it.
        let cleaned = raw.replace(crate::assistant::SCREENSHOT_MARKER, "");
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            return "Conversation".to_string();
        }
        let title: String = trimmed.chars().take(80).collect();
        if trimmed.chars().count() > 80 {
            format!("{}…", title)
        } else {
            title
        }
    }

    /// Insert a new assistant conversation row and return it.
    pub fn create_assistant_session(
        &self,
        messages: &[ChatMessage],
    ) -> Result<AssistantHistoryEntry> {
        let now = Utc::now().timestamp();
        let title = Self::derive_assistant_title(messages);
        let messages_json = serde_json::to_string(messages)?;

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO assistant_history (timestamp, updated_at, title, messages)
             VALUES (?1, ?2, ?3, ?4)",
            params![now, now, &title, &messages_json],
        )?;
        let id = conn.last_insert_rowid();

        self.cleanup_assistant_sessions()?;

        debug!("Saved assistant session with id {}", id);

        Ok(AssistantHistoryEntry {
            id,
            timestamp: now,
            updated_at: now,
            title,
            messages: messages.to_vec(),
        })
    }

    /// Update an existing assistant conversation in place. Returns `Ok(None)`
    /// when the row no longer exists (e.g. it was deleted from the History
    /// view), so the caller can decide to create a fresh session instead.
    pub fn update_assistant_session(
        &self,
        id: i64,
        messages: &[ChatMessage],
    ) -> Result<Option<AssistantHistoryEntry>> {
        let now = Utc::now().timestamp();
        let title = Self::derive_assistant_title(messages);
        let messages_json = serde_json::to_string(messages)?;

        let conn = self.get_connection()?;
        let updated = conn.execute(
            "UPDATE assistant_history
             SET updated_at = ?1, title = ?2, messages = ?3
             WHERE id = ?4",
            params![now, &title, &messages_json, id],
        )?;

        if updated == 0 {
            return Ok(None);
        }

        let timestamp: i64 = conn.query_row(
            "SELECT timestamp FROM assistant_history WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;

        Ok(Some(AssistantHistoryEntry {
            id,
            timestamp,
            updated_at: now,
            title,
            messages: messages.to_vec(),
        }))
    }

    /// Page through assistant conversations, newest first (keyset pagination
    /// on `id`, mirroring `get_history_entries`).
    pub async fn get_assistant_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedAssistantHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(200));

        let mut entries: Vec<AssistantHistoryEntry> = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, timestamp, updated_at, title, messages
                     FROM assistant_history
                     WHERE id < ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                let result = stmt
                    .query_map(params![cursor_id, fetch_count], Self::map_assistant_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (None, Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, timestamp, updated_at, title, messages
                     FROM assistant_history
                     ORDER BY id DESC
                     LIMIT ?1",
                )?;
                let result = stmt
                    .query_map(params![fetch_count], Self::map_assistant_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (_, None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, timestamp, updated_at, title, messages
                     FROM assistant_history
                     ORDER BY id DESC",
                )?;
                let result = stmt
                    .query_map([], Self::map_assistant_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedAssistantHistory { entries, has_more })
    }

    pub fn delete_assistant_session(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute("DELETE FROM assistant_history WHERE id = ?1", params![id])?;
        debug!("Deleted assistant session with id: {}", id);
        Ok(())
    }

    /// Fetch a single assistant conversation by id (for resuming it in the
    /// panel from the History view). `Ok(None)` when the row no longer exists.
    pub fn get_assistant_session(&self, id: i64) -> Result<Option<AssistantHistoryEntry>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, updated_at, title, messages
             FROM assistant_history
             WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::map_assistant_entry)?;
        match rows.next() {
            Some(entry) => Ok(Some(entry?)),
            None => Ok(None),
        }
    }

    /// Trim the oldest conversations beyond [`Self::ASSISTANT_SESSION_CAP`].
    fn cleanup_assistant_sessions(&self) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "DELETE FROM assistant_history
             WHERE id NOT IN (
                 SELECT id FROM assistant_history ORDER BY id DESC LIMIT ?1
             )",
            params![Self::ASSISTANT_SESSION_CAP],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL,
                post_processed_text TEXT,
                post_process_prompt TEXT,
                post_process_requested BOOLEAN NOT NULL DEFAULT 0
            );",
        )
        .expect("create transcription_history table");
        conn
    }

    fn insert_entry(conn: &Connection, timestamp: i64, text: &str, post_processed: Option<&str>) {
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,

                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                format!("speakoflow-{}.wav", timestamp),
                timestamp,
                false,
                format!("Recording {}", timestamp),
                text,
                post_processed,
                Option::<String>::None,
                false,
            ],
        )
        .expect("insert history entry");
    }

    #[test]
    fn legacy_history_schema_migrates_to_current_baseline() {
        let mut conn = Connection::open_in_memory().expect("open legacy database");
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL
            );
            INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text
            ) VALUES ('legacy.wav', 10, 0, 'Legacy', 'hello');
            PRAGMA user_version = 1;",
        )
        .expect("create legacy schema");

        Migrations::new(MIGRATIONS.to_vec())
            .to_latest(&mut conn)
            .expect("migrate legacy schema");

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(transcription_history)")
            .expect("prepare table info")
            .query_map([], |row| row.get(1))
            .expect("query table info")
            .collect::<rusqlite::Result<_>>()
            .expect("collect columns");
        for required in [
            "post_processed_text",
            "post_process_prompt",
            "post_process_requested",
        ] {
            assert!(columns.iter().any(|column| column == required));
        }

        let requested: bool = conn
            .query_row(
                "SELECT post_process_requested FROM transcription_history WHERE file_name = 'legacy.wav'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated row");
        assert!(!requested);
    }

    #[test]
    fn get_latest_entry_returns_none_when_empty() {
        let conn = setup_conn();
        let entry = HistoryManager::get_latest_entry_with_conn(&conn).expect("fetch latest entry");
        assert!(entry.is_none());
    }

    #[test]
    fn get_latest_entry_returns_newest_entry() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "first", None);
        insert_entry(&conn, 200, "second", Some("processed"));

        let entry = HistoryManager::get_latest_entry_with_conn(&conn)
            .expect("fetch latest entry")
            .expect("entry exists");

        assert_eq!(entry.timestamp, 200);
        assert_eq!(entry.transcription_text, "second");
        assert_eq!(entry.post_processed_text.as_deref(), Some("processed"));
    }

    #[test]
    fn get_latest_completed_entry_skips_empty_entries() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "completed", None);
        insert_entry(&conn, 200, "", None);

        let entry = HistoryManager::get_latest_completed_entry_with_conn(&conn)
            .expect("fetch latest completed entry")
            .expect("completed entry exists");

        assert_eq!(entry.timestamp, 100);
        assert_eq!(entry.transcription_text, "completed");
    }

    #[test]
    fn count_retention_keeps_starred_recordings() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "oldest", None);
        insert_entry(&conn, 200, "middle", None);
        insert_entry(&conn, 300, "newest", None);
        insert_entry(&conn, 50, "starred oldest", None);
        conn.execute(
            "UPDATE transcription_history SET saved = 1 WHERE timestamp = 50",
            [],
        )
        .expect("star recording");

        let selected = HistoryManager::entries_beyond_unsaved_limit(&conn, 1)
            .expect("select count-retention entries");
        let selected_files: Vec<&str> = selected.iter().map(|(_, name)| name.as_str()).collect();

        assert_eq!(
            selected_files,
            vec!["speakoflow-200.wav", "speakoflow-100.wav"]
        );
        assert!(!selected_files.contains(&"speakoflow-50.wav"));
    }

    #[test]
    fn time_retention_keeps_starred_recordings() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "old unsaved", None);
        insert_entry(&conn, 50, "old starred", None);
        insert_entry(&conn, 300, "new unsaved", None);
        conn.execute(
            "UPDATE transcription_history SET saved = 1 WHERE timestamp = 50",
            [],
        )
        .expect("star recording");

        let selected = HistoryManager::unsaved_entries_before(&conn, 200)
            .expect("select time-retention entries");
        let selected_files: Vec<&str> = selected.iter().map(|(_, name)| name.as_str()).collect();

        assert_eq!(selected_files, vec!["speakoflow-100.wav"]);
        assert!(!selected_files.contains(&"speakoflow-50.wav"));
    }

    // ---------------------------------------------------------------------
    // Retention policy resolution
    // ---------------------------------------------------------------------

    fn at(iso: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(iso)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    /// "Forever" has to mean forever. This is the setting a user picks precisely
    /// because their history matters, so it gets its own test.
    #[test]
    fn never_deletes_nothing_regardless_of_other_settings() {
        assert_eq!(
            resolve_retention_plan(
                RecordingRetentionPeriod::Never,
                1,
                1,
                at("2026-09-12T00:00:00Z")
            ),
            RetentionPlan::KeepAll
        );
    }

    #[test]
    fn custom_days_uses_exactly_the_days_asked_for() {
        let now = at("2026-09-12T00:00:00Z");
        let plan = resolve_retention_plan(RecordingRetentionPeriod::CustomDays, 20, 20, now);
        assert_eq!(
            plan,
            RetentionPlan::OlderThan(at("2026-08-23T00:00:00Z").timestamp()),
            "20 days must mean 20 days"
        );
    }

    #[test]
    fn custom_days_is_clamped_into_range() {
        let now = at("2026-09-12T00:00:00Z");
        // Zero days would delete a recording the instant it was saved.
        assert_eq!(
            resolve_retention_plan(RecordingRetentionPeriod::CustomDays, 20, 0, now),
            RetentionPlan::OlderThan(at("2026-09-11T00:00:00Z").timestamp())
        );
        assert_eq!(
            resolve_retention_plan(RecordingRetentionPeriod::CustomDays, 20, u32::MAX, now),
            RetentionPlan::OlderThan(
                now.timestamp() - i64::from(crate::settings::MAX_RECORDING_RETENTION_DAYS) * 86_400
            )
        );
    }

    #[test]
    fn fixed_periods_resolve_to_their_documented_windows() {
        let now = at("2026-09-12T00:00:00Z");
        assert_eq!(
            resolve_retention_plan(RecordingRetentionPeriod::Days3, 20, 30, now),
            RetentionPlan::OlderThan(at("2026-09-09T00:00:00Z").timestamp())
        );
        assert_eq!(
            resolve_retention_plan(RecordingRetentionPeriod::Weeks2, 20, 30, now),
            RetentionPlan::OlderThan(at("2026-08-29T00:00:00Z").timestamp())
        );
    }

    /// Three months is three calendar months. The old code used 3 * 30 days,
    /// which deleted up to two days' worth of recordings early.
    #[test]
    fn three_months_is_calendar_months_not_ninety_days() {
        let now = at("2026-05-31T12:00:00Z");
        let plan = resolve_retention_plan(RecordingRetentionPeriod::Months3, 20, 30, now);
        assert_eq!(
            plan,
            RetentionPlan::OlderThan(at("2026-02-28T12:00:00Z").timestamp()),
            "must clamp onto a real date, not drift by 90 fixed days"
        );
        assert_ne!(
            plan,
            RetentionPlan::OlderThan(now.timestamp() - 90 * 86_400),
            "the 90-day approximation is the bug being fixed"
        );
    }

    #[test]
    fn zero_limit_cannot_wipe_history() {
        let plan = resolve_retention_plan(
            RecordingRetentionPeriod::PreserveLimit,
            0,
            30,
            at("2026-09-12T00:00:00Z"),
        );
        assert_eq!(
            plan,
            RetentionPlan::ByCount(crate::settings::MIN_HISTORY_LIMIT)
        );
    }

    #[test]
    fn preserve_limit_passes_the_configured_count_through() {
        assert_eq!(
            resolve_retention_plan(
                RecordingRetentionPeriod::PreserveLimit,
                20,
                30,
                at("2026-09-12T00:00:00Z")
            ),
            RetentionPlan::ByCount(20)
        );
    }

    #[test]
    fn keep_all_selects_no_rows_even_with_old_entries() {
        let conn = setup_conn();
        insert_entry(&conn, 1, "ancient", None);
        insert_entry(&conn, 2, "also ancient", None);

        let selected = HistoryManager::pending_deletions(&conn, RetentionPlan::KeepAll)
            .expect("resolve deletions");
        assert!(selected.is_empty());
    }

    /// The delete re-checks `saved = 0`, so a recording starred between the
    /// selection and the delete survives. Without the guard the row was already
    /// on the doomed list and would have been destroyed along with its audio.
    #[test]
    fn starring_a_row_after_selection_survives_the_delete() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "about to be starred", None);
        insert_entry(&conn, 300, "newest", None);

        let doomed = HistoryManager::pending_deletions(&conn, RetentionPlan::ByCount(1))
            .expect("select doomed rows");
        assert_eq!(doomed.len(), 1);

        // The user stars it in the window between selection and deletion.
        conn.execute(
            "UPDATE transcription_history SET saved = 1 WHERE timestamp = 100",
            [],
        )
        .expect("star recording");

        let removed = conn
            .execute(
                "DELETE FROM transcription_history WHERE id = ?1 AND saved = 0",
                params![doomed[0].0],
            )
            .expect("run guarded delete");

        assert_eq!(removed, 0, "a starred recording must not be deleted");
        let survivors: i64 = conn
            .query_row("SELECT COUNT(*) FROM transcription_history", [], |row| {
                row.get(0)
            })
            .expect("count survivors");
        assert_eq!(survivors, 2);
    }

    #[test]
    fn pending_deletions_dispatches_on_the_plan() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "old", None);
        insert_entry(&conn, 300, "new", None);

        let by_count = HistoryManager::pending_deletions(&conn, RetentionPlan::ByCount(1))
            .expect("count plan");
        assert_eq!(
            by_count.iter().map(|(_, f)| f.as_str()).collect::<Vec<_>>(),
            vec!["speakoflow-100.wav"]
        );

        let by_time = HistoryManager::pending_deletions(&conn, RetentionPlan::OlderThan(200))
            .expect("time plan");
        assert_eq!(
            by_time.iter().map(|(_, f)| f.as_str()).collect::<Vec<_>>(),
            vec!["speakoflow-100.wav"]
        );
    }

    // ---- lifetime usage stats ----

    fn migrated_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        Migrations::new(MIGRATIONS.to_vec())
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    /// Epoch seconds for local noon on a date, so local-day bucketing in the
    /// tests does not depend on the machine's time zone.
    fn local_noon(y: i32, m: u32, d: u32) -> i64 {
        Local
            .from_local_datetime(&date(y, m, d).and_hms_opt(12, 0, 0).expect("valid time"))
            .earliest()
            .expect("local noon exists")
            .timestamp()
    }

    fn usage_row(conn: &Connection, day: &str) -> Option<(i64, i64, i64, f64)> {
        conn.query_row(
            "SELECT dictations, words, timed_words, audio_seconds FROM usage_daily WHERE day = ?1",
            params![day],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .expect("query usage row")
    }

    #[test]
    fn count_words_ignores_punctuation_only_tokens() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   \n\t "), 0);
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words("  hello,   world!  "), 2);
        assert_eq!(count_words("wait — what ..."), 2);
        assert_eq!(count_words("— ... !!"), 0);
        assert_eq!(count_words("it's 3 o'clock"), 3);
        assert_eq!(count_words("naïve café"), 2);
    }

    #[test]
    fn local_day_formats_local_calendar_date() {
        assert_eq!(local_day(local_noon(2026, 3, 15)), "2026-03-15");
    }

    #[test]
    fn wav_duration_reads_header_and_rejects_missing_files() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("speakoflow-usage-test-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(&path, spec).expect("create wav");
            for _ in 0..24000 {
                writer.write_sample(0i16).expect("write sample");
            }
            writer.finalize().expect("finalize wav");
        }
        assert_eq!(wav_duration_seconds(&path), Some(1.5));

        let empty = dir.join(format!("speakoflow-usage-empty-{}.wav", std::process::id()));
        hound::WavWriter::create(&empty, spec)
            .expect("create empty wav")
            .finalize()
            .expect("finalize empty wav");
        assert_eq!(wav_duration_seconds(&empty), None);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&empty);
        assert_eq!(wav_duration_seconds(&dir.join("does-not-exist.wav")), None);
    }

    #[test]
    fn streaks_count_today_or_yesterday_and_track_longest() {
        let today = date(2026, 3, 15);

        // Empty.
        assert_eq!(compute_streaks(&[], today), (0, 0));

        // Active today.
        let days = [date(2026, 3, 13), date(2026, 3, 14), today];
        assert_eq!(compute_streaks(&days, today), (3, 3));

        // Only up to yesterday: the streak is still alive this morning.
        let days = [date(2026, 3, 13), date(2026, 3, 14)];
        assert_eq!(compute_streaks(&days, today), (2, 2));

        // A gap breaks it.
        let days = [date(2026, 3, 12), date(2026, 3, 13)];
        assert_eq!(compute_streaks(&days, today), (0, 2));
        let days = [date(2026, 3, 11), date(2026, 3, 12), today];
        assert_eq!(compute_streaks(&days, today), (1, 2));

        // Longest across history, independent of the current run.
        let days = [
            date(2026, 1, 1),
            date(2026, 1, 2),
            date(2026, 1, 3),
            date(2026, 1, 4),
            date(2026, 3, 14),
            today,
        ];
        assert_eq!(compute_streaks(&days, today), (2, 4));

        // Unsorted with duplicates.
        let days = [
            today,
            date(2026, 3, 13),
            today,
            date(2026, 3, 14),
            date(2026, 3, 13),
        ];
        assert_eq!(compute_streaks(&days, today), (3, 3));

        // Month boundary.
        let days = [date(2026, 2, 28), date(2026, 3, 1)];
        assert_eq!(compute_streaks(&days, date(2026, 3, 1)), (2, 2));
    }

    #[test]
    fn usage_migration_creates_both_tables() {
        let conn = migrated_conn();
        for table in ["usage_daily", "usage_meta"] {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .expect("query sqlite_master");
            assert!(exists, "{} should exist", table);
        }
    }

    #[test]
    fn apply_usage_delta_upserts_and_clamps_at_zero() {
        let conn = migrated_conn();
        apply_usage_delta(&conn, "2026-03-15", 1, 10, 10, 4.0).expect("insert");
        apply_usage_delta(&conn, "2026-03-15", 1, 5, 0, 0.0).expect("add");
        assert_eq!(usage_row(&conn, "2026-03-15"), Some((2, 15, 10, 4.0)));

        apply_usage_delta(&conn, "2026-03-15", -5, -100, -100, -100.0).expect("clamp");
        assert_eq!(usage_row(&conn, "2026-03-15"), Some((0, 0, 0, 0.0)));

        // A negative delta on a new day also starts at zero.
        apply_usage_delta(&conn, "2026-03-16", -1, -3, -3, -1.0).expect("insert negative");
        assert_eq!(usage_row(&conn, "2026-03-16"), Some((0, 0, 0, 0.0)));
    }

    #[test]
    fn retry_delta_covers_every_transition() {
        let known = || Some(3.0);
        let unknown = || None;
        assert_eq!(retry_usage_delta(0, 0, known), None);
        assert_eq!(retry_usage_delta(4, 4, known), None);
        assert_eq!(retry_usage_delta(0, 5, known), Some((1, 5, 5, 3.0)));
        assert_eq!(retry_usage_delta(0, 5, unknown), Some((1, 5, 0, 0.0)));
        assert_eq!(retry_usage_delta(5, 0, known), Some((-1, -5, -5, -3.0)));
        assert_eq!(retry_usage_delta(5, 0, unknown), Some((-1, -5, 0, 0.0)));
        assert_eq!(retry_usage_delta(5, 8, known), Some((0, 3, 3, 0.0)));
        assert_eq!(retry_usage_delta(8, 5, unknown), Some((0, -3, 0, 0.0)));
    }

    #[test]
    fn backfill_aggregates_by_day_skips_empty_and_is_idempotent() {
        let mut conn = migrated_conn();
        let d1 = local_noon(2026, 3, 10);
        let d2 = local_noon(2026, 3, 11);
        let d3 = local_noon(2026, 3, 12);
        insert_entry(&conn, d1, "hello world", None);
        insert_entry(&conn, d1 + 3600, "one two three", Some("One, two, three."));
        insert_entry(&conn, d2, "...", None);
        insert_entry(&conn, d2 + 60, "— !!", None);
        insert_entry(&conn, d3, "alpha", None);

        let counted = backfill_usage(&mut conn, |_| Some(2.0)).expect("backfill");
        assert_eq!(counted, 3);
        assert_eq!(usage_row(&conn, "2026-03-10"), Some((2, 5, 5, 4.0)));
        assert_eq!(usage_row(&conn, "2026-03-11"), None);
        assert_eq!(usage_row(&conn, "2026-03-12"), Some((1, 1, 1, 2.0)));

        // Second run does nothing.
        let again = backfill_usage(&mut conn, |_| Some(2.0)).expect("second backfill");
        assert_eq!(again, 0);
        assert_eq!(usage_row(&conn, "2026-03-10"), Some((2, 5, 5, 4.0)));
        assert_eq!(usage_row(&conn, "2026-03-12"), Some((1, 1, 1, 2.0)));
    }

    #[test]
    fn backfill_counts_unknown_durations_as_untimed() {
        let mut conn = migrated_conn();
        insert_entry(
            &conn,
            local_noon(2026, 3, 10),
            "four words right here",
            None,
        );
        backfill_usage(&mut conn, |_| None).expect("backfill");
        assert_eq!(usage_row(&conn, "2026-03-10"), Some((1, 4, 0, 0.0)));
    }

    #[test]
    fn backfill_on_fresh_install_only_sets_the_flag() {
        let mut conn = migrated_conn();
        assert_eq!(
            backfill_usage(&mut conn, |_| Some(1.0)).expect("backfill"),
            0
        );
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_daily", [], |row| row.get(0))
            .expect("count usage rows");
        assert_eq!(rows, 0);
        let flag: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM usage_meta WHERE key = ?1",
                params![USAGE_BACKFILL_KEY],
                |row| row.get(0),
            )
            .expect("count meta rows");
        assert_eq!(flag, 1);

        // Entries added after the flag is set are not double-counted by a rerun.
        insert_entry(&conn, local_noon(2026, 3, 10), "late entry", None);
        assert_eq!(backfill_usage(&mut conn, |_| Some(1.0)).expect("rerun"), 0);
        assert_eq!(usage_row(&conn, "2026-03-10"), None);
    }

    #[test]
    fn usage_stats_totals_today_and_recent_window() {
        let conn = migrated_conn();
        let today = date(2026, 3, 15);
        let day = |d: NaiveDate| d.format("%Y-%m-%d").to_string();

        // Current streak of three ending today.
        apply_usage_delta(&conn, &day(today), 2, 30, 30, 12.0).unwrap();
        apply_usage_delta(&conn, &day(date(2026, 3, 14)), 1, 10, 10, 5.0).unwrap();
        apply_usage_delta(&conn, &day(date(2026, 3, 13)), 1, 20, 0, 0.0).unwrap();
        // First day of the 371-day window, and the day just outside it.
        apply_usage_delta(&conn, &day(date(2025, 3, 10)), 1, 7, 7, 3.0).unwrap();
        apply_usage_delta(&conn, &day(date(2025, 3, 9)), 1, 8, 8, 4.0).unwrap();
        // An older five-day run: the longest streak.
        for d in 1..=5 {
            apply_usage_delta(&conn, &day(date(2026, 1, d)), 1, 1, 1, 1.0).unwrap();
        }
        // A day corrected down to zero is not active.
        apply_usage_delta(&conn, &day(date(2026, 3, 1)), 1, 3, 3, 1.0).unwrap();
        apply_usage_delta(&conn, &day(date(2026, 3, 1)), -1, -3, -3, -1.0).unwrap();

        let stats = compute_usage_stats(&conn, today).expect("stats");
        assert_eq!(stats.total_dictations, 2 + 1 + 1 + 1 + 1 + 5);
        assert_eq!(stats.total_words, 30 + 10 + 20 + 7 + 8 + 5);
        assert_eq!(stats.timed_words, 30 + 10 + 7 + 8 + 5);
        assert!((stats.total_audio_seconds - (12.0 + 5.0 + 3.0 + 4.0 + 5.0)).abs() < 1e-9);
        assert_eq!(stats.today_words, 30);
        assert_eq!(stats.today_dictations, 2);
        assert_eq!(stats.current_streak_days, 3);
        assert_eq!(stats.longest_streak_days, 5);
        assert_eq!(stats.active_days, 10);
        assert_eq!(
            stats
                .recent_days
                .iter()
                .map(|d| d.day.as_str())
                .collect::<Vec<_>>(),
            vec![
                "2025-03-10",
                "2026-01-01",
                "2026-01-02",
                "2026-01-03",
                "2026-01-04",
                "2026-01-05",
                "2026-03-13",
                "2026-03-14",
                "2026-03-15",
            ]
        );
        let today_entry = stats.recent_days.last().expect("today in window");
        assert_eq!(today_entry.dictations, 2);
        assert_eq!(today_entry.words, 30);

        // Nothing yet today: today's values are zero, the streak survives.
        let tomorrow = date(2026, 3, 16);
        let stats = compute_usage_stats(&conn, tomorrow).expect("stats tomorrow");
        assert_eq!(stats.today_words, 0);
        assert_eq!(stats.today_dictations, 0);
        assert_eq!(stats.current_streak_days, 3);
    }

    #[test]
    fn usage_stats_on_empty_database() {
        let conn = migrated_conn();
        let stats = compute_usage_stats(&conn, date(2026, 3, 15)).expect("stats");
        assert_eq!(stats.total_dictations, 0);
        assert_eq!(stats.total_words, 0);
        assert_eq!(stats.current_streak_days, 0);
        assert_eq!(stats.longest_streak_days, 0);
        assert_eq!(stats.active_days, 0);
        assert!(stats.recent_days.is_empty());
    }
}
