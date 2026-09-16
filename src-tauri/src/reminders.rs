//! Reminders — "remind me to send that invoice in twenty minutes".
//!
//! The assistant already knows what you are doing and can see your screen, which
//! makes it the natural place to park a small future obligation. So this is a
//! tool the model calls (`set_reminder`), not a form the user fills in: the whole
//! interaction is one spoken sentence, and the model turns "after a while" into a
//! concrete time using the clock it already has (`get_current_datetime`).
//!
//! Three properties are what make it trustworthy rather than a toy.
//!
//! **A reminder outlives the app.** It is written to `reminders.json` the moment
//! it is created, so quitting, crashing, or rebooting does not lose it. Anything
//! that came due while the app was closed fires on the next launch instead of
//! being silently dropped — an alarm that only works if you never close the
//! program is worse than no alarm, because you stop checking.
//!
//! **It arrives without stealing your work.** The popup is the recording
//! overlay's discipline applied to a notification: always on top, `focusable`
//! off on Windows (`WS_EX_NOACTIVATE`) so a click cannot pull the foreground out
//! of whatever you are typing into, and never focused on show. It takes the
//! pointer, because Done and Snooze are the reason it exists.
//!
//! **It does not go away on its own.** No auto-dismiss timer. A reminder that
//! disappears while you are looking somewhere else has failed at its one job, so
//! it waits, and several that come due together stack in one window rather than
//! opening one window each.
//!
//! The clock is a parameter in every decision function here ([`resolve_due_at`],
//! [`next_wake`], [`partition_due`]), so the scheduling logic is unit-testable
//! without sleeping.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Local, TimeZone, Utc};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};

/// The popup window's label. One window for every reminder: they stack inside it.
pub const POPUP_LABEL: &str = "reminder_popup";

/// Nothing shorter than this is a reminder; it is a beep. Guards against a model
/// that answers "in 0 minutes" and fires before it has finished speaking.
const MIN_DELAY_SECS: i64 = 5;
/// A year out. Past this the value is far more likely to be a parsing mistake
/// (a year confused, a unit confused) than something the user meant.
const MAX_DELAY_SECS: i64 = 366 * 24 * 60 * 60;

/// A cap on stored reminders, so a runaway loop of tool calls cannot grow the
/// file without bound. Oldest-due survive; the newest additions are refused with
/// a message the model can relay.
const MAX_REMINDERS: usize = 200;

/// How long the scheduler sleeps when nothing is scheduled. It is woken directly
/// by the channel on every change, so this is only a backstop against a missed
/// wake — and against the wall clock moving under us (a laptop resuming from
/// sleep, or the user correcting the system time), which no timer notices.
const IDLE_POLL: Duration = Duration::from_secs(60);

/// Popup geometry. The width is fixed — a notification is a column of text, and
/// letting it track the screen would make a reminder on a 4K display a banner.
/// 400 pt sits in the same range as the assistant panel's own size presets, which
/// is what the card inside this window is.
///
/// The height is only a first frame: the webview measures its content and calls
/// [`fit_popup`] immediately, so these want to be close to a one-line reminder
/// rather than generous — too tall and the card visibly shrinks on arrival.
const POPUP_WIDTH: f64 = 400.0;
const POPUP_FALLBACK_HEIGHT: f64 = 92.0;
const POPUP_MIN_HEIGHT: f64 = 72.0;
const POPUP_MARGIN: f64 = 18.0;

/// The height the webview last measured for its stack of cards, so the window
/// fits the content instead of the content scrolling inside a fixed box.
static POPUP_HEIGHT: AtomicU32 = AtomicU32::new(0);

/// One future obligation.
///
/// Timestamps are RFC 3339 UTC strings rather than `DateTime` values: they cross
/// into the webview and into `reminders.json`, and a string that is already the
/// serialized form cannot drift between the three representations.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct Reminder {
    pub id: String,
    /// What to do, phrased as the user would read it back ("Send Priya the
    /// invoice"). This is the whole content of the popup, so it carries the
    /// meaning on its own.
    pub text: String,
    /// Optional detail the assistant derived rather than was told — the URL it
    /// read off the screen, the name of the file that was open. Shown smaller,
    /// under the text.
    #[serde(default)]
    pub note: Option<String>,
    /// When it fires, RFC 3339 in UTC.
    pub due_at: String,
    /// When it was asked for, RFC 3339 in UTC.
    pub created_at: String,
    /// How many times it has been pushed back. Shown in the popup, because
    /// "you have snoozed this four times" is information.
    #[serde(default)]
    pub snoozes: u32,
    /// True once it has come due and is waiting for the user. Persisted, so a
    /// reminder that fired and was never acknowledged is still waiting after a
    /// restart rather than quietly resolved.
    #[serde(default)]
    pub fired: bool,
}

impl Reminder {
    fn due(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.due_at)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

/// Everything on disk. A struct rather than a bare `Vec` so the file can gain
/// fields later without a migration.
#[derive(Default, Serialize, Deserialize)]
struct StoredReminders {
    #[serde(default)]
    reminders: Vec<Reminder>,
}

/// Managed state: the list, plus the handle that wakes the scheduler.
pub struct ReminderStore {
    path: PathBuf,
    reminders: Mutex<Vec<Reminder>>,
    wake: Sender<()>,
}

impl ReminderStore {
    fn snapshot(&self) -> Vec<Reminder> {
        self.reminders
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default()
    }

    /// Persist under the lock the caller already holds, so the file can never
    /// disagree with memory.
    fn persist_locked(&self, list: &[Reminder]) {
        let stored = StoredReminders {
            reminders: list.to_vec(),
        };
        match serde_json::to_string_pretty(&stored) {
            Ok(json) => {
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&self.path, json) {
                    // Loud: an unwritten reminder is one the next launch will not
                    // have, and the user has already been told it is set.
                    error!("Failed to write {}: {}", self.path.display(), e);
                }
            }
            Err(e) => error!("Failed to serialize reminders: {}", e),
        }
    }

    fn nudge_scheduler(&self) {
        let _ = self.wake.send(());
    }
}

// === Pure scheduling decisions ===========================================

/// Turn what the model asked for into an absolute instant.
///
/// Accepts a relative delay in minutes (fractional, so "in thirty seconds" works
/// without a second parameter) or an absolute local time. Relative wins when both
/// arrive, because a model that supplies both is usually restating one thing and
/// the delay is the form it was actually given.
///
/// `at` is parsed as **local** time on purpose. The user says "at six", they mean
/// six where they are standing, and a naive string with no offset is exactly what
/// a model produces for that. An explicit offset is honoured when present.
pub fn resolve_due_at(
    now: DateTime<Utc>,
    in_minutes: Option<f64>,
    at: Option<&str>,
) -> Result<DateTime<Utc>, String> {
    if let Some(minutes) = in_minutes {
        if !minutes.is_finite() {
            return Err("The delay is not a number.".to_string());
        }
        let seconds = (minutes * 60.0).round() as i64;
        return clamp_delay(now, seconds);
    }

    let Some(raw) = at.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(
            "No time was given. Pass in_minutes for a delay, or at for a specific time."
                .to_string(),
        );
    };

    let parsed = parse_local_datetime(raw)
        .ok_or_else(|| format!("'{raw}' is not a time I can read. Use YYYY-MM-DD HH:MM."))?;
    clamp_delay(now, (parsed - now).num_seconds())
}

fn clamp_delay(now: DateTime<Utc>, seconds: i64) -> Result<DateTime<Utc>, String> {
    if seconds > MAX_DELAY_SECS {
        return Err("That is more than a year away.".to_string());
    }
    // A time already past is nearly always a model resolving "at 9" against
    // yesterday, or arithmetic that landed a minute behind. Firing immediately is
    // the wrong repair (the user gets an alarm mid-sentence), and so is silently
    // moving it to tomorrow (they would never know). Say so instead.
    if seconds < MIN_DELAY_SECS {
        return Err(format!(
            "That time is in the past or less than {MIN_DELAY_SECS} seconds away. \
             Check the current time and pass a future one."
        ));
    }
    Ok(now + chrono::Duration::seconds(seconds))
}

/// Parse the handful of shapes a model actually emits for a wall-clock time.
/// Anything with an explicit offset keeps it; anything naive is read as local.
fn parse_local_datetime(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    let normalized = raw.replace('T', " ");
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d %H"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&normalized, format) {
            // A local time can be ambiguous (the hour a DST change repeats) or
            // nonexistent (the hour it skips). Take the earliest valid reading
            // rather than refusing — being an hour early beats not firing.
            if let Some(local) = Local.from_local_datetime(&naive).earliest() {
                return Some(local.with_timezone(&Utc));
            }
        }
    }
    None
}

/// How long the scheduler should sleep: until the earliest reminder that has not
/// fired, bounded by [`IDLE_POLL`]. `Duration::ZERO` means something is already
/// due, so the caller fires before sleeping again.
pub fn next_wake(now: DateTime<Utc>, reminders: &[Reminder]) -> Duration {
    let earliest = reminders
        .iter()
        .filter(|r| !r.fired)
        .filter_map(|r| r.due())
        .min();
    match earliest {
        None => IDLE_POLL,
        Some(due) if due <= now => Duration::ZERO,
        Some(due) => (due - now)
            .to_std()
            .unwrap_or(Duration::ZERO)
            .min(IDLE_POLL),
    }
}

/// Mark everything due as fired and report whether anything changed.
///
/// One pass over the list rather than "find, then mutate": a reminder that came
/// due while the app was closed is indistinguishable here from one that came due
/// a second ago, which is exactly the point — both fire.
pub fn partition_due(now: DateTime<Utc>, reminders: &mut [Reminder]) -> bool {
    let mut fired_any = false;
    for reminder in reminders.iter_mut() {
        if reminder.fired {
            continue;
        }
        match reminder.due() {
            Some(due) if due <= now => {
                reminder.fired = true;
                fired_any = true;
            }
            // An unparseable due time would otherwise sit in the list forever,
            // never firing and never expiring. Fire it: the text is still the
            // thing the user asked to be told.
            None => {
                warn!(
                    "Reminder {} has an unreadable due time ({}); firing it now",
                    reminder.id, reminder.due_at
                );
                reminder.fired = true;
                fired_any = true;
            }
            Some(_) => {}
        }
    }
    fired_any
}

/// "in 5 minutes", "at 6:30 PM", "tomorrow at 9:00 AM" — the phrase the model
/// gets back so it can confirm in its own words without re-deriving the time.
pub fn describe_due(now: DateTime<Utc>, due: DateTime<Utc>) -> String {
    let local = due.with_timezone(&Local);
    let same_day = now.with_timezone(&Local).date_naive() == local.date_naive();
    if same_day {
        local.format("today at %-I:%M %p").to_string()
    } else {
        local.format("%A %-d %B at %-I:%M %p").to_string()
    }
}

fn new_id() -> String {
    // Microsecond timestamp plus a counter: unique without pulling in a UUID
    // crate, and sortable, which makes a file dump readable.
    static SEQ: AtomicU32 = AtomicU32::new(0);
    format!(
        "rem-{:x}-{:x}",
        Utc::now().timestamp_micros(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

// === Store lifecycle =====================================================

/// Load `reminders.json` and start the scheduler. Call once at startup.
///
/// Overdue reminders are not fired here: the window cannot be built before the
/// app finishes setting up, and the scheduler's first tick handles them a moment
/// later anyway.
pub fn init(app: &AppHandle) {
    let path = match crate::portable::app_data_dir(app) {
        Ok(dir) => dir.join("reminders.json"),
        Err(e) => {
            error!("Reminders disabled: no app data directory ({})", e);
            return;
        }
    };

    let reminders = match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<StoredReminders>(&raw) {
            Ok(stored) => stored.reminders,
            Err(e) => {
                // Keep going with an empty list rather than failing startup, but
                // say so: the file is the user's data and they may want it back.
                warn!(
                    "Invalid {} ({}); starting with no reminders",
                    path.display(),
                    e
                );
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            warn!("Failed to read {}: {}", path.display(), e);
            Vec::new()
        }
    };

    let pending = reminders.iter().filter(|r| !r.fired).count();
    let waiting = reminders.len() - pending;
    if !reminders.is_empty() {
        info!("Loaded {pending} scheduled reminder(s), {waiting} still waiting to be acknowledged");
    }

    let (wake, rx) = mpsc::channel();
    app.manage(ReminderStore {
        path,
        reminders: Mutex::new(reminders),
        wake,
    });

    let app_for_thread = app.clone();
    std::thread::Builder::new()
        .name("reminder-scheduler".into())
        .spawn(move || scheduler(app_for_thread, rx))
        .map_err(|e| error!("Could not start the reminder scheduler: {e}"))
        .ok();
}

/// Sleep until the next reminder is due (or until something changes), then fire
/// whatever has come due. Its own thread rather than a tokio task so a long
/// sleep cannot occupy a runtime worker.
fn scheduler(app: AppHandle, rx: Receiver<()>) {
    debug!("Reminder scheduler started");

    // Anything that fired before the last quit and was never acknowledged is put
    // back on screen, once. Without this, `fired: true` surviving in the file
    // would mean the reminder is still owed and still listed in Settings, but
    // nothing ever says so again — which is the same as losing it, only harder to
    // notice. Deliberately only at startup: re-presenting on every tick would
    // fight the popup's own X button, which exists precisely so the user can put
    // a reminder aside without marking it done.
    if !waiting(&app).is_empty() {
        // A beat, so the window is built after the app has finished its own
        // setup rather than in the middle of it.
        std::thread::sleep(Duration::from_millis(1500));
        info!("Re-showing reminders that were never acknowledged before the last quit");
        present_popup(&app);
    }

    loop {
        let wait = match app.try_state::<ReminderStore>() {
            Some(store) => next_wake(Utc::now(), &store.snapshot()),
            // The store is gone: the app is shutting down.
            None => return,
        };

        if !wait.is_zero() {
            match rx.recv_timeout(wait) {
                // Something was added, cancelled or snoozed: recompute.
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    debug!("Reminder scheduler stopping");
                    return;
                }
            }
        }

        fire_due(&app);
    }
}

/// Move everything due into the "waiting" state and put the popup on screen.
fn fire_due(app: &AppHandle) {
    let Some(store) = app.try_state::<ReminderStore>() else {
        return;
    };
    let fired = {
        let Ok(mut list) = store.reminders.lock() else {
            return;
        };
        if !partition_due(Utc::now(), &mut list) {
            return;
        }
        store.persist_locked(&list);
        list.iter().filter(|r| r.fired).count()
    };

    info!("{fired} reminder(s) came due");
    crate::audio_feedback::play_reminder_sound(app);
    present_popup(app);
    emit_changed(app);
}

/// Everything still waiting for the user, oldest first.
fn waiting(app: &AppHandle) -> Vec<Reminder> {
    let Some(store) = app.try_state::<ReminderStore>() else {
        return Vec::new();
    };
    let mut due: Vec<Reminder> = store.snapshot().into_iter().filter(|r| r.fired).collect();
    due.sort_by(|a, b| a.due_at.cmp(&b.due_at));
    due
}

/// Tell every window the list changed, so Settings and the popup agree.
fn emit_changed(app: &AppHandle) {
    let Some(store) = app.try_state::<ReminderStore>() else {
        return;
    };
    let mut all = store.snapshot();
    all.sort_by(|a, b| a.due_at.cmp(&b.due_at));
    let _ = app.emit("reminders-changed", &all);
    let _ = app.emit("reminder-due", waiting(app));
}

// === Creating, snoozing, cancelling ======================================

/// Schedule a reminder. Returns the stored record so the caller can quote the
/// resolved time back to the user.
pub fn schedule(
    app: &AppHandle,
    text: &str,
    in_minutes: Option<f64>,
    at: Option<&str>,
    note: Option<String>,
) -> Result<Reminder, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A reminder needs to say what to do.".to_string());
    }
    let now = Utc::now();
    let due = resolve_due_at(now, in_minutes, at)?;

    let store = app
        .try_state::<ReminderStore>()
        .ok_or_else(|| "Reminders are not available.".to_string())?;

    let reminder = Reminder {
        id: new_id(),
        text: text.to_string(),
        note: note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        due_at: due.to_rfc3339(),
        created_at: now.to_rfc3339(),
        snoozes: 0,
        fired: false,
    };

    {
        let mut list = store
            .reminders
            .lock()
            .map_err(|_| "The reminder list is unavailable.".to_string())?;
        if list.len() >= MAX_REMINDERS {
            return Err(format!(
                "There are already {MAX_REMINDERS} reminders. Clear some before adding another."
            ));
        }
        list.push(reminder.clone());
        store.persist_locked(&list);
    }
    store.nudge_scheduler();
    emit_changed(app);
    info!(
        "Reminder scheduled for {} — {}",
        describe_due(now, due),
        reminder.text
    );
    Ok(reminder)
}

/// Acknowledge a reminder: it is done, and it goes away for good.
pub fn complete(app: &AppHandle, id: &str) -> Result<(), String> {
    remove(app, id)?;
    refresh_popup(app);
    Ok(())
}

/// Drop a reminder, fired or not. Used by Done, by Cancel, and by the model's
/// `cancel_reminder`.
pub fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let store = app
        .try_state::<ReminderStore>()
        .ok_or_else(|| "Reminders are not available.".to_string())?;
    {
        let mut list = store
            .reminders
            .lock()
            .map_err(|_| "The reminder list is unavailable.".to_string())?;
        let before = list.len();
        list.retain(|r| r.id != id);
        if list.len() == before {
            return Err(format!("No reminder with id '{id}'."));
        }
        store.persist_locked(&list);
    }
    store.nudge_scheduler();
    emit_changed(app);
    Ok(())
}

/// Push a waiting reminder back by `minutes`.
pub fn snooze(app: &AppHandle, id: &str, minutes: u32) -> Result<Reminder, String> {
    let store = app
        .try_state::<ReminderStore>()
        .ok_or_else(|| "Reminders are not available.".to_string())?;
    // Re-based on now, not on the original due time: a reminder snoozed for ten
    // minutes two hours after it fired should arrive in ten minutes, not
    // instantly.
    let due = Utc::now() + chrono::Duration::minutes(minutes.clamp(1, 60 * 24) as i64);
    let updated = {
        let mut list = store
            .reminders
            .lock()
            .map_err(|_| "The reminder list is unavailable.".to_string())?;
        let reminder = list
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("No reminder with id '{id}'."))?;
        reminder.due_at = due.to_rfc3339();
        reminder.fired = false;
        reminder.snoozes = reminder.snoozes.saturating_add(1);
        let updated = reminder.clone();
        store.persist_locked(&list);
        updated
    };
    store.nudge_scheduler();
    emit_changed(app);
    refresh_popup(app);
    Ok(updated)
}

/// Everything on the books, earliest first.
pub fn all(app: &AppHandle) -> Vec<Reminder> {
    let Some(store) = app.try_state::<ReminderStore>() else {
        return Vec::new();
    };
    let mut list = store.snapshot();
    list.sort_by(|a, b| a.due_at.cmp(&b.due_at));
    list
}

/// A plain-text listing for the model, so it can answer "what have I got set?"
/// without the ids leaking into speech unless it needs them for a cancel.
pub fn describe_all(app: &AppHandle) -> String {
    let now = Utc::now();
    let list = all(app);
    if list.is_empty() {
        return "No reminders are set.".to_string();
    }
    let mut out = String::new();
    for reminder in list {
        let when = reminder
            .due()
            .map(|due| describe_due(now, due))
            .unwrap_or_else(|| reminder.due_at.clone());
        let state = if reminder.fired { " (waiting)" } else { "" };
        out.push_str(&format!(
            "- [{}] {} — {}{}\n",
            reminder.id, reminder.text, when, state
        ));
    }
    out
}

// === The popup window ====================================================

/// Show the popup, building it on first use. Safe from any thread: the build and
/// the show are queued onto the main thread, because creating a WebView window
/// off it deadlocks the event loop (the same trap the assistant panel documents).
fn present_popup(app: &AppHandle) {
    let app_main = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if waiting(&app_main).is_empty() {
            return;
        }
        build_popup(&app_main);
        let Some(window) = app_main.get_webview_window(POPUP_LABEL) else {
            return;
        };
        place_popup(&app_main, &window);
        let _ = window.show();
        // Re-assert: another process can take `WS_EX_TOPMOST` at any time, and a
        // reminder underneath a full-screen window has not arrived.
        let _ = window.set_always_on_top(true);
        // The list is emitted after the window exists, and again from
        // `emit_changed`, so a webview that is still booting picks it up on mount
        // via `list_waiting_reminders`.
        let _ = app_main.emit("reminder-due", waiting(&app_main));
    }) {
        warn!("Could not queue the reminder popup onto the main thread: {e}");
    }
}

/// Re-fit or hide the popup after a Done/Snooze. Hides when nothing is left,
/// rather than leaving an empty card on screen.
fn refresh_popup(app: &AppHandle) {
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = app_main.get_webview_window(POPUP_LABEL) else {
            return;
        };
        if waiting(&app_main).is_empty() {
            POPUP_HEIGHT.store(0, Ordering::SeqCst);
            let _ = window.hide();
            return;
        }
        place_popup(&app_main, &window);
    });
}

fn build_popup(app: &AppHandle) {
    if app.get_webview_window(POPUP_LABEL).is_some() {
        return;
    }
    let builder = WebviewWindowBuilder::new(
        app,
        POPUP_LABEL,
        tauri::WebviewUrl::App("src/reminder/index.html".into()),
    )
    .additional_browser_args(crate::WEBVIEW2_BROWSER_ARGS)
    .title("Reminder")
    .inner_size(POPUP_WIDTH, POPUP_FALLBACK_HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    // A click lands on the button it was aimed at, instead of being spent
    // activating the window first.
    .accept_first_mouse(true)
    // Never arrives focused: whatever the user was typing into keeps the caret.
    .focused(false)
    .visible(false);

    // On Windows, go further and make it unfocusable outright
    // (`WS_EX_NOACTIVATE`), so even a click cannot pull the foreground away
    // mid-sentence — the same treatment the recording overlay gets, and for the
    // same reason. Not applied elsewhere: on macOS a non-activating window makes
    // WebView buttons unreliable, and Done and Snooze are the entire point.
    #[cfg(target_os = "windows")]
    let builder = builder.focusable(false);

    let mut builder = builder;
    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    match builder.build() {
        Ok(window) => {
            let app_handle = app.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // Closing the window is not "done" — the reminders behind it
                    // are still owed. Hide instead of destroying, so the next one
                    // does not pay for a rebuild.
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window(POPUP_LABEL) {
                        let _ = window.hide();
                    }
                }
            });
            debug!("Reminder popup window created (hidden)");
        }
        Err(e) => error!("Failed to create the reminder popup window: {}", e),
    }
}

/// Top-right of the display under the cursor — the corner every desktop reserves
/// for notifications, on the screen the user is actually looking at.
fn place_popup(app: &AppHandle, window: &tauri::WebviewWindow) {
    let height = popup_height();
    let _ = window.set_size(tauri::LogicalSize::new(POPUP_WIDTH, height));

    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|pos| app.monitor_from_point(pos.x, pos.y).ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let scale = monitor.scale_factor();
    let mx = monitor.position().x as f64 / scale;
    let my = monitor.position().y as f64 / scale;
    let mw = monitor.size().width as f64 / scale;
    let x = mx + mw - POPUP_WIDTH - POPUP_MARGIN;
    let y = my + POPUP_MARGIN;
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
}

fn popup_height() -> f64 {
    match POPUP_HEIGHT.load(Ordering::SeqCst) {
        0 => POPUP_FALLBACK_HEIGHT,
        measured => (measured as f64).max(POPUP_MIN_HEIGHT),
    }
}

/// The webview reports the height its stack of cards needs, and the window takes
/// it. Same approach as the ask card: the content is the only thing that knows
/// how tall a wrapped reminder is.
pub fn fit_popup(app: &AppHandle, requested: f64) {
    if requested <= 0.0 {
        return;
    }
    let clamped = requested.max(POPUP_MIN_HEIGHT);
    if (POPUP_HEIGHT.swap(clamped.round() as u32, Ordering::SeqCst) as f64 - clamped).abs() < 0.5 {
        return;
    }
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = app_main.get_webview_window(POPUP_LABEL) {
            place_popup(&app_main, &window);
        }
    });
}

// === Commands ============================================================

/// Every reminder, scheduled and waiting, earliest first.
#[tauri::command]
#[specta::specta]
pub fn list_reminders(app: AppHandle) -> Vec<Reminder> {
    all(&app)
}

/// Only the ones that have fired and are waiting to be acknowledged — what the
/// popup renders.
#[tauri::command]
#[specta::specta]
pub fn list_waiting_reminders(app: AppHandle) -> Vec<Reminder> {
    waiting(&app)
}

/// Create one by hand (Settings), rather than by asking the assistant.
#[tauri::command]
#[specta::specta]
pub fn create_reminder(
    app: AppHandle,
    text: String,
    in_minutes: Option<f64>,
    at: Option<String>,
) -> Result<Reminder, String> {
    schedule(&app, &text, in_minutes, at.as_deref(), None)
}

/// "Done" in the popup, and the delete button in Settings.
#[tauri::command]
#[specta::specta]
pub fn complete_reminder(app: AppHandle, id: String) -> Result<(), String> {
    complete(&app, &id)
}

#[tauri::command]
#[specta::specta]
pub fn snooze_reminder(app: AppHandle, id: String, minutes: u32) -> Result<Reminder, String> {
    snooze(&app, &id, minutes)
}

/// Hide the popup without resolving anything: the reminders stay owed and come
/// back on the next fire or restart.
#[tauri::command]
#[specta::specta]
pub fn dismiss_reminder_popup(app: AppHandle) {
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = app_main.get_webview_window(POPUP_LABEL) {
            let _ = window.hide();
        }
    });
}

/// The popup measured its content; take that height.
#[tauri::command]
#[specta::specta]
pub fn fit_reminder_popup(app: AppHandle, height: f64) {
    fit_popup(&app, height);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).expect("valid timestamp")
    }

    fn reminder(id: &str, due: DateTime<Utc>, fired: bool) -> Reminder {
        Reminder {
            id: id.to_string(),
            text: "do the thing".to_string(),
            note: None,
            due_at: due.to_rfc3339(),
            created_at: at(0).to_rfc3339(),
            snoozes: 0,
            fired,
        }
    }

    /// The ordinary case, and the one the whole feature is named for.
    #[test]
    fn a_relative_delay_lands_that_far_ahead() {
        let now = at(0);
        assert_eq!(resolve_due_at(now, Some(5.0), None).unwrap(), at(300));
        // Fractional minutes are how "in thirty seconds" is expressed without a
        // second parameter for the model to choose between.
        assert_eq!(resolve_due_at(now, Some(0.5), None).unwrap(), at(30));
    }

    /// A delay wins over an absolute time. A model that sends both is nearly
    /// always restating one thing, and the delay is the form it was given.
    #[test]
    fn a_delay_takes_priority_over_an_absolute_time() {
        let now = at(0);
        let due = resolve_due_at(now, Some(10.0), Some("2099-01-01 09:00")).unwrap();
        assert_eq!(due, at(600));
    }

    /// The failure that has to be reported rather than repaired: firing at once
    /// interrupts the sentence the user is still speaking, and quietly moving it
    /// to tomorrow hides a wrong answer behind a right-looking one.
    #[test]
    fn a_past_or_immediate_time_is_refused_with_a_reason() {
        let now = at(1000);
        for attempt in [
            resolve_due_at(now, Some(-5.0), None),
            resolve_due_at(now, Some(0.0), None),
            resolve_due_at(now, None, Some("1999-01-01 09:00")),
        ] {
            let err = attempt.expect_err("a non-future time cannot be scheduled");
            assert!(
                err.contains("past") || err.contains("future"),
                "the message must say what is wrong, got: {err}"
            );
        }
    }

    #[test]
    fn an_absurdly_distant_time_is_refused() {
        let err = resolve_due_at(at(0), Some(60.0 * 24.0 * 400.0), None)
            .expect_err("more than a year out is a unit mistake, not a plan");
        assert!(err.contains("year"), "got: {err}");
    }

    #[test]
    fn an_unreadable_time_names_the_format_it_wants() {
        let err = resolve_due_at(at(0), None, Some("next tuesday-ish")).expect_err("not a time");
        assert!(err.contains("YYYY-MM-DD"), "got: {err}");
        let err = resolve_due_at(at(0), None, None).expect_err("nothing was given");
        assert!(err.contains("in_minutes"), "got: {err}");
    }

    /// An explicit offset is honoured, so a model that helpfully sends full
    /// RFC 3339 is not reinterpreted as local time.
    #[test]
    fn an_explicit_offset_is_not_reinterpreted() {
        let parsed = parse_local_datetime("2033-03-01T12:00:00+05:45").expect("valid RFC 3339");
        assert_eq!(parsed.to_rfc3339(), "2033-03-01T06:15:00+00:00");
    }

    /// The scheduler must sleep exactly until the earliest unfired reminder, and
    /// report zero when one is already owed — otherwise a reminder waits for the
    /// idle poll instead of its own time.
    #[test]
    fn the_wait_is_bounded_by_the_earliest_unfired_reminder() {
        let now = at(0);
        assert_eq!(next_wake(now, &[]), IDLE_POLL);
        let list = vec![
            reminder("late", at(900), false),
            reminder("soon", at(30), false),
        ];
        assert_eq!(next_wake(now, &list), Duration::from_secs(30));
        // Already fired ones are waiting on the user, not on the clock.
        let list = vec![reminder("done-firing", at(-60), true)];
        assert_eq!(next_wake(now, &list), IDLE_POLL);
        assert_eq!(
            next_wake(now, &[reminder("overdue", at(-1), false)]),
            Duration::ZERO
        );
    }

    /// A wait longer than the idle poll is still capped, so the wall clock moving
    /// under a sleeping thread (a laptop resuming, a corrected system time) costs
    /// at most one poll interval instead of the whole delay.
    #[test]
    fn a_distant_reminder_still_wakes_for_the_idle_poll() {
        let far = vec![reminder("far", at(48 * 3600), false)];
        assert_eq!(next_wake(at(0), &far), IDLE_POLL);
    }

    /// Everything due fires, including something that came due while the app was
    /// closed — that is the case a naive "fire on the tick" scheduler drops.
    #[test]
    fn everything_already_due_fires_including_from_a_previous_run() {
        let mut list = vec![
            reminder("long-overdue", at(-86_400), false),
            reminder("just-due", at(0), false),
            reminder("later", at(60), false),
        ];
        assert!(partition_due(at(0), &mut list));
        assert!(
            list[0].fired,
            "a reminder from a previous run must not be lost"
        );
        assert!(list[1].fired, "due exactly now counts as due");
        assert!(!list[2].fired);
        // Idempotent: a second pass changes nothing, so a repeated tick cannot
        // re-announce the same reminder.
        assert!(!partition_due(at(0), &mut list));
    }

    /// A corrupt due time must not become a reminder that can never fire and can
    /// never be seen. The text is still what the user asked to be told.
    #[test]
    fn an_unreadable_due_time_fires_rather_than_disappearing() {
        let mut broken = reminder("broken", at(60), false);
        broken.due_at = "not a date".to_string();
        let mut list = vec![broken];
        assert!(partition_due(at(0), &mut list));
        assert!(list[0].fired);
    }

    #[test]
    fn the_spoken_description_says_today_only_when_it_is_today() {
        let now = Utc::now();
        let soon = now + chrono::Duration::minutes(30);
        assert!(
            describe_due(now, soon).contains("today") || describe_due(now, soon).contains("at"),
            "a same-day time reads as a clock time"
        );
        let next_week = now + chrono::Duration::days(7);
        assert!(!describe_due(now, next_week).contains("today"));
    }
}
