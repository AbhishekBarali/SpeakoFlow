//! The speech models that came from Handy's first catalog, and what replaced
//! them.
//!
//! Before transcribe.cpp, Handy (and so SpeakoFlow) shipped speech models in
//! two older formats: whisper.cpp `.bin` files, and ONNX directories run by
//! `transcribe-rs`. Every one of them now has a GGUF twin in the bundled
//! catalog (`crate::catalog`): the same model, packaged for transcribe.cpp.
//!
//! The two kinds age differently:
//!
//! - A Whisper `.bin` is still a model transcribe.cpp can load, so it stays
//!   usable. It is only hidden from "Available to download" once its twin is
//!   listed, because two rows with the same name and no way to tell them
//!   apart is what made the Models page read as messy.
//! - An ONNX model is **retired**: the engine that ran it is gone (it ran on
//!   DirectML on Windows, which is where issue #82's crash lived). One that is
//!   already downloaded is kept on disk, never loaded, and offered its GGUF
//!   replacement; one that is not downloaded is not offered at all.
//!
//! This table is the one place that pairs an old id with its replacement, so
//! the loader, the model list and the migration prompt cannot disagree.

use crate::managers::model::EngineType;

/// One old catalog entry and the catalog model that replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyTwin {
    /// The id the old entry has in `ModelManager` (and in a user's saved
    /// `selected_model`).
    pub legacy_id: &'static str,
    /// The replacement's `slug` in `catalog.json`. Its model id is
    /// `"<slug>-gguf"` (see [`replacement_id`]).
    pub catalog_slug: &'static str,
    /// True for ONNX models, whose engine no longer exists. False for Whisper
    /// `.bin` files, which transcribe.cpp still runs.
    pub retired: bool,
}

/// Every old speech model, in the order the old catalog listed them.
pub const LEGACY_TWINS: &[LegacyTwin] = &[
    // whisper.cpp `.bin`: still runnable, hidden from download.
    LegacyTwin {
        legacy_id: "small",
        catalog_slug: "whisper-small",
        retired: false,
    },
    LegacyTwin {
        legacy_id: "medium",
        catalog_slug: "whisper-medium",
        retired: false,
    },
    LegacyTwin {
        legacy_id: "turbo",
        catalog_slug: "whisper-large-v3-turbo",
        retired: false,
    },
    LegacyTwin {
        legacy_id: "large",
        catalog_slug: "whisper-large-v3",
        retired: false,
    },
    LegacyTwin {
        legacy_id: "breeze-asr",
        catalog_slug: "Breeze-ASR-25",
        retired: false,
    },
    // ONNX through transcribe-rs: retired.
    LegacyTwin {
        legacy_id: "parakeet-tdt-0.6b-v2",
        catalog_slug: "parakeet-tdt-0.6b-v2",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "parakeet-tdt-0.6b-v3",
        catalog_slug: "parakeet-tdt-0.6b-v3",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "moonshine-base",
        catalog_slug: "moonshine-base",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "moonshine-tiny-streaming-en",
        catalog_slug: "moonshine-streaming-tiny",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "moonshine-small-streaming-en",
        catalog_slug: "moonshine-streaming-small",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "moonshine-medium-streaming-en",
        catalog_slug: "moonshine-streaming-medium",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "sense-voice-int8",
        catalog_slug: "SenseVoiceSmall",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "gigaam-v3-e2e-ctc",
        catalog_slug: "gigaam-v3-e2e-ctc",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "canary-180m-flash",
        catalog_slug: "canary-180m-flash",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "canary-1b-v2",
        catalog_slug: "canary-1b-v2",
        retired: true,
    },
    LegacyTwin {
        legacy_id: "cohere-int8",
        catalog_slug: "cohere-transcribe-03-2026",
        retired: true,
    },
];

/// The table entry for an old model id, if it is one.
pub fn twin_of(legacy_id: &str) -> Option<&'static LegacyTwin> {
    LEGACY_TWINS.iter().find(|t| t.legacy_id == legacy_id)
}

/// The model id of the catalog model that replaces `legacy_id`
/// (`"<slug>-gguf"`, the id scheme `ModelManager` gives catalog entries).
pub fn replacement_id(legacy_id: &str) -> Option<String> {
    twin_of(legacy_id).map(|t| format!("{}-gguf", t.catalog_slug))
}

/// Whether `legacy_id` is a retired ONNX model: kept on disk if present, never
/// loaded, offered its replacement.
pub fn is_retired(legacy_id: &str) -> bool {
    twin_of(legacy_id).is_some_and(|t| t.retired)
}

/// Whether models of this engine type can no longer be loaded. These are the
/// `transcribe-rs` ONNX engines; Whisper `.bin` files now load through
/// transcribe.cpp, so `Whisper` is not one of them.
pub fn is_retired_engine(engine: &EngineType) -> bool {
    matches!(
        engine,
        EngineType::Parakeet
            | EngineType::Moonshine
            | EngineType::MoonshineStreaming
            | EngineType::SenseVoice
            | EngineType::GigaAM
            | EngineType::Canary
            | EngineType::Cohere
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_old_id_appears_once() {
        let ids: HashSet<_> = LEGACY_TWINS.iter().map(|t| t.legacy_id).collect();
        assert_eq!(ids.len(), LEGACY_TWINS.len());
    }

    #[test]
    fn every_twin_is_in_the_bundled_catalog() {
        let slugs: HashSet<_> = crate::catalog::catalog()
            .models
            .iter()
            .map(|m| m.slug.as_str())
            .collect();
        for twin in LEGACY_TWINS {
            assert!(
                slugs.contains(twin.catalog_slug),
                "{} names {} as its replacement, which the catalog does not list",
                twin.legacy_id,
                twin.catalog_slug
            );
        }
    }

    #[test]
    fn replacement_ids_use_the_catalog_id_scheme() {
        assert_eq!(
            replacement_id("parakeet-tdt-0.6b-v3").as_deref(),
            Some("parakeet-tdt-0.6b-v3-gguf")
        );
        assert_eq!(
            replacement_id("cohere-int8").as_deref(),
            Some("cohere-transcribe-03-2026-gguf")
        );
        assert_eq!(replacement_id("parakeet-unified-en-0.6b-gguf"), None);
    }

    #[test]
    fn only_onnx_models_are_retired() {
        assert!(is_retired("parakeet-tdt-0.6b-v3"));
        assert!(is_retired("sense-voice-int8"));
        assert!(!is_retired("medium"), "a Whisper .bin still runs");
        assert!(!is_retired("whisper-medium-gguf"));
        assert!(is_retired_engine(&EngineType::Parakeet));
        assert!(!is_retired_engine(&EngineType::Whisper));
        assert!(!is_retired_engine(&EngineType::TranscribeCpp));
    }
}
