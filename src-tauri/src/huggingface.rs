//! Minimal Hugging Face Hub client for discovering and adding custom GGUF
//! language models.
//!
//! The Models tab lets users search the Hub for any GGUF repo, pick a specific
//! quantization file, and add it as a custom local LLM (served by the same
//! bundled llama.cpp engine as the built-in models). This module only handles
//! the read-only discovery calls against the public Hub API:
//!
//!   * [`search_gguf_models`] — full-text search restricted to GGUF repos.
//!   * [`list_repo_gguf_files`] — the `.gguf` files (and vision projectors) in a
//!     repo, with sizes so the UI can show download cost.
//!
//! Adding/persisting the chosen model lives in [`crate::managers::model`]; this
//! module just shapes the data and builds canonical download URLs.

use serde::{Deserialize, Serialize};
use specta::Type;

/// User-Agent sent with Hub API requests (the Hub asks clients to identify
/// themselves; anonymous requests are otherwise rate-limited more aggressively).
const USER_AGENT: &str = "speakoflow";

/// A single repo returned by a Hub model search.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfModelSummary {
    /// Canonical repo id, e.g. `bartowski/Qwen_Qwen3.5-4B-GGUF`.
    pub id: String,
    /// Number of likes (popularity signal shown in the UI).
    pub likes: u64,
    /// Number of downloads (popularity signal shown in the UI).
    pub downloads: u64,
    /// Whether the repo looks like a multimodal/vision model (so the UI can
    /// hint that a projector will be downloaded for image support).
    pub is_vision: bool,
}

/// A single `.gguf` file inside a repo.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfGgufFile {
    /// File name within the repo, e.g. `Qwen_Qwen3.5-4B-Q4_K_M.gguf`.
    pub filename: String,
    /// File size in bytes (from the Hub tree listing).
    pub size_bytes: u64,
    /// Short quantization label extracted from the filename, e.g. `Q4_K_M`.
    pub quant: String,
}

/// The downloadable GGUF assets in a repo, split into model weights and vision
/// projectors (`mmproj-*.gguf`).
#[derive(Debug, Clone, Serialize, Type)]
pub struct HfRepoFiles {
    pub repo_id: String,
    /// Model weight files the user can pick from (one per quantization).
    pub gguf_files: Vec<HfGgufFile>,
    /// Companion vision projectors, if the repo is multimodal.
    pub mmproj_files: Vec<HfGgufFile>,
}

/// Build the canonical direct-download URL for a file in a Hub repo's `main`
/// branch. Matches the form used by the built-in model catalog.
pub fn resolve_url(repo_id: &str, filename: &str) -> String {
    format!(
        "https://huggingface.co/{}/resolve/main/{}",
        repo_id, filename
    )
}

/// Extract a short quantization label (e.g. `Q4_K_M`, `IQ3_XS`, `F16`) from a
/// GGUF filename, scanning for the last known quant marker that begins at a
/// token boundary. Falls back to an empty string when nothing recognizable is
/// found.
pub fn extract_quant(filename: &str) -> String {
    let stem = filename.trim_end_matches(".gguf");
    // ASCII-only uppercase keeps byte indices aligned with `stem` (GGUF
    // filenames are ASCII), so a match index in `upper` slices `stem` safely.
    let upper = stem.to_ascii_uppercase();
    let bytes = upper.as_bytes();

    // We only accept a match that begins at a token boundary (start, or after
    // `-`/`_`/`.`) so the bare `Q3` inside `IQ3` doesn't shadow the real `IQ3`
    // token, and take the right-most such match to skip past the model name.
    const MARKERS: &[&str] = &[
        "IQ1", "IQ2", "IQ3", "IQ4", "Q2_K", "Q3_K", "Q4_K", "Q5_K", "Q6_K", "Q2", "Q3", "Q4", "Q5",
        "Q6", "Q8", "BF16", "FP16", "F16", "F32",
    ];

    let is_boundary = |idx: usize| idx == 0 || matches!(bytes[idx - 1], b'-' | b'_' | b'.');

    let mut best: Option<usize> = None;
    for marker in MARKERS {
        let mut start = 0;
        while let Some(rel) = upper[start..].find(marker) {
            let idx = start + rel;
            if is_boundary(idx) {
                best = Some(best.map_or(idx, |b| b.max(idx)));
            }
            start = idx + 1;
        }
    }

    match best {
        Some(idx) => upper[idx..]
            .trim_matches(|c| c == '-' || c == '_')
            .to_string(),
        None => String::new(),
    }
}

/// Whether a repo's tags/pipeline indicate a multimodal (vision) model.
fn tags_indicate_vision(tags: &[String], pipeline_tag: &Option<String>) -> bool {
    const VISION_MARKERS: &[&str] = &[
        "vision",
        "multimodal",
        "image-text-to-text",
        "any-to-any",
        "visual-question-answering",
    ];
    if let Some(pt) = pipeline_tag {
        if VISION_MARKERS.contains(&pt.as_str()) {
            return true;
        }
    }
    tags.iter()
        .any(|t| VISION_MARKERS.contains(&t.to_lowercase().as_str()))
}

#[derive(Debug, Deserialize)]
struct RawHfModel {
    id: String,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pipeline_tag: Option<String>,
}

/// Search the Hub for GGUF repos matching `query`, sorted by popularity.
///
/// Returns up to 25 results. An empty/whitespace query returns the most
/// downloaded GGUF repos overall (useful as a default browse list).
pub async fn search_gguf_models(query: &str) -> Result<Vec<HfModelSummary>, String> {
    let client = reqwest::Client::new();
    let mut request = client
        .get("https://huggingface.co/api/models")
        .header("User-Agent", USER_AGENT)
        .query(&[
            ("filter", "gguf"),
            ("sort", "downloads"),
            ("direction", "-1"),
            ("limit", "25"),
        ]);

    let trimmed = query.trim();
    if !trimmed.is_empty() {
        request = request.query(&[("search", trimmed)]);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to search Hugging Face: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Hugging Face search failed: HTTP {}",
            response.status()
        ));
    }

    let raw: Vec<RawHfModel> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Hugging Face search results: {}", e))?;

    Ok(raw
        .into_iter()
        .map(|m| HfModelSummary {
            is_vision: tags_indicate_vision(&m.tags, &m.pipeline_tag),
            id: m.id,
            likes: m.likes,
            downloads: m.downloads,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct RawTreeEntry {
    #[serde(rename = "type")]
    entry_type: String,
    path: String,
    #[serde(default)]
    size: u64,
}

/// List the `.gguf` files in a repo's `main` branch, separating model weights
/// from vision projectors. Importance-matrix files (`*imatrix*`) are skipped
/// since they aren't loadable models.
pub async fn list_repo_gguf_files(repo_id: &str) -> Result<HfRepoFiles, String> {
    let repo_id = repo_id.trim();
    if repo_id.is_empty() {
        return Err("Repository id is required".to_string());
    }

    let url = format!("https://huggingface.co/api/models/{}/tree/main", repo_id);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("Failed to list repository files: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Could not list files for '{}': HTTP {}",
            repo_id,
            response.status()
        ));
    }

    let entries: Vec<RawTreeEntry> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse repository file list: {}", e))?;

    let mut gguf_files = Vec::new();
    let mut mmproj_files = Vec::new();

    for entry in entries {
        if entry.entry_type != "file" {
            continue;
        }
        let lower = entry.path.to_lowercase();
        if !lower.ends_with(".gguf") {
            continue;
        }
        // Only root-level files are directly resolvable via /resolve/main/<name>.
        if entry.path.contains('/') {
            continue;
        }

        let file = HfGgufFile {
            quant: extract_quant(&entry.path),
            filename: entry.path.clone(),
            size_bytes: entry.size,
        };

        if lower.starts_with("mmproj") || lower.contains("mmproj") {
            mmproj_files.push(file);
        } else if lower.contains("imatrix") {
            // Importance matrix, not a model — skip.
            continue;
        } else {
            gguf_files.push(file);
        }
    }

    // Smallest weights first so the most accessible quantizations are on top.
    gguf_files.sort_by_key(|f| f.size_bytes);
    // Prefer higher-precision projectors (f16) first.
    mmproj_files.sort_by(|a, b| {
        let score = |f: &HfGgufFile| {
            let l = f.filename.to_lowercase();
            if l.contains("f16") || l.contains("fp16") {
                0
            } else if l.contains("bf16") {
                1
            } else {
                2
            }
        };
        score(a).cmp(&score(b))
    });

    Ok(HfRepoFiles {
        repo_id: repo_id.to_string(),
        gguf_files,
        mmproj_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_quant_handles_common_patterns() {
        assert_eq!(extract_quant("Qwen_Qwen3.5-4B-Q4_K_M.gguf"), "Q4_K_M");
        assert_eq!(extract_quant("Qwen_Qwen3.5-4B-IQ3_XS.gguf"), "IQ3_XS");
        assert_eq!(extract_quant("model-Q8_0.gguf"), "Q8_0");
        assert_eq!(extract_quant("gemma-3-4b-it-f16.gguf"), "F16");
    }

    #[test]
    fn extract_quant_returns_empty_when_unknown() {
        assert_eq!(extract_quant("some-model.gguf"), "");
    }

    #[test]
    fn resolve_url_matches_hub_format() {
        assert_eq!(
            resolve_url("bartowski/Repo-GGUF", "model-Q4_K_M.gguf"),
            "https://huggingface.co/bartowski/Repo-GGUF/resolve/main/model-Q4_K_M.gguf"
        );
    }

    #[test]
    fn vision_detection_reads_tags_and_pipeline() {
        assert!(tags_indicate_vision(
            &["gguf".to_string(), "vision".to_string()],
            &None
        ));
        assert!(tags_indicate_vision(
            &[],
            &Some("image-text-to-text".to_string())
        ));
        assert!(!tags_indicate_vision(&["gguf".to_string()], &None));
    }
}
