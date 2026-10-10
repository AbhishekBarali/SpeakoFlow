//! The settings file, `settings_store.json`, read and written by the app itself.
//!
//! # Why this is not `tauri-plugin-store`
//!
//! The file used to be a `tauri-plugin-store` store. That plugin saves by calling
//! `fs::write` on the real file, which truncates it and then writes it again, with
//! no temporary file, rename or fsync. It also saves at the worst possible time:
//! 100 ms after any change on a runtime thread, and again for every store on
//! `RunEvent::Exit`. During a logout or shutdown the main window's
//! `CloseRequested` / `Focused(false)` handlers write settings, and the process
//! can end while one of those saves sits between the truncate and the write. That
//! leaves an empty or partly written file.
//!
//! The plugin's load ignores a file it cannot parse (`let _ = store.load()`), so
//! the next launch began with an empty store. `get_settings` found no `settings`
//! key and wrote the defaults over the damaged file 100 ms later. No backup was
//! kept and nothing was logged. That is how users lost every setting at once
//! after a restart (custom words, replacements, shortcuts, model choice) while
//! history and models, which live in other files, survived.
//!
//! # What this does instead
//!
//! - **Every write is atomic.** The full file goes to a temporary file in the
//!   same directory and is fsynced, then renamed over the old one. A process that
//!   dies at any point leaves either the old file or the new one, never part of
//!   either. Creating the temporary file and the rename are retried briefly when
//!   another process holds the file (on Windows: Defender, the search indexer, a
//!   sync client), see [`retry_delay`].
//! - **Damaged content is never overwritten.** A file that reads but is empty,
//!   NUL-filled, not JSON or not an object is renamed to
//!   `settings_store.json.corrupt-<unix time>` (the newest
//!   [`KEEP_CORRUPT_COPIES`] are kept), and the settings come back from the last
//!   good backup when there is one. A file that parses but has lost its
//!   `settings` entry gets that entry back from the backup, keeping its other
//!   keys.
//! - **A read error is not damage.** A file that cannot be read even after a few
//!   retries (most often an antivirus scan at login) is left exactly where it
//!   is, and saving is turned off for the session so nothing replaces it. The
//!   session runs on the backup's settings, or the defaults.
//! - **A backup is kept.** `settings_store.json.bak` is refreshed (atomically)
//!   from every launch that read a good file containing settings, and only when
//!   it differs.
//! - **The user is told.** Each of those recoveries, and saves that keep
//!   failing, becomes a [`SettingsNotice`] the main window shows.
//! - **Writes stay off the caller's thread.** A change marks the file dirty and a
//!   writer thread saves it after [`SAVE_DEBOUNCE`], coalescing bursts the way
//!   the plugin's auto-save did. [`SettingsFile::flush`] saves synchronously and
//!   runs before the app exits.
//!
//! The format is unchanged, a pretty-printed JSON object of keys, so a downgrade
//! to an older build still reads it.
//!
//! Nothing may open this path through `tauri-plugin-store` again. The plugin
//! would keep its own copy and write it back non-atomically on exit, over the
//! top of this file.
//!
//! Lock order: `saving`, then `state` or `notices`, each held briefly and never
//! both of the last two at once. The notifier is called with no lock held.

use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long after a change the writer thread waits before saving, so a burst of
/// changes (a slider, a migration pass) becomes one write. Same as the plugin's
/// auto-save.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);

/// How long the writer thread waits before retrying a save that failed (disk
/// full, file locked by a scanner), so a persistent failure is not a busy loop.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(2);

/// Attempts made at one file operation that another process is blocking,
/// including the first. With [`retry_delay`] the pauses add up to 450 ms.
const ATTEMPTS: u32 = 10;

/// Quarantined copies of a damaged file kept beside it; older ones are deleted.
pub const KEEP_CORRUPT_COPIES: usize = 5;

/// Failed saves in a row after which the user is told.
const FAILURES_BEFORE_NOTICE: u32 = 3;

/// How long saves may keep failing before the user is told, however few
/// attempts that was.
const FAILING_FOR_BEFORE_NOTICE: Duration = Duration::from_secs(10);

/// The key the app's whole configuration lives under.
const SETTINGS_KEY: &str = "settings";

/// A UTF-8 byte-order mark, which some editors add when a user edits the file.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

struct State {
    entries: Map<String, Value>,
    /// Changed since the last successful save.
    dirty: bool,
}

/// What [`SettingsFile::open`] found on disk. Logged once, and returned so it
/// can be tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No file yet: a fresh install.
    Fresh,
    /// The file was read normally.
    Loaded,
    /// The file's content was damaged. It was kept at `kept_as`, and the
    /// entries came from the backup.
    RestoredFromBackup { kept_as: PathBuf },
    /// The file was readable but had no `settings` object. That entry came from
    /// the backup; the file's other keys were kept.
    SettingsRestoredFromBackup,
    /// The file's content was damaged and there was no usable backup. It was
    /// kept at `kept_as`, and the app starts from its defaults.
    Unrecoverable { kept_as: PathBuf },
    /// The file could not be read, even after retrying. It was left as it is,
    /// and saving is off for this run so it is not overwritten. The entries
    /// came from the backup when `from_backup`, otherwise the defaults apply.
    Unreadable { from_backup: bool },
    /// The file's content was damaged and it could not be moved aside. Saving
    /// is off for this run so it is not overwritten. The entries came from the
    /// backup when `from_backup`.
    Untouchable { from_backup: bool },
}

impl LoadOutcome {
    /// Whether this run may write the file.
    fn saves(&self) -> bool {
        !matches!(self, Self::Unreadable { .. } | Self::Untouchable { .. })
    }

    /// What the user is told about this outcome, if anything.
    pub fn notice(&self) -> Option<SettingsNotice> {
        let (kind, kept_as) = match self {
            Self::Fresh | Self::Loaded => return None,
            Self::RestoredFromBackup { .. } | Self::SettingsRestoredFromBackup => {
                (SettingsNoticeKind::Restored, None)
            }
            Self::Unrecoverable { kept_as } => (
                SettingsNoticeKind::Unrecoverable,
                kept_as
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
            ),
            Self::Unreadable { .. } | Self::Untouchable { .. } => {
                (SettingsNoticeKind::NotSaving, None)
            }
        };
        Some(SettingsNotice { kind, kept_as })
    }
}

/// Something about the settings file the user has to know, shown in the main
/// window until it is dismissed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SettingsNoticeKind {
    /// The settings were damaged and came back from the backup.
    Restored,
    /// The settings were damaged and no backup could replace them, so the
    /// defaults apply. The damaged file was kept.
    Unrecoverable,
    /// The settings file could not be read or moved aside, so nothing changed
    /// this session is saved.
    NotSaving,
    /// Saving has been failing. Cleared by the next save that works.
    SaveFailing,
}

/// A notice about the settings file, with the name of the kept damaged copy
/// for `unrecoverable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct SettingsNotice {
    /// What happened.
    pub kind: SettingsNoticeKind,
    /// The file name the damaged settings were kept under, for `unrecoverable`.
    pub kept_as: Option<String>,
}

#[derive(Default)]
struct Notices {
    /// What the load found, until the user dismisses it.
    load: Option<SettingsNotice>,
    /// Failed saves since the last one that worked.
    failures: u32,
    /// When the current run of failures began.
    failing_since: Option<Instant>,
    /// The user was told about the current run of failures (and may have
    /// dismissed it since), so it is not raised again until a save works.
    told: bool,
    /// The save-failing notice is up.
    save_failing: bool,
}

impl Notices {
    fn current(&self) -> Vec<SettingsNotice> {
        let mut notices: Vec<SettingsNotice> = self.load.iter().cloned().collect();
        if self.save_failing {
            notices.push(SettingsNotice {
                kind: SettingsNoticeKind::SaveFailing,
                kept_as: None,
            });
        }
        notices
    }
}

type Notifier = Box<dyn Fn(Vec<SettingsNotice>) + Send + Sync>;

/// The open settings file: its entries in memory, saved atomically in the
/// background.
pub struct SettingsFile {
    /// `None` when there is nowhere to save, or when saving was turned off to
    /// protect a file that could not be read or moved aside.
    path: Option<PathBuf>,
    state: Mutex<State>,
    changed: Condvar,
    /// Serialises saves, so snapshots reach the disk in the order they were
    /// taken and a later state is never replaced by an earlier one.
    saving: Mutex<()>,
    /// The writer thread is running. Without it every change saves at once.
    background: AtomicBool,
    notices: Mutex<Notices>,
    /// Called with the current notices whenever a save changes them.
    notifier: OnceLock<Notifier>,
    outcome: LoadOutcome,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere while holding the lock must not take settings down too.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether saves have been failing long enough, or often enough, to tell the
/// user rather than wait for the next retry.
fn save_failure_is_persistent(failures: u32, failing_for: Duration) -> bool {
    failures >= FAILURES_BEFORE_NOTICE || failing_for >= FAILING_FOR_BEFORE_NOTICE
}

impl SettingsFile {
    fn new(
        path: Option<PathBuf>,
        entries: Map<String, Value>,
        dirty: bool,
        outcome: LoadOutcome,
    ) -> Self {
        Self {
            path,
            state: Mutex::new(State { entries, dirty }),
            changed: Condvar::new(),
            saving: Mutex::new(()),
            background: AtomicBool::new(false),
            notices: Mutex::new(Notices {
                load: outcome.notice(),
                ..Notices::default()
            }),
            notifier: OnceLock::new(),
            outcome,
        }
    }

    /// Read the file at `path`, recovering from damage as described in the
    /// module docs, and start the writer thread.
    pub fn open(path: PathBuf) -> Arc<Self> {
        remove_stale_temp_files(&path);
        let (entries, dirty, outcome) = load(&path);
        let path = outcome.saves().then_some(path);
        let file = Arc::new(Self::new(path, entries, dirty, outcome));
        if file.path.is_some() {
            spawn_writer(&file);
            if dirty {
                // Restored from the backup: make it the real file again before
                // anything else runs, so a crash now cannot leave the launch
                // after this one with no file and only a backup.
                let _ = file.flush();
            }
        }
        file
    }

    /// A settings file that is never saved, for when the app data directory
    /// cannot be resolved at all. Settings still work for the session.
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self::new(None, Map::new(), false, LoadOutcome::Fresh))
    }

    /// What opening the file found.
    pub fn outcome(&self) -> &LoadOutcome {
        &self.outcome
    }

    /// Install the callback that hears about notices raised or cleared by a
    /// later save. Only the first call has an effect.
    pub fn set_notifier(&self, notifier: impl Fn(Vec<SettingsNotice>) + Send + Sync + 'static) {
        let _ = self.notifier.set(Box::new(notifier));
    }

    /// The notices the user has not dismissed yet.
    pub fn notices(&self) -> Vec<SettingsNotice> {
        lock(&self.notices).current()
    }

    /// Stop showing a notice. Returns whether one of that kind was up.
    pub fn dismiss_notice(&self, kind: SettingsNoticeKind) -> bool {
        let mut notices = lock(&self.notices);
        if kind == SettingsNoticeKind::SaveFailing {
            return std::mem::take(&mut notices.save_failing);
        }
        if notices.load.as_ref().map(|notice| notice.kind) == Some(kind) {
            notices.load = None;
            return true;
        }
        false
    }

    /// The value stored under `key`.
    pub fn get(&self, key: &str) -> Option<Value> {
        lock(&self.state).entries.get(key).cloned()
    }

    /// Store `value` under `key` and schedule a save if it changed anything.
    pub fn set(&self, key: &str, value: Value) {
        {
            let mut state = lock(&self.state);
            if state.entries.get(key) == Some(&value) {
                return;
            }
            state.entries.insert(key.to_string(), value);
            state.dirty = true;
        }
        self.schedule_save();
    }

    /// Remove `key` and schedule a save. Returns whether it was there.
    pub fn delete(&self, key: &str) -> bool {
        let removed = {
            let mut state = lock(&self.state);
            let removed = state.entries.remove(key).is_some();
            state.dirty |= removed;
            removed
        };
        if removed {
            self.schedule_save();
        }
        removed
    }

    fn schedule_save(&self) {
        if self.background.load(Ordering::Acquire) {
            self.changed.notify_one();
        } else {
            let _ = self.flush();
        }
    }

    /// Save now if anything changed. Logs and returns the error on failure; the
    /// changes stay pending so the next save retries them.
    pub fn flush(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let (result, notices_changed) = {
            let _saving = lock(&self.saving);
            let bytes = {
                let mut state = lock(&self.state);
                if !state.dirty {
                    return Ok(());
                }
                // Cleared only once the snapshot exists, so a change that
                // cannot be serialised stays pending rather than vanishing.
                let bytes = serde_json::to_vec_pretty(&state.entries).map_err(io::Error::other);
                if bytes.is_ok() {
                    state.dirty = false;
                }
                bytes
            };
            let result = bytes.and_then(|bytes| write_atomically(path, &bytes));
            if result.is_err() {
                lock(&self.state).dirty = true;
            }
            let notices_changed = self.record_save(path, &result, Instant::now());
            (result, notices_changed)
        };
        if notices_changed {
            self.notify();
        }
        result
    }

    /// Track a run of failed saves for the user and the log. Returns whether
    /// the notices changed.
    fn record_save(&self, path: &Path, result: &io::Result<()>, now: Instant) -> bool {
        let mut notices = lock(&self.notices);
        match result {
            Ok(()) => {
                if notices.failures > 0 {
                    info!(
                        "Settings saved again after {} failed attempts",
                        notices.failures
                    );
                }
                let was_up = notices.save_failing;
                notices.failures = 0;
                notices.failing_since = None;
                notices.told = false;
                notices.save_failing = false;
                was_up
            }
            Err(err) => {
                notices.failures += 1;
                if notices.failures == 1 {
                    error!("Failed to save settings to {}: {err}", path.display());
                }
                let since = *notices.failing_since.get_or_insert(now);
                if !notices.told
                    && save_failure_is_persistent(notices.failures, now.duration_since(since))
                {
                    warn!(
                        "Settings have not saved for {} attempts ({err}); telling the user",
                        notices.failures
                    );
                    notices.told = true;
                    notices.save_failing = true;
                    return true;
                }
                false
            }
        }
    }

    fn notify(&self) {
        if let Some(notifier) = self.notifier.get() {
            notifier(self.notices());
        }
    }
}

fn spawn_writer(file: &Arc<SettingsFile>) {
    let writer = Arc::clone(file);
    let spawned = std::thread::Builder::new()
        .name("settings-writer".to_string())
        .spawn(move || loop {
            {
                let mut state = lock(&writer.state);
                while !state.dirty {
                    state = writer
                        .changed
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
            std::thread::sleep(SAVE_DEBOUNCE);
            if writer.flush().is_err() {
                std::thread::sleep(RETRY_AFTER_FAILURE);
            }
        });
    match spawned {
        Ok(_) => file.background.store(true, Ordering::Release),
        Err(err) => warn!("No settings writer thread ({err}); saving on every change instead"),
    }
}

/// The backup kept next to the settings file.
pub fn backup_path(path: &Path) -> PathBuf {
    sibling(path, ".bak")
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn temp_prefix(path: &Path) -> String {
    format!(
        ".{}.tmp-",
        path.file_name().unwrap_or_default().to_string_lossy()
    )
}

fn corrupt_prefix(path: &Path) -> String {
    format!(
        "{}.corrupt-",
        path.file_name().unwrap_or_default().to_string_lossy()
    )
}

/// The pause before trying again after `failed` attempts, or `None` once
/// [`ATTEMPTS`] have been made. Grows by 10 ms a step: 10, 20, … 90 ms.
fn retry_delay(failed: u32) -> Option<Duration> {
    (1..ATTEMPTS)
        .contains(&failed)
        .then(|| Duration::from_millis(10 * u64::from(failed)))
}

/// Whether `err` is what another process's open handle on the file causes, so
/// the same operation may work a moment later. The raw codes are Windows'
/// `ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION`;
/// elsewhere those numbers mean unrelated things.
fn is_held_by_another_process(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    {
        matches!(err.raw_os_error(), Some(5 | 32 | 33))
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Run `op` until it works, fails in a way `should_retry` rejects, or
/// [`ATTEMPTS`] are used up, calling `sleep` with each [`retry_delay`].
fn retry_with<T>(
    mut op: impl FnMut() -> io::Result<T>,
    should_retry: impl Fn(&io::Error) -> bool,
    mut sleep: impl FnMut(Duration),
) -> io::Result<T> {
    let mut failed = 0;
    loop {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) => {
                failed += 1;
                match retry_delay(failed) {
                    Some(delay) if should_retry(&err) => sleep(delay),
                    _ => return Err(err),
                }
            }
        }
    }
}

fn retry_while_held<T>(op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    retry_with(op, is_held_by_another_process, std::thread::sleep)
}

/// Parse a store file: a JSON object of keys, as tauri-plugin-store wrote it.
fn parse_entries(bytes: &[u8]) -> Result<Map<String, Value>, String> {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(format!("the file is empty ({} bytes)", bytes.len()));
    }
    if bytes.contains(&0) {
        return Err(format!(
            "the file contains NUL bytes ({} bytes), as left by an interrupted write",
            bytes.len()
        ));
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(entries)) => Ok(entries),
        Ok(_) => Err("the file is valid JSON but not an object".to_string()),
        Err(err) => Err(format!("the file is not valid JSON ({err})")),
    }
}

fn has_settings(entries: &Map<String, Value>) -> bool {
    matches!(entries.get(SETTINGS_KEY), Some(Value::Object(_)))
}

/// The backup's entries, if it exists, parses and holds a `settings` object.
fn read_backup(backup: &Path) -> Option<Map<String, Value>> {
    fs::read(backup)
        .ok()
        .and_then(|bytes| parse_entries(&bytes).ok())
        .filter(has_settings)
}

/// Make the backup hold `bytes`, unless it already does.
fn refresh_backup(backup: &Path, bytes: &[u8]) {
    if fs::read(backup).is_ok_and(|current| current == bytes) {
        return;
    }
    if let Err(err) = write_atomically(backup, bytes) {
        warn!("Could not refresh the settings backup: {err}");
    }
}

/// Read the file, recovering from damage. Returns the entries, whether they
/// must be saved (they came from the backup), and what happened.
fn load(path: &Path) -> (Map<String, Value>, bool, LoadOutcome) {
    let backup = backup_path(path);
    let read = retry_with(
        || fs::read(path),
        |err| err.kind() != io::ErrorKind::NotFound,
        std::thread::sleep,
    );
    let problem = match read {
        Ok(bytes) => match parse_entries(&bytes) {
            Ok(mut entries) => {
                if has_settings(&entries) {
                    refresh_backup(&backup, &bytes);
                    return (entries, false, LoadOutcome::Loaded);
                }
                // Readable, but `settings` is missing or not an object. Left
                // alone, `get_settings` would write the defaults into it and
                // the next launch would copy those over the good backup.
                if let Some(settings) =
                    read_backup(&backup).and_then(|mut entries| entries.remove(SETTINGS_KEY))
                {
                    warn!(
                        "Settings file {} has no settings object. Restored the settings from \
the backup {} and kept the file's other keys.",
                        path.display(),
                        backup.display()
                    );
                    entries.insert(SETTINGS_KEY.to_string(), settings);
                    return (entries, true, LoadOutcome::SettingsRestoredFromBackup);
                }
                return (entries, false, LoadOutcome::Loaded);
            }
            Err(problem) => problem,
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return (Map::new(), false, LoadOutcome::Fresh);
        }
        Err(err) => {
            // Not damage: most often a scanner holding the file at login. The
            // file is probably fine, so it is neither moved nor written.
            let restored = read_backup(&backup);
            let from_backup = restored.is_some();
            error!(
                "Settings file {} could not be read ({err}), even after retrying. It was left \
as it is and settings will not be saved this session; using {} for now.",
                path.display(),
                if from_backup {
                    "the backup"
                } else {
                    "the defaults"
                }
            );
            return (
                restored.unwrap_or_default(),
                false,
                LoadOutcome::Unreadable { from_backup },
            );
        }
    };

    let restored = read_backup(&backup);
    let Some(kept_as) = quarantine(path) else {
        let from_backup = restored.is_some();
        error!(
            "Settings file {} is unusable: {problem}. It could not be moved aside either, \
so settings will not be saved this session to avoid overwriting it; using {} for now.",
            path.display(),
            if from_backup {
                "the backup"
            } else {
                "the defaults"
            }
        );
        return (
            restored.unwrap_or_default(),
            false,
            LoadOutcome::Untouchable { from_backup },
        );
    };
    prune_corrupt_copies(path, &kept_as);

    match restored {
        Some(entries) => {
            error!(
                "Settings file {} was unusable: {problem}. Restored settings from the backup \
{}; the damaged file was kept as {}.",
                path.display(),
                backup.display(),
                kept_as.display()
            );
            (entries, true, LoadOutcome::RestoredFromBackup { kept_as })
        }
        None => {
            error!(
                "Settings file {} was unusable: {problem}, and there is no usable backup. \
Starting from defaults; the damaged file was kept as {}.",
                path.display(),
                kept_as.display()
            );
            (Map::new(), false, LoadOutcome::Unrecoverable { kept_as })
        }
    }
}

/// Move an unusable file aside so nothing overwrites it. `None` if that failed.
fn quarantine(path: &Path) -> Option<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let mut kept_as = sibling(path, &format!(".corrupt-{stamp}"));
    let mut n = 1;
    while kept_as.exists() {
        kept_as = sibling(path, &format!(".corrupt-{stamp}-{n}"));
        n += 1;
    }
    match retry_while_held(|| fs::rename(path, &kept_as)) {
        Ok(()) => Some(kept_as),
        Err(err) => {
            warn!("Could not move {} aside: {err}", path.display());
            None
        }
    }
}

/// When a quarantined copy was made, from its name: `<unix time>` or
/// `<unix time>-<n>`. `None` for anything else, which is never deleted.
fn corrupt_copy_order(suffix: &str) -> Option<(u64, u64)> {
    match suffix.split_once('-') {
        Some((stamp, n)) => Some((stamp.parse().ok()?, n.parse().ok()?)),
        None => Some((suffix.parse().ok()?, 0)),
    }
}

/// Delete all but the newest [`KEEP_CORRUPT_COPIES`] quarantined copies of
/// `path`. `keep` (the copy just made) is never deleted.
fn prune_corrupt_copies(path: &Path, keep: &Path) {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = corrupt_prefix(path);
    let mut copies: Vec<((u64, u64), PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let order = corrupt_copy_order(name.to_str()?.strip_prefix(&prefix)?)?;
            Some((order, entry.path()))
        })
        .collect();
    copies.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
    for (_, old) in copies.into_iter().skip(KEEP_CORRUPT_COPIES) {
        if old.file_name() != keep.file_name() {
            let _ = fs::remove_file(old);
        }
    }
}

/// Write `bytes` to `path` so that `path` always holds either its old contents
/// or all of the new ones.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    // Per process, so two instances never write the same temporary file.
    let temp = dir.join(format!("{}{}", temp_prefix(path), std::process::id()));
    let result = (|| {
        let mut file = retry_while_held(|| File::create(&temp))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        retry_while_held(|| fs::rename(&temp, path))?;
        // Make the rename itself durable. Not possible on Windows, where
        // directories cannot be opened as files, and best effort elsewhere.
        #[cfg(unix)]
        if let Ok(dir) = File::open(dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Remove temporary files left by a process that died mid-save, for the file
/// and its backup. They are never read; this only stops them piling up.
fn remove_stale_temp_files(path: &Path) {
    remove_stale_temp_files_for(path);
    remove_stale_temp_files_for(&backup_path(path));
}

fn remove_stale_temp_files_for(path: &Path) {
    let (Some(dir), Some(_)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = temp_prefix(path);
    let own = format!("{prefix}{}", std::process::id());
    let Ok(entries) = fs::read_dir(if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    }) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name != own {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    fn store_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("settings_store.json")
    }

    fn user_settings() -> Value {
        json!({
            "settings": {
                "custom_words": ["SpeakoFlow", "Qwen"],
                "selected_model": "parakeet-tdt-0.6b-v3",
                "bindings": { "transcribe": { "current_binding": "command_right" } }
            },
            "meeting_pill_position": { "x": 10.0, "y": 20.0, "w": 100.0, "h": 40.0 }
        })
    }

    fn write_json(path: &Path, value: &Value) {
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn files_in(dir: &tempfile::TempDir) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn corrupt_copies(dir: &tempfile::TempDir) -> Vec<PathBuf> {
        files_in(dir)
            .into_iter()
            .filter(|name| name.starts_with("settings_store.json.corrupt-"))
            .map(|name| dir.path().join(name))
            .collect()
    }

    fn kinds(file: &SettingsFile) -> Vec<SettingsNoticeKind> {
        file.notices()
            .into_iter()
            .map(|notice| notice.kind)
            .collect()
    }

    #[test]
    fn a_fresh_install_starts_empty_and_saves_what_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        let file = SettingsFile::open(path.clone());
        assert_eq!(file.outcome(), &LoadOutcome::Fresh);
        assert!(file.get("settings").is_none());
        assert!(file.notices().is_empty());

        file.set("settings", json!({ "push_to_talk": false }));
        file.flush().unwrap();
        assert_eq!(
            read_json(&path),
            json!({ "settings": { "push_to_talk": false } })
        );
    }

    #[test]
    fn a_good_file_round_trips_and_refreshes_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        write_json(&path, &user_settings());

        let file = SettingsFile::open(path.clone());
        assert_eq!(file.outcome(), &LoadOutcome::Loaded);
        assert!(file.notices().is_empty());
        assert_eq!(
            file.get("settings"),
            user_settings().get("settings").cloned()
        );
        assert_eq!(
            fs::read(backup_path(&path)).unwrap(),
            fs::read(&path).unwrap()
        );
        // Nothing changed, so nothing is written back.
        file.flush().unwrap();
        assert_eq!(read_json(&path), user_settings());
    }

    #[test]
    fn an_identical_backup_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        write_json(&path, &user_settings());
        drop(SettingsFile::open(path.clone()));

        // Make the existing backup recognisable: if the launch rewrote it, the
        // new file would not carry this modification time.
        let backup = backup_path(&path);
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        File::options()
            .write(true)
            .open(&backup)
            .unwrap()
            .set_modified(old)
            .unwrap();

        drop(SettingsFile::open(path.clone()));
        assert_eq!(fs::metadata(&backup).unwrap().modified().unwrap(), old);

        // A different file does refresh it.
        write_json(
            &path,
            &json!({ "settings": { "custom_words": ["SpeakoFlow"] } }),
        );
        drop(SettingsFile::open(path.clone()));
        assert_eq!(fs::read(&backup).unwrap(), fs::read(&path).unwrap());
    }

    /// The bug: shutdown left the file empty, the next launch parsed nothing,
    /// and the defaults were written over it. Every shape an interrupted
    /// truncate-then-write can leave must restore from the backup instead.
    #[test]
    fn a_damaged_file_is_kept_and_settings_come_back_from_the_backup() {
        let full = serde_json::to_vec_pretty(&user_settings()).unwrap();
        let damaged: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"   \n".to_vec(),
            vec![0u8; 512],
            full[..full.len() / 2].to_vec(),
            b"[1, 2, 3]".to_vec(),
            UTF8_BOM.to_vec(),
        ];
        for bytes in damaged {
            let dir = tempfile::tempdir().unwrap();
            let path = store_path(&dir);
            write_json(&backup_path(&path), &user_settings());
            fs::write(&path, &bytes).unwrap();

            let file = SettingsFile::open(path.clone());
            let LoadOutcome::RestoredFromBackup { kept_as } = file.outcome().clone() else {
                panic!(
                    "expected a restore for {:?}, got {:?}",
                    bytes,
                    file.outcome()
                );
            };
            assert_eq!(fs::read(&kept_as).unwrap(), bytes, "damaged file is kept");
            assert_eq!(
                file.get("settings"),
                user_settings().get("settings").cloned()
            );
            assert_eq!(
                file.get("meeting_pill_position"),
                user_settings().get("meeting_pill_position").cloned()
            );
            assert_eq!(kinds(&file), vec![SettingsNoticeKind::Restored]);

            // The restored settings are the real file again as soon as it opens.
            assert_eq!(read_json(&path), user_settings());
        }
    }

    #[test]
    fn a_damaged_file_with_no_backup_is_kept_and_not_overwritten_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        fs::write(&path, b"{\"settings\": {\"custom_words\": [\"Spea").unwrap();

        let file = SettingsFile::open(path.clone());
        let LoadOutcome::Unrecoverable { kept_as } = file.outcome().clone() else {
            panic!("expected unrecoverable, got {:?}", file.outcome());
        };
        assert!(file.get("settings").is_none());
        assert_eq!(
            file.notices(),
            vec![SettingsNotice {
                kind: SettingsNoticeKind::Unrecoverable,
                kept_as: Some(kept_as.file_name().unwrap().to_string_lossy().into_owned()),
            }]
        );

        // What get_settings does next: write the defaults.
        file.set("settings", json!({ "custom_words": [] }));
        file.flush().unwrap();
        assert_eq!(
            read_json(&path),
            json!({ "settings": { "custom_words": [] } })
        );
        assert_eq!(
            fs::read(&kept_as).unwrap(),
            b"{\"settings\": {\"custom_words\": [\"Spea".to_vec(),
            "the damaged original survives for manual recovery"
        );
        assert_eq!(corrupt_copies(&dir), vec![kept_as]);
    }

    #[test]
    fn a_damaged_backup_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        fs::write(&path, b"").unwrap();
        fs::write(backup_path(&path), b"{\"settings\": ").unwrap();

        let file = SettingsFile::open(path);
        assert!(matches!(file.outcome(), LoadOutcome::Unrecoverable { .. }));
        assert!(file.get("settings").is_none());
    }

    #[test]
    fn a_byte_order_mark_is_not_damage() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend(serde_json::to_vec_pretty(&user_settings()).unwrap());
        fs::write(&path, &bytes).unwrap();

        let file = SettingsFile::open(path.clone());
        assert_eq!(file.outcome(), &LoadOutcome::Loaded);
        assert_eq!(
            file.get("settings"),
            user_settings().get("settings").cloned()
        );
        assert!(corrupt_copies(&dir).is_empty());
    }

    /// Without the backup, a file with no `settings` object is read as is:
    /// that is a store some other key was saved into first.
    #[test]
    fn a_file_without_settings_and_no_backup_is_read_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        write_json(&path, &json!({ "meeting_pill_position": { "x": 1.0 } }));

        let file = SettingsFile::open(path.clone());
        assert_eq!(file.outcome(), &LoadOutcome::Loaded);
        assert!(file.notices().is_empty());
        assert!(!backup_path(&path).exists());
    }

    /// A file that parses but lost its `settings` object takes it back from the
    /// backup and keeps its other keys. Run over two launches, with the
    /// defaults write `get_settings` would do in between, the backup must still
    /// hold the user's settings at the end.
    #[test]
    fn a_file_without_settings_takes_them_from_the_backup_across_two_launches() {
        for primary_settings in [
            None,
            Some(Value::Null),
            Some(json!("oops")),
            Some(json!([1])),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = store_path(&dir);
            write_json(&backup_path(&path), &user_settings());
            let mut primary = json!({ "meeting_pill_position": { "x": 1.0 } });
            if let Some(value) = primary_settings.clone() {
                primary["settings"] = value;
            }
            write_json(&path, &primary);

            // First launch.
            {
                let file = SettingsFile::open(path.clone());
                assert_eq!(
                    file.outcome(),
                    &LoadOutcome::SettingsRestoredFromBackup,
                    "for {primary_settings:?}"
                );
                assert_eq!(kinds(&file), vec![SettingsNoticeKind::Restored]);
                assert_eq!(
                    file.get("settings"),
                    user_settings().get("settings").cloned()
                );
                assert_eq!(file.get("meeting_pill_position"), Some(json!({ "x": 1.0 })));
                // What get_settings does with settings it can read: nothing.
                // With none it would have written the defaults here.
                file.flush().unwrap();
            }
            assert_eq!(
                read_json(&path),
                json!({
                    "settings": user_settings()["settings"],
                    "meeting_pill_position": { "x": 1.0 }
                })
            );

            // Second launch reads that file normally and refreshes the backup
            // from it, so the backup still holds the user's settings.
            let file = SettingsFile::open(path.clone());
            assert_eq!(file.outcome(), &LoadOutcome::Loaded);
            assert_eq!(
                read_json(&backup_path(&path))["settings"],
                user_settings()["settings"]
            );
        }
    }

    /// A read error (a scanner holding the file) is not damage: the file stays
    /// exactly where it is, nothing is written over it this session, and the
    /// session runs on the backup.
    #[test]
    fn an_unreadable_file_is_left_alone_and_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        write_json(&backup_path(&path), &user_settings());
        // A directory where the file should be: every read fails with an
        // error other than NotFound, on every platform.
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), b"untouched").unwrap();

        let file = SettingsFile::open(path.clone());
        assert_eq!(
            file.outcome(),
            &LoadOutcome::Unreadable { from_backup: true }
        );
        assert_eq!(kinds(&file), vec![SettingsNoticeKind::NotSaving]);
        assert_eq!(
            file.get("settings"),
            user_settings().get("settings").cloned()
        );

        file.set("settings", json!({ "custom_words": [] }));
        file.flush().unwrap();
        assert!(path.is_dir(), "the unreadable file was not replaced");
        assert_eq!(fs::read(path.join("inside")).unwrap(), b"untouched");
        assert!(corrupt_copies(&dir).is_empty(), "nothing was quarantined");
        assert_eq!(read_json(&backup_path(&path)), user_settings());
    }

    #[test]
    fn an_unreadable_file_with_no_backup_uses_the_defaults_and_does_not_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        fs::create_dir(&path).unwrap();

        let file = SettingsFile::open(path.clone());
        assert_eq!(
            file.outcome(),
            &LoadOutcome::Unreadable { from_backup: false }
        );
        assert!(file.get("settings").is_none());
        file.set("settings", json!({ "a": 1 }));
        file.flush().unwrap();
        assert!(path.is_dir());
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn only_the_newest_corrupt_copies_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        for stamp in 100..107 {
            fs::write(sibling(&path, &format!(".corrupt-{stamp}")), b"old").unwrap();
        }
        fs::write(sibling(&path, ".corrupt-105-1"), b"old").unwrap();
        // Not a quarantined copy by name, so never deleted.
        fs::write(sibling(&path, ".corrupt-notes"), b"mine").unwrap();
        fs::write(&path, b"").unwrap();

        let file = SettingsFile::open(path.clone());
        let LoadOutcome::Unrecoverable { kept_as } = file.outcome().clone() else {
            panic!("expected unrecoverable, got {:?}", file.outcome());
        };
        let names: Vec<String> = files_in(&dir)
            .into_iter()
            .filter(|name| name.contains(".corrupt-"))
            .collect();
        let mut expected = vec![
            "settings_store.json.corrupt-104".to_string(),
            "settings_store.json.corrupt-105".to_string(),
            "settings_store.json.corrupt-105-1".to_string(),
            "settings_store.json.corrupt-106".to_string(),
            "settings_store.json.corrupt-notes".to_string(),
            kept_as.file_name().unwrap().to_string_lossy().into_owned(),
        ];
        expected.sort();
        assert_eq!(names, expected);
    }

    #[test]
    fn corrupt_copy_names_order_by_time_then_counter() {
        assert_eq!(corrupt_copy_order("1700000000"), Some((1_700_000_000, 0)));
        assert_eq!(corrupt_copy_order("1700000000-2"), Some((1_700_000_000, 2)));
        assert_eq!(corrupt_copy_order("notes"), None);
        assert_eq!(corrupt_copy_order("1-x"), None);
        assert!(corrupt_copy_order("5-1") > corrupt_copy_order("5"));
        assert!(corrupt_copy_order("6") > corrupt_copy_order("5-9"));
    }

    #[test]
    fn retries_are_bounded_short_and_growing() {
        let delays: Vec<Duration> = (1..).map_while(retry_delay).collect();
        assert_eq!(delays.len() as u32, ATTEMPTS - 1);
        assert!(delays.windows(2).all(|pair| pair[0] < pair[1]));
        let total: Duration = delays.iter().sum();
        assert!(total < Duration::from_secs(1), "{total:?}");
        assert_eq!(retry_delay(0), None);
        assert_eq!(retry_delay(ATTEMPTS), None);
    }

    #[test]
    fn only_errors_another_process_causes_are_retried() {
        assert!(is_held_by_another_process(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!is_held_by_another_process(&io::Error::from(
            io::ErrorKind::NotFound
        )));
        assert!(!is_held_by_another_process(&io::Error::other("disk full")));
        #[cfg(windows)]
        for code in [5, 32, 33] {
            assert!(is_held_by_another_process(&io::Error::from_raw_os_error(
                code
            )));
        }
        #[cfg(windows)]
        assert!(!is_held_by_another_process(&io::Error::from_raw_os_error(
            112 // ERROR_DISK_FULL
        )));
    }

    #[test]
    fn a_held_file_is_retried_until_it_is_released() {
        let calls = Cell::new(0);
        let mut slept = Vec::new();
        let result = retry_with(
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 4 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok("written")
                }
            },
            is_held_by_another_process,
            |delay| slept.push(delay),
        );
        assert_eq!(result.unwrap(), "written");
        assert_eq!(calls.get(), 4);
        assert_eq!(
            slept,
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(30)
            ]
        );
    }

    #[test]
    fn a_file_held_for_good_gives_up_and_other_errors_do_not_wait() {
        let calls = Cell::new(0);
        let held = retry_with(
            || -> io::Result<()> {
                calls.set(calls.get() + 1);
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            is_held_by_another_process,
            |_| {},
        );
        assert_eq!(held.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls.get(), ATTEMPTS);

        calls.set(0);
        let mut slept = 0;
        let full = retry_with(
            || -> io::Result<()> {
                calls.set(calls.get() + 1);
                Err(io::Error::other("disk full"))
            },
            is_held_by_another_process,
            |_| slept += 1,
        );
        assert!(full.is_err());
        assert_eq!((calls.get(), slept), (1, 0));
    }

    #[test]
    fn persistent_save_failures_are_recognised() {
        assert!(!save_failure_is_persistent(1, Duration::ZERO));
        assert!(!save_failure_is_persistent(2, Duration::from_secs(9)));
        assert!(save_failure_is_persistent(3, Duration::ZERO));
        assert!(save_failure_is_persistent(2, Duration::from_secs(10)));
    }

    /// A save that keeps failing is raised once per run of failures, survives
    /// a dismissal without coming back, and clears on the next save that works.
    #[test]
    fn failing_saves_raise_one_notice_that_a_good_save_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        let file = SettingsFile::new(Some(path.clone()), Map::new(), false, LoadOutcome::Fresh);
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        file.set_notifier(move |notices| lock(&sink).push(notices));

        let start = Instant::now();
        let failed: io::Result<()> = Err(io::Error::other("disk full"));
        assert!(!file.record_save(&path, &failed, start));
        assert!(!file.record_save(&path, &failed, start + Duration::from_secs(2)));
        assert!(file.notices().is_empty());
        assert!(file.record_save(&path, &failed, start + Duration::from_secs(4)));
        assert_eq!(kinds(&file), vec![SettingsNoticeKind::SaveFailing]);

        // Dismissed, it stays down for the rest of this run of failures.
        assert!(file.dismiss_notice(SettingsNoticeKind::SaveFailing));
        assert!(!file.record_save(&path, &failed, start + Duration::from_secs(6)));
        assert!(file.notices().is_empty());

        // A save that works ends the run, so the next one can be raised again.
        assert!(!file.record_save(&path, &Ok(()), start + Duration::from_secs(8)));
        let later = start + Duration::from_secs(20);
        assert!(!file.record_save(&path, &failed, later));
        assert!(file.record_save(&path, &failed, later + Duration::from_secs(11)));
        assert!(file.record_save(&path, &Ok(()), later + Duration::from_secs(12)));
        assert!(file.notices().is_empty());

        assert!(lock(&heard).is_empty(), "record_save alone does not notify");
    }

    #[test]
    fn a_save_that_keeps_failing_tells_the_notifier_and_a_good_one_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the settings directory should be, so every save fails
        // at once (without the lock retries) until it is removed.
        let blocker = dir.path().join("app-data");
        fs::write(&blocker, b"not a directory").unwrap();
        let path = blocker.join("settings_store.json");
        let file = SettingsFile::new(Some(path.clone()), Map::new(), false, LoadOutcome::Fresh);
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        file.set_notifier(move |notices| lock(&sink).push(notices));

        // No writer thread here, so `set` saves at once.
        for n in 0..FAILURES_BEFORE_NOTICE {
            file.set("settings", json!({ "n": n }));
        }
        assert_eq!(
            *lock(&heard),
            vec![vec![SettingsNotice {
                kind: SettingsNoticeKind::SaveFailing,
                kept_as: None
            }]]
        );

        fs::remove_file(&blocker).unwrap();
        file.flush().unwrap();
        assert_eq!(lock(&heard).last().cloned(), Some(Vec::new()));
        assert_eq!(read_json(&path), json!({ "settings": { "n": 2 } }));
        assert!(file.notices().is_empty());
    }

    #[test]
    fn a_dismissed_load_notice_stays_dismissed() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        write_json(&backup_path(&path), &user_settings());
        fs::write(&path, b"").unwrap();

        let file = SettingsFile::open(path);
        assert!(!file.dismiss_notice(SettingsNoticeKind::NotSaving));
        assert_eq!(kinds(&file), vec![SettingsNoticeKind::Restored]);
        assert!(file.dismiss_notice(SettingsNoticeKind::Restored));
        assert!(file.notices().is_empty());
    }

    #[test]
    fn saves_leave_no_temporary_files_and_old_ones_are_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        let stale = dir.path().join(".settings_store.json.tmp-999999999");
        fs::write(&stale, b"{\"half").unwrap();

        let file = SettingsFile::open(path.clone());
        assert!(
            !stale.exists(),
            "a dead process's temporary file is removed"
        );
        for n in 0..5 {
            file.set("settings", json!({ "n": n }));
            file.flush().unwrap();
        }
        assert_eq!(files_in(&dir), vec!["settings_store.json".to_string()]);
        assert_eq!(read_json(&path), json!({ "settings": { "n": 4 } }));
    }

    #[test]
    fn other_keys_survive_and_delete_removes_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        write_json(&path, &user_settings());

        let file = SettingsFile::open(path.clone());
        file.set("settings", json!({ "push_to_talk": true }));
        assert!(file.delete("meeting_pill_position"));
        assert!(!file.delete("never_there"));
        file.flush().unwrap();
        assert_eq!(
            read_json(&path),
            json!({ "settings": { "push_to_talk": true } })
        );
    }

    #[test]
    fn the_writer_thread_saves_without_an_explicit_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        let file = SettingsFile::open(path.clone());
        file.set("settings", json!({ "saved_by": "writer" }));

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                == Some(json!({ "settings": { "saved_by": "writer" } }))
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the change was never saved");
    }

    #[test]
    fn a_reopened_file_reads_what_the_last_session_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        {
            let file = SettingsFile::open(path.clone());
            file.set("settings", json!({ "custom_words": ["SpeakoFlow"] }));
            file.flush().unwrap();
        }
        let file = SettingsFile::open(path);
        assert_eq!(file.outcome(), &LoadOutcome::Loaded);
        assert_eq!(
            file.get("settings"),
            Some(json!({ "custom_words": ["SpeakoFlow"] }))
        );
    }

    /// `bindings.ts` is only regenerated while the app runs in dev, so the
    /// notice types are checked against what the generator would write.
    #[test]
    fn the_notice_types_in_bindings_match_the_generator() {
        let bindings = include_str!("../../src/bindings.ts").replace("\r\n", "\n");
        let config = specta_typescript::Typescript::default();
        for exported in [
            specta_typescript::export::<SettingsNotice>(&config).unwrap(),
            specta_typescript::export::<SettingsNoticeKind>(&config).unwrap(),
        ] {
            assert!(
                bindings.contains(&exported),
                "bindings.ts is missing:\n{exported}"
            );
        }
    }

    #[test]
    fn an_in_memory_file_never_touches_the_disk() {
        let file = SettingsFile::in_memory();
        file.set("settings", json!({ "a": 1 }));
        file.flush().unwrap();
        assert_eq!(file.get("settings"), Some(json!({ "a": 1 })));
    }
}
