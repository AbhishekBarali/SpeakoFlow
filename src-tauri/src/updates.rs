//! What the updater plugin does not decide for us: whether *this* install can
//! replace itself in place, and the manual fallback when it cannot.
//!
//! The plugin will happily attempt an install that is wrong for the machine.
//! On Linux an install with no recorded bundle type (a from-source build, the
//! AUR package) falls through to its AppImage path and overwrites the running
//! binary with an AppImage; a `.deb` is installed with `dpkg -i` even on a
//! distribution where dpkg does not own the system. So the app asks
//! [`resolve_install_mode`] first, and offers "Update now" only where the
//! in-place install is the right thing, and "Download installer" everywhere
//! else.
//!
//! The download fallback reads the `speakoflow.downloads` block that
//! `scripts/ci/updater-manifest.ts` writes into `latest.json` (the updater
//! ignores fields it does not know), so the app never has to guess an asset
//! name from a version number.

use futures_util::StreamExt;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::utils::config::BundleType;
use tauri::utils::platform::bundle_type;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

/// Must match `plugins.updater.endpoints[0]` in tauri.conf.json (tested).
pub const UPDATE_MANIFEST_URL: &str =
    "https://github.com/AbhishekBarali/SpeakoFlow/releases/latest/download/latest.json";

/// The only place a downloaded installer may come from.
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/";

pub const RELEASES_PAGE_URL: &str = "https://github.com/AbhishekBarali/SpeakoFlow/releases/latest";

/// Event carrying installer-download progress to the webview.
pub const INSTALLER_PROGRESS_EVENT: &str = "update-installer-progress";

/// How this particular install gets a new version.
#[derive(Serialize, Deserialize, Type, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateInstallMode {
    /// The updater plugin can replace this install where it is.
    InApp,
    /// No safe in-place path, but the full installer will work: download it
    /// and run it (a dev build, an extracted AppImage, a Debian-family system
    /// with no way to ask for the admin password).
    Download,
    /// The Windows portable build. Its installer would install a second,
    /// non-portable copy, so the honest answer is the release page.
    Portable,
    /// Something else owns these files: the AUR, a from-source install, an
    /// .rpm. Updating behind its back would desynchronise it.
    PackageManager,
}

#[derive(Serialize, Type, Clone, Debug)]
pub struct UpdateSupport {
    pub mode: UpdateInstallMode,
    pub release_page: String,
}

/// The inputs are parameters, not lookups, so every branch is testable on any
/// host.
pub fn resolve_install_mode(
    os: &str,
    bundle: Option<BundleType>,
    portable: bool,
    running_as_appimage: bool,
    debian_family: bool,
    can_elevate: bool,
) -> UpdateInstallMode {
    if portable {
        return UpdateInstallMode::Portable;
    }
    match os {
        "windows" => match bundle {
            Some(BundleType::Nsis) | Some(BundleType::Msi) => UpdateInstallMode::InApp,
            _ => UpdateInstallMode::Download,
        },
        "macos" => UpdateInstallMode::InApp,
        "linux" => match bundle {
            Some(BundleType::AppImage) if running_as_appimage => UpdateInstallMode::InApp,
            // Extracted, or copied out of its AppImage: nothing to replace in place.
            Some(BundleType::AppImage) => UpdateInstallMode::Download,
            Some(BundleType::Deb) if debian_family && can_elevate => UpdateInstallMode::InApp,
            Some(BundleType::Deb) if debian_family => UpdateInstallMode::Download,
            // A .deb running on a non-Debian system was repackaged by someone
            // (the AUR does this); dpkg -i there would corrupt nothing useful
            // and update nothing the package manager knows about.
            Some(BundleType::Deb) => UpdateInstallMode::PackageManager,
            _ => UpdateInstallMode::PackageManager,
        },
        _ => UpdateInstallMode::Download,
    }
}

/// `ID` / `ID_LIKE` from /etc/os-release name Debian or Ubuntu.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn os_release_is_debian_family(os_release: &str) -> bool {
    os_release.lines().any(|line| {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        if key.trim() != "ID" && key.trim() != "ID_LIKE" {
            return false;
        }
        value
            .trim()
            .trim_matches('"')
            .split_whitespace()
            .any(|id| id == "debian" || id == "ubuntu")
    })
}

#[cfg(target_os = "linux")]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

fn current_install_mode() -> UpdateInstallMode {
    let bundle = bundle_type();
    let portable = crate::portable::is_portable();

    #[cfg(target_os = "linux")]
    let (appimage, debian, elevate) = (
        std::env::var_os("APPIMAGE").is_some(),
        std::fs::read_to_string("/etc/os-release")
            .map(|text| os_release_is_debian_family(&text))
            .unwrap_or(false),
        // What tauri-plugin-updater tries, in order, to run dpkg as root.
        on_path("pkexec") || on_path("zenity") || on_path("kdialog"),
    );
    #[cfg(not(target_os = "linux"))]
    let (appimage, debian, elevate) = (false, false, false);

    resolve_install_mode(
        std::env::consts::OS,
        bundle,
        portable,
        appimage,
        debian,
        elevate,
    )
}

#[tauri::command]
#[specta::specta]
pub fn get_update_support() -> UpdateSupport {
    let mode = current_install_mode();
    info!(
        "Update support: {:?} (bundle {:?}, portable {})",
        mode,
        bundle_type(),
        crate::portable::is_portable()
    );
    UpdateSupport {
        mode,
        release_page: RELEASES_PAGE_URL.to_string(),
    }
}

// ── Manual download ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ManifestExtras {
    #[serde(default)]
    downloads: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    speakoflow: Option<ManifestExtras>,
}

fn bundle_download_suffix(bundle: Option<BundleType>) -> Option<&'static str> {
    Some(match bundle? {
        BundleType::Nsis => "nsis",
        BundleType::Msi => "msi",
        BundleType::Deb => "deb",
        BundleType::Rpm => "rpm",
        BundleType::AppImage => "appimage",
        BundleType::App | BundleType::Dmg => "dmg",
    })
}

/// Same key scheme as the updater: the installer matching how this copy was
/// installed first, then the platform default.
pub fn pick_installer_url<'a>(
    downloads: &'a HashMap<String, String>,
    os: &str,
    arch: &str,
    bundle: Option<BundleType>,
) -> Option<&'a str> {
    let os = if os == "macos" { "darwin" } else { os };
    let mut keys = Vec::with_capacity(2);
    if let Some(suffix) = bundle_download_suffix(bundle) {
        keys.push(format!("{os}-{arch}-{suffix}"));
    }
    keys.push(format!("{os}-{arch}"));
    keys.iter()
        .find_map(|key| downloads.get(key))
        .map(String::as_str)
        .filter(|url| is_trusted_download(url))
}

/// Only this repository's release files, and only a plain file name, so the
/// manifest can never direct a download somewhere else or out of Downloads.
pub fn is_trusted_download(url: &str) -> bool {
    let Some(rest) = url.strip_prefix(RELEASE_DOWNLOAD_PREFIX) else {
        return false;
    };
    let mut parts = rest.split('/');
    let (Some(tag), Some(file), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !tag.is_empty() && installer_file_name(file).is_some()
}

/// The file name to save under, if it is one we would ever publish.
pub fn installer_file_name(last_segment: &str) -> Option<String> {
    let name = last_segment.split(['?', '#']).next().unwrap_or("");
    let name = percent_decode(name)?;
    let allowed = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    let known = [".exe", ".msi", ".dmg", ".AppImage", ".deb", ".rpm"]
        .iter()
        .any(|ext| name.ends_with(ext));
    (allowed && known && name.starts_with("SpeakoFlow") && !name.contains(".."))
        .then(|| name.to_string())
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = value.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[derive(Serialize, Type, Clone, Debug)]
pub struct InstallerProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Serialize, Type, Clone, Debug)]
pub struct DownloadedInstaller {
    pub path: String,
    pub version: String,
}

fn http_client(timeout: Option<Duration>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("SpeakoFlow/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20));
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Download the full installer for this machine into the Downloads folder.
///
/// Written to a `.part` file and renamed on completion, so a cancelled or
/// failed download never leaves something that looks like a finished
/// installer.
#[tauri::command]
#[specta::specta]
pub async fn download_update_installer(app: AppHandle) -> Result<DownloadedInstaller, String> {
    let client = http_client(Some(Duration::from_secs(30)))?;
    let manifest: Manifest = client
        .get(UPDATE_MANIFEST_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Couldn't reach the update server: {e}"))?
        .json()
        .await
        .map_err(|e| format!("The update information was unreadable: {e}"))?;

    let downloads = manifest.speakoflow.map(|x| x.downloads).unwrap_or_default();
    let url = pick_installer_url(
        &downloads,
        std::env::consts::OS,
        std::env::consts::ARCH,
        bundle_type(),
    )
    .ok_or_else(|| "There's no installer for this system in the latest release.".to_string())?
    .to_string();
    let file_name = url
        .rsplit('/')
        .next()
        .and_then(installer_file_name)
        .ok_or_else(|| "The installer link was not valid.".to_string())?;

    let dir = app
        .path()
        .download_dir()
        .map_err(|e| format!("Couldn't find the Downloads folder: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(&file_name);
    let partial = dir.join(format!("{file_name}.part"));

    info!("Downloading installer {url} → {}", target.display());
    // No overall timeout: installers are large and connections slow. The
    // connect timeout and the per-chunk stall timeout below bound the wait.
    let response = http_client(None)?
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("The download didn't start: {e}"))?;
    let total = response.content_length();

    let result = async {
        // Plain std I/O: chunks are small and the disk is local, and it keeps
        // tokio's `fs` feature out of the build for one file.
        let mut file = std::fs::File::create(&partial)
            .map_err(|e| format!("Couldn't write to Downloads: {e}"))?;
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        let mut last_emit = 0u64;
        loop {
            let next = tokio::time::timeout(Duration::from_secs(60), stream.next())
                .await
                .map_err(|_| "The download stalled.".to_string())?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| format!("The download was interrupted: {e}"))?;
            file.write_all(&chunk)
                .map_err(|e| format!("Couldn't write to Downloads: {e}"))?;
            downloaded += chunk.len() as u64;
            if downloaded - last_emit >= 512 * 1024 || Some(downloaded) == total {
                last_emit = downloaded;
                let _ = app.emit(
                    INSTALLER_PROGRESS_EVENT,
                    InstallerProgress { downloaded, total },
                );
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        if let Some(total) = total {
            if downloaded != total {
                return Err("The download ended early.".to_string());
            }
        }
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = result {
        let _ = std::fs::remove_file(&partial);
        warn!("Installer download failed: {error}");
        return Err(error);
    }

    let _ = std::fs::remove_file(&target);
    std::fs::rename(&partial, &target).map_err(|e| format!("Couldn't save the installer: {e}"))?;

    Ok(DownloadedInstaller {
        path: target.to_string_lossy().into_owned(),
        version: manifest.version,
    })
}

/// A path the app itself saved into Downloads — nothing else may be opened.
fn validate_installer_path(app: &AppHandle, path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(installer_file_name)
        .ok_or_else(|| "Not an installer this app downloaded.".to_string())?;
    let expected = dir.join(name);
    if !same_file_path(&path, &expected) || !expected.is_file() {
        return Err("Not an installer this app downloaded.".to_string());
    }
    Ok(expected)
}

fn same_file_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Start the installer (Windows .exe/.msi, macOS .dmg, Linux .deb opens in
/// the software centre). An AppImage is made executable and shown in its
/// folder, since "opening" it is not something every desktop does.
#[tauri::command]
#[specta::specta]
pub fn open_update_installer(app: AppHandle, path: String) -> Result<(), String> {
    let path = validate_installer_path(&app, &path)?;

    if path.extension().and_then(|e| e.to_str()) == Some("AppImage") {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&path, perms).map_err(|e| e.to_string())?;
        }
        return app
            .opener()
            .reveal_item_in_dir(&path)
            .map_err(|e| e.to_string());
    }

    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn reveal_update_installer(app: AppHandle, path: String) -> Result<(), String> {
    let path = validate_installer_path(&app, &path)?;
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())
}

// ── Coming back after an in-app update ────────────────────────────────────
//
// On Windows the app exits so the installer can replace it, and the installer
// starts the new version. Windows does not let a process that was started in
// the background take the foreground, so that new window opened *behind*
// whatever the person was looking at: they clicked Update, the app vanished,
// and as far as they could tell it had crashed. The old version leaves a note
// before it exits, and the new one reads it at launch: it comes to the front
// even if it would normally start hidden, and says it was updated.

const PENDING_UPDATE_FILE: &str = "update-pending.json";

/// A note older than this is from an install that never finished.
const PENDING_UPDATE_MAX_AGE_SECS: u64 = 15 * 60;

#[derive(Serialize, Deserialize, Type, Clone, Debug, PartialEq, Eq)]
pub struct FinishedUpdate {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Deserialize)]
struct PendingUpdate {
    from: String,
    to: String,
    /// Seconds since the Unix epoch.
    at: u64,
}

static FINISHED_UPDATE: std::sync::Mutex<Option<FinishedUpdate>> = std::sync::Mutex::new(None);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn pending_update_path(app: &AppHandle) -> Option<PathBuf> {
    crate::portable::resolve_app_data(app, PENDING_UPDATE_FILE).ok()
}

/// Whether a note left by the previous version describes the version now
/// running. Pure, so the decision is testable without a real install.
fn finished_update_from_note(
    note: &str,
    running_version: &str,
    now: u64,
) -> Option<FinishedUpdate> {
    let note: PendingUpdate = serde_json::from_str(note).ok()?;
    let fresh = now.saturating_sub(note.at) <= PENDING_UPDATE_MAX_AGE_SECS;
    (fresh && note.to == running_version && note.from != note.to).then_some(FinishedUpdate {
        from: note.from,
        to: note.to,
    })
}

/// Called by the frontend right before it hands over to the installer.
#[tauri::command]
#[specta::specta]
pub fn prepare_update_install(app: AppHandle, version: String) -> Result<(), String> {
    let path = pending_update_path(&app).ok_or("No app data folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let note = PendingUpdate {
        from: app.package_info().version.to_string(),
        to: version,
        at: now_secs(),
    };
    std::fs::write(&path, serde_json::to_vec(&note).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Read (and delete) the previous version's note at launch. Returns true when
/// this launch is the one that finishes an update.
pub fn take_finished_update(app: &AppHandle) -> bool {
    let Some(path) = pending_update_path(app) else {
        return false;
    };
    let Ok(note) = std::fs::read_to_string(&path) else {
        return false;
    };
    let _ = std::fs::remove_file(&path);
    let running = app.package_info().version.to_string();
    match finished_update_from_note(&note, &running, now_secs()) {
        Some(finished) => {
            info!("Updated from {} to {}", finished.from, finished.to);
            *FINISHED_UPDATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(finished);
            true
        }
        None => {
            warn!("Ignoring an update note that does not match this launch ({running}): {note}");
            false
        }
    }
}

/// Put the main window in front of everything once, without keeping it there.
/// `set_focus` alone is refused by Windows for a process the installer started.
pub fn bring_main_window_forward(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let _ = window.set_always_on_top(false);
    });
}

/// The update this launch finished, once, for the "Updated to …" notice.
#[tauri::command]
#[specta::specta]
pub fn take_update_notice() -> Option<FinishedUpdate> {
    FINISHED_UPDATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_url_matches_the_updater_config() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            conf["plugins"]["updater"]["endpoints"][0].as_str(),
            Some(UPDATE_MANIFEST_URL)
        );
    }

    #[test]
    fn the_app_does_not_trust_upstream_handys_key() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let pubkey = conf["plugins"]["updater"]["pubkey"].as_str().unwrap();
        // Handy's key (id BAB72095206601F9). We never had its private half, so
        // an app that trusts it can never accept an update we publish, which
        // is exactly the state every release from 1.0 to 1.4 shipped in.
        const HANDY_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEJBQjcyMDk1MjA2NjAxRjkKUldUNUFXWWdsU0MzdXRRZi8zYzhqV2FaNUVDbDd2Rk5VM1IvWWowVXdmRFNKQ1BrMXF5RFFsLy8K";
        assert_ne!(pubkey, HANDY_PUBKEY);
    }

    use UpdateInstallMode::*;

    #[test]
    fn windows_installs_update_in_place_and_portable_does_not() {
        let m = |b, p| resolve_install_mode("windows", b, p, false, false, false);
        assert_eq!(m(Some(BundleType::Nsis), false), InApp);
        assert_eq!(m(Some(BundleType::Msi), false), InApp);
        assert_eq!(m(Some(BundleType::Nsis), true), Portable);
        // A dev build or an exe copied out of an install.
        assert_eq!(m(None, false), Download);
    }

    #[test]
    fn macos_updates_in_place() {
        assert_eq!(
            resolve_install_mode("macos", Some(BundleType::App), false, false, false, false),
            InApp
        );
    }

    #[test]
    fn linux_only_updates_what_it_can_actually_replace() {
        let m = |b, appimage, debian, elevate| {
            resolve_install_mode("linux", b, false, appimage, debian, elevate)
        };
        assert_eq!(m(Some(BundleType::AppImage), true, false, false), InApp);
        assert_eq!(m(Some(BundleType::AppImage), false, false, false), Download);
        assert_eq!(m(Some(BundleType::Deb), false, true, true), InApp);
        assert_eq!(m(Some(BundleType::Deb), false, true, false), Download);
        // The AUR repackages the .deb; dpkg must never run on Arch.
        assert_eq!(m(Some(BundleType::Deb), false, false, true), PackageManager);
        assert_eq!(m(Some(BundleType::Rpm), false, false, true), PackageManager);
        // From source: the updater would overwrite the binary with an AppImage.
        assert_eq!(m(None, false, true, true), PackageManager);
    }

    #[test]
    fn debian_family_is_read_from_os_release() {
        assert!(os_release_is_debian_family("ID=ubuntu\nID_LIKE=debian\n"));
        assert!(os_release_is_debian_family(
            "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"
        ));
        assert!(os_release_is_debian_family("ID=debian\n"));
        assert!(!os_release_is_debian_family("ID=arch\n"));
        assert!(!os_release_is_debian_family(
            "ID=fedora\nID_LIKE=\"rhel centos\"\n"
        ));
        assert!(!os_release_is_debian_family(
            "NAME=\"Debian GNU/Linux\"\nID=cachyos\n"
        ));
    }

    fn downloads() -> HashMap<String, String> {
        let url = |name: &str| format!("{RELEASE_DOWNLOAD_PREFIX}v1.5.0/{name}");
        [
            ("windows-x86_64", url("SpeakoFlow_1.5.0_x64-setup.exe")),
            ("windows-x86_64-nsis", url("SpeakoFlow_1.5.0_x64-setup.exe")),
            ("windows-x86_64-msi", url("SpeakoFlow_1.5.0_x64_en-US.msi")),
            ("darwin-aarch64", url("SpeakoFlow_1.5.0_aarch64.dmg")),
            ("darwin-aarch64-dmg", url("SpeakoFlow_1.5.0_aarch64.dmg")),
            ("linux-x86_64", url("SpeakoFlow_1.5.0_amd64.AppImage")),
            ("linux-x86_64-deb", url("SpeakoFlow_1.5.0_amd64.deb")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    #[test]
    fn the_installer_matches_how_this_copy_was_installed() {
        let d = downloads();
        let pick = |os, arch, b| pick_installer_url(&d, os, arch, b).unwrap();
        assert!(pick("windows", "x86_64", Some(BundleType::Msi)).ends_with(".msi"));
        assert!(pick("windows", "x86_64", Some(BundleType::Nsis)).ends_with("-setup.exe"));
        assert!(pick("windows", "x86_64", None).ends_with("-setup.exe"));
        assert!(pick("macos", "aarch64", Some(BundleType::App)).ends_with("aarch64.dmg"));
        assert!(pick("linux", "x86_64", Some(BundleType::Deb)).ends_with(".deb"));
        assert!(pick("linux", "x86_64", None).ends_with(".AppImage"));
        assert!(pick_installer_url(&d, "linux", "aarch64", None).is_none());
    }

    #[test]
    fn downloads_are_confined_to_this_repositorys_releases() {
        let ok = format!("{RELEASE_DOWNLOAD_PREFIX}v1.5.0/SpeakoFlow_1.5.0_x64-setup.exe");
        assert!(is_trusted_download(&ok));
        for bad in [
            "https://example.com/SpeakoFlow_1.5.0_x64-setup.exe",
            "https://github.com/someone/SpeakoFlow/releases/download/v1.5.0/SpeakoFlow_1.5.0_x64-setup.exe",
            &format!("{RELEASE_DOWNLOAD_PREFIX}v1.5.0/evil.exe"),
            &format!("{RELEASE_DOWNLOAD_PREFIX}v1.5.0/SpeakoFlow_1.5.0.zip"),
            &format!("{RELEASE_DOWNLOAD_PREFIX}v1.5.0/a/SpeakoFlow_1.5.0_x64-setup.exe"),
            &format!("{RELEASE_DOWNLOAD_PREFIX}v1.5.0/SpeakoFlow%2F..%2F..%2Fx.exe"),
            &format!("{RELEASE_DOWNLOAD_PREFIX}/SpeakoFlow_1.5.0_x64-setup.exe"),
        ] {
            assert!(!is_trusted_download(bad), "{bad}");
        }
    }

    #[test]
    fn a_launch_finishes_an_update_only_if_it_is_the_version_that_was_installed() {
        let note = |from: &str, to: &str, at: u64| {
            serde_json::to_string(&PendingUpdate {
                from: from.into(),
                to: to.into(),
                at,
            })
            .unwrap()
        };
        let now = 1_000_000;
        assert_eq!(
            finished_update_from_note(&note("1.5.0", "1.5.1", now - 30), "1.5.1", now),
            Some(FinishedUpdate {
                from: "1.5.0".into(),
                to: "1.5.1".into()
            })
        );
        // The install failed and the old version started again.
        assert_eq!(
            finished_update_from_note(&note("1.5.0", "1.5.1", now - 30), "1.5.0", now),
            None
        );
        // A note left behind by an install that never finished, long ago.
        assert_eq!(
            finished_update_from_note(&note("1.5.0", "1.5.1", now - 3600), "1.5.1", now),
            None
        );
        assert_eq!(finished_update_from_note("garbage", "1.5.1", now), None);
    }

    #[test]
    fn saved_file_names_are_plain_and_known() {
        assert_eq!(
            installer_file_name("SpeakoFlow_1.5.0_x64-setup.exe").as_deref(),
            Some("SpeakoFlow_1.5.0_x64-setup.exe")
        );
        assert_eq!(installer_file_name("SpeakoFlow..exe"), None);
        assert_eq!(installer_file_name("SpeakoFlow_1.5.0.exe.part"), None);
        assert_eq!(installer_file_name("SpeakoFlow%20x.exe"), None);
        assert_eq!(installer_file_name("SpeakoFlow_%zz.exe"), None);
    }
}
