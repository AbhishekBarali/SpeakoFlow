//! Tying the pieces together: watch the field, notice a correction, keep the word.
//!
//! [`detect`](super::detect) decides what an edit taught us and [`watch`](super::watch)
//! reads the field. This is the part that knows about the app — settings, the on/off
//! switch, the cap on how much is remembered — and it is deliberately the only part
//! that does, so the other two stay testable without a running app.

use std::collections::HashSet;
use std::sync::Arc;

use log::{debug, info};
use tauri::{AppHandle, Emitter, Manager};

use super::detect::{self, Correction};
use super::watch::CorrectionWatcher;

/// Emitted when a word is learned, so Settings can show it arriving.
pub const LEARNED_EVENT: &str = "autolearn-words-changed";

/// Most words kept from corrections.
///
/// A dictionary is a bias, not an archive: every entry makes the fuzzy correction pass
/// slightly more willing to rewrite something, so an unbounded list would eventually
/// degrade transcription rather than improve it. At the cap the oldest entries are
/// dropped, on the reasoning that a name someone dictated six months ago and has not
/// corrected since is either already recognised or no longer relevant.
pub const MAX_LEARNED_WORDS: usize = 200;

/// Start the watcher, if this platform and this user allow it.
///
/// Called once at launch. Returns `None` when the platform cannot read a text field, in
/// which case nothing else in this module ever runs — the setting is still stored, but
/// the UI reports it as unavailable rather than offering a switch that does nothing.
///
/// The setting is checked at *use* time rather than here, so turning it on takes effect
/// on the next dictation instead of the next launch.
pub fn start(app: &AppHandle) -> Option<Arc<CorrectionWatcher>> {
    let learn_app = app.clone();
    let watcher = CorrectionWatcher::start(move |current_text, pasted| {
        // Re-read the settings on every poll rather than capturing them. The user can
        // turn this off mid-watch, and they can add the word by hand while we are
        // watching for it — both should be respected immediately.
        let settings = crate::settings::get_settings(&learn_app);
        if !settings.auto_learn_corrections {
            // Stops the watch: honouring the switch matters more than finishing the
            // window we already opened.
            return true;
        }

        let known = known_words(&settings);
        let corrections = detect::detect(pasted, current_text, &known);
        if corrections.is_empty() {
            // Keep watching: the user may not have got round to fixing it yet.
            return false;
        }

        remember(&learn_app, &corrections);
        // Done with this dictation. A second correction to the same transcript is worth
        // less than the risk of continuing to read a field the user has moved on from.
        true
    })?;
    Some(Arc::new(watcher))
}

/// Start watching the field a transcript was just pasted into.
///
/// A no-op when the feature is off or unavailable, so the caller does not have to
/// check either. Must be called while the target still has keyboard focus.
pub fn watch_after_paste(app: &AppHandle, pasted: &str) {
    if pasted.trim().is_empty() {
        return;
    }
    if !crate::settings::get_settings(app).auto_learn_corrections {
        return;
    }
    let Some(watcher) = app.try_state::<Arc<CorrectionWatcher>>() else {
        // Unsupported platform, or the watcher thread failed to start.
        return;
    };
    watcher.watch(pasted.to_string());
}

/// Stop watching. Called when a new recording starts: the previous field's edits are no
/// longer the ones that matter, and the next paste will start its own watch.
pub fn cancel(app: &AppHandle) {
    if let Some(watcher) = app.try_state::<Arc<CorrectionWatcher>>() {
        watcher.cancel();
    }
}

/// Every word already known, lowercased, from both lists.
///
/// Both, because a word the user typed into their own dictionary must not be learned
/// again as a guess — it is already theirs, and duplicating it into the learned list
/// would present it for review as though the app had inferred it.
fn known_words(settings: &crate::settings::AppSettings) -> HashSet<String> {
    settings
        .custom_words
        .iter()
        .chain(settings.learned_words.iter())
        .map(|word| word.to_lowercase())
        .collect()
}

/// Store the learned words.
fn remember(app: &AppHandle, corrections: &[Correction]) {
    let mut settings = crate::settings::get_settings(app);
    let known = known_words(&settings);
    let mut added = Vec::new();

    for correction in corrections {
        if known.contains(&correction.to.to_lowercase()) {
            continue;
        }
        added.push(correction.to.clone());
        settings.learned_words.push(correction.to.clone());
        info!(
            "Learned \"{}\" from a correction of \"{}\"",
            correction.to, correction.from
        );
    }

    if added.is_empty() {
        return;
    }

    // Oldest first out. A dictionary is a bias rather than an archive.
    if settings.learned_words.len() > MAX_LEARNED_WORDS {
        let excess = settings.learned_words.len() - MAX_LEARNED_WORDS;
        settings.learned_words.drain(..excess);
    }

    crate::settings::write_settings(app, settings);
    let _ = app.emit(LEARNED_EVENT, added);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with(custom: &[&str], learned: &[&str]) -> crate::settings::AppSettings {
        let mut settings = crate::settings::get_default_settings();
        settings.custom_words = custom.iter().map(|word| word.to_string()).collect();
        settings.learned_words = learned.iter().map(|word| word.to_string()).collect();
        settings
    }

    /// Both lists count as known. A word the user typed in themselves must not be
    /// re-learned as a guess and presented for review as though inferred.
    #[test]
    fn known_words_span_both_lists() {
        let known = known_words(&settings_with(&["Barali"], &["Kubernetes"]));
        assert!(known.contains("barali"));
        assert!(known.contains("kubernetes"));
        assert_eq!(known.len(), 2);
    }

    #[test]
    fn known_words_are_case_folded() {
        let known = known_words(&settings_with(&["BARALI"], &[]));
        assert!(known.contains("barali"));
    }

    #[test]
    fn no_words_means_an_empty_set() {
        assert!(known_words(&settings_with(&[], &[])).is_empty());
    }

    /// The cap has to be small enough that the list stays a bias rather than becoming
    /// an archive that degrades transcription.
    #[test]
    fn the_cap_is_bounded() {
        assert!(MAX_LEARNED_WORDS >= 50);
        assert!(MAX_LEARNED_WORDS <= 1_000);
    }
}
