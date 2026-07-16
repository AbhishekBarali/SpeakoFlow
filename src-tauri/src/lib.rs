mod actions;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod apple_intelligence;
mod assistant;
mod audio_feedback;
pub mod audio_toolkit;
mod catalog;
pub mod cli;
mod clipboard;
mod commands;
mod flow;
mod helpers;
mod huggingface;
mod input;
mod llm_client;
mod lock_watch;
mod managers;
mod memory;
mod overlay;
pub mod portable;
mod screenshot;
mod secret_store;
mod settings;
mod shortcut;
mod signal_handle;
mod transcription_coordinator;
mod tray;
mod tray_i18n;
mod tts;
mod utils;
mod web_search;

pub use cli::CliArgs;
#[cfg(debug_assertions)]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

use env_filter::Builder as EnvFilterBuilder;
use managers::audio::AudioRecordingManager;
use managers::history::HistoryManager;
use managers::model::ModelManager;
use managers::transcription::TranscriptionManager;
#[cfg(unix)]
use signal_hook::consts::{SIGUSR1, SIGUSR2};
#[cfg(unix)]
use signal_hook::iterator::Signals;
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
    let transcription_manager = Arc::new(
        TranscriptionManager::new(app_handle, model_manager.clone(), stream_router.clone())
            .expect("Failed to initialize transcription manager"),
    );
    let history_manager =
        Arc::new(HistoryManager::new(app_handle).expect("Failed to initialize history manager"));

    // Built-in local LLM engine (manages the bundled llama.cpp sidecar).
    let local_llm_manager = Arc::new(
        managers::local_llm::LocalLlmManager::new(app_handle)
            .expect("Failed to initialize local LLM manager"),
    );

    // Initialize transcribe.cpp (logging + backend modules) once, before any
    // transcribe.cpp model load or device enumeration. Failures are logged and
    // swallowed so transcribe-rs engines keep working (N1). See PLAN.md S2.
    managers::transcription::init_transcribe_cpp();

    // Apply accelerator preferences before any model loads
    managers::transcription::apply_accelerator_settings(app_handle);

    // Add managers to Tauri's managed state
    app_handle.manage(recording_manager.clone());
    app_handle.manage(model_manager.clone());
    app_handle.manage(transcription_manager.clone());
    app_handle.manage(history_manager.clone());
    app_handle.manage(local_llm_manager.clone());

    // Apply history retention once, at startup. This is the ONLY place old
    // recordings are pruned: changing the history limit / retention period
    // just persists the value (see commands::history), so nothing is ever
    // deleted mid-session — a changed limit takes effect on the next launch.
    // Runs off-thread so DB/file IO can't delay window creation.
    {
        let history_manager = history_manager.clone();
        std::thread::spawn(move || {
            if let Err(e) = history_manager.cleanup_old_entries() {
                log::error!("Startup history cleanup failed: {}", e);
            }
        });
    }

    // Start the idle watcher that unloads the built-in LLM after it has been
    // idle for the configured timeout, freeing RAM/VRAM when it's not in use.
    managers::local_llm::LocalLlmManager::spawn_idle_watcher(&local_llm_manager);

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    #[cfg(unix)]
    let signals = Signals::new(&[SIGUSR1, SIGUSR2]).unwrap();
    // Set up signal handlers for toggling transcription
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone(), signals);

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
            "check_updates" => {
                let settings = settings::get_settings(app);
                if settings.update_checks_enabled {
                    show_main_window(app);
                    let _ = app.emit("check-for-updates", ());
                }
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

    // Refresh tray menu when model state changes
    let app_handle_for_listener = app_handle.clone();
    app_handle.listen("model-state-changed", move |_| {
        tray::update_tray_menu(&app_handle_for_listener, &tray::TrayIconState::Idle, None);
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
    let settings = settings::get_settings(&app);
    if !settings.update_checks_enabled {
        return Ok(());
    }
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
            shortcut::change_tap_to_lock_setting,
            shortcut::change_tap_to_lock_key_setting,
            shortcut::change_assistant_tap_to_lock_key_setting,
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
            shortcut::change_post_process_fix_misheard_setting,
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
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_keyboard_implementation_setting,
            shortcut::get_keyboard_implementation,
            shortcut::change_show_tray_icon_setting,
            shortcut::change_whisper_accelerator_setting,
            shortcut::change_ort_accelerator_setting,
            shortcut::change_whisper_gpu_device,
            shortcut::get_available_accelerators,
            shortcut::handy_keys::start_handy_keys_recording,
            shortcut::handy_keys::stop_handy_keys_recording,
            trigger_update_check,
            show_main_window_command,
            commands::cancel_operation,
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
            commands::history::get_history_entries,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::retry_history_entry_transcription,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            commands::history::get_assistant_history_entries,
            commands::history::delete_assistant_history_entry,
            commands::assistant::assistant_send_text,
            commands::assistant::assistant_send_composed,
            commands::assistant::assistant_read_file,
            commands::assistant::assistant_read_image,
            commands::assistant::assistant_begin_region_snip,
            commands::assistant::assistant_finish_region_snip,
            commands::assistant::assistant_get_conversation,
            commands::assistant::assistant_regenerate,
            commands::assistant::assistant_summarize,
            commands::assistant::assistant_resume_session,
            commands::assistant::assistant_clear_conversation,
            commands::assistant::toggle_assistant_panel,
            commands::assistant::hide_assistant_panel,
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
            commands::assistant::set_assistant_screen_access_mode,
            commands::assistant::set_assistant_screenshot_enabled,
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
            commands::assistant::set_assistant_tts_speed,
            commands::assistant::set_assistant_panel_opacity,
            commands::assistant::set_assistant_panel_size,
            commands::assistant::set_assistant_tts_stop_on_dictation,
            commands::assistant::assistant_set_pending_attachments,
            commands::assistant::redirect_transcription_to_assistant,
            commands::assistant::set_assistant_panel_collapsed,
            commands::assistant::get_assistant_panel_collapsed,
            commands::assistant::set_assistant_screen_armed,
            commands::assistant::get_assistant_screen_armed,
            commands::assistant::assistant_toggle_voice,
            commands::assistant::assistant_speak,
            commands::assistant::assistant_test_tts,
            commands::assistant::assistant_list_azure_voices,
            commands::assistant::assistant_list_tts_voices,
            commands::assistant::assistant_list_tts_models,
            commands::assistant::assistant_stop,
            commands::assistant::set_assistant_max_history_messages,
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
            helpers::clamshell::is_laptop,
        ])
        .events(collect_events![managers::history::HistoryUpdatePayload,]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            Typescript::default().bigint(BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let invoke_handler = specta_builder.invoke_handler();

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

    builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle-transcription") {
                signal_handle::send_transcription_input(app, "transcribe", "CLI");
            } else if args.iter().any(|a| a == "--toggle-post-process") {
                signal_handle::send_transcription_input(app, "transcribe_with_post_process", "CLI");
            } else if args.iter().any(|a| a == "--toggle-assistant") {
                signal_handle::send_transcription_input(app, "assistant", "CLI");
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
            specta_builder.mount_events(app);

            // Create main window programmatically so we can set data_directory
            // for portable mode (redirects WebView2 cache to portable Data dir)
            let mut win_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
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
            // Tap-to-lock: watches for a Shift tap to flip a hold recording to
            // hands-free. Spawns its own global keyboard listener thread.
            app.manage(lock_watch::LockWatch::new(app_handle.clone()));
            app.manage(assistant::AssistantConversation::new());

            initialize_core_logic(&app_handle);

            // Create the assistant panel window (hidden until first use)
            assistant::create_assistant_panel(&app_handle);

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
            if should_force_show || !should_hide || !tray_available {
                show_main_window(&app_handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                // Remember the size before hiding, so it reopens as left.
                save_main_window_size(window);
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
            let _ = (app, event); // suppress unused warnings on non-macOS
        });
}
