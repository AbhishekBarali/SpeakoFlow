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

/// Withhold the keystrokes we are about to synthesize from global-hotkey
/// matching, until the returned guard is dropped.
///
/// Every synthetic sequence in this module and in `clipboard.rs` opens one, and
/// the reason is a bug that made hold-to-talk record nothing at all. The
/// assistant harvests the focused app's selection with a synthetic Ctrl+C at
/// recording start (see `selection.rs`). On Windows our own low-level keyboard
/// hook sees that injected Ctrl key-up, and for a modifier-only hotkey like the
/// default `Left Ctrl + Left Alt` it is indistinguishable from the user letting
/// go — so the recording stopped roughly 30 ms after it started, every time.
///
/// [`conflicting_modifier_held`] was supposed to prevent exactly this and could
/// not: it asks Windows whether Alt is down, and the hotkey engine had *blocked*
/// that Alt key-down from ever reaching Windows.
///
/// The guard is a window rather than a blanket rule, so an external macro
/// keyboard or accessibility tool that fires a registered hotkey still works.
/// The keystrokes themselves are unaffected and still reach the target app.
fn synthesizing() -> handy_keys::InjectedInputGuard {
    handy_keys::ignore_injected_input()
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
    let _injected = synthesizing();
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
    let _injected = synthesizing();
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
    let _injected = synthesizing();
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
    let _injected = synthesizing();
    enigo
        .text(text)
        .map_err(|e| format!("Failed to send text directly: {}", e))?;

    Ok(())
}

/// Sends a Ctrl+C or Cmd+C copy command using platform-specific virtual key
/// codes, so it works regardless of keyboard layout.
///
/// The mirror image of [`send_paste_ctrl_v`], and it carries the same guarantee:
/// once the modifier is down a matching release is always sent, even if the C
/// keypress itself fails. A modifier left pressed at the OS level reads to the
/// user as a physically stuck key.
///
/// Used to harvest the focused application's selection. A successful return means
/// the keystroke was *sent*, not that the target did anything with it — plenty of
/// apps ignore it. Proving something actually arrived is the caller's job (see
/// `selection.rs`), and it matters: reading the clipboard after a copy that did
/// nothing yields whatever the user copied earlier, which could be anything.
pub fn send_copy_combo(enigo: &mut Enigo) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (modifier_key, c_key_code) = (Key::Meta, Key::Other(8)); // kVK_ANSI_C
    #[cfg(target_os = "windows")]
    let (modifier_key, c_key_code) = (Key::Control, Key::Other(0x43)); // VK_C
    #[cfg(target_os = "linux")]
    let (modifier_key, c_key_code) = (Key::Control, Key::Unicode('c'));

    let _injected = synthesizing();

    // The user is very often already holding this exact modifier, because it is
    // in both shipped Windows hotkeys (dictation is Left Ctrl + Left Super, the
    // assistant is Left Ctrl + Left Alt) and this runs at recording start. Press
    // and release it anyway and two things go wrong: the release desynchronises
    // the OS from the key they are still physically holding, so the rest of their
    // hold behaves as though Ctrl were up; and the keystroke is redundant, since
    // their own Ctrl already supplies the modifier. So borrow theirs instead, and
    // only release what we actually pressed.
    let modifier_already_held = copy_modifier_held();
    if !modifier_already_held {
        enigo
            .key(modifier_key, enigo::Direction::Press)
            .map_err(|e| format!("Failed to press modifier key: {}", e))?;
    }

    let click = enigo
        .key(c_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click C key: {}", e));

    if click.is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(30));
    }

    if modifier_already_held {
        return click;
    }

    let release = enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e));

    click.and(release)
}

/// Whether the user is already physically holding the modifier
/// [`send_copy_combo`] would otherwise press for them.
///
/// Unlike the Alt/Super half of a hotkey, this modifier is reliably visible to
/// the OS: a modifier-only combo is only matched (and therefore only blocked by
/// the keyboard hook) once its *last* key goes down, so the first one — Ctrl in
/// both shipped Windows defaults — was passed straight through.
#[cfg(target_os = "windows")]
fn copy_modifier_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LCONTROL, VK_RCONTROL};
    [VK_LCONTROL, VK_RCONTROL]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) as u16 & 0x8000 != 0 })
}

#[cfg(not(target_os = "windows"))]
fn copy_modifier_held() -> bool {
    // macOS copies with Cmd while its hotkeys use Option/Ctrl, and the Linux path
    // prefers the X11 PRIMARY selection, which sends no keystroke at all. Neither
    // has the collision this exists to avoid.
    false
}

/// Whether the user is physically holding a modifier that would corrupt a
/// synthetic copy.
///
/// Not hypothetical. The Windows dictation combo is Left Ctrl + Left Super and
/// the assistant's is Left Ctrl + Left Alt, so when a turn begins the user is
/// usually still holding Ctrl *and* one of Super/Alt. Sending Ctrl+C on top of
/// that makes the OS see Ctrl+Super+C or Ctrl+Alt+C, which is not copy — and
/// calling [`release_all_modifiers`] afterwards would send releases for keys the
/// user is still pressing, desynchronising the OS from the physical keyboard and
/// potentially eating the release that ends the recording.
///
/// Ctrl is deliberately excluded: it is part of the combo being sent anyway, so
/// the user holding it changes nothing. [`send_copy_combo`] handles that case by
/// borrowing the modifier they are already holding instead of cycling its own.
///
/// One blind spot, worth knowing before trusting this function: it asks the OS,
/// and the OS does not know about a key the hotkey engine blocked. A modifier-only
/// combo is matched and blocked on its *last* key, so with the assistant's
/// `Left Ctrl + Left Alt` held, `GetAsyncKeyState(VK_LMENU)` answers "not down"
/// and this returns false. That is why the synthetic copy still ran during a
/// hold-to-talk recording, and why the injected keystrokes are now withheld from
/// hotkey matching at the source (see [`synthesizing`]) rather than relying on
/// this check alone.
#[cfg(target_os = "windows")]
pub fn conflicting_modifier_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RMENU, VK_RSHIFT, VK_RWIN,
    };
    // The high-order bit of the return value means "currently down".
    [VK_LWIN, VK_RWIN, VK_LMENU, VK_RMENU, VK_LSHIFT, VK_RSHIFT]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) as u16 & 0x8000 != 0 })
}

#[cfg(not(target_os = "windows"))]
pub fn conflicting_modifier_held() -> bool {
    // macOS copies with Cmd+C while its default hotkey is Option+Space, so the
    // two cannot collide. On Linux the X11 tools pass `--clearmodifiers`, which
    // solves this at the tool level, and the preferred X11 route reads the PRIMARY
    // selection without sending any keystroke at all.
    false
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
    let _injected = synthesizing();
    for key in [Key::Control, Key::Shift, Key::Alt, Key::Meta] {
        // Ignore errors: this is best-effort cleanup, and there's nothing useful
        // to do if a release fails.
        let _ = enigo.key(key, enigo::Direction::Release);
    }
}

// === Paste target ========================================================
//
// A synthetic Ctrl+V goes wherever keyboard focus happens to be at the instant
// it is sent, and nothing in the pipeline used to record where that was supposed
// to be. Between pressing the shortcut and the paste there is a whole recording,
// a transcription, and possibly an LLM cleanup pass — plenty of time for focus
// to move. When it did, the transcript landed in some other field, or in one of
// our own always-on-top windows, where it simply disappeared.
//
// So the window that was in front when recording began is remembered, and the
// paste path puts it back if one of *our* windows took its place. A move to
// another application is left alone on purpose: a user who alt-tabs mid-dictation
// usually means to dictate into the window they switched to, and stealing the
// foreground back from them would be a worse bug than the one being fixed.

/// The foreground window at recording start, as a raw handle.
///
/// Stored as an integer rather than an `HWND` because `HWND` is not `Sync`; it is
/// only ever handed back to Win32, and always revalidated with `IsWindow` first,
/// since the target may have closed while the user was talking.
#[cfg(target_os = "windows")]
static PASTE_TARGET: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Remember the window that should receive this recording's transcript.
///
/// Called at recording start. A foreground window that belongs to us is recorded
/// as "no target": the in-app dictation paths (character creation, the assistant
/// composer) legitimately paste into our own windows, and pinning the overlay or
/// the settings window as a restore target would fight them.
#[cfg(target_os = "windows")]
pub fn remember_paste_target() {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            PASTE_TARGET.store(0, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        let mut process_id = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == GetCurrentProcessId() {
            PASTE_TARGET.store(0, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        PASTE_TARGET.store(hwnd.0 as isize, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn remember_paste_target() {
    // macOS and Linux cannot reach this failure mode: the overlay is an NSPanel
    // with `can_become_key_window: false` there, and a layer-shell surface with
    // `KeyboardMode::None` on Wayland, so neither can take keyboard focus from
    // the window being dictated into in the first place.
}

/// Forget the remembered target (cancellation, or a completed paste).
#[cfg(target_os = "windows")]
pub fn forget_paste_target() {
    PASTE_TARGET.store(0, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(not(target_os = "windows"))]
pub fn forget_paste_target() {}

/// Put the remembered window back in front if one of our own windows took the
/// foreground. Returns true when it actually moved the foreground.
///
/// `SetForegroundWindow` is only granted to the process that already owns the
/// foreground, which is exactly the situation this repairs — and exactly why the
/// third-party case is not attempted rather than attempted and silently failed.
#[cfg(target_os = "windows")]
pub fn restore_paste_target() -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        SetForegroundWindow,
    };

    let stored = PASTE_TARGET.load(std::sync::atomic::Ordering::SeqCst);
    if stored == 0 {
        return false;
    }
    let target = HWND(stored as *mut std::ffi::c_void);

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground == target {
            return false;
        }
        // Only repair a foreground we took ourselves. Anything else is the user's
        // own window switch and is respected.
        let mut foreground_process = 0u32;
        if !foreground.is_invalid() {
            GetWindowThreadProcessId(foreground, Some(&mut foreground_process));
        }
        if foreground_process != GetCurrentProcessId() {
            if !foreground.is_invalid() {
                log::info!(
                    "Pasting into the window that is in front now, not the one \
                     recording started in — the user switched windows"
                );
            }
            return false;
        }
        if !IsWindow(Some(target)).as_bool()
            || !IsWindowVisible(target).as_bool()
            || IsIconic(target).as_bool()
        {
            log::warn!("The window dictation started in is gone; pasting where focus is");
            return false;
        }
        log::info!(
            "One of our own windows held the foreground at paste time; handing it \
             back to the window dictation started in"
        );
        let restored = SetForegroundWindow(target).as_bool();
        if !restored {
            log::warn!("Could not hand the foreground back before pasting");
            return false;
        }
        // Activation is asynchronous: the target's message loop has to process it
        // before it owns keyboard focus, and a Ctrl+V sent inside that window goes
        // to the old focus. Short and fixed, because this only runs on the repair
        // path — the ordinary paste is untouched.
        std::thread::sleep(std::time::Duration::from_millis(60));
        true
    }
}

#[cfg(not(target_os = "windows"))]
pub fn restore_paste_target() -> bool {
    false
}
