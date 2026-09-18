//! Tauri commands for auto-learn from corrections.
//!
//! Thin: the decisions live in [`crate::autolearn`]. What is here is the on/off switch,
//! the reviewable word list, and a status the UI needs in order to say "not available
//! on this system" instead of offering a switch that does nothing.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

/// Whether auto-learn can work here, whether it is on, and what it has learned.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct AutoLearnStatus {
    /// False on macOS and Linux, where reading the field just dictated into is not
    /// implemented yet. The UI hides the switch rather than offering a dead one.
    pub supported: bool,
    pub enabled: bool,
    /// Words learned so far, oldest first. Shown for review, because a learned word is
    /// a guess and the user is entitled to see and remove it.
    pub learned: Vec<String>,
    pub max_learned: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn get_auto_learn_status(app: AppHandle) -> Result<AutoLearnStatus, String> {
    let settings = crate::settings::get_settings(&app);
    Ok(AutoLearnStatus {
        supported: crate::autolearn::watch::supported(),
        enabled: settings.auto_learn_corrections,
        learned: settings.learned_words,
        max_learned: crate::autolearn::learner::MAX_LEARNED_WORDS as u32,
    })
}

/// Turn auto-learn on or off.
///
/// Turning it off also stops any watch already running, rather than letting the current
/// one finish: the switch means "stop reading my text fields", and honouring it a minute
/// later would not be honouring it.
#[tauri::command]
#[specta::specta]
pub async fn set_auto_learn_corrections(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.auto_learn_corrections = enabled;
    crate::settings::write_settings(&app, settings);
    if !enabled {
        crate::autolearn::learner::cancel(&app);
    }
    Ok(())
}

/// Replace the learned-word list.
///
/// The whole list rather than a remove-one command, for the reason the text-replacement
/// and custom-word commands take whole lists too: the UI holds the authoritative order,
/// and a per-item API would need to identify an item by a value that is itself editable.
///
/// Blank entries are dropped and the list is de-duplicated case-insensitively, because a
/// duplicate would be sent to the recogniser twice and bias it twice.
#[tauri::command]
#[specta::specta]
pub async fn set_learned_words(app: AppHandle, words: Vec<String>) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.learned_words = dedupe(words);
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// Promote a learned word into the user's own dictionary.
///
/// The point of the two lists: accepting a guess makes it theirs, at which point it stops
/// being reviewable as a guess and stops counting against the learned-word cap.
#[tauri::command]
#[specta::specta]
pub async fn keep_learned_word(app: AppHandle, word: String) -> Result<(), String> {
    let trimmed = word.trim().to_string();
    if trimmed.is_empty() {
        return Err("A word cannot be empty.".to_string());
    }

    let mut settings = crate::settings::get_settings(&app);
    settings
        .learned_words
        .retain(|existing| !existing.eq_ignore_ascii_case(&trimmed));
    if !settings
        .custom_words
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&trimmed))
    {
        settings.custom_words.push(trimmed);
    }
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// Trim, drop blanks, and de-duplicate case-insensitively, keeping first appearance.
fn dedupe(words: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for word in words {
        let trimmed = word.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(trimmed.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_entries_are_dropped() {
        assert_eq!(
            dedupe(vec!["  ".into(), "Barali".into(), "".into()]),
            vec!["Barali".to_string()]
        );
    }

    /// A duplicate would be sent to the recogniser twice and bias it twice.
    #[test]
    fn duplicates_are_removed_case_insensitively() {
        assert_eq!(
            dedupe(vec!["Barali".into(), "barali".into(), "BARALI".into()]),
            vec!["Barali".to_string()]
        );
    }

    #[test]
    fn the_first_spelling_wins() {
        assert_eq!(
            dedupe(vec!["Kubernetes".into(), "kubernetes".into()]),
            vec!["Kubernetes".to_string()]
        );
    }

    #[test]
    fn entries_are_trimmed() {
        assert_eq!(
            dedupe(vec!["  Barali  ".into()]),
            vec!["Barali".to_string()]
        );
    }

    #[test]
    fn an_empty_list_stays_empty() {
        assert!(dedupe(Vec::new()).is_empty());
    }
}
