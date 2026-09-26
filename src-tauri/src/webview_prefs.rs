//! Repairs to the WebView2 profile, applied at launch before any window exists.
//!
//! **Caret browsing.** Chromium's caret browsing mode (F7) draws a blinking
//! insertion caret into any text that is clicked — a label, a history row, a
//! heading — as if the window were a document. WebView2 toggles it with no
//! confirmation and saves the choice in the profile's `Preferences` file
//! (`settings.a11y.caretbrowsing.enabled`), so one stray F7 left every window of
//! the app showing a text cursor in text that cannot be edited, across restarts.
//!
//! The frontend already hides that caret and swallows F7
//! (`src/lib/caretBrowsing.ts`); this puts the saved state back to off so the
//! rest of the mode — arrow keys moving an invisible caret instead of scrolling
//! — goes too. It runs before the first webview is built, which is the only
//! moment the file is not owned by a running browser process, and it rewrites
//! the file only when caret browsing is actually on.

use serde_json::Value;
use std::path::{Path, PathBuf};

const CARET_BROWSING_POINTER: &str = "/settings/a11y/caretbrowsing/enabled";

/// The `Preferences` file of the profile WebView2 creates in `user_data_dir`.
fn preferences_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir
        .join("EBWebView")
        .join("Default")
        .join("Preferences")
}

/// `prefs` with caret browsing switched off, or `None` when there is nothing to
/// change (already off, never set, or not a JSON object we understand).
fn with_caret_browsing_off(prefs: &str) -> Option<String> {
    let mut doc: Value = serde_json::from_str(prefs).ok()?;
    let enabled = doc.pointer_mut(CARET_BROWSING_POINTER)?;
    if enabled.as_bool() != Some(true) {
        return None;
    }
    *enabled = Value::Bool(false);
    serde_json::to_string(&doc).ok()
}

/// Switch caret browsing off in the WebView2 profile under `user_data_dir`.
/// Best effort: any failure leaves the file as it was.
pub fn disable_caret_browsing(user_data_dir: &Path) {
    let path = preferences_path(user_data_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let Some(fixed) = with_caret_browsing_off(&raw) else {
        return;
    };
    // Write beside the original and rename over it, so an interrupted write can
    // never leave a truncated profile behind.
    let staged = path.with_extension("speakoflow-staged");
    let result = std::fs::write(&staged, fixed).and_then(|()| std::fs::rename(&staged, &path));
    match result {
        Ok(()) => log::info!("Turned off WebView2 caret browsing (it had been toggled with F7)"),
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            log::warn!("Could not turn off WebView2 caret browsing: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_caret_browsing_off_and_keeps_everything_else() {
        let prefs = r#"{"settings":{"a11y":{"caretbrowsing":{"enabled":true,"show_dialog":false}},"other":1},"profile":{"name":"x"}}"#;
        let fixed = with_caret_browsing_off(prefs).expect("should rewrite");
        let doc: Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(
            doc.pointer(CARET_BROWSING_POINTER),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            doc.pointer("/settings/a11y/caretbrowsing/show_dialog"),
            Some(&Value::Bool(false))
        );
        assert_eq!(doc.pointer("/settings/other"), Some(&Value::from(1)));
        assert_eq!(doc.pointer("/profile/name"), Some(&Value::from("x")));
    }

    #[test]
    fn leaves_the_file_alone_when_there_is_nothing_to_fix() {
        for prefs in [
            r#"{"settings":{"a11y":{"caretbrowsing":{"enabled":false}}}}"#,
            r#"{"settings":{}}"#,
            r#"{}"#,
            "not json",
        ] {
            assert_eq!(with_caret_browsing_off(prefs), None, "{prefs}");
        }
    }

    #[test]
    fn rewrites_the_profile_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = preferences_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"settings":{"a11y":{"caretbrowsing":{"enabled":true}}}}"#,
        )
        .unwrap();

        disable_caret_browsing(dir.path());

        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            doc.pointer(CARET_BROWSING_POINTER),
            Some(&Value::Bool(false))
        );
        assert!(!path.with_extension("speakoflow-staged").exists());
    }

    #[test]
    fn a_missing_profile_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        disable_caret_browsing(dir.path());
        assert!(!preferences_path(dir.path()).exists());
    }
}
