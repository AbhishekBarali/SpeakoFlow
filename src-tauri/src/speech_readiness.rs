//! Whether a voice shortcut can be turned into text right now, and what to say
//! when it cannot.
//!
//! First-run setup starts the speech model downloading and then tours the app,
//! so the first thing a new user does after setup is often press the dictation
//! keys while a few hundred megabytes are still arriving. Before this guard the
//! press opened a recording that could never be transcribed: the pill listened,
//! then failed, and the only explanation was a "model not downloaded" toast in
//! a window the user was not looking at. Now the press is refused up front and
//! the overlay says what is happening, with the download's own progress.
//!
//! The decision is a pure function over a snapshot ([`resolve`]) so every case
//! is a unit test; [`allow_voice_input`] is the thin part that reads the app's
//! state and draws the pill.

use crate::managers::model::ModelManager;
use crate::settings::get_settings;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// What a voice shortcut can do right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechReadiness {
    /// Go ahead. This includes cases the pipeline already handles on its own
    /// (a cloud engine, or a downloaded model the selection has not caught up
    /// with yet): the guard only refuses what is certain to fail.
    Ready,
    /// A speech model is on its way. Carries the catalog id whose
    /// `model-download-progress` events the overlay follows.
    Downloading { model_id: String },
    /// No speech model on disk and none coming.
    Missing,
}

/// The parts of a catalog entry the decision reads.
#[derive(Debug, Clone, Copy)]
pub struct ModelFacts<'a> {
    pub id: &'a str,
    pub is_transcription: bool,
    pub is_downloaded: bool,
    pub is_downloading: bool,
}

/// Decide from a snapshot. `selected` is the `selected_model` setting.
///
/// Order matters:
/// 1. A complete cloud configuration needs nothing on disk.
/// 2. A downloaded selection is the normal case.
/// 3. A download in flight wins over "another model is on disk": the setup
///    queue only selects a model once its file lands, so during first-run
///    setup the selection still names the default while the user's actual
///    choice downloads.
/// 4. Any other downloaded speech model means the selection is merely stale,
///    which the model store corrects on its own; refusing would be wrong.
pub fn resolve(cloud_active: bool, selected: &str, models: &[ModelFacts<'_>]) -> SpeechReadiness {
    if cloud_active {
        return SpeechReadiness::Ready;
    }
    let speech = || models.iter().filter(|m| m.is_transcription);
    if speech().any(|m| m.id == selected && m.is_downloaded) {
        return SpeechReadiness::Ready;
    }
    let downloading = speech()
        .find(|m| m.id == selected && m.is_downloading)
        .or_else(|| speech().find(|m| m.is_downloading));
    if let Some(model) = downloading {
        return SpeechReadiness::Downloading {
            model_id: model.id.to_string(),
        };
    }
    if speech().any(|m| m.is_downloaded) {
        return SpeechReadiness::Ready;
    }
    SpeechReadiness::Missing
}

/// The readiness of this app, read from its settings and model catalog.
pub fn current(app: &AppHandle) -> SpeechReadiness {
    let settings = get_settings(app);
    let cloud = crate::stt_cloud::cloud_stt_active(&settings);
    if cloud {
        return SpeechReadiness::Ready;
    }
    let Some(manager) = app.try_state::<Arc<ModelManager>>() else {
        // Too early in startup to know. Let the pipeline report its own error.
        return SpeechReadiness::Ready;
    };
    let models = manager.get_available_models();
    let facts: Vec<ModelFacts<'_>> = models
        .iter()
        .map(|m| ModelFacts {
            id: &m.id,
            is_transcription: m.engine_type.is_transcription(),
            is_downloaded: m.is_downloaded,
            is_downloading: m.is_downloading,
        })
        .collect();
    resolve(false, &settings.selected_model, &facts)
}

/// Gate for every shortcut that records speech. Returns false, and puts the
/// reason on the overlay, when the recording could not be transcribed.
pub fn allow_voice_input(app: &AppHandle) -> bool {
    match current(app) {
        SpeechReadiness::Ready => true,
        SpeechReadiness::Downloading { model_id } => {
            log::info!("Voice shortcut ignored: speech model '{model_id}' is still downloading");
            crate::overlay::show_speech_download_overlay(app, &model_id);
            false
        }
        SpeechReadiness::Missing => {
            log::info!("Voice shortcut ignored: no speech model is installed");
            crate::overlay::show_overlay_notice(app, "noSpeechModel");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model<'a>(id: &'a str, downloaded: bool, downloading: bool) -> ModelFacts<'a> {
        ModelFacts {
            id,
            is_transcription: true,
            is_downloaded: downloaded,
            is_downloading: downloading,
        }
    }

    fn llm<'a>(id: &'a str, downloaded: bool, downloading: bool) -> ModelFacts<'a> {
        ModelFacts {
            is_transcription: false,
            ..model(id, downloaded, downloading)
        }
    }

    #[test]
    fn cloud_needs_nothing_on_disk() {
        assert_eq!(resolve(true, "", &[]), SpeechReadiness::Ready);
        assert_eq!(
            resolve(true, "parakeet", &[model("parakeet", false, true)]),
            SpeechReadiness::Ready
        );
    }

    #[test]
    fn downloaded_selection_is_ready() {
        let models = [model("parakeet", true, false)];
        assert_eq!(resolve(false, "parakeet", &models), SpeechReadiness::Ready);
    }

    #[test]
    fn selection_still_downloading_is_reported() {
        let models = [model("parakeet", false, true)];
        assert_eq!(
            resolve(false, "parakeet", &models),
            SpeechReadiness::Downloading {
                model_id: "parakeet".into()
            }
        );
    }

    #[test]
    fn setup_choice_downloading_behind_the_default_selection() {
        // Fresh install: the default is selected but the user chose the
        // multilingual model, which is the one arriving.
        let models = [
            model("parakeet", false, false),
            model("nemotron", false, true),
        ];
        assert_eq!(
            resolve(false, "parakeet", &models),
            SpeechReadiness::Downloading {
                model_id: "nemotron".into()
            }
        );
    }

    #[test]
    fn the_selected_download_is_preferred_over_another() {
        let models = [
            model("nemotron", false, true),
            model("parakeet", false, true),
        ];
        assert_eq!(
            resolve(false, "parakeet", &models),
            SpeechReadiness::Downloading {
                model_id: "parakeet".into()
            }
        );
    }

    #[test]
    fn a_download_wins_over_a_stale_selection() {
        // An old model is on disk but the user is switching to a new one; the
        // new one is not selected until it lands.
        let models = [
            model("whisper-small", true, false),
            model("parakeet", false, true),
        ];
        assert_eq!(
            resolve(false, "parakeet", &models),
            SpeechReadiness::Downloading {
                model_id: "parakeet".into()
            }
        );
    }

    #[test]
    fn a_stale_selection_with_another_model_on_disk_is_left_alone() {
        let models = [
            model("parakeet", false, false),
            model("whisper-small", true, false),
        ];
        assert_eq!(resolve(false, "parakeet", &models), SpeechReadiness::Ready);
    }

    #[test]
    fn nothing_on_disk_and_nothing_coming_is_missing() {
        let models = [
            model("parakeet", false, false),
            model("nemotron", false, false),
        ];
        assert_eq!(
            resolve(false, "parakeet", &models),
            SpeechReadiness::Missing
        );
        assert_eq!(resolve(false, "", &[]), SpeechReadiness::Missing);
    }

    #[test]
    fn other_kinds_of_model_do_not_count() {
        // An assistant model downloading, or on disk, cannot transcribe.
        let models = [
            model("parakeet", false, false),
            llm("gemma-4-e2b", false, true),
            llm("speakoflow-mini", true, false),
        ];
        assert_eq!(
            resolve(false, "parakeet", &models),
            SpeechReadiness::Missing
        );
        // …even when its id is (wrongly) the selection.
        assert_eq!(
            resolve(false, "speakoflow-mini", &models),
            SpeechReadiness::Missing
        );
    }
}
