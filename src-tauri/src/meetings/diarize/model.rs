//! Getting the speaker-embedding model onto disk.
//!
//! Deliberately **not** registered in [`crate::managers::model`]'s catalog. That
//! catalog exists so the user can choose between models — it drives the pickers,
//! the download list, and `is_transcription`'s routing. This model is not a choice:
//! it is a capability that is either installed or not, and putting it in the
//! catalog would mean filtering it back out of three UIs and extending an
//! `EngineType` denylist so it could never be selected as a dictation engine.
//!
//! So it gets one file, one URL, one predicate. What it gives up by not being in
//! the catalog is the resumable chunked downloader — acceptable for 26 MB, where a
//! failed download costs a retry rather than an hour.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use log::{info, warn};

/// File name on disk, matching the upstream asset so a user who already has it can
/// drop it in themselves.
pub const MODEL_FILE: &str = "wespeaker_en_voxceleb_resnet34_LM.onnx";

/// Canonical source.
///
/// WeSpeaker ResNet34 with large-margin finetuning, trained on VoxCeleb. Chosen
/// over the (smaller, similarly accurate) CAM++ models for one specific reason:
/// these are the same weights pyannote 3.1 uses, which means pyannote's published,
/// tuned clustering threshold transfers directly instead of having to be
/// rediscovered by trial and error against real recordings.
const MODEL_URL: &str = "https://huggingface.co/csukuangfj/speaker-embedding-models/resolve/main/wespeaker_en_voxceleb_resnet34_LM.onnx";

/// Mirror on the GitHub release CDN. Same bytes; the upstream tag's spelling of
/// "recongition" is theirs, not a typo here.
const MODEL_MIRROR: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx";

/// Approximate download size, for the UI to promise before it starts.
pub const MODEL_SIZE_MB: u32 = 27;

/// Smallest plausible size for a complete file.
///
/// A truncated download is the failure mode that matters: the file exists, the path
/// resolves, and the ONNX session then fails to load with a message about protobuf
/// rather than about the download. Checking the length catches it before that.
const MIN_PLAUSIBLE_BYTES: u64 = 20 * 1024 * 1024;

/// Where the model lives.
pub fn model_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    Ok(crate::portable::app_data_dir(app)?
        .join("models")
        .join(MODEL_FILE))
}

/// Whether a usable model is already on disk.
///
/// Length-checked rather than existence-checked, so an interrupted download reports
/// "not installed" and is re-fetched instead of loaded and failing.
pub fn is_installed(app: &tauri::AppHandle) -> bool {
    match model_path(app) {
        Ok(path) => std::fs::metadata(&path)
            .map(|meta| meta.len() >= MIN_PLAUSIBLE_BYTES)
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Download the model, replacing any partial file.
///
/// Writes to a `.partial` and renames on success, so an interrupted download can
/// never leave a truncated file where a complete one is expected. Tries the
/// canonical URL first and the mirror second — a Hugging Face outage or a network
/// that blocks it should not make the feature unavailable.
pub async fn download(app: &tauri::AppHandle) -> Result<PathBuf> {
    let destination = model_path(app)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating the models directory {parent:?}"))?;
    }
    let partial = destination.with_extension("partial");

    let mut last_error = None;
    for url in [MODEL_URL, MODEL_MIRROR] {
        match fetch_to(url, &partial).await {
            Ok(bytes) if bytes >= MIN_PLAUSIBLE_BYTES => {
                std::fs::rename(&partial, &destination).with_context(|| {
                    format!("moving the downloaded model into place at {destination:?}")
                })?;
                info!(
                    "Downloaded the speaker embedding model ({} MB) from {url}",
                    bytes / (1024 * 1024)
                );
                return Ok(destination);
            }
            Ok(bytes) => {
                // A 200 that returned an error page rather than a model. Reported
                // as a failure so the mirror is tried, instead of renaming a few
                // kilobytes of HTML into place.
                let _ = std::fs::remove_file(&partial);
                last_error = Some(anyhow!(
                    "{url} returned only {bytes} bytes, which is not the model"
                ));
                warn!("Speaker model download from {url} was too small; trying the next source");
            }
            Err(error) => {
                let _ = std::fs::remove_file(&partial);
                warn!("Speaker model download from {url} failed: {error}");
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Could not download the speaker embedding model")))
}

/// Stream one URL to a file, returning how many bytes landed.
async fn fetch_to(url: &str, path: &std::path::Path) -> Result<u64> {
    use futures_util::StreamExt;
    use std::io::Write;

    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!("{url} answered {}", response.status()));
    }

    let mut file = std::fs::File::create(path).with_context(|| format!("creating {path:?}"))?;
    let mut written = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading the download stream")?;
        file.write_all(&chunk).context("writing the model file")?;
        written += chunk.len() as u64;
    }
    file.flush().context("flushing the model file")?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both sources must name the same file, or a mirror fallback would silently
    /// install different weights and the tuned threshold would no longer apply.
    #[test]
    fn both_sources_serve_the_same_file() {
        assert!(MODEL_URL.ends_with(MODEL_FILE));
        assert!(MODEL_MIRROR.ends_with(MODEL_FILE));
    }

    /// The floor has to be below the real size and far above an error page.
    #[test]
    fn the_size_floor_is_between_an_error_page_and_the_model() {
        let expected = u64::from(MODEL_SIZE_MB) * 1024 * 1024;
        assert!(MIN_PLAUSIBLE_BYTES < expected);
        assert!(MIN_PLAUSIBLE_BYTES > 1024 * 1024);
    }

    #[test]
    fn the_sources_are_https() {
        assert!(MODEL_URL.starts_with("https://"));
        assert!(MODEL_MIRROR.starts_with("https://"));
    }
}
