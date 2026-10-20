//! Insights: what SpeakoFlow has counted about how it is used, read in one go
//! for the Insights page and its share card.
//!
//! Each feature keeps its own lifetime counters beside its own data, so none of
//! them shrink when history is pruned or a conversation or meeting is deleted:
//! dictation in `usage_daily` and the assistant in `assistant_usage_daily`
//! (both in `history.db`), meetings in `meeting_usage` (`meetings.db`). This
//! module only reads them together, which is also where the one figure that
//! spans features comes from: the streak, where a day counts when anything was
//! said to the app.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chrono::{Days, Local, NaiveDate};
use log::{error, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};

use crate::managers::history::{
    compute_streaks, local_day, AssistantUsage, HistoryManager, UsageStats,
    USAGE_RECENT_WINDOW_DAYS,
};
use crate::meetings::store::{MeetingStore, MeetingUsageRow};

/// A call longer than this was a window left open, not a conversation; it is
/// counted at this length.
const MAX_CALL: Duration = Duration::from_secs(6 * 60 * 60);

/// One local day of meetings, for the Insights activity grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct MeetingUsageDay {
    /// Local calendar date the meetings started on, `%Y-%m-%d`.
    pub day: String,
    pub meetings: i64,
    pub seconds: i64,
    pub words: i64,
}

/// Lifetime meetings: how many, how long, and how much was said in them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct MeetingUsage {
    pub meetings: i64,
    pub seconds: i64,
    /// Words in the transcripts, everyone's, not only the user's.
    pub words: i64,
    pub longest_seconds: i64,
    /// Every day a meeting started on, ascending.
    pub active_days: Vec<String>,
    /// Days within the recent window, ascending. Days without a meeting are
    /// omitted.
    pub recent_days: Vec<MeetingUsageDay>,
}

/// Everything the Insights page shows.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Insights {
    pub dictation: UsageStats,
    pub assistant: AssistantUsage,
    pub meetings: MeetingUsage,
    /// Consecutive days with any use, ending today or yesterday (see
    /// [`compute_streaks`]).
    pub current_streak_days: u32,
    pub longest_streak_days: u32,
    /// Days with any use.
    pub active_days: u32,
    /// The first day anything was counted, `%Y-%m-%d`.
    pub first_day: Option<String>,
}

/// Meetings by the local day they started on, as of `today`.
pub fn summarize_meetings(rows: &[MeetingUsageRow], today: NaiveDate) -> MeetingUsage {
    let window_start = today
        .checked_sub_days(Days::new(USAGE_RECENT_WINDOW_DAYS - 1))
        .unwrap_or(today);
    let mut usage = MeetingUsage::default();
    for row in rows {
        let seconds = row.seconds.max(0);
        let words = row.words.max(0);
        usage.meetings += 1;
        usage.seconds += seconds;
        usage.words += words;
        usage.longest_seconds = usage.longest_seconds.max(seconds);

        let day = local_day(row.started_at);
        if usage.active_days.last() != Some(&day) {
            usage.active_days.push(day.clone());
        }
        let Ok(date) = NaiveDate::parse_from_str(&day, "%Y-%m-%d") else {
            continue;
        };
        if date < window_start || date > today {
            continue;
        }
        match usage.recent_days.last_mut() {
            Some(last) if last.day == day => {
                last.meetings += 1;
                last.seconds += seconds;
                last.words += words;
            }
            _ => usage.recent_days.push(MeetingUsageDay {
                day,
                meetings: 1,
                seconds,
                words,
            }),
        }
    }
    // Rows arrive by start time, but a clock change can put two days out of
    // order; the lists promise ascending days.
    usage.active_days.sort_unstable();
    usage.active_days.dedup();
    usage.recent_days.sort_by(|a, b| a.day.cmp(&b.day));
    usage
}

/// Put the three features' usage together, with the streak over all of them.
pub fn combine(
    dictation: UsageStats,
    assistant: AssistantUsage,
    meetings: MeetingUsage,
    dictation_days: &[String],
    today: NaiveDate,
) -> Insights {
    let days: BTreeSet<NaiveDate> = dictation_days
        .iter()
        .chain(&assistant.active_days)
        .chain(&meetings.active_days)
        .filter_map(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
        .collect();
    let active: Vec<NaiveDate> = days.iter().copied().collect();
    let (current, longest) = compute_streaks(&active, today);
    Insights {
        dictation,
        assistant,
        meetings,
        current_streak_days: current,
        longest_streak_days: longest,
        active_days: active.len() as u32,
        first_day: active.first().map(|day| day.format("%Y-%m-%d").to_string()),
    }
}

/// Count a question put to the assistant: a quick ask, or something said on a
/// call. Best effort, like every usage count: a counter must never cost the
/// user their answer.
pub fn record_question(app: &AppHandle) {
    let on_call = crate::voice_conversation::is_active(app);
    if on_call {
        crate::voice_conversation::note_turn(app);
    }
    if let Some(history) = app.try_state::<Arc<HistoryManager>>() {
        if let Err(e) = history.record_assistant_question(on_call) {
            error!("Failed to count an assistant question: {}", e);
        }
    }
}

/// Count a call that has ended, on the day it started.
pub fn record_call(app: &AppHandle, started_at: i64, length: Duration) {
    if let Some(history) = app.try_state::<Arc<HistoryManager>>() {
        let seconds = length.min(MAX_CALL).as_secs_f64();
        if let Err(e) = history.record_assistant_call(started_at, seconds) {
            error!("Failed to count an assistant call: {}", e);
        }
    }
}

/// Dictation, the assistant and meetings, counted for the Insights page.
#[tauri::command]
#[specta::specta]
pub async fn get_insights(
    app: AppHandle,
    history: State<'_, Arc<HistoryManager>>,
) -> Result<Insights, String> {
    let today = Local::now().date_naive();
    let (dictation, assistant, dictation_days) =
        history.insights_usage(today).map_err(|e| e.to_string())?;
    // Meetings are optional (see `lib.rs`): a store that failed to open costs
    // their figures, not the page.
    let meetings = match app.try_state::<Arc<MeetingStore>>() {
        Some(store) => match store.usage() {
            Ok(rows) => summarize_meetings(&rows, today),
            Err(e) => {
                warn!("Could not read meeting usage: {}", e);
                MeetingUsage::default()
            }
        },
        None => MeetingUsage::default(),
    };
    Ok(combine(
        dictation,
        assistant,
        meetings,
        &dictation_days,
        today,
    ))
}

/// Largest share card accepted, decoded. A 1080 x 1920 PNG is around 2 MB.
const MAX_CARD_BYTES: usize = 16 * 1024 * 1024;

fn decode_card(png_base64: &str) -> Result<Vec<u8>, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .map_err(|e| format!("The image could not be read: {e}"))?;
    if bytes.len() > MAX_CARD_BYTES {
        return Err("The image is too large".into());
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The image is not a PNG".into());
    }
    Ok(bytes)
}

/// Save a share card, rendered by the page, to the path the user chose in the
/// save dialog.
#[tauri::command]
#[specta::specta]
pub fn save_insights_card(path: String, png_base64: String) -> Result<(), String> {
    let bytes = decode_card(&png_base64)?;
    let mut path = std::path::PathBuf::from(path);
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        path.set_extension("png");
    }
    std::fs::write(&path, bytes).map_err(|e| format!("Could not save the image: {e}"))
}

/// Put a share card on the clipboard as an image, for a webview whose own
/// clipboard refuses images (WebKit without `ClipboardItem`, or a write that
/// lost its user activation).
#[tauri::command]
#[specta::specta]
pub fn copy_insights_card(app: AppHandle, png_base64: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let bytes = decode_card(&png_base64)?;
    let image = tauri::image::Image::from_bytes(&bytes).map_err(|e| e.to_string())?;
    app.clipboard()
        .write_image(&image)
        .map_err(|e| format!("Could not copy the image: {e}"))
}

/// Hand a share card to the system's share sheet (Windows' Share panel, the
/// macOS Share menu beside `anchor`). The image is written to the app's cache
/// first, under a name the receiving app may show, because the sheet passes
/// files rather than pixels.
#[tauri::command]
#[specta::specta]
pub async fn share_insights_card(
    app: AppHandle,
    window: tauri::WebviewWindow,
    png_base64: String,
    title: String,
    anchor: crate::share_sheet::ShareAnchor,
) -> Result<(), String> {
    let bytes = decode_card(&png_base64)?;
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("share");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not prepare the image: {e}"))?;
    let mut file = dir.join("SpeakoFlow insights.png");
    // An app may still be reading the last one; a fresh name never collides.
    if std::fs::write(&file, &bytes).is_err() {
        file = dir.join(format!(
            "SpeakoFlow insights {}.png",
            chrono::Local::now().format("%Y-%m-%d %H%M%S")
        ));
        std::fs::write(&file, &bytes).map_err(|e| format!("Could not prepare the image: {e}"))?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::share_sheet::show(&window, &file, &title, anchor)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    fn local_noon(y: i32, m: u32, d: u32) -> i64 {
        Local
            .from_local_datetime(&date(y, m, d).and_hms_opt(12, 0, 0).expect("valid time"))
            .earliest()
            .expect("local noon exists")
            .timestamp()
    }

    fn row(started_at: i64, seconds: i64, words: i64) -> MeetingUsageRow {
        MeetingUsageRow {
            started_at,
            seconds,
            words,
        }
    }

    fn empty_dictation() -> UsageStats {
        UsageStats {
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
        }
    }

    #[test]
    fn meetings_are_summed_and_grouped_by_the_day_they_started() {
        let today = date(2026, 5, 10);
        let rows = [
            row(local_noon(2025, 1, 1), 900, 100),
            row(local_noon(2026, 5, 9), 1_800, 2_000),
            row(local_noon(2026, 5, 9) + 3_600, 600, 300),
            row(local_noon(2026, 5, 10), 3_000, 4_000),
        ];
        let usage = summarize_meetings(&rows, today);
        assert_eq!(usage.meetings, 4);
        assert_eq!(usage.seconds, 6_300);
        assert_eq!(usage.words, 6_400);
        assert_eq!(usage.longest_seconds, 3_000);
        assert_eq!(
            usage.active_days,
            vec!["2025-01-01", "2026-05-09", "2026-05-10"]
        );
        // The 2025 meeting is outside the recent window.
        assert_eq!(
            usage.recent_days,
            vec![
                MeetingUsageDay {
                    day: "2026-05-09".into(),
                    meetings: 2,
                    seconds: 2_400,
                    words: 2_300,
                },
                MeetingUsageDay {
                    day: "2026-05-10".into(),
                    meetings: 1,
                    seconds: 3_000,
                    words: 4_000,
                },
            ]
        );
    }

    #[test]
    fn the_streak_counts_a_day_when_any_feature_was_used() {
        let today = date(2026, 5, 10);
        let assistant = AssistantUsage {
            asks: 1,
            active_days: vec!["2026-05-09".into()],
            ..AssistantUsage::default()
        };
        let meetings = summarize_meetings(&[row(local_noon(2026, 5, 8), 60, 10)], today);
        let insights = combine(
            empty_dictation(),
            assistant,
            meetings,
            &["2026-05-10".into(), "2026-05-01".into()],
            today,
        );
        assert_eq!(insights.current_streak_days, 3);
        assert_eq!(insights.longest_streak_days, 3);
        assert_eq!(insights.active_days, 4);
        assert_eq!(insights.first_day.as_deref(), Some("2026-05-01"));
    }

    #[test]
    fn nothing_used_means_no_streak_and_no_first_day() {
        let insights = combine(
            empty_dictation(),
            AssistantUsage::default(),
            MeetingUsage::default(),
            &[],
            date(2026, 5, 10),
        );
        assert_eq!(insights.current_streak_days, 0);
        assert_eq!(insights.active_days, 0);
        assert_eq!(insights.first_day, None);
    }

    #[test]
    fn a_card_must_be_a_png() {
        let engine = base64::engine::general_purpose::STANDARD;
        assert!(decode_card(&engine.encode(b"\x89PNG\r\n\x1a\nrest")).is_ok());
        assert!(decode_card(&engine.encode(b"GIF89a")).is_err());
        assert!(decode_card("not base64!").is_err());
    }
}
