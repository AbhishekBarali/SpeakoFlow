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
//! An upgrade can leave the same dead end behind: a user who dictated with one
//! of the ONNX models keeps it selected after its engine was removed (see
//! `managers::legacy_models`), because silently switching to some other model
//! would hide that their model has a new version. Its new version starts
//! downloading at launch ([`start_retired_replacement`]) and is switched to
//! when it lands, so a press meanwhile is refused with the download's progress
//! on the pill. Only when nothing is on its way (offline, a failed or cancelled
//! download, no new version) does the pill name the model and the main window
//! open on the prompt that downloads its new version.
//!
//! The decision is a pure function over a snapshot ([`resolve`]) so every case
//! is a unit test; [`allow_voice_input`] is the thin part that reads the app's
//! state and draws the pill.

use crate::managers::model::ModelManager;
use crate::settings::get_settings;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// Tells the main window to open the Models page's speech tab, where the
/// selected model's update prompt is. A plain event, like `open-feedback`:
/// it carries nothing, the webview reads the selection from its own store.
pub const SPEECH_UPDATE_EVENT: &str = "speech-model-update-needed";

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
    /// The selected model used an engine SpeakoFlow no longer includes, and
    /// nothing that could replace it is downloading. Carries the name the
    /// pill shows and the catalog id of its new version, when there is one.
    NeedsUpdate {
        model_name: String,
        replacement_id: Option<String>,
    },
}

/// The parts of a catalog entry the decision reads.
#[derive(Debug, Clone, Copy)]
pub struct ModelFacts<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub is_transcription: bool,
    pub is_downloaded: bool,
    pub is_downloading: bool,
    /// Its engine is gone: it is never loaded, whatever is on disk.
    pub is_retired: bool,
    /// For an old model, the catalog model that replaces it.
    pub replacement_id: Option<&'a str>,
}

/// Decide from a snapshot. `selected` is the `selected_model` setting.
///
/// Order matters:
/// 1. A complete cloud configuration needs nothing on disk.
/// 2. A retired selection can never run. Its new version downloading is
///    reported like any download (it is selected when it lands), then any
///    other speech download (the model store selects that one too); with
///    nothing on its way the press needs the update.
/// 3. A downloaded selection is the normal case.
/// 4. A download in flight wins over "another model is on disk": the setup
///    queue only selects a model once its file lands, so during first-run
///    setup the selection still names the default while the user's actual
///    choice downloads.
/// 5. Any other downloaded speech model means the selection is merely stale,
///    which the model store corrects on its own; refusing would be wrong.
///
/// A retired model never counts as a usable one in 3 to 5: its files being on
/// disk does not make it something the pipeline can run.
pub fn resolve(cloud_active: bool, selected: &str, models: &[ModelFacts<'_>]) -> SpeechReadiness {
    if cloud_active {
        return SpeechReadiness::Ready;
    }
    let speech = || models.iter().filter(|m| m.is_transcription);
    let usable = || speech().filter(|m| !m.is_retired);

    if let Some(retired) = speech().find(|m| m.id == selected && m.is_retired) {
        let arriving = retired
            .replacement_id
            .and_then(|id| usable().find(|m| m.id == id && m.is_downloading))
            .or_else(|| usable().find(|m| m.is_downloading));
        if let Some(model) = arriving {
            return SpeechReadiness::Downloading {
                model_id: model.id.to_string(),
            };
        }
        return SpeechReadiness::NeedsUpdate {
            model_name: retired.name.to_string(),
            replacement_id: retired.replacement_id.map(str::to_string),
        };
    }

    if usable().any(|m| m.id == selected && m.is_downloaded) {
        return SpeechReadiness::Ready;
    }
    let downloading = usable()
        .find(|m| m.id == selected && m.is_downloading)
        .or_else(|| usable().find(|m| m.is_downloading));
    if let Some(model) = downloading {
        return SpeechReadiness::Downloading {
            model_id: model.id.to_string(),
        };
    }
    if usable().any(|m| m.is_downloaded) {
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
            name: &m.name,
            is_transcription: m.engine_type.is_transcription(),
            is_downloaded: m.is_downloaded,
            is_downloading: m.is_downloading,
            is_retired: m.is_retired,
            replacement_id: m.replacement_id.as_deref(),
        })
        .collect();
    resolve(false, &settings.selected_model, &facts)
}

/// Why the local engine cannot run the selected model, when that model is
/// retired: the same sentence the loader would fail with
/// (`retired_engine_message`), so a meeting, a History retranscription and a
/// failed load all say one thing. `None` when the local engine is not what
/// would run (cloud transcription is set up) or the selection can run.
///
/// Unlike [`current`], a new version that is still downloading does not make
/// this `None`: those callers transcribe now, not after the download.
pub fn retired_selection_message(app: &AppHandle) -> Option<String> {
    if crate::stt_cloud::cloud_stt_active(&get_settings(app)) {
        return None;
    }
    let manager = app.try_state::<Arc<ModelManager>>()?;
    let model = manager.retired_selection()?;
    Some(crate::managers::transcription::retired_engine_message(
        &model.name,
    ))
}

/// Start downloading the selected model's new version when the selection is
/// retired, and switch to it when it lands. Called once at startup.
///
/// Without this, someone who updated the app found dictation refused and had
/// to find and press "Download new version" first. Now the download is
/// already running when they first dictate, so the pill shows its progress
/// (`SpeechReadiness::Downloading`) and dictation works again on its own. It
/// is the download that button starts (same progress events, mirror fallback
/// and checksum), so the prompt shows it in place; if it fails (offline, or
/// cancelled), the prompt offers it again.
///
/// Skipped when cloud transcription is set up: the local model is then only
/// a fallback, which is not worth several hundred megabytes unasked.
pub fn start_retired_replacement(app: &AppHandle) {
    if crate::stt_cloud::cloud_stt_active(&get_settings(app)) {
        return;
    }
    let Some(manager) = app.try_state::<Arc<ModelManager>>() else {
        return;
    };
    let manager = Arc::clone(&manager);
    let Some(retired) = manager.retired_selection() else {
        return;
    };
    let Some(replacement) = retired
        .replacement_id
        .as_deref()
        .and_then(|id| manager.get_model_info(id))
    else {
        return;
    };
    if replacement.is_downloaded || replacement.is_downloading {
        return;
    }
    log::info!(
        "Selected speech model '{}' used a removed engine; downloading its new version '{}'",
        retired.id,
        replacement.id
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match manager.download_model(&replacement.id).await {
            Ok(()) => {
                // Unless the user picked something else in the meantime.
                if get_settings(&app).selected_model != retired.id {
                    return;
                }
                let switched = tauri::async_runtime::spawn_blocking(move || {
                    crate::commands::models::switch_active_model(&app, &replacement.id)
                })
                .await;
                match switched {
                    Ok(Ok(())) => log::info!("Switched to the new version of '{}'", retired.id),
                    Ok(Err(e)) => log::warn!("Could not switch to the new speech model: {e}"),
                    Err(e) => log::warn!("Could not switch to the new speech model: {e}"),
                }
            }
            Err(e) => {
                let error = e.to_string();
                log::warn!(
                    "Downloading the new version of '{}' failed: {error}",
                    retired.id
                );
                if error != crate::managers::model::DOWNLOAD_CANCELLED_ERROR {
                    let _ = app.emit(
                        "model-download-failed",
                        serde_json::json!({ "model_id": &replacement.id, "error": error }),
                    );
                }
            }
        }
    });
}

/// Bring the main window forward on the update prompt.
///
/// Showing and focusing a window belongs on the main thread; shortcut actions
/// run on the keyboard engine's thread (see `assistant.rs` on why window work
/// is queued rather than done there).
fn open_update_prompt(app: &AppHandle) {
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        crate::show_main_window(&handle);
        if let Err(e) = handle.emit(SPEECH_UPDATE_EVENT, ()) {
            log::warn!("Could not open the speech model update prompt: {e}");
        }
    });
    if let Err(e) = queued {
        log::warn!("Could not queue the speech model update prompt: {e}");
    }
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
        SpeechReadiness::NeedsUpdate {
            model_name,
            replacement_id,
        } => {
            log::info!(
                "Voice shortcut ignored: speech model '{model_name}' used a removed engine \
                 (new version: {})",
                replacement_id.as_deref().unwrap_or("none")
            );
            crate::overlay::show_speech_update_overlay(app, &model_name);
            open_update_prompt(app);
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
            name: id,
            is_transcription: true,
            is_downloaded: downloaded,
            is_downloading: downloading,
            is_retired: false,
            replacement_id: None,
        }
    }

    fn llm<'a>(id: &'a str, downloaded: bool, downloading: bool) -> ModelFacts<'a> {
        ModelFacts {
            is_transcription: false,
            ..model(id, downloaded, downloading)
        }
    }

    /// An ONNX model from before the engine was removed, on disk.
    fn retired<'a>(id: &'a str, name: &'a str, replacement: Option<&'a str>) -> ModelFacts<'a> {
        ModelFacts {
            name,
            is_retired: true,
            replacement_id: replacement,
            ..model(id, true, false)
        }
    }

    const OLD: &str = "parakeet-tdt-0.6b-v3";
    const NEW: &str = "parakeet-tdt-0.6b-v3-gguf";

    fn needs_update(replacement: Option<&str>) -> SpeechReadiness {
        SpeechReadiness::NeedsUpdate {
            model_name: "Parakeet V3".into(),
            replacement_id: replacement.map(str::to_string),
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

    #[test]
    fn a_retired_selection_on_disk_needs_its_update() {
        let models = [
            retired(OLD, "Parakeet V3", Some(NEW)),
            model(NEW, false, false),
        ];
        assert_eq!(resolve(false, OLD, &models), needs_update(Some(NEW)));
    }

    #[test]
    fn a_retired_selection_is_never_ready_beside_a_working_model() {
        // The stale-selection rule must not apply: the pipeline would load the
        // retired selection, not the other model, and fail.
        let models = [
            retired(OLD, "Parakeet V3", Some(NEW)),
            model("whisper-small-gguf", true, false),
        ];
        assert_eq!(resolve(false, OLD, &models), needs_update(Some(NEW)));
    }

    #[test]
    fn a_retired_selection_whose_new_version_is_on_disk_still_needs_switching() {
        // The new version being downloaded does not run until it is selected;
        // the prompt then offers "Use new version".
        let models = [
            retired(OLD, "Parakeet V3", Some(NEW)),
            model(NEW, true, false),
        ];
        assert_eq!(resolve(false, OLD, &models), needs_update(Some(NEW)));
    }

    #[test]
    fn a_retired_selection_with_its_files_gone_still_needs_its_update() {
        let models = [ModelFacts {
            is_downloaded: false,
            ..retired(OLD, "Parakeet V3", Some(NEW))
        }];
        assert_eq!(resolve(false, OLD, &models), needs_update(Some(NEW)));
    }

    #[test]
    fn a_retired_selection_without_a_replacement_names_none() {
        let models = [retired("an-old-onnx-model", "Parakeet V3", None)];
        assert_eq!(
            resolve(false, "an-old-onnx-model", &models),
            needs_update(None)
        );
    }

    #[test]
    fn the_new_version_downloading_is_followed() {
        let models = [
            retired(OLD, "Parakeet V3", Some(NEW)),
            model("nemotron", false, true),
            model(NEW, false, true),
        ];
        assert_eq!(
            resolve(false, OLD, &models),
            SpeechReadiness::Downloading {
                model_id: NEW.into()
            }
        );
    }

    #[test]
    fn another_model_chosen_instead_is_followed_while_it_downloads() {
        // "Choose another model": the store selects it when it lands.
        let models = [
            retired(OLD, "Parakeet V3", Some(NEW)),
            model(NEW, false, false),
            model("nemotron", false, true),
        ];
        assert_eq!(
            resolve(false, OLD, &models),
            SpeechReadiness::Downloading {
                model_id: "nemotron".into()
            }
        );
    }

    #[test]
    fn cloud_transcription_does_not_care_about_a_retired_selection() {
        let models = [retired(OLD, "Parakeet V3", Some(NEW))];
        assert_eq!(resolve(true, OLD, &models), SpeechReadiness::Ready);
    }

    #[test]
    fn a_retired_model_on_disk_does_not_count_as_a_working_one() {
        // The selection's file was deleted; the only thing on disk is an old
        // model that cannot run, so there is nothing to dictate with.
        let models = [
            model(NEW, false, false),
            retired(OLD, "Parakeet V3", Some(NEW)),
        ];
        assert_eq!(resolve(false, NEW, &models), SpeechReadiness::Missing);
    }
}
