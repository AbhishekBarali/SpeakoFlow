//! Handy-keys based keyboard shortcut implementation
//!
//! This module provides an alternative to Tauri's global-shortcut plugin
//! using the handy-keys library for more control over keyboard events.
//!
//! ## Architecture
//!
//! The implementation uses a dedicated manager thread that owns the `HotkeyManager`:
//!
//! ```text
//! ┌─────────────────┐     commands      ┌──────────────────────┐
//! │   Main Thread   │ ───────────────▶ │   Manager Thread     │
//! │                 │   (via channel)   │                      │
//! │ - register()    │                   │ - owns HotkeyManager │
//! │ - unregister()  │                   │ - waits for events   │
//! └─────────────────┘                   │ - dispatches actions │
//!                                       └──────────────────────┘
//! ```
//!
//! This design ensures thread-safety since `HotkeyManager` is only accessed
//! from a single thread. Commands (register/unregister) are sent via an mpsc
//! channel and responses are synchronously awaited.
//!
//! ## Recording Mode
//!
//! For UI key capture, a separate `KeyboardListener` is created on-demand and
//! polled from a dedicated recording thread. Events are emitted to the frontend
//! via Tauri's event system.

use handy_keys::{Hotkey, HotkeyEvent, HotkeyId, HotkeyManager, HotkeyState, KeyboardListener};
use log::{debug, error, info, warn};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tauri::{AppHandle, Emitter, Manager};

use crate::hands_free;
use crate::settings::{self, get_settings, AppSettings, ShortcutBinding};

use super::handler::{handle_shortcut_cancelled, handle_shortcut_event};

/// How long a caller waits for the manager thread to answer a register /
/// unregister request. The channel wakes the thread immediately unless it is
/// busy running a shortcut action, so this is generous — its job is to make sure
/// a wedged engine can never freeze the caller (which may be the UI thread)
/// forever, as an unbounded `recv()` did.
const MANAGER_REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Commands that can be sent to the hotkey manager thread
enum ManagerCommand {
    Hotkey(HotkeyEvent),
    Register {
        binding_id: String,
        hotkey_string: String,
        /// Carries on the modifier-only hotkey it extends instead of replacing
        /// it (`HotkeyManager::register_continuation`).
        continues: bool,
        response: Sender<Result<(), String>>,
    },
    Unregister {
        binding_id: String,
        response: Sender<Result<(), String>>,
    },
    /// Fire each shortcut's modifier-only hotkey on release (Tap to toggle) or
    /// on press (Hold to talk), by its own setting
    /// (`settings::holds_to_record`). See
    /// `HotkeyManager::set_fire_on_release_for`.
    ApplyRecordingModes,
    Shutdown,
}

/// State for the handy-keys shortcut manager
pub struct HandyKeysState {
    /// Channel to send commands to the manager thread (wrapped in Mutex for Sync)
    command_sender: Mutex<Sender<ManagerCommand>>,
    /// Handle to the manager thread (wrapped in Mutex for Sync, allows proper join on drop)
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    /// Recording listener for UI key capture (only active during recording)
    recording_listener: Mutex<Option<KeyboardListener>>,
    /// Flag indicating if we're in recording mode
    is_recording: AtomicBool,
    /// The binding ID being recorded (if any)
    recording_binding_id: Mutex<Option<String>>,
    /// Flag to stop recording loop
    recording_running: Arc<AtomicBool>,
    /// The keys each shortcut is registered on right now. A hands-free key is
    /// derived from its dictation shortcut's live keys, which while that
    /// shortcut is being changed are not yet the stored ones, and while it is
    /// being edited are none at all.
    registered: Mutex<HashMap<String, String>>,
}

/// Key event sent to frontend during recording mode
#[derive(Debug, Clone, Serialize, Type)]
pub struct FrontendKeyEvent {
    /// Currently pressed modifier keys
    pub modifiers: Vec<String>,
    /// The key that was pressed (if any)
    pub key: Option<String>,
    /// Whether this is a key down event
    pub is_key_down: bool,
    /// The full hotkey string (e.g., "option+space")
    pub hotkey_string: String,
}

impl HandyKeysState {
    /// Create a new HandyKeysState
    pub fn new(app: AppHandle) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<ManagerCommand>();

        // Start the manager thread
        let app_clone = app.clone();
        let event_tx = cmd_tx.clone();
        let thread_handle = thread::spawn(move || {
            Self::manager_thread(cmd_rx, event_tx, app_clone);
        });

        Ok(Self {
            command_sender: Mutex::new(cmd_tx),
            thread_handle: Mutex::new(Some(thread_handle)),
            recording_listener: Mutex::new(None),
            is_recording: AtomicBool::new(false),
            recording_binding_id: Mutex::new(None),
            recording_running: Arc::new(AtomicBool::new(false)),
            registered: Mutex::new(HashMap::new()),
        })
    }

    /// The main manager thread - owns the HotkeyManager and processes commands
    fn manager_thread(
        cmd_rx: Receiver<ManagerCommand>,
        event_tx: Sender<ManagerCommand>,
        app: AppHandle,
    ) {
        info!("handy-keys manager thread started");

        // Create the HotkeyManager in this thread
        let manager = match HotkeyManager::new_with_blocking_handler(move |event| {
            event_tx.send(ManagerCommand::Hotkey(event)).is_ok()
        }) {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to create HotkeyManager: {}", e);
                return;
            }
        };
        // Maps binding IDs to HotkeyIds and hotkey strings
        let mut binding_to_hotkey: HashMap<String, HotkeyId> = HashMap::new();
        let mut hotkey_to_binding: HashMap<HotkeyId, (String, String)> = HashMap::new(); // (binding_id, hotkey_string)

        // Hotkeys and configuration commands wake the same blocking receive.
        // No periodic idle wakeups and no poll delay before a recording starts.
        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                ManagerCommand::Hotkey(event) => {
                    if let Some((binding_id, hotkey_string)) = hotkey_to_binding.get(&event.id) {
                        debug!(
                            "handy-keys event: binding={}, hotkey={}, state={:?}",
                            binding_id, hotkey_string, event.state
                        );
                        match event.state {
                            HotkeyState::Pressed | HotkeyState::Released => handle_shortcut_event(
                                &app,
                                binding_id,
                                hotkey_string,
                                event.state == HotkeyState::Pressed,
                            ),
                            HotkeyState::Cancelled => handle_shortcut_cancelled(&app, binding_id),
                        }
                    }
                }
                ManagerCommand::Register {
                    binding_id,
                    hotkey_string,
                    continues,
                    response,
                } => {
                    let result = Self::do_register(
                        &manager,
                        &mut binding_to_hotkey,
                        &mut hotkey_to_binding,
                        &binding_id,
                        &hotkey_string,
                        continues,
                    );
                    if result.is_ok() {
                        Self::apply_recording_mode_to(
                            &manager,
                            &binding_to_hotkey,
                            &get_settings(&app),
                            &binding_id,
                        );
                    }
                    let _ = response.send(result);
                }
                ManagerCommand::Unregister {
                    binding_id,
                    response,
                } => {
                    let result = Self::do_unregister(
                        &manager,
                        &mut binding_to_hotkey,
                        &mut hotkey_to_binding,
                        &binding_id,
                    );
                    let _ = response.send(result);
                }
                ManagerCommand::ApplyRecordingModes => {
                    let settings = get_settings(&app);
                    for binding_id in binding_to_hotkey.keys() {
                        Self::apply_recording_mode_to(
                            &manager,
                            &binding_to_hotkey,
                            &settings,
                            binding_id,
                        );
                    }
                }
                ManagerCommand::Shutdown => {
                    info!("handy-keys manager thread shutting down");
                    break;
                }
            }
        }

        info!("handy-keys manager thread stopped");
    }

    /// Register a hotkey
    fn do_register(
        manager: &HotkeyManager,
        binding_to_hotkey: &mut HashMap<String, HotkeyId>,
        hotkey_to_binding: &mut HashMap<HotkeyId, (String, String)>,
        binding_id: &str,
        hotkey_string: &str,
        continues: bool,
    ) -> Result<(), String> {
        let hotkey: Hotkey = hotkey_string
            .parse()
            .map_err(|e| format!("Failed to parse hotkey '{}': {}", hotkey_string, e))?;

        let registered = if continues {
            manager.register_continuation(hotkey)
        } else {
            manager.register(hotkey)
        };
        let id = registered.map_err(|e| format!("Failed to register hotkey: {}", e))?;

        binding_to_hotkey.insert(binding_id.to_string(), id);
        hotkey_to_binding.insert(id, (binding_id.to_string(), hotkey_string.to_string()));

        debug!(
            "Registered handy-keys shortcut: {} -> {:?}",
            binding_id, hotkey
        );
        Ok(())
    }

    /// With Tap to toggle a modifier-only shortcut fires when its keys are let
    /// go, so the ask never starts on the way to the call's Ctrl+Alt+C; with
    /// Hold to talk it fires at once. Dictation and the ask each have their own
    /// setting, and every other shortcut fires on release, which only matters
    /// for one made of modifiers alone: a press that might be the start of
    /// another shortcut must not start a call.
    fn apply_recording_mode_to(
        manager: &HotkeyManager,
        binding_to_hotkey: &HashMap<String, HotkeyId>,
        settings: &AppSettings,
        binding_id: &str,
    ) {
        if let Some(&id) = binding_to_hotkey.get(binding_id) {
            manager.set_fire_on_release_for(id, !settings::holds_to_record(settings, binding_id));
        }
    }

    /// Unregister a hotkey
    fn do_unregister(
        manager: &HotkeyManager,
        binding_to_hotkey: &mut HashMap<String, HotkeyId>,
        hotkey_to_binding: &mut HashMap<HotkeyId, (String, String)>,
        binding_id: &str,
    ) -> Result<(), String> {
        if let Some(id) = binding_to_hotkey.remove(binding_id) {
            manager
                .unregister(id)
                .map_err(|e| format!("Failed to unregister hotkey: {}", e))?;
            hotkey_to_binding.remove(&id);
            debug!("Unregistered handy-keys shortcut: {}", binding_id);
        }
        Ok(())
    }

    /// Register a shortcut binding
    pub fn register(&self, binding: &ShortcutBinding) -> Result<(), String> {
        self.send_register(&binding.id, &binding.current_binding, false)?;
        if let Ok(mut registered) = self.registered.lock() {
            registered.insert(binding.id.clone(), binding.current_binding.clone());
        }
        Ok(())
    }

    /// Register `keys` under `id` as a continuation of the modifier-only
    /// shortcut they extend (`HotkeyManager::register_continuation`).
    fn register_continuation(&self, id: &str, keys: &str) -> Result<(), String> {
        self.send_register(id, keys, true)
    }

    fn send_register(&self, id: &str, keys: &str, continues: bool) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.command_sender
            .lock()
            .map_err(|_| "Failed to lock command_sender")?
            .send(ManagerCommand::Register {
                binding_id: id.to_string(),
                hotkey_string: keys.to_string(),
                continues,
                response: tx,
            })
            .map_err(|_| "Failed to send register command")?;

        rx.recv_timeout(MANAGER_REPLY_TIMEOUT)
            .map_err(|_| "Timed out waiting for the hotkey engine to register".to_string())?
    }

    /// Unregister a shortcut binding
    pub fn unregister(&self, binding: &ShortcutBinding) -> Result<(), String> {
        self.unregister_id(&binding.id)?;
        if let Ok(mut registered) = self.registered.lock() {
            registered.remove(&binding.id);
        }
        Ok(())
    }

    /// Unregister whatever is registered under `id`. Nothing there is fine.
    fn unregister_id(&self, id: &str) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.command_sender
            .lock()
            .map_err(|_| "Failed to lock command_sender")?
            .send(ManagerCommand::Unregister {
                binding_id: id.to_string(),
                response: tx,
            })
            .map_err(|_| "Failed to send unregister command")?;

        rx.recv_timeout(MANAGER_REPLY_TIMEOUT)
            .map_err(|_| "Timed out waiting for the hotkey engine to unregister".to_string())?
    }

    /// Release both dictation shortcuts' hands-free keys, so a shortcut about
    /// to be registered on the same keys is not refused as a duplicate. The
    /// next [`Self::sync_hands_free_keys`] puts back whichever still belong.
    fn release_hands_free_keys(&self) {
        for binding_id in ["transcribe", settings::CLEANUP_BINDING_ID] {
            let _ = self.unregister_id(&hands_free::key_id(binding_id));
        }
    }

    /// Bring both dictation shortcuts' hands-free hotkeys (`hands_free`) in
    /// line with `settings` and with what is registered: one for each dictation
    /// shortcut that is live, held to talk, and can have one, on its live keys,
    /// and none for a shortcut that is off, being edited, tapped, or whose keys
    /// plus the hands-free key another live shortcut holds. Releases first, so
    /// it is safe to repeat.
    fn sync_hands_free_keys(&self, settings: &AppSettings) {
        self.release_hands_free_keys();
        let live = match self.registered.lock() {
            Ok(registered) => registered.clone(),
            Err(_) => return,
        };
        let pairs = || live.iter().map(|(id, keys)| (id.as_str(), keys.as_str()));
        for binding_id in ["transcribe", settings::CLEANUP_BINDING_ID] {
            let Some(keys) = live.get(binding_id) else {
                continue;
            };
            let Some(extended) = hands_free::keys_for(settings, binding_id, keys, pairs()) else {
                continue;
            };
            let id = hands_free::key_id(binding_id);
            match self.register_continuation(&id, &extended) {
                Ok(()) => debug!("Hands-free key for '{binding_id}': {extended}"),
                Err(e) => warn!("Could not register the hands-free key {extended}: {e}"),
            }
        }
    }

    /// Start recording mode for a specific binding
    pub fn start_recording(&self, app: &AppHandle, binding_id: String) -> Result<(), String> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Err("Already recording".into());
        }

        // Create a new keyboard listener for recording
        let listener = KeyboardListener::new()
            .map_err(|e| format!("Failed to create keyboard listener: {}", e))?;

        {
            let mut recording = self
                .recording_listener
                .lock()
                .map_err(|_| "Failed to lock recording_listener")?;
            *recording = Some(listener);
        }
        {
            let mut binding = self
                .recording_binding_id
                .lock()
                .map_err(|_| "Failed to lock recording_binding_id")?;
            *binding = Some(binding_id);
        }

        self.is_recording.store(true, Ordering::SeqCst);
        self.recording_running.store(true, Ordering::SeqCst);

        // Start a thread to emit key events to the frontend
        let app_clone = app.clone();
        let recording_running = Arc::clone(&self.recording_running);
        thread::spawn(move || {
            Self::recording_loop(app_clone, recording_running);
        });

        debug!("Started handy-keys recording mode");
        Ok(())
    }

    /// Recording loop - emits key events to frontend during recording
    fn recording_loop(app: AppHandle, running: Arc<AtomicBool>) {
        while running.load(Ordering::SeqCst) {
            let event = {
                let state = match app.try_state::<HandyKeysState>() {
                    Some(s) => s,
                    None => break,
                };
                let listener = state.recording_listener.lock().ok();
                listener.as_ref().and_then(|l| l.as_ref()?.try_recv())
            };

            if let Some(key_event) = event {
                // Convert to frontend-friendly format
                let frontend_event = FrontendKeyEvent {
                    modifiers: modifiers_to_strings(key_event.modifiers),
                    key: key_event.key.map(|k| k.to_string().to_lowercase()),
                    is_key_down: key_event.is_key_down,
                    hotkey_string: key_event
                        .as_hotkey()
                        .map(|h| h.to_handy_string())
                        .unwrap_or_default(),
                };

                // Emit to frontend
                if let Err(e) = app.emit("handy-keys-event", &frontend_event) {
                    error!("Failed to emit key event: {}", e);
                }
            } else {
                thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        debug!("Recording loop ended");
    }

    /// Stop recording mode
    pub fn stop_recording(&self) -> Result<(), String> {
        self.is_recording.store(false, Ordering::SeqCst);
        self.recording_running.store(false, Ordering::SeqCst);

        {
            let mut recording = self
                .recording_listener
                .lock()
                .map_err(|_| "Failed to lock recording_listener")?;
            *recording = None;
        }
        {
            let mut binding = self
                .recording_binding_id
                .lock()
                .map_err(|_| "Failed to lock recording_binding_id")?;
            *binding = None;
        }

        debug!("Stopped handy-keys recording mode");
        Ok(())
    }
}

impl Drop for HandyKeysState {
    fn drop(&mut self) {
        // Signal recording to stop
        self.recording_running.store(false, Ordering::SeqCst);
        self.is_recording.store(false, Ordering::SeqCst);

        // Send shutdown command
        if let Ok(sender) = self.command_sender.lock() {
            let _ = sender.send(ManagerCommand::Shutdown);
        }

        // Wait for the manager thread to finish
        if let Ok(mut handle) = self.thread_handle.lock() {
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
        }
    }
}

/// Convert handy-keys Modifiers to a list of strings
fn modifiers_to_strings(modifiers: handy_keys::Modifiers) -> Vec<String> {
    let mut result = Vec::new();

    if modifiers.contains(handy_keys::Modifiers::CTRL) {
        result.push("ctrl".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::OPT) {
        #[cfg(target_os = "macos")]
        result.push("option".to_string());
        #[cfg(not(target_os = "macos"))]
        result.push("alt".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::SHIFT) {
        result.push("shift".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::CMD) {
        #[cfg(target_os = "macos")]
        result.push("command".to_string());
        #[cfg(not(target_os = "macos"))]
        result.push("super".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::FN) {
        result.push("fn".to_string());
    }

    result
}

/// Validate a shortcut string for the HandyKeys implementation.
/// HandyKeys is more permissive: allows modifier-only combos and the fn key.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }
    // HandyKeys accepts modifier-only, key-only, and modifier+key combos
    // Just verify the string is parseable
    raw.parse::<Hotkey>()
        .map(|_| ())
        .map_err(|e| format!("Invalid shortcut for HandyKeys: {}", e))
}

/// Initialize handy-keys shortcuts
pub fn init_shortcuts(app: &AppHandle) -> Result<(), String> {
    let state = HandyKeysState::new(app.clone())?;

    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    // Register all bindings except cancel (which is dynamic) and the
    // hands-free key (which only ever extends the dictation keys).
    for (id, default_binding) in default_bindings {
        if id == "cancel" || id == settings::HANDS_FREE_BINDING_ID {
            continue;
        }
        // Skip the separate cleanup shortcut unless it is the one that runs
        // cleanup: off with the feature, and off when cleanup rides on the
        // dictation shortcut instead.
        if id == settings::CLEANUP_BINDING_ID && !settings::cleanup_binding_active(&user_settings) {
            continue;
        }
        // Same for the assistant's own shortcuts when the master switch is off:
        // `set_assistant_enabled` unregisters them at runtime, and without this
        // the next launch would silently grab those combos again — swallowing
        // them from other apps for a feature that does nothing.
        if crate::assistant::is_assistant_binding(&id) && !user_settings.assistant_enabled {
            continue;
        }

        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        // An empty binding means "disabled" — nothing to register.
        if binding.current_binding.trim().is_empty() {
            continue;
        }

        if let Err(e) = state.register(&binding) {
            error!(
                "Failed to register handy-keys shortcut {} during init: {}",
                id, e
            );
        }
    }
    // Once every shortcut is in, so whether one of them holds a hands-free
    // key's keys never depends on the order they were registered in.
    state.sync_hands_free_keys(&user_settings);

    app.manage(state);
    info!("handy-keys shortcuts initialized");
    Ok(())
}

/// Re-derive the dictation shortcuts' hands-free keys after a setting that
/// decides them has changed. No-op when this engine is not running.
pub fn sync_hands_free_keys(app: &AppHandle) {
    if let Some(state) = app.try_state::<HandyKeysState>() {
        state.sync_hands_free_keys(&get_settings(app));
    }
}

/// Tell the hotkey engine whether each recording shortcut is held or tapped,
/// so it fires its modifier-only hotkey on press or on release, and give a
/// held dictation its hands-free hotkey (a tapped one has none). No-op when
/// this engine is not running.
pub fn apply_recording_mode(app: &AppHandle) {
    let Some(state) = app.try_state::<HandyKeysState>() else {
        return;
    };
    if let Ok(sender) = state.command_sender.lock() {
        let _ = sender.send(ManagerCommand::ApplyRecordingModes);
    };
    state.sync_hands_free_keys(&get_settings(app));
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(app: &AppHandle) {
    // Disabled on Linux due to instability
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                // Empty binding = cancel shortcut disabled.
                if cancel_binding.current_binding.trim().is_empty() {
                    return;
                }
                if let Some(state) = app_clone.try_state::<HandyKeysState>() {
                    if let Err(e) = state.register(&cancel_binding) {
                        error!("Failed to register cancel shortcut: {}", e);
                    }
                }
            }
        });
    }
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                if let Some(state) = app_clone.try_state::<HandyKeysState>() {
                    let _ = state.unregister(&cancel_binding);
                }
            }
        });
    }
}

/// Register a shortcut
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    // An empty binding means "disabled" — nothing to register.
    if binding.current_binding.trim().is_empty() {
        return Ok(());
    }
    let state = app
        .try_state::<HandyKeysState>()
        .ok_or("HandyKeysState not initialized")?;
    // A hands-free key on these very keys would get this shortcut refused as a
    // duplicate. It steps aside, and the sync puts back whichever still belong
    // (a dictation shortcut's own among them).
    state.release_hands_free_keys();
    let result = state.register(&binding);
    state.sync_hands_free_keys(&get_settings(app));
    result
}

/// Unregister a shortcut
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    if binding.current_binding.trim().is_empty() {
        return Ok(());
    }
    let state = app
        .try_state::<HandyKeysState>()
        .ok_or("HandyKeysState not initialized")?;
    let result = state.unregister(&binding);
    // A dictation shortcut takes its hands-free key with it; any other may have
    // been holding the keys one would use.
    state.sync_hands_free_keys(&get_settings(app));
    result
}

/// Start key recording mode
#[tauri::command]
#[specta::specta]
pub fn start_handy_keys_recording(app: AppHandle, binding_id: String) -> Result<(), String> {
    let settings = get_settings(&app);
    if settings.keyboard_implementation != settings::KeyboardImplementation::HandyKeys {
        return Err("handy-keys is not the active keyboard implementation".into());
    }

    let state = app
        .try_state::<HandyKeysState>()
        .ok_or("HandyKeysState not initialized")?;
    state.start_recording(&app, binding_id)
}

/// Stop key recording mode
#[tauri::command]
#[specta::specta]
pub fn stop_handy_keys_recording(app: AppHandle) -> Result<(), String> {
    let settings = get_settings(&app);
    if settings.keyboard_implementation != settings::KeyboardImplementation::HandyKeys {
        return Err("handy-keys is not the active keyboard implementation".into());
    }

    let state = app
        .try_state::<HandyKeysState>()
        .ok_or("HandyKeysState not initialized")?;
    state.stop_recording()
}
