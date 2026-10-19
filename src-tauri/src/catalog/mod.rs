//! Bundled GGUF model catalog (transcribe.cpp engine).
//!
//! The single source of truth for the GGUF model set is Handy's
//! `catalog.json`, shipped verbatim next to this file and embedded at compile
//! time via [`include_str!`]. Bundling the whole file (rather than hardcoding a
//! handful of models in `model.rs`) is a deliberate choice: pulling a future
//! Handy model release is a one-file copy
//! (`git show upstream/main:src-tauri/src/catalog/catalog.json`) with no Rust
//! edits.
//!
//! This module only *parses* the catalog into typed structs and answers the
//! questions that need the raw document (download URLs, mirrors, rank).
//! Mapping a catalog entry to a `ModelInfo` (the app's download/UI record)
//! lives in [`crate::managers::model`], which owns that type, keeping the
//! dependency one-directional (`model` → `catalog`).
//!
//! Schema mirrors `catalog_version: 2`: a root `mirrors` list, a per-model
//! `revision` (the Hugging Face commit the sizes and hashes were generated
//! from) and a per-file `sha256`. Parsing is tolerant: unknown fields are
//! ignored (so a newer catalog still loads), every v2 field has a default (so
//! a v1 file still loads, just without pinning or mirrors), and a malformed
//! file degrades to an empty catalog with a logged warning rather than breaking
//! startup.

use log::warn;
use serde::Deserialize;
use std::sync::OnceLock;

/// The GGUF catalog bundled from Handy (`src/catalog/catalog.json`).
const CATALOG_JSON: &str = include_str!("catalog.json");

/// The branch a model is fetched from when the catalog does not pin a commit.
const UNPINNED_REVISION: &str = "main";

/// Top-level catalog document.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[allow(dead_code)]
    pub catalog_version: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pub generated_at: String,
    /// Static hosts that serve a copy of every catalog file, tried after
    /// Hugging Face fails. A file's URL there is
    /// `{mirror}/{repo_id}/{revision}/{filename}`: the same three values that
    /// form the Hugging Face `resolve` URL, so a mirror is a plain file host.
    #[serde(default)]
    pub mirrors: Vec<String>,
    #[serde(default)]
    pub models: Vec<CatalogModel>,
}

/// One model entry in the catalog. Only the fields the app consumes are typed;
/// serde ignores the rest, so future catalog additions parse without changes.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogModel {
    /// Full Hugging Face repo id, e.g. `handy-computer/parakeet-unified-en-0.6b-gguf`.
    pub id: String,
    /// The Hugging Face commit the catalog's sizes and hashes were generated
    /// from. Downloading at this commit rather than `main` is what makes the
    /// `sha256` below a promise: a later push to the repo cannot change the
    /// bytes we fetch, and a mirror keyed on the same commit holds the same file.
    #[serde(default)]
    pub revision: Option<String>,
    /// Short slug, e.g. `parakeet-unified-en-0.6b`.
    pub slug: String,
    pub name: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub architecture: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub family: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub language_count: u32,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub capabilities: CatalogCapabilities,
    /// 0–100 in the catalog (higher = faster).
    #[serde(default)]
    pub speed_score: u32,
    /// 0–100 in the catalog (higher = more accurate).
    #[serde(default)]
    pub accuracy_score: u32,
    #[serde(default)]
    pub files: Vec<CatalogFile>,
    /// The quant to download by default (e.g. `Q8_0`).
    #[serde(default)]
    pub default_quant: String,
    /// Part of the small curated set onboarding offers (badged "Recommended").
    /// Distinct from `recommended_rank`, which orders the whole list.
    #[serde(default)]
    pub recommended: bool,
    /// Handy's overall ordering of the list (1 = top). `None` when not ranked.
    /// Set on more models than `recommended`, so it is read on its own.
    #[serde(default)]
    pub recommended_rank: Option<u32>,
}

/// Declared model capabilities (canonical in the GGUF itself; the catalog copy
/// is what we can show *before* download, reconciled post-load — see
/// `managers::model_capabilities`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CatalogCapabilities {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub translate: bool,
    /// Whether the model can work out the spoken language itself, which is
    /// what decides whether "Auto" is a real choice for it.
    #[serde(default)]
    pub lang_detect: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub timestamps: String,
}

/// One downloadable quantization of a model.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogFile {
    pub filename: String,
    pub quant: String,
    pub size_bytes: u64,
    /// Content hash of the file at the model's `revision`: the trust anchor for
    /// a download from any source. `None` only in catalogs that predate it.
    #[serde(default)]
    pub sha256: Option<String>,
}

impl CatalogModel {
    /// The id `ModelManager` gives this model (`"<slug>-gguf"`, the Hugging
    /// Face repo suffix), kept distinct from the legacy ids that share a slug
    /// (e.g. `canary-180m-flash`) so neither shadows the other.
    pub fn model_id(&self) -> String {
        format!("{}-gguf", self.slug)
    }

    /// The file entry for this model's `default_quant`, falling back to the
    /// first file if the default isn't listed (shouldn't happen for a valid
    /// catalog, but keeps the mapping infallible).
    pub fn default_file(&self) -> Option<&CatalogFile> {
        self.files
            .iter()
            .find(|f| f.quant == self.default_quant)
            .or_else(|| self.files.first())
    }

    /// The revision to fetch: the pinned commit, or `main` for an entry
    /// without one.
    pub fn revision(&self) -> &str {
        self.revision.as_deref().unwrap_or(UNPINNED_REVISION)
    }

    /// Hugging Face `resolve` URL for `file` at the pinned revision, e.g.
    /// `https://huggingface.co/handy-computer/<slug>-gguf/resolve/<sha>/<file>`.
    pub fn download_url(&self, file: &CatalogFile) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.id,
            self.revision(),
            file.filename
        )
    }

    /// The mirror copies of `file`, in the catalog's order.
    ///
    /// Empty unless the file has a `sha256` and the model a pinned
    /// `revision`: a mirror is an untrusted host, and the hash compiled into
    /// this binary is the only thing that makes its bytes acceptable. Without
    /// one there is nothing to check a mirror's file against, so it is never
    /// asked.
    pub fn mirror_urls(&self, file: &CatalogFile, mirrors: &[String]) -> Vec<String> {
        let (Some(revision), Some(_)) = (self.revision.as_deref(), file.sha256.as_deref()) else {
            return Vec::new();
        };
        mirrors
            .iter()
            .map(|base| {
                format!(
                    "{}/{}/{}/{}",
                    base.trim_end_matches('/'),
                    self.id,
                    revision,
                    file.filename
                )
            })
            .collect()
    }
}

/// Parse the bundled catalog once and cache it. A malformed bundle logs a
/// warning and yields an empty catalog so the app still starts.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| match serde_json::from_str::<Catalog>(CATALOG_JSON) {
        Ok(catalog) => catalog,
        Err(e) => {
            warn!("Failed to parse bundled catalog.json: {}", e);
            Catalog {
                catalog_version: 0,
                generated_at: String::new(),
                mirrors: Vec::new(),
                models: Vec::new(),
            }
        }
    })
}

/// The catalog entry behind a `ModelManager` id (`"<slug>-gguf"`), if any.
pub fn model_for_id(model_id: &str) -> Option<&'static CatalogModel> {
    catalog().models.iter().find(|m| m.model_id() == model_id)
}

/// Mirror URLs for the catalog file `filename` of model `model_id`, or empty
/// when the model isn't from the catalog, the file has no hash, or no mirrors
/// are configured. Matches any listed quant, not just the default.
pub fn mirror_urls_for(model_id: &str, filename: &str) -> Vec<String> {
    let Some(model) = model_for_id(model_id) else {
        return Vec::new();
    };
    let Some(file) = model.files.iter().find(|f| f.filename == filename) else {
        return Vec::new();
    };
    model.mirror_urls(file, &catalog().mirrors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::model_capabilities::KNOWN_ARCHES;
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn bundled_catalog_is_version_two_with_every_model() {
        let catalog = catalog();
        assert_eq!(catalog.catalog_version, 2, "bundled catalog must parse");
        assert_eq!(catalog.models.len(), 69, "the full Handy catalog");
        assert_eq!(catalog.mirrors, vec!["https://blob.handy.computer"]);
    }

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<String> = catalog().models.iter().map(|m| m.model_id()).collect();
        assert_eq!(ids.len(), catalog().models.len());
    }

    #[test]
    fn every_model_is_pinned_and_every_file_hashed() {
        for m in &catalog().models {
            let revision = m
                .revision
                .as_deref()
                .unwrap_or_else(|| panic!("{} has no revision", m.slug));
            assert_eq!(revision.len(), 40, "{}: revision is a commit sha", m.slug);
            for f in &m.files {
                let sha = f
                    .sha256
                    .as_deref()
                    .unwrap_or_else(|| panic!("{} / {} has no sha256", m.slug, f.filename));
                assert!(
                    sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit()),
                    "{} / {}: not a sha256",
                    m.slug,
                    f.filename
                );
                assert!(f.size_bytes > 0, "{} / {} has no size", m.slug, f.filename);
            }
        }
    }

    #[test]
    fn download_urls_are_pinned_to_the_revision() {
        for m in &catalog().models {
            let file = m
                .default_file()
                .unwrap_or_else(|| panic!("{} has no default file", m.slug));
            let url = m.download_url(file);
            let expected = format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                m.id,
                m.revision.as_deref().unwrap_or_default(),
                file.filename
            );
            assert_eq!(url, expected);
            assert!(
                url.starts_with("https://huggingface.co/handy-computer/") && url.ends_with(".gguf"),
                "unexpected url for {}: {}",
                m.slug,
                url
            );
            assert!(!url.contains("/resolve/main/"), "{} is not pinned", m.slug);
        }
    }

    #[test]
    fn an_unpinned_entry_downloads_from_main() {
        let mut m = catalog().models[0].clone();
        m.revision = None;
        let file = m.default_file().unwrap().clone();
        assert!(m.download_url(&file).contains("/resolve/main/"));
    }

    #[test]
    fn mirrors_follow_the_hugging_face_path() {
        let m = catalog()
            .models
            .iter()
            .find(|m| m.slug == "parakeet-unified-en-0.6b")
            .unwrap();
        let file = m.default_file().unwrap();
        let urls = mirror_urls_for(&m.model_id(), &file.filename);
        assert_eq!(
            urls,
            vec![format!(
                "https://blob.handy.computer/handy-computer/parakeet-unified-en-0.6b-gguf/{}/{}",
                m.revision.as_deref().unwrap(),
                file.filename
            )]
        );
        // Any listed quant has a mirror, not just the default.
        let other = m.files.iter().find(|f| f.quant != m.default_quant).unwrap();
        assert_eq!(mirror_urls_for(&m.model_id(), &other.filename).len(), 1);
        // Unknown ids and files have none.
        assert!(mirror_urls_for("medium", "whisper-medium-q4_1.bin").is_empty());
        assert!(mirror_urls_for(&m.model_id(), "nope.gguf").is_empty());
    }

    #[test]
    fn a_file_without_a_hash_is_never_offered_from_a_mirror() {
        let mut m = catalog().models[0].clone();
        let mut file = m.default_file().unwrap().clone();
        let mirrors = vec!["https://mirror.example/".to_string()];
        assert_eq!(m.mirror_urls(&file, &mirrors).len(), 1);
        file.sha256 = None;
        assert!(
            m.mirror_urls(&file, &mirrors).is_empty(),
            "no hash, no mirror"
        );
        let file = m.default_file().unwrap().clone();
        m.revision = None;
        assert!(
            m.mirror_urls(&file, &mirrors).is_empty(),
            "no pin, no mirror"
        );
    }

    #[test]
    fn ranks_order_more_than_the_recommended_set() {
        // Upstream sorts on `rank_of(id)`; ours reads the same number from
        // `ModelInfo::recommended_rank`, copied from here at insert time.
        let rank = |id: &str| model_for_id(id).and_then(|m| m.recommended_rank);
        assert_eq!(rank("parakeet-unified-en-0.6b-gguf"), Some(1));
        assert_eq!(rank("nemotron-3.5-asr-streaming-0.6b-gguf"), Some(2));
        // Ranked but not part of the curated recommended set.
        let voxtral = model_for_id("Voxtral-Mini-4B-Realtime-2602-gguf").unwrap();
        assert!(!voxtral.recommended);
        assert_eq!(voxtral.recommended_rank, Some(6));
        assert_eq!(rank("whisper-tiny-gguf"), None);
        assert!(model_for_id("medium").is_none());
    }

    #[test]
    fn parakeet_unified_is_rank_one_streaming() {
        let parakeet = model_for_id("parakeet-unified-en-0.6b-gguf").expect("present");
        assert!(parakeet.recommended);
        assert_eq!(parakeet.recommended_rank, Some(1));
        assert!(parakeet.capabilities.streaming);
        assert!(!parakeet.capabilities.lang_detect);
        assert_eq!(parakeet.default_quant, "Q8_0");
        assert_eq!(parakeet.default_file().unwrap().size_bytes, 731_357_568);
    }

    #[test]
    fn pure_diarization_models_are_not_listed() {
        assert!(
            catalog()
                .models
                .iter()
                .all(|m| m.architecture != "sortformer"),
            "Sortformer produces speaker segments, not transcription text"
        );
    }

    #[test]
    fn catalog_architectures_are_known_to_capability_probe() {
        let missing: BTreeSet<&str> = catalog()
            .models
            .iter()
            .map(|m| m.architecture.as_str())
            .filter(|arch| !arch.is_empty() && !KNOWN_ARCHES.contains(arch))
            .collect();
        assert!(
            missing.is_empty(),
            "catalog architecture(s) missing from KNOWN_ARCHES: {:?}",
            missing
        );
    }
}
