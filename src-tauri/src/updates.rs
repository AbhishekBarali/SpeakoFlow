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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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

// ── Fetching a release asset on several connections ───────────────────────
//
// GitHub throttles release-asset downloads per connection, not per client.
// Measured from Kathmandu against the 38 MB 2.0.1 Windows installer, on a line
// that pulled ~97 Mbit/s from Cloudflare at the same moment: one connection
// ran at 1.6–2.2 Mbit/s, so the updater plugin, which downloads on one
// connection, took three to five minutes for a 38 MB update. The app now
// fetches the asset itself with ranged requests, checks the signature, and
// hands the plugin only the verified bytes to install.
//
// Anything that stops the parallel path (a host that ignores `Range`, a chunk
// that keeps failing) falls back to one stream rather than failing the
// update, so this can only ever be faster than what it replaced.

/// `live_parallel_sweep`, two interleaved rounds on the same link: 8 × 1 MiB
/// gave 7.1 / 10.9 Mbit/s, 8 × 5 MiB 8.4 / 9.1, 16 × 2 MiB 12.8 / 9.7, and
/// 16 × 3 MiB 12.4 / 11.3 — the best floor, and five to six times one
/// connection. GitHub's own variance between runs is as large as the gaps
/// between settings, so take the ordering as a tendency, not a law.
const ASSET_CONNECTIONS: usize = 16;

/// Small enough that the last chunks don't leave most connections idle while
/// one finishes alone, large enough that the per-request round trip is noise.
const ASSET_CHUNK_BYTES: u64 = 3 * 1024 * 1024;

/// Below this one connection finishes before parallelism pays for its probe.
const ASSET_PARALLEL_MIN_BYTES: u64 = 4 * 1024 * 1024;

/// A connection that sends nothing for this long is abandoned and retried.
const ASSET_STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Tries per chunk before the whole parallel attempt gives way to one stream.
const ASSET_CHUNK_ATTEMPTS: u32 = 3;

/// Progress is reported in steps of this, not once per network packet.
const ASSET_PROGRESS_STEP: u64 = 256 * 1024;

/// Inclusive byte ranges covering `total` bytes in `chunk`-sized pieces.
fn chunk_ranges(total: u64, chunk: u64) -> Vec<(u64, u64)> {
    (0..total.div_ceil(chunk))
        .map(|i| {
            let start = i * chunk;
            (start, (start + chunk).min(total) - 1)
        })
        .collect()
}

/// The total size out of a `Content-Range` header (`bytes 0-0/39951234`).
fn total_from_content_range(header: &str) -> Option<u64> {
    let total = header.rsplit('/').next()?.trim().parse::<u64>().ok()?;
    (total > 0).then_some(total)
}

/// Whether a `Content-Range` header names exactly `start..=end`. A cache or
/// proxy can answer a ranged request with `206` and another range of the same
/// length; the in-app install would refuse those bytes at the signature check,
/// but the installer saved to Downloads has no such check.
fn content_range_is(header: &str, start: u64, end: u64) -> bool {
    header
        .trim()
        .strip_prefix("bytes ")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|range| range.trim() == format!("{start}-{end}"))
}

/// Where the asset actually lives and how large it is, but only when the
/// host answers byte ranges with `206`.
///
/// Asks for one byte rather than sending `HEAD`, because a host that ignores
/// `Range` would otherwise hand every connection the whole file. The final
/// URL is kept so the chunks skip GitHub's redirect: `releases/download/...`
/// answers with a 302 to a signed `release-assets.githubusercontent.com` URL
/// valid for an hour, far longer than any download takes.
async fn probe_ranged_asset(client: &reqwest::Client, url: &str) -> Option<(reqwest::Url, u64)> {
    let request = client.get(url).header(reqwest::header::RANGE, "bytes=0-0");
    let response = tokio::time::timeout(ASSET_STALL_TIMEOUT, request.send())
        .await
        .ok()?
        .ok()?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return None;
    }
    let total = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()
        .and_then(total_from_content_range)?;
    Some((response.url().clone(), total))
}

/// One inclusive byte range. Bytes are credited to `downloaded` as they
/// arrive (so progress moves smoothly) and handed back if the chunk fails.
async fn fetch_chunk(
    client: &reqwest::Client,
    url: &reqwest::Url,
    (start, end): (u64, u64),
    downloaded: &AtomicU64,
    report: &(dyn Fn(u64) + Sync),
) -> Result<Vec<u8>, String> {
    let expected = (end - start + 1) as usize;
    let request = client
        .get(url.clone())
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
    let response = tokio::time::timeout(ASSET_STALL_TIMEOUT, request.send())
        .await
        .map_err(|_| "no response".to_string())?
        .map_err(|e| e.to_string())?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("HTTP {}", response.status()));
    }
    let range_matches = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| content_range_is(value, start, end));
    if !range_matches {
        return Err("the server answered a different range".to_string());
    }

    let mut buffer = Vec::with_capacity(expected);
    let mut credited = 0u64;
    let mut stream = response.bytes_stream();
    let result = loop {
        match tokio::time::timeout(ASSET_STALL_TIMEOUT, stream.next()).await {
            Err(_) => break Err("stalled".to_string()),
            Ok(None) => break Ok(()),
            Ok(Some(Err(e))) => break Err(e.to_string()),
            Ok(Some(Ok(part))) => {
                buffer.extend_from_slice(&part);
                credited += part.len() as u64;
                report(
                    downloaded.fetch_add(part.len() as u64, Ordering::Relaxed) + part.len() as u64,
                );
            }
        }
    };
    let result = result.and_then(|()| {
        if buffer.len() == expected {
            Ok(())
        } else {
            Err(format!("got {} of {expected} bytes", buffer.len()))
        }
    });
    if let Err(error) = result {
        downloaded.fetch_sub(credited, Ordering::Relaxed);
        return Err(error);
    }
    Ok(buffer)
}

/// Download `url` on [`ASSET_CONNECTIONS`] connections into memory.
///
/// `None` means "use one stream instead": the host doesn't support ranges,
/// the file is too small to bother, or a chunk failed every attempt. The
/// failure is logged here, so callers only have to fall back.
/// `on_progress(downloaded, total)` is called in [`ASSET_PROGRESS_STEP`]s.
async fn fetch_asset_parallel(
    client: &reqwest::Client,
    url: &str,
    on_progress: &(dyn Fn(u64, u64) + Sync),
) -> Option<Vec<u8>> {
    fetch_asset_parallel_with(
        client,
        url,
        on_progress,
        ASSET_CONNECTIONS,
        ASSET_CHUNK_BYTES,
    )
    .await
}

/// [`fetch_asset_parallel`] with the connection count and chunk size as
/// parameters, so the live test can compare settings on the same code.
async fn fetch_asset_parallel_with(
    client: &reqwest::Client,
    url: &str,
    on_progress: &(dyn Fn(u64, u64) + Sync),
    connections: usize,
    chunk_bytes: u64,
) -> Option<Vec<u8>> {
    let Some((final_url, total)) = probe_ranged_asset(client, url).await else {
        info!("{url} doesn't answer byte ranges; downloading on one connection");
        return None;
    };
    if total < ASSET_PARALLEL_MIN_BYTES {
        return None;
    }
    let Ok(size) = usize::try_from(total) else {
        return None;
    };
    // Retries go through the published URL rather than the one it redirected
    // to. GitHub's signed asset URL carries a token that expires 30 minutes
    // after it is issued, so on a slow enough link every chunk after that
    // would be refused, and the whole download would start over on one
    // connection. Going through the redirect again hands out a fresh token.
    let Ok(origin_url) = reqwest::Url::parse(url) else {
        return None;
    };

    let started = std::time::Instant::now();
    let ranges = chunk_ranges(total, chunk_bytes);
    let buffer = std::sync::Mutex::new(vec![0u8; size]);
    let next = AtomicUsize::new(0);
    let downloaded = AtomicU64::new(0);
    let reported = AtomicU64::new(0);
    let failed = AtomicBool::new(false);

    let report = |now: u64| {
        let last = reported.load(Ordering::Relaxed);
        if now >= last + ASSET_PROGRESS_STEP || now == total {
            reported.store(now, Ordering::Relaxed);
            on_progress(now, total);
        }
    };
    on_progress(0, total);

    // References, so each worker's `async move` copies them instead of
    // moving the values out of the shared closure.
    let (ranges, shared, next, downloaded, failed, final_url, origin_url) = (
        &ranges,
        &buffer,
        &next,
        &downloaded,
        &failed,
        &final_url,
        &origin_url,
    );
    let report: &(dyn Fn(u64) + Sync) = &report;

    let worker = move || async move {
        loop {
            if failed.load(Ordering::Relaxed) {
                return Ok(());
            }
            let Some(&range) = ranges.get(next.fetch_add(1, Ordering::Relaxed)) else {
                return Ok(());
            };
            let mut last_error = String::new();
            let mut bytes = None;
            for attempt in 1..=ASSET_CHUNK_ATTEMPTS {
                let from = if attempt == 1 { final_url } else { origin_url };
                match fetch_chunk(client, from, range, downloaded, report).await {
                    Ok(chunk) => {
                        bytes = Some(chunk);
                        break;
                    }
                    Err(error) => {
                        warn!(
                            "Update chunk {}-{} failed (try {attempt}/{ASSET_CHUNK_ATTEMPTS}): {error}",
                            range.0, range.1
                        );
                        last_error = error;
                        if attempt < ASSET_CHUNK_ATTEMPTS {
                            tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt)))
                                .await;
                        }
                    }
                }
            }
            let Some(bytes) = bytes else {
                failed.store(true, Ordering::Relaxed);
                return Err(format!("bytes {}-{}: {last_error}", range.0, range.1));
            };
            shared.lock().unwrap_or_else(|e| e.into_inner())[range.0 as usize..=range.1 as usize]
                .copy_from_slice(&bytes);
        }
    };

    // One task polling every connection is plenty for I/O this size, and it
    // lets the workers borrow instead of needing `'static` state.
    let results =
        futures_util::future::join_all((0..connections.min(ranges.len())).map(|_| worker())).await;
    if let Some(error) = results.into_iter().find_map(Result::err) {
        warn!("Parallel download of {url} failed, falling back to one connection: {error}");
        return None;
    }

    let secs = started.elapsed().as_secs_f64().max(0.001);
    info!(
        "Downloaded {url}: {:.1} MB in {secs:.1}s ({:.1} Mbit/s, {} connections)",
        total as f64 / 1e6,
        total as f64 * 8.0 / 1e6 / secs,
        connections.min(ranges.len()),
    );
    on_progress(total, total);
    Some(buffer.into_inner().unwrap_or_else(|e| e.into_inner()))
}

/// Check `data` against the release's minisign signature with the key this
/// build trusts — the same check the plugin's own `download()` makes, which
/// a download the plugin didn't do would otherwise skip.
fn verify_update_signature(
    data: &[u8],
    signature_b64: &str,
    pubkey_b64: &str,
) -> Result<(), String> {
    use base64::Engine;
    let decode = |value: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    };
    let key = decode(pubkey_b64)
        .and_then(|text| minisign_verify::PublicKey::decode(&text).ok())
        .ok_or("This build's update key is unreadable.")?;
    let signature = decode(signature_b64)
        .and_then(|text| minisign_verify::Signature::decode(&text).ok())
        .ok_or("The update's signature is unreadable.")?;
    key.verify(data, &signature, true).map_err(|e| {
        format!("The update didn't match its signature ({e}), so it wasn't installed.")
    })
}

/// The parallel download, if it passes the signature check the plugin
/// makes; `None` sends the caller to the plugin's own single-stream download,
/// which checks the signature itself. So a range a proxy mangled, or bytes
/// that went wrong in any other way, cost a second download rather than the
/// update.
fn keep_if_verified(bytes: Vec<u8>, signature: &str, pubkey: Option<&str>) -> Option<Vec<u8>> {
    let checked = match pubkey {
        Some(key) => verify_update_signature(&bytes, signature, key),
        None => Err("this build has no update key".to_string()),
    };
    match checked {
        Ok(()) => Some(bytes),
        Err(error) => {
            warn!("The parallel download didn't verify ({error}); downloading on one connection");
            None
        }
    }
}

/// The key `tauri.conf.json` gives the updater plugin.
fn updater_pubkey(app: &AppHandle) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("updater")?
        .get("pubkey")?
        .as_str()
        .map(str::to_owned)
}

/// The update found and downloaded by [`download_update`], waiting for
/// [`install_downloaded_update`]. Kept on the Rust side so 38 MB never
/// crosses into the webview.
static DOWNLOADED_UPDATE: std::sync::Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>> =
    std::sync::Mutex::new(None);

pub const UPDATE_PROGRESS_EVENT: &str = "update-download-progress";

/// Find the update and download it on several connections, verified.
/// Returns the version downloaded; progress arrives as
/// [`UPDATE_PROGRESS_EVENT`] carrying an [`InstallerProgress`].
///
/// Replaces the plugin's JS `update.download()`, which fetched on one
/// connection. The update is looked up again here because the plugin's
/// handle from the webview's `check()` is not reachable from Rust; it is one
/// small request for `latest.json`.
#[tauri::command]
#[specta::specta]
pub async fn download_update(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_updater::UpdaterExt;

    let update = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        // The plugin builds its single-stream download client from this, and
        // the update it returns carries no overall timeout, so without a stall
        // limit a dead connection would leave the progress bar where it was
        // for ever. A slow link that is still moving is never cut off.
        .configure_client(|client| client.read_timeout(ASSET_STALL_TIMEOUT))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't reach the update server: {e}"))?
        .ok_or_else(|| "There's no update to install.".to_string())?;

    let emit = |downloaded: u64, total: u64| {
        let _ = app.emit(
            UPDATE_PROGRESS_EVENT,
            InstallerProgress {
                downloaded,
                total: Some(total),
            },
        );
    };

    let client = crate::managers::model::download_client().map_err(|e| e.to_string())?;
    let url = update.download_url.to_string();
    let pubkey = updater_pubkey(&app);
    let parallel = fetch_asset_parallel(&client, &url, &emit)
        .await
        .and_then(|bytes| keep_if_verified(bytes, &update.signature, pubkey.as_deref()));
    let bytes = match parallel {
        Some(bytes) => bytes,
        None => {
            info!("Downloading the update on one connection");
            // The plugin's own download, which verifies the signature itself.
            let downloaded = AtomicU64::new(0);
            update
                .download(
                    |chunk, total| {
                        let now =
                            downloaded.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
                        let _ = app.emit(
                            UPDATE_PROGRESS_EVENT,
                            InstallerProgress {
                                downloaded: now,
                                total,
                            },
                        );
                    },
                    || {},
                )
                .await
                .map_err(|e| format!("The update didn't download: {e}"))?
        }
    };

    let version = update.version.clone();
    *DOWNLOADED_UPDATE.lock().unwrap_or_else(|e| e.into_inner()) = Some((update, bytes));
    Ok(version)
}

/// Install what [`download_update`] fetched. On Windows this starts the
/// installer and ends the process, so call `prepare_update_install` first.
#[tauri::command]
#[specta::specta]
pub async fn install_downloaded_update() -> Result<(), String> {
    let (update, bytes) = DOWNLOADED_UPDATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or("The update hasn't been downloaded.")?;
    update
        .install(&bytes)
        .map_err(|e| format!("The update didn't install: {e}"))
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

    let emit = |downloaded: u64, total: u64| {
        let _ = app.emit(
            INSTALLER_PROGRESS_EVENT,
            InstallerProgress {
                downloaded,
                total: Some(total),
            },
        );
    };
    let fast_client = crate::managers::model::download_client().map_err(|e| e.to_string())?;
    if let Some(bytes) = fetch_asset_parallel(&fast_client, &url, &emit).await {
        let written = std::fs::write(&partial, &bytes)
            .map_err(|e| format!("Couldn't write to Downloads: {e}"));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
        let _ = std::fs::remove_file(&target);
        std::fs::rename(&partial, &target)
            .map_err(|e| format!("Couldn't save the installer: {e}"))?;
        return Ok(DownloadedInstaller {
            path: target.to_string_lossy().into_owned(),
            version: manifest.version,
        });
    }

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
///
/// On Windows it also does the app's quit cleanup. `install()` there starts
/// the installer and ends the process with `std::process::exit`, which skips
/// `RunEvent::Exit`, so without this a meeting recording at that moment lost
/// its tail and stayed unfinalised until the next launch recovered it.
/// macOS and Linux don't need it: they install in place and then relaunch
/// through `request_restart`, which does fire `RunEvent::Exit`.
///
/// Async so the cleanup (up to five seconds for a meeting) runs off the main
/// thread and the "closing and reopening" message stays on screen meanwhile.
#[tauri::command]
#[specta::specta]
pub async fn prepare_update_install(app: AppHandle, version: String) -> Result<(), String> {
    let noted = write_pending_update(&app, version);
    if cfg!(target_os = "windows") {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || crate::release_before_exit(&handle))
            .await
            .map_err(|e| e.to_string())?;
    }
    noted
}

fn write_pending_update(app: &AppHandle, version: String) -> Result<(), String> {
    let path = pending_update_path(app).ok_or("No app data folder")?;
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
    fn chunks_cover_every_byte_once() {
        assert_eq!(chunk_ranges(10, 4), vec![(0, 3), (4, 7), (8, 9)]);
        assert_eq!(chunk_ranges(8, 4), vec![(0, 3), (4, 7)]);
        assert_eq!(chunk_ranges(1, 4), vec![(0, 0)]);
        assert!(chunk_ranges(0, 4).is_empty());
        let total = 39_951_234;
        let ranges = chunk_ranges(total, ASSET_CHUNK_BYTES);
        assert_eq!(ranges.first().unwrap().0, 0);
        assert_eq!(ranges.last().unwrap().1, total - 1);
        assert!(ranges.windows(2).all(|w| w[1].0 == w[0].1 + 1));
    }

    #[test]
    fn total_is_read_from_content_range() {
        assert_eq!(
            total_from_content_range("bytes 0-0/39951234"),
            Some(39_951_234)
        );
        assert_eq!(total_from_content_range("bytes 0-0/*"), None);
        assert_eq!(total_from_content_range("bytes 0-0/0"), None);
    }

    #[test]
    fn a_chunk_must_be_the_range_it_asked_for() {
        assert!(content_range_is("bytes 0-3145727/39951234", 0, 3_145_727));
        assert!(content_range_is(
            " bytes 3145728-6291455/* ",
            3_145_728,
            6_291_455
        ));
        // Same length, other bytes.
        assert!(!content_range_is(
            "bytes 3145728-6291455/39951234",
            0,
            3_145_727
        ));
        assert!(!content_range_is("bytes */39951234", 0, 3_145_727));
        assert!(!content_range_is("", 0, 0));
    }

    // Made with `tauri signer sign` and the real SpeakoFlow key over the 16
    // bytes "hello speakoflow"; the same vector `updater-manifest.test.ts`
    // checks the CI verifier with.
    const SIGNED_PAYLOAD: &[u8] = b"hello speakoflow";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTQkEzaUgvLzNSYzJ0dVM1emFGdGtKaGEzUVpHRU9YY253VWtmR0dyUUtQUFc2eDhsQzZCRGs1czVvTXY3NnAxdlVVeTdVSUZDd3BzbldpM2RLYkhLbHVwQzlZRVJVdEF3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwNTkxMjU5CWZpbGU6c2Ytc2lnbi10ZXN0LmJpbgpnQlRzTm9HZGtFcjVvNEVKay9WUlJKVWpVSUxBSktZRklQMGRzeDJwSUhlWEFydUNGODJzRlVpYVkyNDYvT3BlcTQ1SEU0R0R3eUFpZ2RHMTJmbDNBQT09Cg==";

    fn configured_pubkey() -> String {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        conf["plugins"]["updater"]["pubkey"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_download_signed_with_our_key_is_accepted() {
        assert_eq!(
            verify_update_signature(SIGNED_PAYLOAD, SIGNATURE, &configured_pubkey()),
            Ok(())
        );
    }

    #[test]
    fn a_download_that_changed_is_refused() {
        assert!(
            verify_update_signature(b"hello speakoflox", SIGNATURE, &configured_pubkey()).is_err()
        );
        assert!(
            verify_update_signature(SIGNED_PAYLOAD, "not a signature", &configured_pubkey())
                .is_err()
        );
    }

    #[test]
    fn a_parallel_download_is_kept_only_if_it_verifies() {
        let key = configured_pubkey();
        assert_eq!(
            keep_if_verified(SIGNED_PAYLOAD.to_vec(), SIGNATURE, Some(&key)),
            Some(SIGNED_PAYLOAD.to_vec())
        );
        // Bytes that went wrong on the way send the caller to one stream
        // instead of failing the update.
        assert_eq!(
            keep_if_verified(b"hello speakoflox".to_vec(), SIGNATURE, Some(&key)),
            None
        );
        assert_eq!(
            keep_if_verified(SIGNED_PAYLOAD.to_vec(), SIGNATURE, None),
            None
        );
    }

    /* ── The parallel downloader against a local server ──────────────────
     *
     * These run the real `fetch_asset_parallel_with` over real sockets, so
     * what they prove is what an update download does: the bytes come back
     * whole and in order, and every way a host can misbehave ends in `None`,
     * which is the caller's cue to use one connection. */

    #[derive(Clone, Copy, PartialEq)]
    enum Host {
        /// Answers ranges correctly, like GitHub.
        Ranges,
        /// Ignores `Range` and sends the whole file with 200.
        IgnoresRange,
        /// Answers the probe, then sends other bytes than were asked for.
        WrongRange,
        /// Ranges, but every chunk fails with 503.
        Fails,
        /// `/asset` redirects to a signed URL, like GitHub. The first token
        /// handed out only answers the probe, as an expired token would.
        ExpiringToken,
    }

    fn test_asset(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 + i / 4099) as u8).collect()
    }

    fn http_response(status: &str, headers: &[String], body: &[u8]) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        for header in headers {
            out.push_str(header);
            out.push_str("\r\n");
        }
        out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        let mut out = out.into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn answer(
        host: Host,
        base: &str,
        path: &str,
        range: Option<&str>,
        data: &[u8],
        tokens: &AtomicU64,
    ) -> Vec<u8> {
        if host == Host::ExpiringToken && path == "/asset" {
            let token = tokens.fetch_add(1, Ordering::SeqCst) + 1;
            return http_response(
                "302 Found",
                &[format!("Location: {base}/signed?t={token}")],
                b"",
            );
        }
        let probe = range == Some("bytes=0-0");
        if let Some(token) = path.strip_prefix("/signed?t=") {
            if token == "1" && !probe {
                return http_response("403 Forbidden", &[], b"expired");
            }
        }
        let Some((start, end)) = range
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.split_once('-'))
            .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
        else {
            return http_response("200 OK", &[], data);
        };
        let total = data.len();
        match host {
            Host::IgnoresRange => http_response("200 OK", &[], data),
            Host::Fails if !probe => http_response("503 Service Unavailable", &[], b""),
            Host::WrongRange if !probe => http_response(
                "206 Partial Content",
                &[format!(
                    "Content-Range: bytes {}-{}/{total}",
                    start + 1,
                    end + 1
                )],
                &data[start..=end],
            ),
            _ => http_response(
                "206 Partial Content",
                &[format!("Content-Range: bytes {start}-{end}/{total}")],
                &data[start..=end],
            ),
        }
    }

    /// Serves `data` on a local port and returns the asset's URL.
    async fn serve(host: Host, data: std::sync::Arc<Vec<u8>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let tokens = std::sync::Arc::new(AtomicU64::new(0));
        let url = format!("{base}/asset");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (data, tokens, base) = (data.clone(), tokens.clone(), base.clone());
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 2048];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&request).into_owned();
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let range = text.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("range")
                            .then(|| value.trim().to_string())
                    });
                    let response = answer(host, &base, &path, range.as_deref(), &data, &tokens);
                    let _ = socket.write_all(&response).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        url
    }

    /// Downloads from a local `host` in small chunks, and reports what came
    /// back and the last progress seen.
    fn download_from(host: Host, len: usize) -> (Vec<u8>, Option<Vec<u8>>, (u64, u64)) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let data = std::sync::Arc::new(test_asset(len));
            let url = serve(host, data.clone()).await;
            let client = crate::managers::model::download_client().unwrap();
            let last = std::sync::Mutex::new((0u64, 0u64));
            let got = fetch_asset_parallel_with(
                &client,
                &url,
                &|done, total| *last.lock().unwrap() = (done, total),
                4,
                512 * 1024,
            )
            .await;
            let last = *last.lock().unwrap();
            ((*data).clone(), got, last)
        })
    }

    #[test]
    fn a_host_that_answers_ranges_gets_the_whole_file_back_in_order() {
        // Not a multiple of the chunk size, so the last chunk is short.
        let len = ASSET_PARALLEL_MIN_BYTES as usize + 777_777;
        let (sent, got, last) = download_from(Host::Ranges, len);
        assert!(got.as_deref() == Some(sent.as_slice()), "bytes differ");
        assert_eq!(last, (len as u64, len as u64));
    }

    #[test]
    fn a_host_that_ignores_ranges_falls_back_to_one_connection() {
        let (_, got, _) = download_from(Host::IgnoresRange, ASSET_PARALLEL_MIN_BYTES as usize);
        assert!(got.is_none());
    }

    #[test]
    fn other_bytes_than_asked_for_fall_back_to_one_connection() {
        let (_, got, _) = download_from(Host::WrongRange, ASSET_PARALLEL_MIN_BYTES as usize);
        assert!(got.is_none());
    }

    #[test]
    fn chunks_that_keep_failing_fall_back_to_one_connection() {
        let (_, got, _) = download_from(Host::Fails, ASSET_PARALLEL_MIN_BYTES as usize);
        assert!(got.is_none());
    }

    #[test]
    fn a_small_file_uses_one_connection() {
        let (_, got, _) = download_from(Host::Ranges, ASSET_PARALLEL_MIN_BYTES as usize - 1);
        assert!(got.is_none());
    }

    #[test]
    fn an_expired_signed_url_is_replaced_through_the_redirect() {
        let len = ASSET_PARALLEL_MIN_BYTES as usize + 123_457;
        let (sent, got, _) = download_from(Host::ExpiringToken, len);
        assert!(got.as_deref() == Some(sent.as_slice()), "bytes differ");
    }

    /// Downloads the real Windows installer from the latest release through
    /// the parallel path, checks it against `latest.json`'s signature, and
    /// prints the speed beside a single-connection baseline over the same
    /// time budget.
    ///
    /// `cargo test --lib updates::tests::live_parallel_download -- --ignored --nocapture`
    /// Compares connection counts and chunk sizes on the real installer,
    /// interleaved over two rounds so a change in the network hits every
    /// setting alike. Evidence for [`ASSET_CONNECTIONS`] / [`ASSET_CHUNK_BYTES`].
    ///
    /// `cargo test --lib updates::tests::live_parallel_sweep -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_parallel_sweep() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let client = crate::managers::model::download_client().unwrap();
            let url = "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/v2.0.1/SpeakoFlow_2.0.1_x64-setup.exe";
            let mib = 1024 * 1024;
            let settings: [(usize, u64); 4] = [(8, mib), (8, 5 * mib), (16, 2 * mib), (16, 3 * mib)];
            for round in 1..=2 {
                for (connections, chunk) in settings {
                    let started = std::time::Instant::now();
                    let bytes =
                        fetch_asset_parallel_with(&client, url, &|_, _| {}, connections, chunk)
                            .await
                            .expect("fell back");
                    let secs = started.elapsed().as_secs_f64();
                    println!(
                        "round {round}: {connections:>2} conns x {:>1} MiB: {secs:5.1}s = {:5.1} Mbit/s",
                        chunk / mib,
                        bytes.len() as f64 * 8.0 / 1e6 / secs
                    );
                }
            }
        });
    }

    #[test]
    #[ignore]
    fn live_parallel_download() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let client = crate::managers::model::download_client().unwrap();
            let manifest: serde_json::Value = client
                .get(UPDATE_MANIFEST_URL)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let platform = &manifest["platforms"]["windows-x86_64"];
            let url = platform["url"].as_str().unwrap();
            let signature = platform["signature"].as_str().unwrap();

            // Single connection for 15 s, to compare against.
            let started = std::time::Instant::now();
            let mut stream = client.get(url).send().await.unwrap().bytes_stream();
            let mut single = 0u64;
            while started.elapsed() < Duration::from_secs(15) {
                match tokio::time::timeout(Duration::from_secs(5), stream.next()).await {
                    Ok(Some(Ok(part))) => single += part.len() as u64,
                    _ => break,
                }
            }
            drop(stream);
            let single_mbps = single as f64 * 8.0 / 1e6 / started.elapsed().as_secs_f64();

            let started = std::time::Instant::now();
            let bytes = fetch_asset_parallel(&client, url, &|_, _| {})
                .await
                .expect("the parallel path fell back");
            let secs = started.elapsed().as_secs_f64();
            verify_update_signature(&bytes, signature, &configured_pubkey()).unwrap();
            println!(
                "{url}\n  one connection: {single_mbps:.1} Mbit/s (≈{:.0}s for the file)\n  parallel:       {:.1} MB in {secs:.1}s = {:.1} Mbit/s, signature OK",
                bytes.len() as f64 * 8.0 / 1e6 / single_mbps,
                bytes.len() as f64 / 1e6,
                bytes.len() as f64 * 8.0 / 1e6 / secs,
            );
        });
    }

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
