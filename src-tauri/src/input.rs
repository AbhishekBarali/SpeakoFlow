use enigo::{Enigo, Key, Keyboard, Mouse, Settings};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Wrapper for Enigo to store in Tauri's managed state.
/// Enigo is wrapped in a Mutex since it requires mutable access.
pub struct EnigoState(pub Mutex<Enigo>);

impl EnigoState {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to initialize Enigo: {}", e))?;
        Ok(Self(Mutex::new(enigo)))
    }
}

/// Get the current mouse cursor position using the managed Enigo instance.
/// Returns None if the state is not available or if getting the location fails.
pub fn get_cursor_position(app_handle: &AppHandle) -> Option<(i32, i32)> {
    let enigo_state = app_handle.try_state::<EnigoState>()?;
    let enigo = enigo_state.0.lock().ok()?;
    enigo.location().ok()
}

/// Sends a Ctrl+V or Cmd+V paste command using platform-specific virtual key codes.
/// This ensures the paste works regardless of keyboard layout (e.g., Russian, AZERTY, DVORAK).
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_ctrl_v(enigo: &mut Enigo) -> Result<(), String> {
    // Platform-specific key definitions
    #[cfg(target_os = "macos")]
    let (modifier_key, v_key_code) = (Key::Meta, Key::Other(9));
    #[cfg(target_os = "windows")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Other(0x56)); // VK_V
    #[cfg(target_os = "linux")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Unicode('v'));

    // Press the modifier, then click V. From the moment the modifier is down we
    // must guarantee a matching release — even if clicking V fails — otherwise
    // the modifier (Ctrl/Cmd) is left "pressed" at the OS level, which shows up
    // as a key stuck down continuously.
    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press modifier key: {}", e))?;

    let click = enigo
        .key(v_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click V key: {}", e));

    if click.is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let release = enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e));

    // Always attempt the release; surface the click error first if it failed.
    click.and(release)
}

/// Sends a Ctrl+Shift+V paste command.
/// This is commonly used in terminal applications on Linux to paste without formatting.
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_ctrl_shift_v(enigo: &mut Enigo) -> Result<(), String> {
    // Platform-specific key definitions
    #[cfg(target_os = "macos")]
    let (modifier_key, v_key_code) = (Key::Meta, Key::Other(9)); // Cmd+Shift+V on macOS
    #[cfg(target_os = "windows")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Other(0x56)); // VK_V
    #[cfg(target_os = "linux")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Unicode('v'));

    // Hold modifier + Shift, click V, then release both. Any failure after a key
    // goes down must still release everything, or Ctrl/Shift can be left stuck
    // "pressed" at the OS level.
    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press modifier key: {}", e))?;

    // If Shift fails to press, release the modifier we already pressed.
    if let Err(e) = enigo.key(Key::Shift, enigo::Direction::Press) {
        let _ = enigo.key(modifier_key, enigo::Direction::Release);
        return Err(format!("Failed to press Shift key: {}", e));
    }

    let click = enigo
        .key(v_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click V key: {}", e));

    if click.is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let release_shift = enigo
        .key(Key::Shift, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release Shift key: {}", e));
    let release_modifier = enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e));

    // Always release both; surface the first error encountered.
    click.and(release_shift).and(release_modifier)
}

/// Sends a Shift+Insert paste command (Windows and Linux only).
/// This is more universal for terminal applications and legacy software.
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_shift_insert(enigo: &mut Enigo) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let insert_key_code = Key::Other(0x2D); // VK_INSERT
    #[cfg(not(target_os = "windows"))]
    let insert_key_code = Key::Other(0x76); // XK_Insert (keycode 118 / 0x76, also used as fallback)

    // Hold Shift, click Insert, then release Shift. Release even if the Insert
    // click fails, so Shift is never left stuck "pressed".
    enigo
        .key(Key::Shift, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press Shift key: {}", e))?;

    let click = enigo
        .key(insert_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click Insert key: {}", e));

    if click.is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let release = enigo
        .key(Key::Shift, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release Shift key: {}", e));

    click.and(release)
}

/// Pastes text directly using the enigo text method.
/// This tries to use system input methods if possible, otherwise simulates keystrokes one by one.
pub fn paste_text_direct(enigo: &mut Enigo, text: &str) -> Result<(), String> {
    enigo
        .text(text)
        .map_err(|e| format!("Failed to send text directly: {}", e))?;

    Ok(())
}

/// Releases the common modifier keys (Ctrl, Shift, Alt, and Meta/Cmd/Super).
///
/// This is a safety net for synthetic-input flows. If a paste key-combo is
/// interrupted midway (an intermediate `enigo` call errors), a modifier could
/// otherwise be left "pressed" at the OS level, which manifests as a key being
/// held down continuously (e.g. Ctrl appearing stuck on). Sending a release for
/// a key that isn't currently down is harmless, so it's always safe to clear
/// them all after we're done synthesizing keystrokes.
pub fn release_all_modifiers(enigo: &mut Enigo) {
    for key in [Key::Control, Key::Shift, Key::Alt, Key::Meta] {
        // Ignore errors: this is best-effort cleanup, and there's nothing useful
        // to do if a release fails.
        let _ = enigo.key(key, enigo::Direction::Release);
    }
}
