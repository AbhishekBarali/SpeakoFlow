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
//!   either.
//! - **An unreadable file is never overwritten silently.** It is renamed to
//!   `settings_store.json.corrupt-<unix time>` so nothing is destroyed, the error
//!   is logged, and the settings come back from the last good backup when there
//!   is one.
//! - **A backup is kept.** `settings_store.json.bak` is refreshed (atomically)
//!   from every launch that read a good file containing settings.
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

use log::{error, info, warn};
use serde_json::{Map, Value};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long after a change the writer thread waits before saving, so a burst of
/// changes (a slider, a migration pass) becomes one write. Same as the plugin's
/// auto-save.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);

/// How long the writer thread waits before retrying a save that failed (disk
/// full, file locked by a scanner), so a persistent failure is not a busy loop.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(2);

/// The key the app's whole configuration lives under.
const SETTINGS_KEY: &str = "settings";

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
    /// The file could not be read or parsed. It was kept at `kept_as`, and the
    /// entries came from the backup.
    RestoredFromBackup { kept_as: PathBuf },
    /// The file could not be read or parsed and there was no usable backup.
    /// It was kept at `kept_as`, and the app starts from its defaults.
    Unrecoverable { kept_as: PathBuf },
    /// The file could not be read or parsed, and it could not be moved aside
    /// either. Saving is turned off for this run so it is not overwritten.
    Untouchable,
}

pub struct SettingsFile {
    /// `None` when there is nowhere to save, or when saving was turned off to
    /// protect a file that could not be moved aside.
    path: Option<PathBuf>,
    state: Mutex<State>,
    changed: Condvar,
    /// Serialises saves, so snapshots reach the disk in the order they were
    /// taken and a later state is never replaced by an earlier one.
    saving: Mutex<()>,
    /// The writer thread is running. Without it every change saves at once.
    background: AtomicBool,
    /// The last save failed. Used to log a failure streak once, not per retry.
    failing: AtomicBool,
    outcome: LoadOutcome,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere while holding the lock must not take settings down too.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SettingsFile {
    /// Read the file at `path`, recovering from damage as described in the
    /// module docs, and start the writer thread.
    pub fn open(path: PathBuf) -> Arc<Self> {
        remove_stale_temp_files(&path);
        let (entries, dirty, outcome) = load(&path);
        let path = (outcome != LoadOutcome::Untouchable).then_some(path);
        let file = Arc::new(Self {
            path,
            state: Mutex::new(State { entries, dirty }),
            changed: Condvar::new(),
            saving: Mutex::new(()),
            background: AtomicBool::new(false),
            failing: AtomicBool::new(false),
            outcome,
        });
        if file.path.is_some() {
            spawn_writer(&file);
            if dirty {
                // Restored from the backup: make it the real file again now,
                // rather than whenever the next setting changes.
                file.changed.notify_one();
            }
        }
        file
    }

    /// A settings file that is never saved, for when the app data directory
    /// cannot be resolved at all. Settings still work for the session.
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            path: None,
            state: Mutex::new(State {
                entries: Map::new(),
                dirty: false,
            }),
            changed: Condvar::new(),
            saving: Mutex::new(()),
            background: AtomicBool::new(false),
            failing: AtomicBool::new(false),
            outcome: LoadOutcome::Fresh,
        })
    }

    pub fn outcome(&self) -> &LoadOutcome {
        &self.outcome
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        lock(&self.state).entries.get(key).cloned()
    }

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
        let _saving = lock(&self.saving);
        let bytes = {
            let mut state = lock(&self.state);
            if !state.dirty {
                return Ok(());
            }
            state.dirty = false;
            serde_json::to_vec_pretty(&state.entries).map_err(io::Error::other)?
        };
        match write_atomically(path, &bytes) {
            Ok(()) => {
                if self.failing.swap(false, Ordering::Relaxed) {
                    info!("Settings saved again after an earlier failure");
                }
                Ok(())
            }
            Err(err) => {
                lock(&self.state).dirty = true;
                if !self.failing.swap(true, Ordering::Relaxed) {
                    error!("Failed to save settings to {}: {err}", path.display());
                }
                Err(err)
            }
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

/// Parse a store file: a JSON object of keys, as tauri-plugin-store wrote it.
fn parse_entries(bytes: &[u8]) -> Result<Map<String, Value>, String> {
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

/// Read the file, recovering from damage. Returns the entries, whether they
/// must be saved (they came from the backup), and what happened.
fn load(path: &Path) -> (Map<String, Value>, bool, LoadOutcome) {
    let problem = match fs::read(path) {
        Ok(bytes) => match parse_entries(&bytes) {
            Ok(entries) => {
                if has_settings(&entries) {
                    if let Err(err) = write_atomically(&backup_path(path), &bytes) {
                        warn!("Could not refresh the settings backup: {err}");
                    }
                }
                return (entries, false, LoadOutcome::Loaded);
            }
            Err(problem) => problem,
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return (Map::new(), false, LoadOutcome::Fresh);
        }
        Err(err) => format!("the file could not be read ({err})"),
    };

    let Some(kept_as) = quarantine(path) else {
        error!(
            "Settings file {} is unusable: {problem}. It could not be moved aside either, \
so settings will not be saved this session to avoid overwriting it.",
            path.display()
        );
        return (Map::new(), false, LoadOutcome::Untouchable);
    };

    let backup = backup_path(path);
    let restored = fs::read(&backup)
        .ok()
        .and_then(|bytes| parse_entries(&bytes).ok())
        .filter(has_settings);
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
    match fs::rename(path, &kept_as) {
        Ok(()) => Some(kept_as),
        Err(err) => {
            warn!("Could not move {} aside: {err}", path.display());
            None
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
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
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
    use std::time::Instant;

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

    #[test]
    fn a_fresh_install_starts_empty_and_saves_what_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        let file = SettingsFile::open(path.clone());
        assert_eq!(file.outcome(), &LoadOutcome::Fresh);
        assert!(file.get("settings").is_none());

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

            // The restored settings become the real file again.
            file.flush().unwrap();
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
    fn a_file_without_settings_does_not_replace_a_good_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        write_json(&backup_path(&path), &user_settings());
        write_json(&path, &json!({ "meeting_pill_position": { "x": 1.0 } }));

        let file = SettingsFile::open(path.clone());
        assert_eq!(file.outcome(), &LoadOutcome::Loaded);
        assert_eq!(read_json(&backup_path(&path)), user_settings());
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
            file.set("settings", json!({ "custom_words": ["Kiro"] }));
            file.flush().unwrap();
        }
        let file = SettingsFile::open(path);
        assert_eq!(file.outcome(), &LoadOutcome::Loaded);
        assert_eq!(
            file.get("settings"),
            Some(json!({ "custom_words": ["Kiro"] }))
        );
    }

    #[test]
    fn an_in_memory_file_never_touches_the_disk() {
        let file = SettingsFile::in_memory();
        file.set("settings", json!({ "a": 1 }));
        file.flush().unwrap();
        assert_eq!(file.get("settings"), Some(json!({ "a": 1 })));
    }
}
