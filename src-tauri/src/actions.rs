#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::apple_intelligence;
use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::audio_toolkit::{is_microphone_access_denied, is_no_input_device_error};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{
    get_settings, AppSettings, ModelUnloadTimeout, APPLE_INTELLIGENCE_PROVIDER_ID,
};
use crate::shortcut;
use crate::tray::{change_tray_icon, TrayIconState};
use crate::utils::{
    self, show_processing_overlay, show_recording_overlay, show_transcribing_overlay,
};
use crate::TranscriptionCoordinator;
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use log::{debug, error, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tauri::Manager;
use tauri::{AppHandle, Emitter};

#[derive(Clone, serde::Serialize)]
struct RecordingErrorEvent {
    error_type: String,
    detail: Option<String>,
}

/// Drop guard that notifies the [`TranscriptionCoordinator`] when the
/// transcription pipeline finishes — whether it completes normally or panics.
struct FinishGuard(AppHandle);
impl Drop for FinishGuard {
    fn drop(&mut self) {
        // The whole pipeline (recording + transcription + any assistant
        // generation) is done, so drop the cancel shortcut here rather than at
        // recording-stop. Keeping it registered through generation is what lets
        // Esc abort a streaming assistant answer, not just a recording.
        shortcut::unregister_cancel_shortcut(&self.0);
        if let Some(c) = self.0.try_state::<TranscriptionCoordinator>() {
            c.notify_processing_finished();
        }
        // Catch-all release of any live-transcription streaming worker. The
        // early-exit paths in TranscribeAction::stop (empty samples, no samples
        // returned, or a transcription error) never call finalize_stream(),
        // which would otherwise orphan the worker — leaking its thread and the
        // leased model and leaving the router stuck open, breaking streaming for
        // later recordings until restart. cancel_stream() is a guaranteed no-op
        // when no stream is active, and finalize_stream() already take()s the
        // router on the success path, so this only ever releases a worker that
        // was never finalized. Harmless for AssistantAction, which never starts
        // a stream. The guard drops after finalize_stream()/paste, so a
        // still-wanted stream is never cancelled.
        if let Some(tm) = self.0.try_state::<Arc<TranscriptionManager>>() {
            tm.cancel_stream();
        }
    }
}

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
}

// Transcribe Action
struct TranscribeAction {
    post_process: bool,
}

/// Field name for structured output JSON schema
const TRANSCRIPTION_FIELD: &str = "transcription";

/// Strip invisible Unicode characters that some LLMs may insert
fn strip_invisible_chars(s: &str) -> String {
    s.replace(['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}'], "")
}

/// Build a system prompt from the user's prompt template.
/// Removes `${output}` placeholder since the transcription is sent as the user message.
fn build_system_prompt(prompt_template: &str) -> String {
    prompt_template.replace("${output}", "").trim().to_string()
}

/// Append the optional tone directive as an explicit, highest-priority override
/// so it wins over a cleanup prompt that insists on "don't paraphrase / output
/// exactly / keep the wording" — that conflict is why tone previously appeared
/// to do nothing. No-op for `PostProcessTone::None`. Used by both the
/// structured-output and legacy paths so tone behaves identically either way.
fn append_tone_directive(prompt: &mut String, tone: crate::settings::PostProcessTone) {
    if let Some(directive) = tone.directive() {
        prompt.push_str(
            "\n\n---\nTONE (highest priority — this overrides any earlier instruction to keep the exact wording, word order, or formality):\n",
        );
        prompt.push_str(directive);
    }
}

/// Clean an LLM's post-processing output before it's pasted. A deterministic
/// safety net that does NOT depend on the model obeying the prompt: weak/local
/// models sometimes echo the prompt's `<transcript>` wrapper verbatim, wrap the
/// answer in a Markdown code fence, or add stray surrounding whitespace. None of
/// that should ever land in the user's document. Also removes the zero-width
/// characters some models insert. Only the exact `<transcript>` wrapper tags are
/// stripped — never arbitrary angle-bracket text the speaker may have dictated.
fn sanitize_post_process_output(s: &str) -> String {
    let mut text = strip_invisible_chars(s).trim().to_string();

    // Strip a single surrounding Markdown code fence: ```lang\n … \n``` (or a
    // one-line ```…```). Only when the whole output is fenced, which is a model
    // artifact — dictated text virtually never both starts and ends with ```.
    if text.starts_with("```") && text.ends_with("```") && text.len() > 6 {
        let after_open = &text[3..];
        let body = match after_open.find('\n') {
            Some(nl) => &after_open[nl + 1..],
            None => after_open,
        };
        let body = body.strip_suffix("```").unwrap_or(body);
        text = body.trim().to_string();
    }

    // Remove the literal <transcript> wrapper tags that weak models copy from
    // the prompt. `str::replace` is UTF-8 safe and only matches the exact tags.
    for tag in [
        "<transcript>",
        "</transcript>",
        "<TRANSCRIPT>",
        "</TRANSCRIPT>",
    ] {
        text = text.replace(tag, "");
    }

    text.trim().to_string()
}

/// Kick off loading the built-in LLM engine in the background so its (slow)
/// first-time load overlaps with recording + transcription instead of blocking
/// the first response. Errors are ignored here; the real request path surfaces
/// them and retries. No-op unless the built-in provider is the active one.
fn prewarm_builtin_llm(app: &AppHandle, model: String) {
    let manager = app
        .state::<Arc<crate::managers::local_llm::LocalLlmManager>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = manager.ensure_running(&model).await {
            debug!(
                "Built-in LLM prewarm failed (will retry on first use): {}",
                e
            );
        }
    });
}

/// Pick the provider + model used for dictation post-processing. Prefers the
/// explicit post-process selection; when that isn't configured, falls back to
/// the assistant's provider + model. The assistant and post-processing share
/// the same provider catalog and API-key store, so this "just works" out of the
/// box — and on the built-in local engine it reuses the model that's already
/// loaded, avoiding a GGUF reload when the user bounces between chat and
/// dictation. Returns `None` only when neither is configured.
fn resolve_post_process_provider_and_model(
    settings: &AppSettings,
) -> Option<(crate::settings::PostProcessProvider, String)> {
    if let Some(provider) = settings.active_post_process_provider() {
        if let Some(model) = settings
            .post_process_models
            .get(&provider.id)
            .filter(|m| !m.trim().is_empty())
        {
            return Some((provider.clone(), model.clone()));
        }
    }
    // Fall back to the assistant's provider + model (shared catalog + keys).
    let provider = settings.active_assistant_provider()?;
    let model = settings
        .assistant_models
        .get(&provider.id)
        .filter(|m| !m.trim().is_empty())?;
    debug!("Post-processing reusing the assistant's provider/model (no dedicated fix model set)");
    Some((provider.clone(), model.clone()))
}

async fn post_process_transcription(
    app: &AppHandle,
    settings: &AppSettings,
    transcription: &str,
) -> Option<String> {
    let (provider, model) = match resolve_post_process_provider_and_model(settings) {
        Some(pair) => pair,
        None => {
            debug!(
                "Post-processing skipped: no dedicated fix model and no assistant model configured"
            );
            return None;
        }
    };

    let selected_prompt_id = match &settings.post_process_selected_prompt_id {
        Some(id) => id.clone(),
        None => {
            debug!("Post-processing skipped because no prompt is selected");
            return None;
        }
    };

    let prompt = match settings
        .post_process_prompts
        .iter()
        .find(|prompt| prompt.id == selected_prompt_id)
    {
        Some(prompt) => prompt.prompt.clone(),
        None => {
            debug!(
                "Post-processing skipped because prompt '{}' was not found",
                selected_prompt_id
            );
            return None;
        }
    };

    if prompt.trim().is_empty() {
        debug!("Post-processing skipped because the selected prompt is empty");
        return None;
    }

    debug!(
        "Starting LLM post-processing with provider '{}' (model: {})",
        provider.id, model
    );

    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    // The built-in provider is backed by the bundled llama.cpp engine; make
    // sure it is running and serving the selected model before requesting.
    // Built-in provider: ensure the engine is running, then hold an activity
    // guard for the rest of the request so the idle watcher won't unload it
    // mid-generation.
    let _llm_activity_guard = if provider.id == "builtin" {
        let manager = app.state::<std::sync::Arc<crate::managers::local_llm::LocalLlmManager>>();
        if let Err(e) = manager.ensure_running(&model).await {
            error!(
                "Built-in LLM engine failed to start for post-processing: {}",
                e
            );
            return None;
        }
        Some(manager.begin_request())
    } else {
        None
    };

    // Disable reasoning for providers where post-processing rarely benefits from it.
    // - custom: top-level reasoning_effort (works for local OpenAI-compat servers)
    // - openrouter: nested reasoning object; exclude:true also keeps reasoning text
    //   out of the response so it can't pollute structured-output JSON parsing
    let (reasoning_effort, reasoning) = match provider.id.as_str() {
        "custom" => (Some("none".to_string()), None),
        "openrouter" => (
            None,
            Some(crate::llm_client::ReasoningConfig {
                effort: Some("none".to_string()),
                exclude: Some(true),
            }),
        ),
        _ => (None, None),
    };

    // Instructions become the SYSTEM message and the raw transcript the USER
    // message — for BOTH structured and non-structured providers. Sending the
    // transcript as its own turn (instead of concatenating it onto the
    // instructions in one blob) is what makes weak/local models behave: given a
    // single message that opens with a wall of raw text, they tend to echo it
    // back verbatim — `<transcript>` tags and all. `build_system_prompt` strips
    // the legacy `${output}` placeholder, so older custom prompts that used it
    // keep working (the transcript still arrives as the user turn).
    let mut system_prompt = build_system_prompt(&prompt);
    // Optional tone directive (formal/casual/…), appended as an explicit
    // highest-priority override so it can't be smothered by the cleanup prompt.
    append_tone_directive(&mut system_prompt, settings.post_process_tone);
    let user_content = transcription.to_string();

    // Apple Intelligence uses native Swift APIs rather than the HTTP client.
    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            if !apple_intelligence::check_apple_intelligence_availability() {
                debug!("Apple Intelligence selected but not currently available on this device");
                return None;
            }

            let token_limit = model.trim().parse::<i32>().unwrap_or(0);
            return match apple_intelligence::process_text_with_system_prompt(
                &system_prompt,
                &user_content,
                token_limit,
            ) {
                Ok(result) => {
                    let result = sanitize_post_process_output(&result);
                    if result.is_empty() {
                        debug!("Apple Intelligence returned an empty response");
                        None
                    } else {
                        debug!(
                            "Apple Intelligence post-processing succeeded. Output length: {} chars",
                            result.len()
                        );
                        Some(result)
                    }
                }
                Err(err) => {
                    error!("Apple Intelligence post-processing failed: {}", err);
                    None
                }
            };
        }

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            debug!("Apple Intelligence provider selected on unsupported platform");
            return None;
        }
    }

    // Structured-output providers: ask for a JSON object and extract the
    // `transcription` field. On any failure, fall through to a plain-text
    // request below rather than losing the turn.
    if provider.supports_structured_output {
        debug!("Using structured outputs for provider '{}'", provider.id);

        // Define JSON schema for transcription output
        let json_schema = serde_json::json!({
            "type": "object",
            "properties": {
                (TRANSCRIPTION_FIELD): {
                    "type": "string",
                    "description": "The cleaned and processed transcription text"
                }
            },
            "required": [TRANSCRIPTION_FIELD],
            "additionalProperties": false
        });

        match crate::llm_client::send_chat_completion_with_schema(
            &provider,
            api_key.clone(),
            &model,
            user_content.clone(),
            Some(system_prompt.clone()),
            Some(json_schema),
            reasoning_effort.clone(),
            reasoning.clone(),
        )
        .await
        {
            Ok(Some(content)) => {
                // Parse the JSON response to extract the transcription field
                match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(json) => {
                        if let Some(transcription_value) =
                            json.get(TRANSCRIPTION_FIELD).and_then(|t| t.as_str())
                        {
                            let result = sanitize_post_process_output(transcription_value);
                            debug!(
                                "Structured output post-processing succeeded for provider '{}'. Output length: {} chars",
                                provider.id,
                                result.len()
                            );
                            return Some(result);
                        } else {
                            error!("Structured output response missing 'transcription' field");
                            return Some(sanitize_post_process_output(&content));
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to parse structured output JSON: {}. Returning raw content.",
                            e
                        );
                        return Some(sanitize_post_process_output(&content));
                    }
                }
            }
            Ok(None) => {
                error!("LLM API response has no content");
                return None;
            }
            Err(e) => {
                warn!(
                    "Structured output failed for provider '{}': {}. Falling back to a plain-text request.",
                    provider.id, e
                );
                // Fall through to the plain-text request below.
            }
        }
    }

    // Plain-text request: non-structured providers (built-in local, Groq, …) and
    // the structured fallback above. Same system+user shape, just no JSON schema.
    match crate::llm_client::send_chat_completion_with_schema(
        &provider,
        api_key,
        &model,
        user_content,
        Some(system_prompt),
        None,
        reasoning_effort,
        reasoning,
    )
    .await
    {
        Ok(Some(content)) => {
            let content = sanitize_post_process_output(&content);
            debug!(
                "LLM post-processing succeeded for provider '{}'. Output length: {} chars",
                provider.id,
                content.len()
            );
            Some(content)
        }
        Ok(None) => {
            error!("LLM API response has no content");
            None
        }
        Err(e) => {
            error!(
                "LLM post-processing failed for provider '{}': {}. Falling back to original transcription.",
                provider.id,
                e
            );
            None
        }
    }
}

async fn maybe_convert_chinese_variant(
    settings: &AppSettings,
    transcription: &str,
) -> Option<String> {
    // Check if language is set to Simplified or Traditional Chinese
    let is_simplified = settings.selected_language == "zh-Hans";
    let is_traditional = settings.selected_language == "zh-Hant";

    if !is_simplified && !is_traditional {
        debug!("selected_language is not Simplified or Traditional Chinese; skipping translation");
        return None;
    }

    debug!(
        "Starting Chinese translation using OpenCC for language: {}",
        settings.selected_language
    );

    // Use OpenCC to convert based on selected language
    let config = if is_simplified {
        // Convert Traditional Chinese to Simplified Chinese
        BuiltinConfig::Tw2sp
    } else {
        // Convert Simplified Chinese to Traditional Chinese
        BuiltinConfig::S2tw
    };

    match OpenCC::from_config(config) {
        Ok(converter) => {
            let converted = converter.convert(transcription);
            debug!(
                "OpenCC translation completed. Input length: {}, Output length: {}",
                transcription.len(),
                converted.len()
            );
            Some(converted)
        }
        Err(e) => {
            error!("Failed to initialize OpenCC converter: {}. Falling back to original transcription.", e);
            None
        }
    }
}

pub(crate) struct ProcessedTranscription {
    pub final_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
}

pub(crate) async fn process_transcription_output(
    app: &AppHandle,
    transcription: &str,
    post_process: bool,
) -> ProcessedTranscription {
    let settings = get_settings(app);
    let mut final_text = transcription.to_string();
    let mut post_processed_text: Option<String> = None;
    let mut post_process_prompt: Option<String> = None;

    if let Some(converted_text) = maybe_convert_chinese_variant(&settings, transcription).await {
        final_text = converted_text;
    }

    if post_process {
        // Cap post-processing so a slow or stuck LLM can never hold up the
        // paste. On timeout — or any failure inside — we fall through and paste
        // the raw transcription, so the user never loses their words.
        let timeout =
            std::time::Duration::from_secs(settings.post_process_timeout_secs.max(1) as u64);
        match tokio::time::timeout(
            timeout,
            post_process_transcription(app, &settings, &final_text),
        )
        .await
        {
            Ok(Some(processed_text)) => {
                post_processed_text = Some(processed_text.clone());
                final_text = processed_text;

                if let Some(prompt_id) = &settings.post_process_selected_prompt_id {
                    if let Some(prompt) = settings
                        .post_process_prompts
                        .iter()
                        .find(|prompt| &prompt.id == prompt_id)
                    {
                        post_process_prompt = Some(prompt.prompt.clone());
                    }
                }
            }
            // Skipped or failed internally — keep the raw transcription.
            Ok(None) => {}
            Err(_) => {
                warn!(
                    "Post-processing timed out after {}s; pasting the raw transcription",
                    settings.post_process_timeout_secs
                );
            }
        }
    } else if final_text != transcription {
        post_processed_text = Some(final_text.clone());
    }

    // === Deterministic text replacements =================================
    // Rule-based find/replace + magic commands. This runs AFTER LLM
    // post-processing by default so hand-written, deterministic fix-ups always
    // win over the model's output. To run replacements BEFORE the LLM instead,
    // move this single block above the `if post_process` block above.
    if settings.replacements_enabled && !settings.text_replacements.is_empty() {
        let replaced =
            crate::audio_toolkit::apply_replacements(&final_text, &settings.text_replacements);
        if replaced != final_text {
            final_text = replaced;
            // Keep history's "post-processed" view aligned with what we paste.
            post_processed_text = Some(final_text.clone());
        }
    }

    ProcessedTranscription {
        final_text,
        post_processed_text,
        post_process_prompt,
    }
}

impl ShortcutAction for TranscribeAction {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        let start_time = Instant::now();
        debug!("TranscribeAction::start called for binding: {}", binding_id);

        // A fresh dictation can't be redirected by a stale Ask-Assistant click.
        crate::assistant::clear_transcribe_redirect();

        // Route the transcript: an in-app dictation (the Create-with-AI persona
        // box uses source "in-app") delivers its text to the webview via an
        // event; every other dictation pastes into the focused OS window as
        // usual. Setting/clearing here — rather than in the command — means a
        // stale in-app click can never hijack a later global dictation.
        if shortcut_str == "in-app" {
            crate::assistant::set_dictate_to_field();
        } else {
            crate::assistant::clear_dictate_to_field();
        }

        // Optionally silence a still-playing assistant reply. Off by default —
        // earphone users often want to keep listening while they dictate.
        if get_settings(app).assistant_tts_stop_on_dictation {
            crate::tts::stop_all(app);
        }

        // Load model in the background
        let tm = app.state::<Arc<TranscriptionManager>>();
        let rm = app.state::<Arc<AudioRecordingManager>>();

        // Load ASR model and VAD model in parallel
        tm.initiate_model_load();

        // Live/streaming transcription. Start the streaming worker now so it
        // waits for the model load and begins consuming frames as soon as
        // recording starts. The batch transcribe() path stays the fallback
        // (see stop()).
        //
        // Streaming is strictly capability-gated: it only ever runs for a model
        // that natively supports live streaming (e.g. Parakeet, Nemotron). For
        // such a model it's on automatically — the Auto overlay default already
        // resolves to Live, so a first-run user gets streaming with no settings
        // toggle. A model that does not support streaming never starts the
        // worker (so streaming can't be attempted on it and misbehave), even if
        // the global live-transcription toggle happens to be on.
        {
            let s = get_settings(app);
            let supports_live = crate::overlay::selected_model_supports_live(app);
            let want_stream = supports_live
                && (s.live_transcription_enabled
                    || crate::settings::resolve_overlay_style(s.overlay_style, supports_live)
                        == crate::settings::OverlayStyle::Live);
            if want_stream {
                tm.start_stream();
            }
        }

        let rm_clone = Arc::clone(&rm);
        std::thread::spawn(move || {
            if let Err(e) = rm_clone.preload_vad() {
                debug!("VAD pre-load failed: {}", e);
            }
        });

        let binding_id = binding_id.to_string();
        change_tray_icon(app, TrayIconState::Recording);
        show_recording_overlay(app);

        // Get the microphone mode to determine audio feedback timing
        let settings = get_settings(app);

        // Prewarm the built-in LLM during recording (when this action will
        // post-process with it) so its load overlaps with recording +
        // transcription instead of stalling the first response.
        if self.post_process && settings.local_llm_unload_timeout != ModelUnloadTimeout::Immediately
        {
            if let Some(provider) = settings.active_post_process_provider() {
                if provider.id == "builtin" {
                    if let Some(model) = settings.post_process_models.get("builtin") {
                        if !model.trim().is_empty() {
                            prewarm_builtin_llm(app, model.clone());
                        }
                    }
                }
            }
        }

        let is_always_on = settings.always_on_microphone;
        debug!("Microphone mode - always_on: {}", is_always_on);

        let mut recording_error: Option<String> = None;
        if is_always_on {
            // Always-on mode: Play audio feedback immediately, then apply mute after sound finishes
            debug!("Always-on mode: Playing audio feedback immediately");
            let rm_clone = Arc::clone(&rm);
            let app_clone = app.clone();
            // The blocking helper exits immediately if audio feedback is disabled,
            // so we can always reuse this thread to ensure mute happens right after playback.
            std::thread::spawn(move || {
                play_feedback_sound_blocking(&app_clone, SoundType::Start);
                rm_clone.apply_mute();
            });

            if let Err(e) = rm.try_start_recording(&binding_id) {
                debug!("Recording failed: {}", e);
                recording_error = Some(e);
            }
        } else {
            // On-demand mode: open the mic + start capture, then cue the user
            // and apply mute. The cue is played only once the microphone is
            // genuinely delivering audio (via `wait_for_capture_ready`), so a
            // slow-to-wake device (Bluetooth/USB, or a cold-started stream)
            // can't swallow the user's first words — the cue itself is the
            // "you can speak now" signal. Backport of Handy PR #1582 / #1283
            // (mic-init delay clips the first word), reconciled with
            // SpeakoFlow's capture-ready signal rather than a fixed warm-up
            // guess. Faster mic init (config caching) keeps this snappy: the
            // wait returns as soon as the first real frame arrives.
            debug!("On-demand mode: starting recording, then audio feedback");
            let recording_start_time = Instant::now();
            match rm.try_start_recording(&binding_id) {
                Ok(()) => {
                    debug!("Recording started in {:?}", recording_start_time.elapsed());
                    let app_clone = app.clone();
                    let rm_clone = Arc::clone(&rm);
                    // The blocking helper exits immediately when audio feedback
                    // is disabled, so we always reuse this thread to keep mute
                    // sequenced right after the (possible) cue.
                    std::thread::spawn(move || {
                        // Bounded so a device that never reports readiness can't
                        // hang the cue; in practice this returns within one
                        // buffer period of the mic going live.
                        rm_clone.wait_for_capture_ready(std::time::Duration::from_millis(1500));
                        play_feedback_sound_blocking(&app_clone, SoundType::Start);
                        rm_clone.apply_mute();
                    });
                }
                Err(e) => {
                    debug!("Failed to start recording: {}", e);
                    recording_error = Some(e);
                }
            }
        }

        if recording_error.is_none() {
            // Dynamically register the cancel shortcut in a separate task to avoid deadlock
            shortcut::register_cancel_shortcut(app);
        } else {
            // Starting failed (for example due to blocked microphone permissions).
            // Revert UI state so we don't stay stuck in the recording overlay.
            utils::hide_recording_overlay(app);
            change_tray_icon(app, TrayIconState::Idle);
            if let Some(err) = recording_error {
                let error_type = if is_microphone_access_denied(&err) {
                    "microphone_permission_denied"
                } else if is_no_input_device_error(&err) {
                    "no_input_device"
                } else {
                    "unknown"
                };
                let _ = app.emit(
                    "recording-error",
                    RecordingErrorEvent {
                        error_type: error_type.to_string(),
                        detail: Some(err),
                    },
                );
            }
        }

        debug!(
            "TranscribeAction::start completed in {:?}",
            start_time.elapsed()
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        // Unregister the cancel shortcut when transcription stops
        shortcut::unregister_cancel_shortcut(app);

        let stop_time = Instant::now();
        debug!("TranscribeAction::stop called for binding: {}", binding_id);

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());

        change_tray_icon(app, TrayIconState::Transcribing);
        show_transcribing_overlay(app);

        // Unmute before playing audio feedback so the stop sound is audible
        rm.remove_mute();

        // Play audio feedback for recording stop
        play_feedback_sound(app, SoundType::Stop);

        let binding_id = binding_id.to_string(); // Clone binding_id for the async task
        let post_process = self.post_process;

        tauri::async_runtime::spawn(async move {
            let _guard = FinishGuard(ah.clone());
            debug!(
                "Starting async transcription task for binding: {}",
                binding_id
            );

            let stop_recording_time = Instant::now();
            if let Some(samples) = rm.stop_recording(&binding_id) {
                debug!(
                    "Recording stopped and samples retrieved in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                if samples.is_empty() {
                    debug!("Recording produced no audio samples; skipping persistence");
                    utils::hide_recording_overlay(&ah);
                    change_tray_icon(&ah, TrayIconState::Idle);
                } else {
                    // Save WAV concurrently with transcription
                    let sample_count = samples.len();
                    let file_name = format!("speakoflow-{}.wav", chrono::Utc::now().timestamp());
                    let wav_path = hm.recordings_dir().join(&file_name);
                    let wav_path_for_verify = wav_path.clone();
                    let samples_for_wav = samples.clone();
                    let wav_handle = tauri::async_runtime::spawn_blocking(move || {
                        crate::audio_toolkit::save_wav_file(&wav_path, &samples_for_wav)
                    });

                    // Transcribe concurrently with WAV save.
                    // Live transcription: finalize the streaming worker for the
                    // merged result. finalize_stream() is a no-op returning
                    // Ok(None) when no stream is active (the default), so the
                    // batch transcribe() path is used exactly as before. It also
                    // falls back to batch when the stream produced nothing or
                    // errored/timed out, so the user never loses their words.
                    let transcription_time = Instant::now();
                    let transcription_result = match tm.finalize_stream() {
                        Ok(Some(text)) => Ok(text),
                        Ok(None) => tm.transcribe(samples),
                        Err(e) => {
                            warn!(
                                "Live transcription finalize failed ({}); using batch transcription",
                                e
                            );
                            tm.transcribe(samples)
                        }
                    };

                    // Await WAV save and verify
                    let wav_saved = match wav_handle.await {
                        Ok(Ok(())) => {
                            match crate::audio_toolkit::verify_wav_file(
                                &wav_path_for_verify,
                                sample_count,
                            ) {
                                Ok(()) => true,
                                Err(e) => {
                                    error!("WAV verification failed: {}", e);
                                    false
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            error!("Failed to save WAV file: {}", e);
                            false
                        }
                        Err(e) => {
                            error!("WAV save task panicked: {}", e);
                            false
                        }
                    };

                    match transcription_result {
                        Ok(transcription) => {
                            debug!(
                                "Transcription completed in {:?}: '{}'",
                                transcription_time.elapsed(),
                                transcription
                            );

                            // Rerouted to the assistant (the overlay's Ask-
                            // Assistant button): hand the transcript to the
                            // assistant instead of pasting it anywhere.
                            if crate::assistant::take_transcribe_redirect() {
                                utils::hide_recording_overlay(&ah);
                                change_tray_icon(&ah, TrayIconState::Idle);
                                crate::assistant::show_assistant_panel(&ah);
                                crate::assistant::run_voice_turn(ah.clone(), transcription).await;
                                return;
                            }

                            // In-app dictation (e.g. the Create-with-AI persona
                            // description box): deliver the transcript to the
                            // webview as an event so it lands in the focused
                            // in-app field reliably, without a synthetic paste
                            // or touching the OS clipboard.
                            if crate::assistant::take_dictate_to_field() {
                                utils::hide_recording_overlay(&ah);
                                change_tray_icon(&ah, TrayIconState::Idle);
                                if let Err(e) =
                                    ah.emit("dictation-transcript", transcription.clone())
                                {
                                    error!("Failed to emit dictation-transcript: {}", e);
                                }
                                return;
                            }

                            if post_process {
                                show_processing_overlay(&ah);
                            }
                            let processed =
                                process_transcription_output(&ah, &transcription, post_process)
                                    .await;

                            // Save to history if WAV was saved
                            if wav_saved {
                                if let Err(err) = hm.save_entry(
                                    file_name,
                                    transcription,
                                    post_process,
                                    processed.post_processed_text.clone(),
                                    processed.post_process_prompt.clone(),
                                ) {
                                    error!("Failed to save history entry: {}", err);
                                }
                            }

                            if processed.final_text.is_empty() {
                                utils::hide_recording_overlay(&ah);
                                change_tray_icon(&ah, TrayIconState::Idle);
                            } else {
                                let ah_clone = ah.clone();
                                let paste_time = Instant::now();
                                let final_text = processed.final_text;
                                ah.run_on_main_thread(move || {
                                    match utils::paste(final_text, ah_clone.clone()) {
                                        Ok(()) => debug!(
                                            "Text pasted successfully in {:?}",
                                            paste_time.elapsed()
                                        ),
                                        Err(e) => {
                                            error!("Failed to paste transcription: {}", e);
                                            let _ = ah_clone.emit("paste-error", ());
                                        }
                                    }
                                    utils::hide_recording_overlay(&ah_clone);
                                    change_tray_icon(&ah_clone, TrayIconState::Idle);
                                })
                                .unwrap_or_else(|e| {
                                    error!("Failed to run paste on main thread: {:?}", e);
                                    utils::hide_recording_overlay(&ah);
                                    change_tray_icon(&ah, TrayIconState::Idle);
                                });
                            }
                        }
                        Err(err) => {
                            debug!("Global Shortcut Transcription error: {}", err);
                            // Save entry with empty text so user can retry
                            if wav_saved {
                                if let Err(save_err) = hm.save_entry(
                                    file_name,
                                    String::new(),
                                    post_process,
                                    None,
                                    None,
                                ) {
                                    error!("Failed to save failed history entry: {}", save_err);
                                }
                            }
                            utils::hide_recording_overlay(&ah);
                            change_tray_icon(&ah, TrayIconState::Idle);
                        }
                    }
                }
            } else {
                debug!("No samples retrieved from recording stop");
                utils::hide_recording_overlay(&ah);
                change_tray_icon(&ah, TrayIconState::Idle);
            }
        });

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

// Cancel Action
struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        utils::cancel_current_operation(app);
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for cancel
    }
}

// Assistant Action: record → STT → LLM → stream into the assistant panel.
// Reuses TranscribeAction's record/transcribe flow but never pastes.
struct AssistantAction;

impl ShortcutAction for AssistantAction {
    fn start(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        debug!("AssistantAction::start called for binding: {}", binding_id);

        // Vision timing: when set to Immediate and the camera is armed, grab the
        // screen NOW — at the start of the question — so it reflects what the
        // user was looking at when they began, not what's on screen after they
        // stop talking. The frame is stashed and consumed by `run_voice_turn`.
        // Always clear first so a stale/cancelled capture can never leak in.
        crate::assistant::clear_immediate_capture();
        {
            let settings = get_settings(app);
            if settings.assistant_screenshot_enabled
                && settings.assistant_vision_capture_timing
                    == crate::settings::VisionCaptureTiming::Immediate
                && crate::assistant::screen_armed()
                && !settings.active_character_is_cat()
            {
                let profile = settings
                    .active_assistant_provider()
                    .map(|p| crate::screenshot::CaptureProfile::for_base_url(&p.base_url))
                    .unwrap_or(crate::screenshot::CaptureProfile::Generous);
                // Grab the monitor the mouse cursor is on right now (the screen
                // the user is working on), falling back to the primary display.
                std::thread::spawn(move || {
                    match crate::screenshot::capture_screen_data_url_at(None, profile) {
                        Ok(url) => crate::assistant::stash_immediate_capture(url),
                        Err(e) => debug!("Immediate vision capture failed: {}", e),
                    }
                });
            }
        }

        // Starting a new question interrupts the previous spoken answer — the
        // assistant must never talk over the user's next recording.
        crate::tts::stop_all(app);

        let tm = app.state::<Arc<TranscriptionManager>>();
        let rm = app.state::<Arc<AudioRecordingManager>>();

        tm.initiate_model_load();
        let rm_clone = Arc::clone(&rm);
        std::thread::spawn(move || {
            if let Err(e) = rm_clone.preload_vad() {
                debug!("VAD pre-load failed: {}", e);
            }
        });

        // Prewarm the built-in LLM during recording (when the assistant uses
        // it) so its load overlaps with recording + transcription.
        {
            let settings = get_settings(app);
            if settings.local_llm_unload_timeout != ModelUnloadTimeout::Immediately {
                if let Some(provider) = settings.active_assistant_provider() {
                    if provider.id == "builtin" {
                        if let Some(model) = settings.assistant_models.get("builtin") {
                            if !model.trim().is_empty() {
                                prewarm_builtin_llm(app, model.clone());
                            }
                        }
                    }
                }
            }
        }

        // Show the panel right away so the user sees the listening state.
        crate::assistant::show_assistant_panel(app);
        crate::assistant::emit_state(app, "listening");
        // Tell the floating panel whether this turn will capture the screen
        // so it can show a "vision" indicator. The actual capture decision is
        // re-evaluated at stop, but the dedicated vision binding always does.
        let _ = app.emit("assistant-vision-active", binding_id == "assistant_vision");

        // The assistant panel renders its own listening/transcribing state, so
        // we intentionally do NOT show the STT recording lozenge here — that
        // would put two status surfaces on screen for one voice turn.
        change_tray_icon(app, TrayIconState::Recording);

        let binding_id = binding_id.to_string();
        let mut recording_error: Option<String> = None;
        match rm.try_start_recording(&binding_id) {
            Ok(()) => {
                let app_clone = app.clone();
                let rm_clone = Arc::clone(&rm);
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    play_feedback_sound_blocking(&app_clone, SoundType::Start);
                    rm_clone.apply_mute();
                });
            }
            Err(e) => {
                debug!("Failed to start assistant recording: {}", e);
                recording_error = Some(e);
            }
        }

        if recording_error.is_none() {
            shortcut::register_cancel_shortcut(app);
        } else {
            change_tray_icon(app, TrayIconState::Idle);
            crate::assistant::emit_state(app, "idle");
            if let Some(err) = recording_error {
                let error_type = if is_microphone_access_denied(&err) {
                    "microphone_permission_denied"
                } else if is_no_input_device_error(&err) {
                    "no_input_device"
                } else {
                    "unknown"
                };
                // Mirror the failure onto the assistant surfaces (pill/panel)
                // so a voice turn that can't start is never a silent no-op.
                let assistant_code = match error_type {
                    "microphone_permission_denied" => "mic_denied",
                    "no_input_device" => "mic_unavailable",
                    _ => "mic_error",
                };
                crate::assistant::emit_error(app, assistant_code, err.clone());
                let _ = app.emit(
                    "recording-error",
                    RecordingErrorEvent {
                        error_type: error_type.to_string(),
                        detail: Some(err),
                    },
                );
            }
        }
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        // NOTE: the cancel shortcut is intentionally NOT unregistered here (as
        // it is for dictation). It stays registered through transcription and
        // the assistant's answer generation so Esc can stop a streaming reply;
        // the pipeline's FinishGuard drops it when the whole turn completes.
        debug!("AssistantAction::stop called for binding: {}", binding_id);

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());

        change_tray_icon(app, TrayIconState::Transcribing);
        crate::assistant::emit_state(app, "transcribing");

        rm.remove_mute();
        play_feedback_sound(app, SoundType::Stop);

        let binding_id = binding_id.to_string();
        tauri::async_runtime::spawn(async move {
            let _guard = FinishGuard(ah.clone());

            let samples = match rm.stop_recording(&binding_id) {
                Some(samples) if !samples.is_empty() => samples,
                _ => {
                    debug!("Assistant recording produced no audio samples");
                    change_tray_icon(&ah, TrayIconState::Idle);
                    crate::assistant::emit_state(&ah, "idle");
                    return;
                }
            };

            // Vision: the dedicated vision binding always captures; the
            // normal binding captures when the question clearly refers to
            // the screen ("what's on my display..."). Capture happens after
            // transcription so we know the intent — the screen content is
            // unchanged in those ~150ms.
            match tm.transcribe(samples) {
                Ok(transcription) => {
                    change_tray_icon(&ah, TrayIconState::Idle);
                    // Screen decision + staged attachments + turn, shared with
                    // the STT overlay's Ask-Assistant redirect.
                    crate::assistant::run_voice_turn(ah.clone(), transcription).await;
                }
                Err(err) => {
                    error!("Assistant transcription error: {}", err);
                    change_tray_icon(&ah, TrayIconState::Idle);
                    crate::assistant::emit_error(&ah, "transcription", err.to_string());
                    crate::assistant::emit_state(&ah, "idle");
                }
            }
        });
    }
}

// Assistant Panel Toggle Action: show/hide the floating panel.
struct AssistantPanelToggleAction;

impl ShortcutAction for AssistantPanelToggleAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        crate::assistant::toggle_assistant_panel(app);
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for panel toggle
    }
}

// Test Action
struct TestAction;

impl ShortcutAction for TestAction {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Started - {} (App: {})", // Changed "Pressed" to "Started" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Stopped - {} (App: {})", // Changed "Released" to "Stopped" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }
}

// Static Action Map
pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "transcribe".to_string(),
        Arc::new(TranscribeAction {
            post_process: false,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transcribe_with_post_process".to_string(),
        Arc::new(TranscribeAction { post_process: true }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "assistant".to_string(),
        Arc::new(AssistantAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "assistant_panel_toggle".to_string(),
        Arc::new(AssistantPanelToggleAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});



#[cfg(test)]
mod tests {
    use super::{append_tone_directive, build_system_prompt, sanitize_post_process_output};
    use crate::settings::PostProcessTone;

    #[test]
    fn tone_none_leaves_prompt_untouched() {
        let base = "Clean up the transcript. Do not paraphrase.".to_string();
        let mut prompt = base.clone();
        append_tone_directive(&mut prompt, PostProcessTone::None);
        assert_eq!(prompt, base, "None tone must not alter the cleanup prompt");
    }

    #[test]
    fn tone_directive_is_appended_as_override() {
        let mut prompt = "Clean up the transcript. Output exactly the cleaned text.".to_string();
        append_tone_directive(&mut prompt, PostProcessTone::Formal);

        // The tone text itself must be present…
        assert!(
            prompt.contains(PostProcessTone::Formal.directive().unwrap()),
            "formal directive text should be appended"
        );
        // …framed as an explicit, highest-priority override so it wins over a
        // "keep the exact wording" cleanup prompt (the original tone bug).
        assert!(
            prompt.contains("highest priority"),
            "directive should be framed as a priority override"
        );
        // And it must come AFTER the base cleanup prompt, not replace it.
        assert!(prompt.starts_with("Clean up the transcript."));
    }

    #[test]
    fn every_non_none_tone_has_a_directive() {
        for tone in [
            PostProcessTone::Formal,
            PostProcessTone::Casual,
            PostProcessTone::Professional,
            PostProcessTone::Friendly,
            PostProcessTone::Concise,
        ] {
            assert!(
                tone.directive().is_some_and(|d| !d.trim().is_empty()),
                "{:?} must provide a non-empty directive",
                tone
            );
        }
    }

    #[test]
    fn build_system_prompt_strips_output_placeholder() {
        let out = build_system_prompt("<transcript>\n${output}\n</transcript>\nClean it.");
        assert!(!out.contains("${output}"), "placeholder should be removed");
        assert!(out.contains("Clean it."));
    }

    #[test]
    fn sanitizer_strips_leaked_transcript_tags() {
        // The exact screenshot-1 failure: a weak model echoed the wrapper tags.
        let raw = "<transcript>one two three ten</transcript>";
        assert_eq!(sanitize_post_process_output(raw), "one two three ten");
        // Multi-line with surrounding whitespace, uppercase variant too.
        let raw2 = "\n<TRANSCRIPT>\nHello there.\n</TRANSCRIPT>\n";
        assert_eq!(sanitize_post_process_output(raw2), "Hello there.");
    }

    #[test]
    fn sanitizer_strips_surrounding_code_fence() {
        let fenced = "```\nCleaned text here.\n```";
        assert_eq!(sanitize_post_process_output(fenced), "Cleaned text here.");
        let fenced_lang = "```text\nCleaned text here.\n```";
        assert_eq!(sanitize_post_process_output(fenced_lang), "Cleaned text here.");
    }

    #[test]
    fn sanitizer_leaves_clean_text_untouched() {
        let clean = "Can you send me the report by Friday?";
        assert_eq!(sanitize_post_process_output(clean), clean);
        // A lone angle-bracket phrase the speaker dictated must NOT be mangled —
        // only the exact <transcript> wrapper tags are removed.
        let dictated = "Use the <div> tag here.";
        assert_eq!(sanitize_post_process_output(dictated), dictated);
    }
}
