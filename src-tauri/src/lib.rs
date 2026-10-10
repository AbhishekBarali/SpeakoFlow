mod actions;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod apple_intelligence;
mod assistant;
mod audio_feedback;
mod audio_tags;
pub mod audio_toolkit;
mod autolearn;
mod catalog;
pub mod cli;
mod clipboard;
mod commands;
mod dictation_recovery;
mod feedback;
mod flow;
mod helpers;
mod huggingface;
mod input;
mod llm_client;
mod managers;
mod meetings;
mod memory;
mod native_tts;
mod overlay;
mod overlay_follow;
mod overlay_lifecycle;
pub mod portable;
mod reminders;
mod screenshot;
mod secret_store;
mod selection;
mod settings;
mod shortcut;
mod signal_handle;
mod speech_readiness;
mod speech_stream;
mod stt_cloud;
#[cfg(test)]
mod stt_cloud_bench;
#[cfg(test)]
mod stt_cloud_live_tests;
mod stt_cloud_stream;
mod transcription_coordinator;
mod tray;
mod tray_i18n;
mod tts;
mod updates;
mod utils;
mod voice_conversation;
mod web_search;
#[cfg(any(windows, test))]
mod webview_prefs;
mod window_drag;

pub use cli::CliArgs;

/// WebView2 (Windows) browser arguments, applied IDENTICALLY to every window.
///
/// Why identical on every window: `wry` creates a WebView2 environment per
/// window sharing one user-data directory, and WebView2 refuses to create a
/// window whose browser args differ from an existing environment on the same
/// directory. So every `WebviewWindowBuilder` in this app MUST pass exactly this
/// string via `.additional_browser_args(crate::WEBVIEW2_BROWSER_ARGS)`.
///
/// - Preserves wry's defaults (`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`),
///   which `additional_browser_args` would otherwise REPLACE.
/// - `--enable-unsafe-webgpu` lets the in-panel Kokoro TTS run on the GPU (fp32)
///   instead of the robotic wasm/q8 fallback. If the GPU/driver can't do WebGPU,
///   kokoro-js safely falls back to wasm exactly as before (no regression).
/// - `--autoplay-policy=no-user-gesture-required` lets a spoken reply start
///   without a prior click (otherwise the WebView blocks TTS audio silently).
///
/// Has no effect on macOS (WKWebView) or Linux (WebKitGTK) — those backends
/// ignore this attribute — so it is safe to pass unconditionally.
pub const WEBVIEW2_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --enable-unsafe-webgpu --autoplay-policy=no-user-gesture-required";
#[cfg(debug_assertions)]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

use env_filter::Builder as EnvFilterBuilder;
use managers::audio::AudioRecordingManager;
use managers::history::HistoryManager;
use managers::model::ModelManager;
use managers::transcription::TranscriptionManager;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tauri::image::Image;
pub use transcription_coordinator::TranscriptionCoordinator;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use crate::settings::get_settings;

// Global atomic to store the file log level filter
// We use u8 to store the log::LevelFilter as a number
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

fn level_filter_from_u8(value: u8) -> log::LevelFilter {
    match value {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        5 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Trace,
    }
}

fn build_console_filter() -> env_filter::Filter {
    let mut builder = EnvFilterBuilder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(log::LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(log::LevelFilter::Info);
        }
    }

    builder.build()
}

fn show_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        if let Err(e) = main_window.unminimize() {
            log::error!("Failed to unminimize webview window: {}", e);
        }
        if let Err(e) = main_window.show() {
            log::error!("Failed to show webview window: {}", e);
        }
        if let Err(e) = main_window.set_focus() {
            log::error!("Failed to focus webview window: {}", e);
        }
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                log::error!("Failed to set activation policy to Regular: {}", e);
            }
        }
        return;
    }

    let webview_labels = app.webview_windows().keys().cloned().collect::<Vec<_>>();
    log::error!(
        "Main window not found. Webview labels: {:?}",
        webview_labels
    );
}

/// Persist the main window's current size (logical px) so it reopens at the
/// size the user left it. Called when the window loses focus or is closed to
/// the tray — natural, low-frequency save points, so there's no write churn
/// during a resize drag. Only the "main" window is remembered (the assistant
/// panel and overlays manage their own geometry); minimized states and no-op
/// writes are skipped.
fn save_main_window_size(window: &tauri::Window) {
    if window.label() != "main" {
        return;
    }
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let Ok(size) = window.inner_size() else {
        return;
    };
    if size.width == 0 || size.height == 0 {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let width = size.width as f64 / scale;
    let height = size.height as f64 / scale;
    let app = window.app_handle();
    let mut settings = get_settings(&app);
    if settings.main_window_width == Some(width) && settings.main_window_height == Some(height) {
        return;
    }
    settings.main_window_width = Some(width);
    settings.main_window_height = Some(height);
    crate::settings::write_settings(&app, settings);
}

#[allow(unused_variables)]
fn should_force_show_permissions_window(app: &AppHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        let model_manager = app.state::<Arc<ModelManager>>();
        let has_downloaded_models = model_manager
            .get_available_models()
            .iter()
            .any(|model| model.is_downloaded && model.engine_type.is_transcription());

        if !has_downloaded_models {
            return false;
        }

        let status = commands::audio::get_windows_microphone_permission_status();
        if status.supported && status.overall_access == commands::audio::PermissionAccess::Denied {
            log::info!(
                "Windows microphone permissions are denied; forcing main window visible for onboarding"
            );
            return true;
        }
    }

    false
}

fn initialize_core_logic(app_handle: &AppHandle) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    // Initialize the managers
    // Shared router for the opt-in live/streaming transcription path. Created
    // once and handed to BOTH the recorder (which feeds raw frames) and the
    // transcription manager (which starts/finalizes streams).
    let stream_router = Arc::new(managers::transcription::StreamRouter::new());
    let recording_manager = Arc::new(
        AudioRecordingManager::new(app_handle, stream_router.clone())
            .expect("Failed to initialize recording manager"),
    );
    let model_manager =
        Arc::new(ModelManager::new(app_handle).expect("Failed to initialize model manager"));
    // The native voice engine finds its packs through the model manager's
    // folder, and tidies an engine download no voice pack uses any more.
    native_tts::init(model_manager.models_dir().to_path_buf());
    {
        let app = app_handle.clone();
        native_tts::set_status_listener(move || commands::assistant::emit_local_voice_status(&app));
    }
    let transcription_manager = Arc::new(
        TranscriptionManager::new(app_handle, model_manager.clone(), stream_router.clone())
            .expect("Failed to initialize transcription manager"),
    );
    // Screen vision captures the monitor under the pointer; on macOS it needs
    // the app to ask where that is.
    screenshot::set_cursor_source(app_handle);
    let history_manager =
        Arc::new(HistoryManager::new(app_handle).expect("Failed to initialize history manager"));

    // Built-in local LLM engine (manages the bundled llama.cpp sidecar).
    let local_llm_manager = Arc::new(
        managers::local_llm::LocalLlmManager::new(app_handle)
            .expect("Failed to initialize local LLM manager"),
    );

    // A SECOND engine instance, dedicated to dictation AI cleanup. Sharing one
    // engine with the assistant meant a different model on either side evicted
    // the other on every use, so each dictation paid a full model load. Separate
    // processes also let cleanup run leaner (no vision projector, thinking off,
    // small context, CPU for small models) and keep their own residency policy.
    let cleanup_llm_manager = Arc::new(
        managers::local_llm::LocalLlmManager::new_for_role(
            app_handle,
            managers::local_llm::LlmRole::Cleanup,
        )
        .expect("Failed to initialize cleanup LLM manager"),
    );

    // transcribe.cpp (logging + backend modules) initializes lazily, once, at
    // its first model load or device enumeration — see `init_transcribe_cpp`.
    // Failures there are logged and swallowed so transcribe-rs engines keep
    // working (N1).

    // Apply accelerator preferences before any model loads
    managers::transcription::apply_accelerator_settings(app_handle);

    // Add managers to Tauri's managed state
    app_handle.manage(recording_manager.clone());
    app_handle.manage(model_manager.clone());
    app_handle.manage(transcription_manager.clone());
    app_handle.manage(history_manager.clone());
    app_handle.manage(local_llm_manager.clone());
    app_handle.manage(managers::local_llm::CleanupLlm(cleanup_llm_manager.clone()));

    // Meetings: its own SQLite store (never joined with dictation history) plus
    // the recorder that owns the dual-stream capture lifecycle.
    //
    // Unlike the managers above this does NOT `expect`. Meetings are an additive
    // feature; a store that fails to open must cost the user the meetings panel,
    // not the app. When it fails, the state is simply never managed and every
    // meetings command answers with Tauri's "state not managed" error while the
    // real cause sits in the log right here.
    match meetings::store::MeetingStore::new(app_handle) {
        Ok(store) => {
            let meeting_store = Arc::new(store);

            // Exactly once, before anything reads the list. A row left in
            // `recording` cannot be recording — this process just started — and
            // showing it as live tells the user audio is being captured when it
            // is not.
            match meeting_store.reconcile_interrupted() {
                Ok(0) => {}
                Ok(count) => log::info!("Reconciled {} interrupted meeting(s)", count),
                Err(e) => log::error!("Could not reconcile interrupted meetings: {}", e),
            }

            let meeting_recorder = Arc::new(meetings::session::MeetingRecorder::new(
                app_handle.clone(),
                meeting_store.clone(),
            ));
            app_handle.manage(meeting_store);
            app_handle.manage(meeting_recorder);
            // The question-and-answer thread for whichever meeting is open. Its
            // own instance rather than the assistant's conversation: sharing one
            // message list between two features is what put a whole call
            // transcript into the assistant's quick-ask card.
            app_handle.manage(Arc::new(meetings::chat::MeetingChat::new()));

            // Notice when a call starts, and *offer* to record it.
            //
            // The watcher has no handle to the recorder and no route by which it
            // could start a recording — that is a hard rule, documented at length
            // in `call_detect`. Its whole output is a card the user accepts or
            // dismisses, so defaulting it on is safe: a wrong guess costs one
            // dismissal, while software that silently began recording a private
            // conversation would be unacceptable however accurate it was.
            //
            // `is_recording` is read through the recorder's own state rather than
            // captured as a bool, because the answer changes constantly and a
            // stale one would offer to record a meeting already being recorded.
            if meetings::call_detect::detection_supported() {
                let watcher_app = app_handle.clone();
                let recording_probe = app_handle.clone();
                if let Some(watcher) = meetings::call_detect::CallWatcher::start(
                    move || {
                        recording_probe
                            .try_state::<Arc<meetings::session::MeetingRecorder>>()
                            .map(|recorder| recorder.is_recording())
                            .unwrap_or(false)
                    },
                    move |event, observation| {
                        use meetings::call_detect::CallEvent;
                        match event {
                            CallEvent::Prompt => {
                                // Checked here rather than inside the watcher so
                                // turning the setting off takes effect immediately,
                                // without restarting a thread or losing its debounce
                                // state.
                                if !settings::get_settings(&watcher_app).meeting_auto_detect {
                                    return;
                                }
                                // A hands-free call with the assistant holds the
                                // microphone too. The detector no longer sees it
                                // (it excludes our own process tree), and this is
                                // the second line: never ask to record over a
                                // conversation the user is having with us.
                                if voice_conversation::is_active(&watcher_app) {
                                    return;
                                }
                                meetings::pill::show_call_offer(
                                    &watcher_app,
                                    observation.app_label().map(str::to_string),
                                );
                            }
                            // The call is over. Take down an offer nobody answered —
                            // deliberately *not* gated on `meeting_auto_detect`, because
                            // a card raised while the setting was on must still be
                            // cleanable after it is turned off. `withdraw_call_offer` is
                            // a no-op unless an offer is actually on screen, so it can
                            // never hide a live recording.
                            CallEvent::Ended => {
                                meetings::pill::withdraw_call_offer(&watcher_app);
                            }
                            CallEvent::Quiet => {}
                        }
                    },
                ) {
                    app_handle.manage(Arc::new(watcher));
                }
            }
        }
        Err(e) => log::error!(
            "Meetings unavailable — could not open the meetings store: {}",
            e
        ),
    }

    // Auto-learn from corrections. Started unconditionally so the thread exists and a
    // dictation can begin a watch the moment the user turns the setting on — the switch
    // is read per dictation, not here. Answers `None` where a text field cannot be
    // read, in which case nothing else in that module ever runs and the UI says so.
    if let Some(watcher) = autolearn::learner::start(app_handle) {
        app_handle.manage(watcher);
    }

    // Enforce history retention at startup and then on a slow tick for as long
    // as the app runs.
    //
    // The tick is the fix for a real hole: retention used to be applied only at
    // launch and immediately after a new recording was saved. A time-based policy
    // ("after 3 days") therefore never fired on a machine where the app stays
    // open and no new dictation happens to land — entries crossed the cutoff and
    // stayed listed indefinitely, which reads as the retention setting doing
    // nothing at all. `Never` and the count policy make this a no-op, so the tick
    // costs one settings read per interval for anyone not using an age policy.
    // Runs off-thread so DB/file IO can't delay window creation.
    {
        let history_manager = history_manager.clone();
        let sweep_handle = app_handle.clone();
        std::thread::spawn(move || {
            /// Long enough to be invisible, short enough that a recording never
            /// outlives its retention window by a meaningful margin.
            const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

            loop {
                match history_manager.cleanup_old_entries() {
                    Ok(0) => {}
                    Ok(deleted) => {
                        log::info!("History retention sweep removed {} recording(s)", deleted);
                        if let Err(e) = sweep_handle.emit("history-retention-applied", ()) {
                            log::error!("Failed to emit history-retention-applied: {}", e);
                        }
                    }
                    Err(e) => log::error!("History retention sweep failed: {}", e),
                }
                std::thread::sleep(SWEEP_INTERVAL);
            }
        });
    }

    // Start the idle watcher that unloads the built-in LLM after it has been
    // idle for the configured timeout, freeing RAM/VRAM when it's not in use.
    managers::local_llm::LocalLlmManager::spawn_idle_watcher(&local_llm_manager);
    managers::local_llm::LocalLlmManager::spawn_idle_watcher(&cleanup_llm_manager);

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    // Set up signal handlers for toggling transcription. On Linux, SIGUSR1 is
    // deliberately not handled — it belongs to WebKitGTK's garbage collector —
    // see signal_handle.rs.
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone());

    // Apply macOS Accessory policy if starting hidden and tray is available.
    // If the tray icon is disabled, keep the dock icon so the user can reopen.
    #[cfg(target_os = "macos")]
    {
        let settings = settings::get_settings(app_handle);
        if settings.start_hidden && settings.show_tray_icon {
            let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }
    // Get the current theme to set the appropriate initial icon
    let initial_theme = tray::get_current_theme(app_handle);

    // Choose the appropriate initial icon based on theme
    let initial_icon_path = tray::get_icon_path(initial_theme, tray::TrayIconState::Idle);

    let tray = TrayIconBuilder::new()
        .icon(
            Image::from_path(
                app_handle
                    .path()
                    .resolve(initial_icon_path, tauri::path::BaseDirectory::Resource)
                    .unwrap(),
            )
            .unwrap(),
        )
        .tooltip(tray::tray_tooltip())
        .show_menu_on_left_click(true)
        .icon_as_template(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "home" => {
                show_main_window(app);
            }
            "open_assistant" => {
                // Always opens (never toggles): this is the recovery path for a
                // panel the user cannot find, and a toggle would hide it again
                // for anyone who reached for the tray because the window was
                // already on screen but off in a corner.
                assistant::open_assistant_panel(app);
            }
            "check_updates" => {
                // Always honoured: the setting governs background checks only.
                show_main_window(app);
                let _ = app.emit("check-for-updates", ());
            }
            "send_feedback" => {
                show_main_window(app);
                let _ = app.emit("open-feedback", ());
            }
            "copy_last_transcript" => {
                tray::copy_last_transcript(app);
            }
            "unload_model" => {
                let transcription_manager = app.state::<Arc<TranscriptionManager>>();
                if !transcription_manager.is_model_loaded() {
                    log::warn!("No model is currently loaded.");
                    return;
                }
                match transcription_manager.unload_model() {
                    Ok(()) => log::info!("Model unloaded via tray."),
                    Err(e) => log::error!("Failed to unload model via tray: {}", e),
                }
            }
            "cancel" => {
                use crate::utils::cancel_current_operation;

                // Use centralized cancellation that handles all operations
                cancel_current_operation(app);
            }
            "quit" => {
                app.exit(0);
            }
            id if id.starts_with("model_select:") => {
                let model_id = id.strip_prefix("model_select:").unwrap().to_string();
                let current_model = settings::get_settings(app).selected_model;
                if model_id == current_model {
                    return;
                }
                let app_clone = app.clone();
                std::thread::spawn(move || {
                    match commands::models::switch_active_model(&app_clone, &model_id) {
                        Ok(()) => {
                            log::info!("Model switched to {} via tray.", model_id);
                        }
                        Err(e) => {
                            log::error!("Failed to switch model via tray: {}", e);
                        }
                    }
                    tray::update_tray_menu(&app_clone, &tray::TrayIconState::Idle, None);
                });
            }
            _ => {}
        })
        .build(app_handle)
        .unwrap();
    app_handle.manage(tray);

    // Initialize tray menu with idle state
    utils::update_tray_menu(app_handle, &utils::TrayIconState::Idle, None);

    // Apply show_tray_icon setting
    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    // Refresh tray menu when model state changes. For the state the tray is in,
    // not Idle: a model finishing its load mid-recording must not take Cancel away.
    let app_handle_for_listener = app_handle.clone();
    app_handle.listen("model-state-changed", move |_| {
        tray::refresh_tray_menu(&app_handle_for_listener, None);
    });

    // Pointer transitions wake the completed-card dismissal task; no polling.
    app_handle.listen("overlay-hover", |event| {
        overlay::set_overlay_hovered(event.payload().trim() == "true");
    });

    // The quick ask's frame is fixed while it is on screen, so the one thing the
    // webview has to ask for is the keyboard: the panel does not take it when it
    // appears (see `assistant::set_panel_keyboard`), and the text field needs it.
    let app_handle_for_ask_keyboard = app_handle.clone();
    app_handle.listen("assistant-ask-keyboard", move |event| {
        let payload = event.payload();
        let want = serde_json::from_str::<serde_json::Value>(payload)
            .ok()
            .and_then(|value| value.get("want").and_then(|w| w.as_bool()))
            .unwrap_or_else(|| payload.trim() == "true");
        assistant::set_panel_keyboard(&app_handle_for_ask_keyboard, want);
    });

    // Which part of the panel window is actually drawn, so the rest of it can pass
    // clicks through to whatever is underneath. Physical pixels relative to the
    // window origin — the webview knows its own `devicePixelRatio`, so converting
    // there means no scale factor has to be agreed on across the boundary. An event
    // rather than a command, exactly like the fit report above: the panel states a
    // measurement, it does not ask for a window operation.
    //
    // `{"tangible":true}` is the third case and not a rect at all: a form with no
    // measurable surface in it (the voice conversation view, the full chat panel)
    // has to stay fully clickable, and it also has to *clear* whatever the previous
    // form measured, or a call would inherit the ask pill's little rectangle and be
    // left with no reachable Mute or End button.
    let app_handle_for_hit_rect = app_handle.clone();
    app_handle.listen("assistant-hit-rect", move |event| {
        let payload = event.payload();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            log::debug!("Ignoring unparseable assistant-hit-rect: {payload}");
            return;
        };
        if value
            .get("tangible")
            .and_then(|t| t.as_bool())
            .unwrap_or(false)
        {
            assistant::clear_panel_hit_rect(&app_handle_for_hit_rect);
            return;
        }
        let rect_of = |value: &serde_json::Value| {
            let number = |key: &str| value.get(key).and_then(|v| v.as_f64());
            match (number("x"), number("y"), number("width"), number("height")) {
                (Some(x), Some(y), Some(width), Some(height)) => Some((x, y, width, height)),
                _ => None,
            }
        };
        // Each drawn surface on its own (`rects`), so the empty space between them
        // passes clicks through. A payload without the list is one rect.
        let parts = match value.get("rects").and_then(|r| r.as_array()) {
            Some(list) => Some(list.iter().filter_map(rect_of).collect::<Vec<_>>()),
            None => rect_of(&value).map(|rect| vec![rect]),
        };
        match parts {
            Some(parts) => assistant::set_panel_hit_rects(&app_handle_for_hit_rect, &parts),
            None => log::debug!("Ignoring assistant-hit-rect with no usable rect: {payload}"),
        }
    });

    // A pointer held down inside the panel keeps it tangible for the length of a
    // drag or a resize, which the OS runs well outside the drawn rect.
    app_handle.listen("assistant-panel-hold", move |event| {
        let payload = event.payload();
        let held = serde_json::from_str::<serde_json::Value>(payload)
            .ok()
            .and_then(|value| value.get("held").and_then(|h| h.as_bool()))
            .unwrap_or_else(|| payload.trim() == "true");
        assistant::set_panel_pointer_held(held);
    });

    // Get the autostart manager and configure based on user setting
    let autostart_manager = app_handle.autolaunch();
    let settings = settings::get_settings(&app_handle);

    if settings.autostart_enabled {
        // Enable autostart if user has opted in
        let _ = autostart_manager.enable();
    } else {
        // Disable autostart if user has opted out
        let _ = autostart_manager.disable();
    }

    // Create the recording overlay window (hidden by default)
    utils::create_recording_overlay(app_handle);
}

#[tauri::command]
#[specta::specta]
fn trigger_update_check(app: AppHandle) -> Result<(), String> {
    // A requested check always runs; `update_checks_enabled` only stops the
    // app from checking on its own.
    app.emit("check-for-updates", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn show_main_window_command(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

/// Handle the `--list-devices` CLI flag: initialize the transcribe.cpp backends
/// (loading the bundled ggml backend libraries), enumerate the compute devices,
/// print them plus backend availability, and return so the process exits
/// without launching the GUI.
///
/// This is the Session 7 clean-machine smoke test: on a packaged install with
/// no dev toolchain and no Vulkan SDK, a successful run (exit 0, a device
/// listed) proves the app's own bundled DLLs/.so's are sufficient for the
/// native engine to come up. CI installs the MSI/NSIS package and runs this.
pub fn list_transcribe_devices() {
    // Same FMA3 guard as the in-app GPU probe (managers::transcription): ggml's
    // Vulkan backend uses FMA3, which SIGILLs on CPUs without it. On such a CPU
    // report CPU-only rather than risk a crash while loading the Vulkan module.
    #[cfg(target_arch = "x86_64")]
    let fma3 = std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(target_arch = "x86_64"))]
    let fma3 = true;

    // Bring the backends up (init_logging + init_backends_default). Failures are
    // logged, not fatal — mirrors the in-app path.
    managers::transcription::init_transcribe_cpp();

    if !fma3 {
        println!("transcribe.cpp: CPU lacks FMA3 — GPU backends skipped (CPU-only).");
    }

    let devices = if fma3 {
        transcribe_cpp::devices()
    } else {
        Vec::new()
    };

    println!("transcribe.cpp compute devices: {}", devices.len());
    for d in &devices {
        let idx = d
            .index
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".to_string());
        let label = if d.description.is_empty() {
            d.name.as_str()
        } else {
            d.description.as_str()
        };
        let mem_mb = d.memory_total / (1024 * 1024);
        println!(
            "  [{idx}] {name} — {label} (kind={kind}, {mem_mb} MiB)",
            name = d.name,
            kind = d.kind,
        );
    }

    // Report which backends this build can actually use at runtime (compiled in
    // AND their module loaded), so the audit sees e.g. "Vulkan: true" on x64.
    for backend in [
        transcribe_cpp::Backend::Vulkan,
        transcribe_cpp::Backend::Metal,
        transcribe_cpp::Backend::Cpu,
    ] {
        println!(
            "  backend {backend:?} available: {}",
            transcribe_cpp::backend_available(backend)
        );
    }
}

/// Handle the `--probe-devices` flag: enumerate the compute devices in this
/// short-lived process and print them as a single line of JSON, then return so
/// the process exits without launching the GUI.
///
/// This is the child half of the Linux crash-isolated device probe. The running
/// app spawns itself with this flag instead of loading ggml's Vulkan backend
/// in-process, because the vendored, statically linked whisper.cpp/ggml is built
/// with `-march=native` and can raise SIGILL on a machine narrower than the one
/// that built the package. If that happens the child dies here and the app just
/// carries on without GPU acceleration.
pub fn print_device_probe_json() {
    // The backend modules must be loaded before transcribe.cpp can report any
    // devices; harmless for the whisper probe.
    managers::transcription::init_transcribe_cpp();

    let probe = managers::transcription::enumerate_devices_in_process();
    match serde_json::to_string(&probe) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("device probe serialization failed: {error}"),
    }
}

/// Turn off WebView2's built-in browser accelerator keys for one webview.
///
/// WebView2 handles browser shortcuts itself, before page JS can see or
/// `preventDefault()` them: F5 / Ctrl+R reload the app window, Ctrl+F opens a
/// find bar, Ctrl+P a print dialog, F7 toggles caret browsing, and F6 focus
/// cycling was reported to turn the whole window white when assigned as a
/// dictation shortcut (Handy #1940). None of them belong in an app window.
/// Text-editing keys (Ctrl+C/V/X/A, arrows, Home/End) are not affected.
/// Backport of Handy #2060, applied to every window rather than only the main
/// one, since the assistant panel and reminder popup are just as exposed.
#[cfg(target_os = "windows")]
fn disable_webview2_browser_accelerators<R: tauri::Runtime>(webview: &tauri::Webview<R>) {
    let label = webview.label().to_string();
    let _ = webview.with_webview(move |platform| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows::core::Interface;

        let result = platform
            .controller()
            .CoreWebView2()
            .and_then(|core| core.Settings())
            .and_then(|settings| settings.cast::<ICoreWebView2Settings3>())
            .and_then(|settings| settings.SetAreBrowserAcceleratorKeysEnabled(false));

        if let Err(error) = result {
            log::warn!("Failed to disable WebView2 browser accelerators for '{label}': {error}");
        }
    });
}

/// Context-menu items an app window may show, by WebView2's unlocalized name
/// (the English label in lower camel case). Everything else — Back, Reload,
/// Save as, Print, Copy link to highlight, Share, More tools, Inspect — is a
/// browser command that has no place in an app window.
///
/// An allowlist rather than a blocklist on purpose: WebView2 adds entries
/// across runtime versions, and a blocklist lets each new one through until
/// someone notices it in a screenshot.
#[cfg(target_os = "windows")]
const ALLOWED_CONTEXT_MENU_ITEMS: &[&str] = &[
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "pasteAsPlainText",
    "pasteAndMatchStyle",
    "selectAll",
    "emoji",
    "spellCheck",
    "addToDictionary",
];

/// Whether a WebView2 context-menu item survives the filter. Inspect is kept in
/// development builds only, where it is the reason anyone right-clicks.
#[cfg(target_os = "windows")]
fn context_menu_item_allowed(name: &str) -> bool {
    ALLOWED_CONTEXT_MENU_ITEMS.contains(&name)
        || (cfg!(debug_assertions) && matches!(name, "inspectElement" | "inspect"))
}

/// Which items of a menu to keep, given each item's name and whether it is a
/// separator. Separators survive only between two kept items, so removing
/// everything around one never leaves a stray rule at an edge or two in a row.
#[cfg(any(target_os = "windows", test))]
fn context_menu_keep_mask(items: &[(String, bool)], allowed: impl Fn(&str) -> bool) -> Vec<bool> {
    let mut keep: Vec<bool> = items
        .iter()
        .map(|(name, separator)| !separator && allowed(name))
        .collect();
    let mut seen_item = false;
    let mut pending_separator: Option<usize> = None;
    for (index, (_, separator)) in items.iter().enumerate() {
        if *separator {
            if seen_item && pending_separator.is_none() {
                pending_separator = Some(index);
            }
        } else if keep[index] {
            if let Some(separator) = pending_separator.take() {
                keep[separator] = true;
            }
            seen_item = true;
        }
    }
    keep
}

/// Trim WebView2's own right-click menu down to editing commands.
///
/// `lib/contextMenu.ts` decides *whether* the menu opens (text fields and
/// selected text only). This decides what is *in* it: the stock menu on a
/// selection carried Copy link to highlight, Print, More tools and Inspect, so
/// selecting a word in an assistant answer and right-clicking still read as a
/// browser. If nothing survives the filter the menu is suppressed entirely,
/// which also covers any window whose script has not installed the JS half.
///
/// Registered on every page load, so the previous registration on the same
/// webview is removed first rather than stacking handlers.
#[cfg(target_os = "windows")]
fn filter_webview2_context_menu<R: tauri::Runtime>(webview: &tauri::Webview<R>) {
    use std::collections::HashMap;
    use std::sync::Mutex;

    static TOKENS: once_cell::sync::Lazy<Mutex<HashMap<String, i64>>> =
        once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

    let label = webview.label().to_string();
    let _ = webview.with_webview(move |platform| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_11, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND,
            COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
        };
        use webview2_com::{take_pwstr, ContextMenuRequestedEventHandler};
        use windows::core::{Interface, PWSTR};

        let handler = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let items = args.MenuItems()?;
            let mut count = 0u32;
            items.Count(&mut count)?;

            let mut described = Vec::with_capacity(count as usize);
            for index in 0..count {
                let item = items.GetValueAtIndex(index)?;
                let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
                item.Kind(&mut kind)?;
                let mut name = PWSTR::null();
                item.Name(&mut name)?;
                described.push((
                    take_pwstr(name),
                    kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
                ));
            }

            let keep = context_menu_keep_mask(&described, context_menu_item_allowed);
            // Back to front, so removing an item never shifts one still to visit.
            for index in (0..count).rev() {
                if !keep[index as usize] {
                    items.RemoveValueAtIndex(index)?;
                }
            }
            if !keep.contains(&true) {
                // Handled with no menu of our own: nothing is shown.
                args.SetHandled(true)?;
            }
            Ok(())
        }));

        let result = platform
            .controller()
            .CoreWebView2()
            .and_then(|core| core.cast::<ICoreWebView2_11>())
            .and_then(|core| {
                let mut tokens = TOKENS.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(previous) = tokens.remove(&label) {
                    // Fails harmlessly when this is a new webview reusing the label.
                    let _ = core.remove_ContextMenuRequested(previous);
                }
                let mut token = 0i64;
                core.add_ContextMenuRequested(&handler, &mut token)?;
                tokens.insert(label.clone(), token);
                Ok(())
            });

        if let Err(error) = result {
            log::warn!("Failed to filter the WebView2 context menu for '{label}': {error}");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(cli_args: CliArgs) {
    // Detect portable mode before anything else
    portable::init();

    // Allow the assistant panel to play TTS audio without a user gesture
    // (WebView2 reads this env var at creation time).
    #[cfg(target_os = "windows")]
    {
        let mut args = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
        if !args.contains("--autoplay-policy") {
            if !args.is_empty() {
                args.push(' ');
            }
            args.push_str("--autoplay-policy=no-user-gesture-required");
        }
        // Keep audio and timers alive when the panel is hidden or occluded.
        // Without this, WebView2 marks the (frequently hidden) panel window as
        // occluded and suspends its media, so Kokoro TTS only played when the
        // panel happened to be visible/foreground — e.g. right after opening it
        // via the shortcut — and stayed silent otherwise.
        if !args.contains("CalculateNativeWinOcclusion") {
            if !args.is_empty() {
                args.push(' ');
            }
            args.push_str("--disable-features=CalculateNativeWinOcclusion");
        }
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", args);
    }

    // Parse console logging directives from RUST_LOG, falling back to info-level logging
    // when the variable is unset
    let console_filter = build_console_filter();

    let specta_builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            shortcut::change_binding,
            shortcut::reset_binding,
            shortcut::change_ptt_setting,
            shortcut::change_dynamic_shortcuts_setting,
            shortcut::change_audio_feedback_setting,
            shortcut::change_audio_feedback_volume_setting,
            shortcut::change_sound_theme_setting,
            shortcut::change_theme_setting,
            shortcut::change_ui_text_size_setting,
            shortcut::change_start_hidden_setting,
            shortcut::change_autostart_setting,
            shortcut::change_translate_to_english_setting,
            shortcut::change_selected_language_setting,
            shortcut::change_overlay_position_setting,
            shortcut::change_debug_mode_setting,
            shortcut::change_word_correction_threshold_setting,
            shortcut::change_extra_recording_buffer_setting,
            shortcut::change_paste_delay_ms_setting,
            shortcut::change_paste_method_setting,
            shortcut::get_available_typing_tools,
            shortcut::change_typing_tool_setting,
            shortcut::change_external_script_path_setting,
            shortcut::change_clipboard_handling_setting,
            shortcut::change_auto_submit_setting,
            shortcut::change_auto_submit_key_setting,
            shortcut::get_post_process_readiness,
            shortcut::change_post_process_enabled_setting,
            shortcut::change_post_process_on_dictation_setting,
            shortcut::set_cleanup_local_model,
            shortcut::restore_post_process_prompt,
            shortcut::change_flow_enabled_setting,
            shortcut::change_flow_phrase_setting,
            shortcut::change_flow_screen_access_setting,
            shortcut::change_post_process_tone_setting,
            shortcut::add_post_process_custom_tone,
            shortcut::update_post_process_custom_tone,
            shortcut::delete_post_process_custom_tone,
            shortcut::change_post_process_timeout_setting,
            shortcut::change_experimental_enabled_setting,
            shortcut::change_post_process_base_url_setting,
            shortcut::change_post_process_api_key_setting,
            shortcut::change_post_process_model_setting,
            shortcut::set_post_process_provider,
            shortcut::fetch_post_process_models,
            shortcut::add_post_process_prompt,
            shortcut::update_post_process_prompt,
            shortcut::delete_post_process_prompt,
            shortcut::set_post_process_selected_prompt,
            shortcut::update_custom_words,
            shortcut::change_spoken_emojis_enabled_setting,
            shortcut::change_replacements_enabled_setting,
            shortcut::update_text_replacements,
            shortcut::export_text_replacements,
            shortcut::import_text_replacements,
            shortcut::suspend_binding,
            shortcut::resume_binding,
            shortcut::change_mute_while_recording_setting,
            shortcut::change_append_trailing_space_setting,
            shortcut::change_lazy_stream_close_setting,
            shortcut::change_live_transcription_enabled_setting,
            shortcut::change_live_transcription_window_enabled_setting,
            shortcut::change_overlay_style_setting,
            shortcut::change_assistant_overlay_style_setting,
            shortcut::change_overlay_linger_setting,
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_keyboard_implementation_setting,
            shortcut::get_keyboard_implementation,
            shortcut::globe_key_has_own_action,
            shortcut::environment::get_shortcut_environment,
            shortcut::open_keyboard_settings,
            shortcut::change_show_tray_icon_setting,
            shortcut::change_close_behavior_setting,
            shortcut::change_whisper_accelerator_setting,
            shortcut::change_ort_accelerator_setting,
            shortcut::change_whisper_gpu_device,
            shortcut::get_available_accelerators,
            shortcut::handy_keys::start_handy_keys_recording,
            shortcut::handy_keys::stop_handy_keys_recording,
            trigger_update_check,
            show_main_window_command,
            updates::get_update_support,
            updates::download_update_installer,
            updates::open_update_installer,
            updates::reveal_update_installer,
            updates::prepare_update_install,
            updates::take_update_notice,
            feedback::get_feedback_system_info,
            feedback::send_feedback,
            commands::cancel_operation,
            commands::copy_overlay_transcript,
            commands::recover_dictation,
            commands::commit_recording,
            commands::toggle_dictation,
            commands::is_portable,
            commands::get_app_dir_path,
            commands::get_app_settings,
            commands::get_default_settings,
            settings::get_system_memory_gb,
            commands::get_log_dir_path,
            commands::set_log_level,
            commands::open_recordings_folder,
            commands::open_log_dir,
            commands::open_app_data_dir,
            commands::check_apple_intelligence_available,
            commands::initialize_enigo,
            commands::initialize_shortcuts,
            commands::reset_macos_accessibility_permission,
            commands::open_macos_accessibility_settings,
            commands::reset_macos_microphone_permission,
            commands::reset_macos_screen_recording_permission,
            commands::open_macos_screen_recording_settings,
            commands::models::get_available_models,
            commands::models::get_model_info,
            commands::models::download_model,
            commands::models::delete_model,
            commands::models::cancel_download,
            commands::models::set_active_model,
            commands::models::get_current_model,
            commands::models::get_transcription_model_status,
            commands::models::is_model_loading,
            commands::models::has_any_models_available,
            commands::models::has_any_models_or_downloads,
            commands::models::search_huggingface_models,
            commands::models::list_huggingface_gguf_files,
            commands::models::add_custom_llm_model,
            commands::models::add_local_models,
            commands::models::add_model_folder,
            commands::models::remove_model_folder,
            commands::models::get_model_folders,
            commands::models::rescan_local_models,
            commands::local_llm::get_local_llm_status,
            commands::local_llm::start_local_llm,
            commands::local_llm::stop_local_llm,
            commands::local_llm::set_local_llm_context_size,
            commands::local_llm::set_local_llm_unload_timeout,
            commands::audio::update_microphone_mode,
            commands::audio::get_microphone_mode,
            commands::audio::get_windows_microphone_permission_status,
            commands::audio::open_microphone_privacy_settings,
            commands::audio::get_available_microphones,
            commands::audio::set_selected_microphone,
            commands::audio::get_selected_microphone,
            commands::audio::get_available_output_devices,
            commands::audio::set_selected_output_device,
            commands::audio::get_selected_output_device,
            commands::audio::play_test_sound,
            commands::audio::check_custom_sounds,
            commands::audio::set_clamshell_microphone,
            commands::audio::get_clamshell_microphone,
            commands::audio::is_recording,
            commands::transcription::set_model_unload_timeout,
            commands::transcription::get_model_load_status,
            commands::transcription::unload_model_manually,
            commands::stt_cloud::get_cloud_stt_providers,
            commands::stt_cloud::get_cloud_stt_readiness,
            commands::stt_cloud::set_stt_engine_mode,
            commands::stt_cloud::set_cloud_stt_api_key,
            commands::stt_cloud::set_cloud_stt_provider,
            commands::stt_cloud::set_cloud_stt_model,
            commands::stt_cloud::set_cloud_stt_base_url,
            commands::stt_cloud::set_cloud_stt_streaming,
            commands::stt_cloud::set_cloud_stt_send_custom_words,
            commands::stt_cloud::set_cloud_stt_no_verbatim,
            commands::stt_cloud::get_cloud_stt_key_status,
            commands::stt_cloud::list_cloud_stt_models,
            commands::stt_cloud::test_cloud_stt,
            commands::history::get_history_entries,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::retry_history_entry_transcription,
            commands::history::recover_history_entry,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            commands::history::update_recording_retention_days,
            commands::history::preview_recording_retention,
            commands::history::enforce_recording_retention,
            commands::history::get_assistant_history_entries,
            commands::history::list_assistant_conversations,
            commands::history::get_assistant_history_entry,
            commands::history::delete_assistant_history_entry,
            commands::history::get_usage_stats,
            commands::assistant::assistant_send_text,
            commands::assistant::assistant_read_file,
            commands::assistant::assistant_read_image,
            commands::assistant::assistant_get_conversation,
            commands::assistant::assistant_regenerate,
            commands::assistant::assistant_summarize,
            commands::assistant::assistant_resume_session,
            commands::assistant::assistant_clear_conversation,
            commands::assistant::hide_assistant_panel,
            commands::assistant::assistant_branch_session,
            commands::assistant::assistant_discuss_meeting,
            commands::assistant::assistant_conversation_meeting,
            commands::assistant::set_assistant_ask_anchor,
            commands::assistant::set_assistant_ask_display,
            commands::assistant::list_assistant_displays,
            commands::assistant::assistant_insert_text,
            commands::assistant::set_assistant_provider,
            commands::assistant::change_assistant_model_setting,
            commands::assistant::change_assistant_system_prompt_setting,
            commands::assistant::set_assistant_active_character,
            commands::assistant::set_assistant_characters,
            commands::assistant::assistant_read_avatar,
            commands::assistant::assistant_import_character,
            commands::assistant::assistant_export_character,
            commands::assistant::assistant_generate_character,
            commands::assistant::assistant_restore_builtin_character,
            commands::assistant::assistant_restore_missing_builtins,
            commands::assistant::set_assistant_enabled,
            commands::assistant::set_assistant_ask_screen_access,
            commands::assistant::set_assistant_call_screen_access,
            commands::assistant::set_assistant_vision_capture_timing,
            commands::assistant::set_assistant_tts_enabled,
            commands::assistant::set_assistant_tts_voice,
            commands::assistant::set_assistant_response_length,
            commands::assistant::set_assistant_font_size,
            commands::assistant::set_assistant_tts_engine,
            commands::assistant::set_assistant_tts_base_url,
            commands::assistant::set_assistant_tts_api_key,
            commands::assistant::set_assistant_tts_model,
            commands::assistant::set_assistant_tts_remote_voice,
            commands::assistant::set_assistant_tts_kokoro_dtype,
            commands::assistant::set_assistant_tts_kokoro_device,
            commands::assistant::get_local_voice_status,
            commands::assistant::assistant_report_webgpu,
            commands::assistant::set_assistant_tts_speed,
            commands::assistant::set_assistant_tts_volume,
            commands::assistant::set_assistant_tts_elevenlabs_stability,
            commands::assistant::set_assistant_tts_elevenlabs_audio_tags,
            commands::assistant::set_assistant_tts_elevenlabs_audio_tag_intensity,
            commands::assistant::set_assistant_conversation_pace,
            commands::assistant::set_assistant_conversation_sensitivity,
            commands::assistant::set_assistant_panel_opacity,
            commands::assistant::set_assistant_tts_stop_on_dictation,
            commands::assistant::redirect_transcription_to_assistant,
            commands::assistant::assistant_finish_local_tts,
            commands::assistant::assistant_stop_local_tts,
            commands::assistant::assistant_toggle_voice,
            commands::assistant::assistant_speak,
            commands::assistant::assistant_test_tts,
            commands::assistant::assistant_list_azure_voices,
            commands::assistant::assistant_list_tts_voices,
            commands::assistant::assistant_list_tts_models,
            commands::assistant::assistant_stop,
            commands::assistant::assistant_test_connection,
            reminders::list_reminders,
            reminders::list_waiting_reminders,
            reminders::create_reminder,
            reminders::complete_reminder,
            reminders::snooze_reminder,
            reminders::dismiss_reminder_popup,
            reminders::fit_reminder_popup,
            voice_conversation::assistant_conversation_start,
            voice_conversation::assistant_conversation_end,
            voice_conversation::assistant_conversation_set_expanded,
            voice_conversation::assistant_conversation_interrupt,
            voice_conversation::assistant_conversation_text,
            voice_conversation::assistant_conversation_new,
            voice_conversation::assistant_conversation_load,
            voice_conversation::assistant_conversation_branch,
            voice_conversation::assistant_conversation_discuss,
            voice_conversation::assistant_conversation_set_speaker,
            voice_conversation::assistant_conversation_dictation_active,
            commands::assistant::set_assistant_max_history_messages,
            commands::assistant::set_assistant_auto_summarize,
            commands::assistant::set_assistant_web_search_enabled,
            commands::assistant::set_assistant_prefer_provider_web_search,
            commands::assistant::set_assistant_web_search_provider,
            commands::assistant::set_assistant_web_search_max_results,
            commands::assistant::set_assistant_search_depth,
            commands::assistant::set_assistant_web_search_daily_credit_budget,
            commands::assistant::set_assistant_local_search_smart,
            commands::assistant::set_assistant_web_search_fetch_content,
            commands::assistant::set_assistant_web_search_api_key,
            commands::assistant::assistant_test_web_search,
            commands::memory::set_assistant_memory_enabled,
            commands::memory::set_assistant_memory_detail,
            commands::memory::set_assistant_memory_incognito,
            commands::memory::set_assistant_memory_about_you,
            commands::memory::add_assistant_memory_note,
            commands::memory::update_assistant_memory_note,
            commands::memory::delete_assistant_memory_note,
            commands::memory::clear_assistant_memory,
            commands::memory::export_assistant_memory,
            commands::memory::import_assistant_memory,
            commands::memory::assistant_distill_memory_now,
            commands::meetings::start_meeting,
            commands::meetings::stop_meeting,
            commands::meetings::set_meeting_paused,
            commands::meetings::get_meeting_state,
            commands::meetings::get_system_audio_status,
            commands::meetings::list_meetings,
            commands::meetings::get_meeting,
            commands::meetings::get_meeting_segments,
            commands::meetings::get_meeting_speakers,
            commands::meetings::rename_meeting,
            commands::meetings::rename_meeting_speaker,
            commands::meetings::set_meeting_my_notes,
            commands::meetings::generate_meeting_notes,
            commands::meetings::is_meeting_notes_running,
            commands::meetings::delete_meeting,
            commands::meetings::fit_meeting_pill,
            commands::meetings::set_meeting_pill_expanded,
            commands::meetings::get_meeting_pill_expanded,
            commands::meetings::ask_about_meeting,
            commands::meetings::get_meeting_chat,
            commands::meetings::clear_meeting_chat,
            commands::meetings::cancel_meeting_chat,
            commands::meetings::get_diarization_status,
            commands::meetings::download_diarization_model,
            commands::meetings::diarize_meeting,
            commands::meetings::dismiss_call_offer,
            commands::meetings::accept_call_offer,
            commands::meetings::get_call_detection_status,
            commands::meetings::set_meeting_auto_detect,
            commands::meetings::get_meeting_indicator,
            commands::meetings::set_meeting_indicator,
            commands::meetings::set_meeting_notes,
            commands::autolearn::get_auto_learn_status,
            commands::autolearn::set_auto_learn_corrections,
            commands::autolearn::set_learned_words,
            commands::autolearn::keep_learned_word,
            helpers::clamshell::is_laptop,
            window_drag::start_window_drag,
            window_drag::start_window_resize,
        ])
        .events(collect_events![managers::history::HistoryUpdatePayload,]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            Typescript::default().bigint(BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    // Almost every command is typed and registered through tauri-specta. The one
    // exception streams Kokoro's audio in as a *raw* binary body, which the
    // bindings generator cannot describe (`tauri::ipc::Request` has no
    // `specta::Type`). Tauri allows a single invoke handler, so the two are
    // composed by command name: each invocation goes to exactly one of them, and
    // ownership is handed over without cloning.
    //
    // A named function rather than a `let` binding, so the runtime type is
    // concrete instead of needing inference through the macro.
    fn raw_body_handler(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
        let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
            commands::assistant::assistant_play_local_tts_chunk,
            voice_conversation::assistant_conversation_audio
        ];
        handler(invoke)
    }
    let specta_handler = specta_builder.invoke_handler();
    let invoke_handler = move |invoke: tauri::ipc::Invoke<tauri::Wry>| {
        if matches!(
            invoke.message.command(),
            "assistant_play_local_tts_chunk" | "assistant_conversation_audio"
        ) {
            raw_body_handler(invoke)
        } else {
            specta_handler(invoke)
        }
    };

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            LogBuilder::new()
                .level(log::LevelFilter::Trace) // Set to most verbose level globally
                .max_file_size(500_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .clear_targets()
                .targets([
                    // Console output respects RUST_LOG environment variable
                    Target::new(TargetKind::Stdout).filter({
                        let console_filter = console_filter.clone();
                        move |metadata| console_filter.enabled(metadata)
                    }),
                    // File logs respect the user's settings (stored in FILE_LOG_LEVEL atomic)
                    Target::new(if let Some(data_dir) = portable::data_dir() {
                        TargetKind::Folder {
                            path: data_dir.join("logs"),
                            file_name: Some("speakoflow".into()),
                        }
                    } else {
                        TargetKind::LogDir {
                            file_name: Some("speakoflow".into()),
                        }
                    })
                    .filter(|metadata| {
                        let file_level = FILE_LOG_LEVEL.load(Ordering::Relaxed);
                        metadata.level() <= level_filter_from_u8(file_level)
                    }),
                ])
                .build(),
        );

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    // Every webview in the app (settings, assistant panel, overlay, reminder
    // popup, meeting pill, live transcript) gets WebView2's browser accelerator
    // keys turned off as it loads. See `disable_webview2_browser_accelerators`.
    #[cfg(target_os = "windows")]
    {
        builder = builder.on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                disable_webview2_browser_accelerators(webview);
                filter_webview2_context_menu(webview);
            }
        });
    }

    builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle-transcription") {
                signal_handle::send_transcription_input(app, "transcribe", "CLI");
            } else if args.iter().any(|a| a == "--toggle-post-process") {
                signal_handle::send_transcription_input(app, "transcribe_with_post_process", "CLI");
            } else if args.iter().any(|a| a == "--toggle-assistant") {
                signal_handle::send_transcription_input(app, "assistant", "CLI");
            } else if args.iter().any(|a| a == "--toggle-call") {
                signal_handle::toggle_call(app, "CLI");
            } else if args.iter().any(|a| a == "--cancel") {
                crate::utils::cancel_current_operation(app);
            } else {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(cli_args.clone())
        .setup(move |app| {
            // Record the GPU environment policy main() applied, so a crash
            // report can tell whether an overlay layer could have been involved.
            #[cfg(target_os = "windows")]
            log::info!(
                "Vulkan layer policy: VK_LOADER_LAYERS_DISABLE={:?}",
                std::env::var_os("VK_LOADER_LAYERS_DISABLE")
            );
            #[cfg(target_os = "macos")]
            log::info!(
                "Metal residency sets disabled: {}",
                std::env::var_os("GGML_METAL_NO_RESIDENCY").is_some()
            );

            specta_builder.mount_events(app);

            // Undo a caret browsing mode switched on by a stray F7 before any
            // webview claims the profile (see webview_prefs.rs). The profile
            // lives in the portable Data dir or, by default, in the app's local
            // data dir.
            #[cfg(windows)]
            {
                let user_data_dir = match portable::data_dir() {
                    Some(data_dir) => Some(data_dir.join("webview")),
                    None => app.path().app_local_data_dir().ok(),
                };
                if let Some(user_data_dir) = user_data_dir {
                    webview_prefs::disable_caret_browsing(&user_data_dir);
                }
            }

            // Create main window programmatically so we can set data_directory
            // for portable mode (redirects WebView2 cache to portable Data dir)
            let mut win_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
                    // WebView2 args shared identically across every window (see
                    // WEBVIEW2_BROWSER_ARGS). Windows-only effect.
                    .additional_browser_args(crate::WEBVIEW2_BROWSER_ARGS)
                    // Empty title + a blanked caption icon (see
                    // tray::update_window_icon) keep the top of the window clear
                    // — no "SpeakoFlow" text and no logo in the title bar.
                    .title("")
                    // Open a bit wider/taller so content (max-w-3xl) breathes
                    // next to the sidebar instead of feeling cramped. Min stays
                    // smaller so users on small laptops can shrink it to fit
                    // (paired with the "Small" UI text size for extra room).
                    .inner_size(900.0, 680.0)
                    .min_inner_size(600.0, 500.0)
                    .resizable(true)
                    .maximizable(true)
                    .visible(false);

            if let Some(data_dir) = portable::data_dir() {
                win_builder = win_builder.data_directory(data_dir.join("webview"));
            }

            // Custom title bar: the top of the window is drawn by the webview
            // (see src/components/TitleBar.tsx) so the brand + window controls
            // live inside the app surface instead of an empty native caption.
            //   - Windows/Linux: drop the native chrome entirely and render our
            //     own minimize/close controls. Mouse edge-resize still works
            //     with `resizable(true)` on Tauri 2 stable.
            //   - macOS: keep the window decorated but make the title bar an
            //     overlay so the native traffic lights still show and behave,
            //     while our content (and drag region) extends to the top edge.
            #[cfg(not(target_os = "macos"))]
            {
                win_builder = win_builder.decorations(false);
            }
            #[cfg(target_os = "macos")]
            {
                win_builder = win_builder
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .hidden_title(true);
            }

            let main_webview = win_builder.build()?;

            let mut settings = get_settings(&app.handle());

            // Size the main window. Prefer the size the user last left it at
            // (remembered across launches); otherwise use a default that fits
            // the settings content — a sidebar plus the centered, width-capped
            // content column with comfortable margins — rather than sprawling
            // to fill the whole display (which just strands the content in a big
            // empty frame). The chosen size is clamped to the current monitor so
            // a size remembered from a larger screen still fits a smaller one,
            // then centered. Logical pixels, so it behaves the same at any DPI.
            {
                // Content-fitting default (not display-relative).
                const DEFAULT_W: f64 = 1000.0;
                const DEFAULT_H: f64 = 720.0;
                // Keep in sync with min_inner_size on the builder.
                const MIN_W: f64 = 680.0;
                const MIN_H: f64 = 570.0;

                let mut width = settings.main_window_width.unwrap_or(DEFAULT_W);
                let mut height = settings.main_window_height.unwrap_or(DEFAULT_H);

                if let Some(monitor) = main_webview
                    .current_monitor()
                    .ok()
                    .flatten()
                    .or_else(|| main_webview.primary_monitor().ok().flatten())
                {
                    let scale = monitor.scale_factor();
                    let mon_w = monitor.size().width as f64 / scale;
                    let mon_h = monitor.size().height as f64 / scale;
                    // Leave room for the taskbar/dock and a small margin.
                    if mon_w > 0.0 {
                        width = width.min(mon_w - 40.0).max(MIN_W);
                    }
                    if mon_h > 0.0 {
                        height = height.min(mon_h - 100.0).max(MIN_H);
                    }
                }

                let _ = main_webview.set_size(tauri::LogicalSize::new(width, height));
                let _ = main_webview.center();
            }

            // Match the native window (title bar) theme to the appearance
            // choice so it doesn't stay dark while the UI is light. System
            // maps to None, which lets the OS drive the title bar.
            let window_theme = match settings.theme {
                settings::Theme::Light => Some(tauri::Theme::Light),
                settings::Theme::Dark => Some(tauri::Theme::Dark),
                settings::Theme::System => None,
            };
            let _ = main_webview.set_theme(window_theme);

            // Scale the UI to the saved text-size preference (webview zoom, so
            // px and rem sizes scale together) before the window is shown.
            let _ = main_webview.set_zoom(settings.ui_text_size.zoom_factor());

            // Paint the taskbar / alt-tab icon with a transparent, theme-matched
            // mark (light mark on dark, dark mark on light) so there is no white
            // box and the logo stays visible. On Windows this also blanks the
            // title bar caption icon so the top of the window shows nothing.
            tray::update_window_icon(app.handle());

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            // Store the file log level in the atomic for the filter to use
            FILE_LOG_LEVEL.store(file_log_level.to_level_filter() as u8, Ordering::Relaxed);
            let app_handle = app.handle().clone();
            app.manage(TranscriptionCoordinator::new(app_handle.clone()));
            app.manage(assistant::AssistantConversation::new());
            app.manage(voice_conversation::VoiceConversation::default());

            initialize_core_logic(&app_handle);

            // Create the assistant panel window (hidden until first use). Skipped
            // entirely when the assistant is switched off: that saves a whole
            // WebView renderer process, and everything it would have loaded, for
            // people who only want dictation. Enabling it later creates it on
            // demand (see `commands::assistant::set_assistant_enabled`).
            if settings::get_settings(&app_handle).assistant_enabled {
                assistant::create_assistant_panel(&app_handle);
            }

            // Load saved reminders and start their scheduler. Deliberately not
            // behind `assistant_enabled`: the assistant is how a reminder gets
            // *created*, but one already on the books is owed regardless, and
            // switching the assistant off to stop the panel loading must not
            // silently swallow an alarm the user is relying on.
            reminders::init(&app_handle);

            // Pre-warm GPU/accelerator enumeration on a background thread.
            // The first call into transcribe_rs::whisper_cpp::gpu::list_gpu_devices
            // loads the Metal/Vulkan backend and probes devices, which can take
            // several seconds. Without this, that cost is paid synchronously the
            // first time the user opens the Advanced settings page (which calls
            // the get_available_accelerators command), causing a UI freeze.
            // Result is cached in a OnceLock inside the transcription manager.
            std::thread::spawn(|| {
                let _ = crate::managers::transcription::get_available_accelerators();
            });

            // Hide tray icon if --no-tray was passed
            if cli_args.no_tray {
                tray::set_tray_visibility(&app_handle, false);
            }

            // Show main window only if not starting hidden.
            // CLI --start-hidden flag overrides the setting.
            // But if permission onboarding is required, always show the window.
            let should_hide = settings.start_hidden || cli_args.start_hidden;
            let should_force_show = should_force_show_permissions_window(&app_handle);

            // If start_hidden but tray is disabled, we must show the window
            // anyway. Without a tray icon, the dock is the only way back in.
            let tray_available = settings.show_tray_icon && !cli_args.no_tray;
            // The launch the installer starts after an in-app update always
            // comes to the front: otherwise the app just vanishes on Update.
            let finished_update = updates::take_finished_update(&app_handle);
            if finished_update || should_force_show || !should_hide || !tray_available {
                show_main_window(&app_handle);
            }
            if finished_update {
                updates::bring_main_window_forward(&app_handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                // Remember the size before hiding/quitting, so it reopens as left.
                save_main_window_size(window);

                // Issue #6: honor the user's close preference. The default stays
                // "minimize to tray" (handled below); only fully quit when the
                // user explicitly opted in.
                if matches!(
                    get_settings(&window.app_handle()).close_behavior,
                    crate::settings::CloseBehavior::Quit
                ) {
                    // Reuse the same clean exit path as the tray "Quit" item:
                    // `app.exit(0)` fires `RunEvent::Exit`, which tears down the
                    // built-in LLM sidecar (the fix from issue #2). We must not
                    // prevent the close in this branch.
                    window.app_handle().exit(0);
                    return;
                }

                api.prevent_close();
                let _res = window.hide();

                #[cfg(target_os = "macos")]
                {
                    let settings = get_settings(&window.app_handle());
                    let tray_visible =
                        settings.show_tray_icon && !window.app_handle().state::<CliArgs>().no_tray;
                    if tray_visible {
                        // Tray is available: hide the dock icon, app lives in the tray
                        let res = window
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
                        if let Err(e) = res {
                            log::error!("Failed to set activation policy: {}", e);
                        }
                    }
                    // No tray: keep the dock icon visible so the user can reopen
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                // Update tray icon to match new theme, maintaining idle state
                utils::change_tray_icon(&window.app_handle(), utils::TrayIconState::Idle);
                // Re-tint the title bar / taskbar mark for the new theme.
                tray::update_window_icon(&window.app_handle());
            }
            tauri::WindowEvent::Focused(false) => {
                // Backup save point: catches a resize followed by quitting via
                // the tray (which may never fire CloseRequested).
                save_main_window_size(window);
            }
            _ => {}
        })
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &event {
                show_main_window(app);
            }
            // Tear down the built-in LLM sidecar on quit. `app.exit()` calls
            // `std::process::exit`, which skips `Drop` (and everything after
            // `.run()`), so without this the multi-GB `llama-server` child is
            // orphaned on every graceful quit and accumulates across relaunches
            // until RAM is exhausted (memory climbs -> disk/swap 100% -> freeze).
            // The Windows Job Object in `local_llm.rs` is the backstop for the
            // crash / hard-kill paths where even this handler cannot run.
            if let tauri::RunEvent::Exit = &event {
                release_before_exit(app);
            }
            let _ = (app, event); // suppress unused warnings on non-macOS
        });
}

/// Everything that has to be put away before the process ends: a meeting
/// still recording, both llama.cpp engines, and the native voice.
///
/// Runs from `RunEvent::Exit` on every ordinary quit and restart. The one exit
/// that skips that event is an in-app update on Windows, where the updater
/// plugin starts the installer and calls `std::process::exit` itself, so
/// `updates::prepare_update_install` calls this first. Safe to call twice:
/// each step is a no-op once its thing is already stopped.
pub(crate) fn release_before_exit(app: &AppHandle) {
    // A meeting still recording would otherwise lose its tail and leave its
    // recordings unfinalised and unreferenced. Bounded, because draining a
    // transcription backlog can take minutes.
    if let Some(recorder) = app.try_state::<std::sync::Arc<meetings::session::MeetingRecorder>>() {
        recorder.stop_before_exit(std::time::Duration::from_secs(5));
    }
    if let Some(mgr) = app.try_state::<std::sync::Arc<managers::local_llm::LocalLlmManager>>() {
        mgr.stop();
    }
    // The cleanup engine is a second process and needs the same teardown, or
    // it outlives the app holding its model in memory.
    if let Some(cleanup) = app.try_state::<managers::local_llm::CleanupLlm>() {
        cleanup.0.stop();
    }
    // Destroy the native voice (if loaded) while ONNX Runtime is still intact,
    // rather than leaving its session to the library's own teardown at process
    // exit.
    native_tts::release(None);
}

#[cfg(test)]
mod context_menu_tests {
    use super::context_menu_keep_mask;

    fn menu(entries: &[&str]) -> Vec<(String, bool)> {
        entries.iter().map(|e| (e.to_string(), *e == "-")).collect()
    }

    fn kept(entries: &[&str], allowed: &[&str]) -> Vec<String> {
        let items = menu(entries);
        let mask = context_menu_keep_mask(&items, |n| allowed.contains(&n));
        items
            .into_iter()
            .zip(mask)
            .filter(|(_, k)| *k)
            .map(|((n, _), _)| n)
            .collect()
    }

    /// The menu from the screenshot: selected text in an assistant answer.
    #[test]
    fn selection_menu_keeps_only_copy() {
        let entries = [
            "copy",
            "copyLinkToHighlight",
            "print",
            "-",
            "other",
            "-",
            "inspectElement",
        ];
        assert_eq!(kept(&entries, &["copy"]), ["copy"]);
    }

    #[test]
    fn separators_only_between_kept_items() {
        let entries = [
            "-", "undo", "redo", "-", "-", "back", "-", "cut", "copy", "paste", "-", "print",
        ];
        assert_eq!(
            kept(&entries, &["undo", "redo", "cut", "copy", "paste"]),
            ["undo", "redo", "-", "cut", "copy", "paste"]
        );
    }

    #[test]
    fn nothing_allowed_keeps_nothing() {
        let entries = ["back", "reload", "-", "saveAs", "print"];
        assert!(kept(&entries, &[]).is_empty());
    }
}
