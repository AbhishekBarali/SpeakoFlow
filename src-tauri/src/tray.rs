use crate::managers::history::{HistoryEntry, HistoryManager};
use crate::managers::model::ModelManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings;
use crate::tray_i18n::get_tray_translations;
use log::{error, info, warn};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Manager, Theme};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Debug, PartialEq)]
pub enum TrayIconState {
    Idle,
    Recording,
    Transcribing,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppTheme {
    Dark,
    Light,
    Colored, // Pink/colored theme for Linux
}

/// Gets the current app theme, with Linux defaulting to Colored theme
pub fn get_current_theme(app: &AppHandle) -> AppTheme {
    if cfg!(target_os = "linux") {
        // On Linux, always use the colored theme
        AppTheme::Colored
    } else if cfg!(target_os = "windows") {
        // The Windows taskbar and system tray (incl. the "show hidden icons"
        // flyout) are dark by default on Win11 and do NOT follow the app's
        // light/dark appearance. Pin the tray + window icons to the WHITE mark
        // (AppTheme::Dark selects the light/white assets) so the logo stays
        // visible and never flips to an invisible dark glyph when the in-app
        // appearance changes.
        AppTheme::Dark
    } else {
        // macOS: the menu-bar tray uses template mode (auto-inverts), and the
        // title bar follows the window theme — map the window theme through.
        if let Some(main_window) = app.get_webview_window("main") {
            match main_window.theme().unwrap_or(Theme::Dark) {
                Theme::Light => AppTheme::Light,
                Theme::Dark => AppTheme::Dark,
                _ => AppTheme::Dark, // Default fallback
            }
        } else {
            AppTheme::Dark
        }
    }
}

/// Gets the appropriate icon path for the given theme and state
pub fn get_icon_path(theme: AppTheme, state: TrayIconState) -> &'static str {
    match (theme, state) {
        // Dark theme uses light icons
        (AppTheme::Dark, TrayIconState::Idle) => "resources/tray_idle.png",
        (AppTheme::Dark, TrayIconState::Recording) => "resources/tray_recording.png",
        (AppTheme::Dark, TrayIconState::Transcribing) => "resources/tray_transcribing.png",
        // Light theme uses dark icons
        (AppTheme::Light, TrayIconState::Idle) => "resources/tray_idle_dark.png",
        (AppTheme::Light, TrayIconState::Recording) => "resources/tray_recording_dark.png",
        (AppTheme::Light, TrayIconState::Transcribing) => "resources/tray_transcribing_dark.png",
        // Colored theme uses pink icons (for Linux)
        (AppTheme::Colored, TrayIconState::Idle) => "resources/speakoflow.png",
        (AppTheme::Colored, TrayIconState::Recording) => "resources/recording.png",
        (AppTheme::Colored, TrayIconState::Transcribing) => "resources/transcribing.png",
    }
}

/// Window/taskbar icon for the given theme.
///
/// Unlike the tray (a monochrome template), the window icon is the full mark on
/// a transparent background, so it must be a LIGHT mark on dark title bars and a
/// DARK mark on light ones — that keeps the logo visible with no box on either
/// theme. Linux (Colored) falls back to the dark mark.
pub fn window_icon_path(theme: AppTheme) -> &'static str {
    match theme {
        AppTheme::Dark => "resources/window_icon_light.png",
        AppTheme::Light => "resources/window_icon_dark.png",
        AppTheme::Colored => "resources/window_icon_dark.png",
    }
}

/// Set the main window's taskbar / alt-tab icon to a transparent, theme-matched
/// mark. On Windows the title bar (caption) icon is then blanked separately (see
/// `blank_caption_icon_keep_taskbar`) so the top of the window stays empty while
/// the taskbar keeps the mark. Safe to call before/after theme changes; no-ops
/// if the window is gone.
pub fn update_window_icon(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let path = window_icon_path(get_current_theme(app));
    match app
        .path()
        .resolve(path, tauri::path::BaseDirectory::Resource)
    {
        Ok(resolved) => match Image::from_path(&resolved) {
            Ok(img) => {
                if let Err(e) = window.set_icon(img) {
                    warn!("Failed to set window icon: {}", e);
                }
            }
            Err(e) => warn!("Failed to load window icon {:?}: {}", resolved, e),
        },
        Err(e) => warn!("Failed to resolve window icon {}: {}", path, e),
    }

    // Windows: keep the mark in the taskbar but blank the title bar caption icon.
    // Runs after every set_icon (including on theme changes) because set_icon
    // re-sets the small (caption) icon each time.
    #[cfg(target_os = "windows")]
    blank_caption_icon_keep_taskbar(&window);
}

/// Windows only: show the mark in the taskbar / alt-tab while leaving the title
/// bar (caption) icon blank.
///
/// The underlying window layer (tao) only sets `ICON_SMALL` in `set_icon`, and
/// with no `ICON_BIG` set the taskbar falls back to that same small icon. So we
/// (1) promote the mark tao just set onto `ICON_BIG` so the taskbar keeps a
/// visible icon, then (2) overwrite `ICON_SMALL` with a fully transparent icon
/// so the caption shows nothing. A transparent icon is used instead of a NULL
/// handle on purpose: a NULL small icon makes Windows fall back to the big icon,
/// so the mark would reappear in the caption.
#[cfg(target_os = "windows")]
fn blank_caption_icon_keep_taskbar(window: &tauri::webview::WebviewWindow) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, SendMessageW};

    // Defined locally so this does not depend on the constants being re-exported
    // by the `windows` crate. ICON_SMALL = caption, ICON_BIG = taskbar/alt-tab.
    const WM_GETICON: u32 = 0x007F;
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;

    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd,
        Err(e) => {
            warn!("Failed to get main window handle for window icons: {}", e);
            return;
        }
    };

    unsafe {
        // 1) Promote the icon tao set (ICON_SMALL) onto ICON_BIG so the taskbar
        //    and alt-tab keep a visible mark of their own.
        let mark = SendMessageW(hwnd, WM_GETICON, Some(WPARAM(ICON_SMALL)), Some(LPARAM(0)));
        if mark.0 != 0 {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG)),
                Some(LPARAM(mark.0)),
            );
        }

        // 2) Blank the caption icon with a 16x16 fully transparent icon. For a
        //    monochrome icon, AND=1 + XOR=0 means "leave the background
        //    untouched" → transparent. A 16px row is 2 bytes (WORD aligned), so
        //    each mask is 16 rows * 2 bytes = 32 bytes.
        let and_mask = [0xFFu8; 32];
        let xor_mask = [0x00u8; 32];
        match CreateIcon(None, 16, 16, 1, 1, and_mask.as_ptr(), xor_mask.as_ptr()) {
            Ok(transparent) => {
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL)),
                    Some(LPARAM(transparent.0 as isize)),
                );
            }
            Err(e) => warn!("Failed to create transparent caption icon: {}", e),
        }
    }
}

pub fn change_tray_icon(app: &AppHandle, icon: TrayIconState) {
    let tray = app.state::<TrayIcon>();
    let theme = get_current_theme(app);

    let icon_path = get_icon_path(theme, icon.clone());

    // Log tray-icon failures instead of panicking (backport of Handy #1355):
    // a `.expect()` here would abort the whole app just because an icon asset
    // couldn't be resolved or decoded. Instead the tray keeps its old icon.
    if let Err(err) = load_tray_icon(
        app.path()
            .resolve(icon_path, tauri::path::BaseDirectory::Resource),
    )
    .and_then(|image| tray.set_icon(Some(image)))
    {
        error!("Failed to update tray icon '{}': {}", icon_path, err);
    }

    // Update menu based on state
    update_tray_menu(app, &icon, None);
}

/// Load a tray icon from a resolved resource path, returning any error instead
/// of panicking. Split out so it can be unit-tested and so `change_tray_icon`
/// can log-and-continue rather than abort the app (backport of Handy #1355).
fn load_tray_icon(resolved_icon_path: tauri::Result<PathBuf>) -> tauri::Result<Image<'static>> {
    let resolved_icon_path = resolved_icon_path?;
    Image::from_path(&resolved_icon_path).map(Image::to_owned)
}

pub fn tray_tooltip() -> String {
    version_label()
}

fn version_label() -> String {
    if cfg!(debug_assertions) {
        format!("SpeakoFlow v{} (Dev)", env!("CARGO_PKG_VERSION"))
    } else {
        format!("SpeakoFlow v{}", env!("CARGO_PKG_VERSION"))
    }
}

pub fn update_tray_menu(app: &AppHandle, state: &TrayIconState, locale: Option<&str>) {
    let settings = settings::get_settings(app);

    let locale = locale.unwrap_or(&settings.app_language);
    let strings = get_tray_translations(Some(locale.to_string()));

    // Platform-specific accelerators. Cmd/Ctrl+, keeps the familiar
    // "open preferences" shortcut for the main window (now labeled Home).
    #[cfg(target_os = "macos")]
    let (home_accelerator, quit_accelerator) = (Some("Cmd+,"), Some("Cmd+Q"));
    #[cfg(not(target_os = "macos"))]
    let (home_accelerator, quit_accelerator) = (Some("Ctrl+,"), Some("Ctrl+Q"));

    // Create common menu items
    let version_label = version_label();
    let version_i = MenuItem::with_id(app, "version", &version_label, false, None::<&str>)
        .expect("failed to create version item");
    // "Home" opens/focuses the main window. It replaces the old "Settings"
    // item — both pointed at the same window, so a single, clearer entry
    // avoids two tray items doing the exact same thing.
    let home_i = MenuItem::with_id(app, "home", &strings.home, true, home_accelerator)
        .expect("failed to create home item");
    let check_updates_i = MenuItem::with_id(
        app,
        "check_updates",
        &strings.check_updates,
        settings.update_checks_enabled,
        None::<&str>,
    )
    .expect("failed to create check updates item");
    let copy_last_transcript_i = MenuItem::with_id(
        app,
        "copy_last_transcript",
        &strings.copy_last_transcript,
        true,
        None::<&str>,
    )
    .expect("failed to create copy last transcript item");
    let model_loaded = app.state::<Arc<TranscriptionManager>>().is_model_loaded();
    let quit_i = MenuItem::with_id(app, "quit", &strings.quit, true, quit_accelerator)
        .expect("failed to create quit item");
    let separator = || PredefinedMenuItem::separator(app).expect("failed to create separator");

    // Build model submenu — label is the active model name
    let model_manager = app.state::<Arc<ModelManager>>();
    let models = model_manager.get_available_models();
    let current_model_id = &settings.selected_model;

    let mut downloaded: Vec<_> = models.into_iter().filter(|m| m.is_downloaded).collect();
    downloaded.sort_by(|a, b| a.name.cmp(&b.name));

    let submenu_label = downloaded
        .iter()
        .find(|m| m.id == *current_model_id)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| strings.model.clone());

    let model_submenu = {
        let submenu = Submenu::with_id(app, "model_submenu", &submenu_label, true)
            .expect("failed to create model submenu");

        for model in &downloaded {
            let is_active = model.id == *current_model_id;
            let item_id = format!("model_select:{}", model.id);
            let item =
                CheckMenuItem::with_id(app, &item_id, &model.name, true, is_active, None::<&str>)
                    .expect("failed to create model item");
            let _ = submenu.append(&item);
        }

        submenu
    };

    let unload_model_i = MenuItem::with_id(
        app,
        "unload_model",
        &strings.unload_model,
        model_loaded,
        None::<&str>,
    )
    .expect("failed to create unload model item");

    let menu = match state {
        TrayIconState::Recording | TrayIconState::Transcribing => {
            let cancel_i = MenuItem::with_id(app, "cancel", &strings.cancel, true, None::<&str>)
                .expect("failed to create cancel item");
            Menu::with_items(
                app,
                &[
                    &version_i,
                    &separator(),
                    &cancel_i,
                    &separator(),
                    &copy_last_transcript_i,
                    &separator(),
                    &home_i,
                    &check_updates_i,
                    &separator(),
                    &quit_i,
                ],
            )
            .expect("failed to create menu")
        }
        TrayIconState::Idle => Menu::with_items(
            app,
            &[
                &version_i,
                &separator(),
                &copy_last_transcript_i,
                &separator(),
                &model_submenu,
                &unload_model_i,
                &separator(),
                &home_i,
                &check_updates_i,
                &separator(),
                &quit_i,
            ],
        )
        .expect("failed to create menu"),
    };

    let tray = app.state::<TrayIcon>();
    let _ = tray.set_menu(Some(menu));
    let _ = tray.set_icon_as_template(true);
    let _ = tray.set_tooltip(Some(version_label));
}

fn last_transcript_text(entry: &HistoryEntry) -> &str {
    entry
        .post_processed_text
        .as_deref()
        .unwrap_or(&entry.transcription_text)
}

pub fn set_tray_visibility(app: &AppHandle, visible: bool) {
    let tray = app.state::<TrayIcon>();
    if let Err(e) = tray.set_visible(visible) {
        error!("Failed to set tray visibility: {}", e);
    } else {
        info!("Tray visibility set to: {}", visible);
    }
}

pub fn copy_last_transcript(app: &AppHandle) {
    let history_manager = app.state::<Arc<HistoryManager>>();
    let entry = match history_manager.get_latest_completed_entry() {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            warn!("No completed transcription history entries available for tray copy.");
            return;
        }
        Err(err) => {
            error!(
                "Failed to fetch last completed transcription entry: {}",
                err
            );
            return;
        }
    };

    let text = last_transcript_text(&entry);
    if text.trim().is_empty() {
        warn!("Last completed transcription is empty; skipping tray copy.");
        return;
    }

    if let Err(err) = app.clipboard().write_text(text) {
        error!("Failed to copy last transcript to clipboard: {}", err);
        return;
    }

    info!("Copied last transcript to clipboard via tray.");
}

#[cfg(test)]
mod tests {
    use super::{last_transcript_text, load_tray_icon};
    use crate::managers::history::HistoryEntry;

    fn build_entry(transcription: &str, post_processed: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id: 1,
            file_name: "speakoflow-1.wav".to_string(),
            timestamp: 0,
            saved: false,
            title: "Recording".to_string(),
            transcription_text: transcription.to_string(),
            post_processed_text: post_processed.map(|text| text.to_string()),
            post_process_prompt: None,
            post_process_requested: false,
        }
    }

    #[test]
    fn uses_post_processed_text_when_available() {
        let entry = build_entry("raw", Some("processed"));
        assert_eq!(last_transcript_text(&entry), "processed");
    }

    #[test]
    fn falls_back_to_raw_transcription() {
        let entry = build_entry("raw", None);
        assert_eq!(last_transcript_text(&entry), "raw");
    }

    #[test]
    fn tray_icon_returns_err_when_file_does_not_exist() {
        // A missing/undecodable icon must surface as an Err (which
        // change_tray_icon logs), never a panic that would abort the app.
        let missing = std::path::PathBuf::from("speakoflow_nonexistent_tray_icon_xyz.png");
        assert!(load_tray_icon(Ok(missing)).is_err());
    }
}
