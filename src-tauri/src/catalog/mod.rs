//! Bundled GGUF model catalog (transcribe.cpp engine).
//!
//! The single source of truth for the new GGUF model set is Handy's
//! `catalog.json`, shipped verbatim next to this file and embedded at compile
//! time via [`include_str!`]. Bundling the whole file (rather than hardcoding a
//! handful of models in `model.rs`) is a deliberate choice: pulling a future
//! Handy model release becomes a one-file copy
//! (`curl … catalog.json > src/catalog/catalog.json`) with no Rust edits — see
//! `docs/engine-migration/PLAN.md` §4 / Session 7's FOLLOW_HANDY routine.
//!
//! This module only *parses* the catalog into typed structs. Mapping a catalog
//! entry to a `ModelInfo` (the app's download/UI record) lives in
//! [`crate::managers::model`], which owns that type — keeping the dependency
//! one-directional (`model` → `catalog`).
//!
//! Schema mirrors `catalog_version: 1`. Parsing is tolerant: unknown fields are
//! ignored (so a newer catalog still loads), and a malformed file degrades to an
//! empty catalog with a logged warning rather than breaking startup (N1).

use log::warn;
use serde::Deserialize;
use std::sync::OnceLock;

/// The GGUF catalog bundled from Handy (`src/catalog/catalog.json`).
const CATALOG_JSON: &str = include_str!("catalog.json");

/// Top-level catalog document.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[allow(dead_code)]
    pub catalog_version: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pub generated_at: String,
    #[serde(default)]
    pub models: Vec<CatalogModel>,
}

/// One model entry in the catalog. Only the fields the app consumes are typed;
/// serde ignores the rest, so future catalog additions parse without changes.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogModel {
    /// Full Hugging Face repo id, e.g. `handy-computer/parakeet-unified-en-0.6b-gguf`.
    pub id: String,
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
    #[serde(default)]
    pub recommended: bool,
    /// Handy's overall recommendation ordering (1 = top). `None` when not ranked.
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
    #[serde(default)]
    #[allow(dead_code)]
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
}

impl CatalogModel {
    /// The file entry for this model's `default_quant`, falling back to the
    /// first file if the default isn't listed (shouldn't happen for a valid
    /// catalog, but keeps the mapping infallible).
    pub fn default_file(&self) -> Option<&CatalogFile> {
        self.files
            .iter()
            .find(|f| f.quant == self.default_quant)
            .or_else(|| self.files.first())
    }

    /// Direct Hugging Face `resolve` URL for the default-quant file, e.g.
    /// `https://huggingface.co/handy-computer/<slug>-gguf/resolve/main/<file>`.
    pub fn download_url(&self, file: &CatalogFile) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}",
            self.id, file.filename
        )
    }
}

/// Parse the bundled catalog once and cache it. A malformed bundle logs a
/// warning and yields an empty catalog so the app still starts (N1).
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| match serde_json::from_str::<Catalog>(CATALOG_JSON) {
        Ok(catalog) => catalog,
        Err(e) => {
            warn!("Failed to parse bundled catalog.json: {}", e);
            Catalog {
                catalog_version: 0,
                generated_at: String::new(),
                models: Vec::new(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_parses() {
        let catalog = catalog();
        assert_eq!(catalog.catalog_version, 1, "bundled catalog must parse");
        assert!(
            catalog.models.len() >= 5,
            "expected the full Handy catalog, got {}",
            catalog.models.len()
        );
    }

    #[test]
    fn recommended_set_is_well_formed() {
        let catalog = catalog();
        let recommended: Vec<&CatalogModel> =
            catalog.models.iter().filter(|m| m.recommended).collect();
        assert!(
            recommended.len() >= 5,
            "expected at least the 5 ranked recommended models"
        );
        for m in &recommended {
            // Every surfaced model must resolve to a downloadable default file
            // and a well-formed HF URL.
            let file = m
                .default_file()
                .unwrap_or_else(|| panic!("{} has no default file", m.slug));
            assert!(file.size_bytes > 0, "{} default file has no size", m.slug);
            let url = m.download_url(file);
            assert!(
                url.starts_with("https://huggingface.co/handy-computer/")
                    && url.ends_with(".gguf"),
                "unexpected url for {}: {}",
                m.slug,
                url
            );
        }
    }

    #[test]
    fn parakeet_unified_is_rank_one_streaming() {
        let catalog = catalog();
        let parakeet = catalog
            .models
            .iter()
            .find(|m| m.slug == "parakeet-unified-en-0.6b")
            .expect("parakeet-unified-en-0.6b present");
        assert!(parakeet.recommended);
        assert_eq!(parakeet.recommended_rank, Some(1));
        assert!(parakeet.capabilities.streaming);
        assert_eq!(parakeet.default_quant, "Q8_0");
        assert_eq!(parakeet.default_file().unwrap().size_bytes, 731_357_568);
    }
}
