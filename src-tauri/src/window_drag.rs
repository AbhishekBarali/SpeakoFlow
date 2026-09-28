//! Moving our frameless windows without Windows' modal move loop.
//!
//! On Windows, Tauri's `start_dragging` is `ReleaseCapture()` plus a posted
//! `WM_NCLBUTTONDOWN` on the caption, which hands the mouse to the system's modal
//! move loop. That loop is the source of a whole family of "the mouse stopped
//! working" reports (tauri-apps/tauri#10767, open upstream):
//!
//! - It swallows the button release, so the webview never sees `pointerup`. The
//!   assistant panel holds its window tangible from `pointerdown` to `pointerup`
//!   (see the cursor pass-through section in `assistant.rs`), so after every
//!   drag its whole transparent frame kept eating clicks meant for the app
//!   underneath, until the next click inside the panel.
//! - If the button is already up by the time the posted message is handled — the
//!   call is asynchronous, so a quick flick can beat it — the loop starts with
//!   nothing to end it. The window then follows the cursor, and owns the mouse,
//!   until the next click, which the loop consumes too.
//! - The loop also applies Aero Snap, which can dock or maximise a floating HUD
//!   into a screen-sized transparent window.
//!
//! None of that is needed to move a small always-on-top surface. Here the window
//! is moved by a short-lived thread that follows the cursor while the primary
//! button is **physically** held (`GetAsyncKeyState`), and stops the moment it is
//! not. Nothing captures the mouse, the webview receives its own release like any
//! other, and a missed release is impossible by construction: the loop reads the
//! button, not a message about it.
//!
//! The main settings window keeps the system move loop on purpose (see
//! `useSafeWindowDrag`'s `systemMove`), because a real application window is
//! expected to snap to screen edges.

/// Start moving the calling window with the cursor, for as long as the primary
/// mouse button stays down.
///
/// Called by the webview once a press on a drag region has travelled far enough
/// to be a drag (see `src/lib/useSafeWindowDrag.ts`). On macOS and Linux the
/// system drag has none of the Windows problems, so it is used there as before.
///
/// `system` asks for the system move loop anyway, for the main window, which
/// people expect to snap to screen edges. On Windows it is only entered while the
/// button is still physically down: the request crosses the IPC boundary after
/// the webview saw the press, and a flick that is over by then used to start a
/// loop with nothing left to end it.
#[tauri::command]
#[specta::specta]
pub fn start_window_drag(window: tauri::WebviewWindow, system: Option<bool>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if system.unwrap_or(false) {
            if !platform::primary_button_down() {
                return Ok(());
            }
            return window.start_dragging().map_err(|e| e.to_string());
        }
        platform::start(&window)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = system;
        window.start_dragging().map_err(|e| e.to_string())
    }
}

/// Start the system resize loop from one of the calling window's resize grips,
/// if the button that pressed the grip is still down.
///
/// The same guard as a system move, for the same reason: a resize loop entered
/// after the release has nothing to end it, and resizes the window with the
/// cursor — holding the mouse — until the next click. Windows only; everywhere
/// else the webview's own `startResizeDragging` is used and this is never called.
#[tauri::command]
#[specta::specta]
pub fn start_window_resize(window: tauri::WebviewWindow, direction: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        platform::start_resize(&window, &direction)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, direction);
        Err("start_window_resize is only used on Windows".to_string())
    }
}

/// Is any mouse button physically held right now?
///
/// For state that is only valid "while the pointer is down", such as the
/// assistant panel's drag hold: a webview's `pointerup` can be lost (a system
/// move or resize loop consumes it), and the OS is the one source that cannot be
/// out of date. Outside Windows there is no cheap equivalent, and nothing there
/// loses the release, so the caller's own bookkeeping is trusted.
pub fn any_mouse_button_down() -> bool {
    #[cfg(target_os = "windows")]
    {
        platform::any_button_down()
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VIRTUAL_KEY, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1, VK_XBUTTON2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetSystemMetrics, IsWindow, SetWindowPos, SM_SWAPBUTTON, SWP_NOACTIVATE,
        SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
    };

    /// One drag at a time: a new one retires the previous follower.
    static DRAG_GENERATION: AtomicU64 = AtomicU64::new(0);

    /// How often the cursor is followed. Std's sleep uses a high-resolution
    /// waitable timer on Windows, so this is a real 8 ms (~120 Hz), not the
    /// 15.6 ms default tick.
    const DRAG_TICK: Duration = Duration::from_millis(8);

    fn key_down(key: VIRTUAL_KEY) -> bool {
        // High bit set = physically down right now.
        unsafe { GetAsyncKeyState(key.0 as i32) as u16 & 0x8000 != 0 }
    }

    /// The button the user drags with. `GetAsyncKeyState` reads *physical*
    /// buttons, so on a left-handed setup the primary button is the right one.
    pub fn primary_button_down() -> bool {
        let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
        key_down(if swapped { VK_RBUTTON } else { VK_LBUTTON })
    }

    pub fn any_button_down() -> bool {
        [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
            .into_iter()
            .any(key_down)
    }

    fn cursor() -> Option<(i32, i32)> {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.ok()?;
        Some((point.x, point.y))
    }

    /// Hand a resize to the system loop, the way tao does for
    /// `start_resize_dragging` (release capture, then a synthetic press on the
    /// border), but only while the button that started it is still down.
    pub fn start_resize(window: &tauri::WebviewWindow, direction: &str) -> Result<(), String> {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows::Win32::UI::WindowsAndMessaging::{
            PostMessageW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT,
            HTTOPRIGHT, WM_NCLBUTTONDOWN,
        };
        let hit = match direction {
            "North" => HTTOP,
            "South" => HTBOTTOM,
            "East" => HTRIGHT,
            "West" => HTLEFT,
            "NorthEast" => HTTOPRIGHT,
            "NorthWest" => HTTOPLEFT,
            "SouthEast" => HTBOTTOMRIGHT,
            "SouthWest" => HTBOTTOMLEFT,
            other => return Err(format!("Unknown resize direction: {other}")),
        };
        if !primary_button_down() {
            return Ok(());
        }
        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        let (x, y) = cursor().ok_or_else(|| "The cursor position is unavailable".to_string())?;
        // MAKELPARAM(x, y): screen coordinates packed as two 16-bit halves.
        let point = ((y as u16 as u32) << 16) | (x as u16 as u32);
        unsafe {
            let _ = ReleaseCapture();
            PostMessageW(
                Some(hwnd),
                WM_NCLBUTTONDOWN,
                WPARAM(hit as usize),
                LPARAM(point as isize),
            )
            .map_err(|e| e.to_string())
        }
    }

    pub fn start(window: &tauri::WebviewWindow) -> Result<(), String> {
        let generation = DRAG_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        // Released before we got here: there is nothing to drag, and starting
        // anyway is exactly the stuck-window bug this module exists to remove.
        if !primary_button_down() {
            return Ok(());
        }
        let origin = window.outer_position().map_err(|e| e.to_string())?;
        let grab = cursor().ok_or_else(|| "The cursor position is unavailable".to_string())?;
        // `HWND` is not `Send`; it only ever goes back to Win32, and is checked
        // with `IsWindow` on every move in case the window is destroyed mid-drag.
        let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        std::thread::spawn(move || {
            let hwnd = HWND(hwnd as *mut std::ffi::c_void);
            let mut last = grab;
            while DRAG_GENERATION.load(Ordering::SeqCst) == generation && primary_button_down() {
                if let Some(now) = cursor() {
                    if now != last {
                        last = now;
                        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                            break;
                        }
                        let (x, y) = (origin.x + now.0 - grab.0, origin.y + now.1 - grab.1);
                        // Synchronous on purpose: the call waits for the window's
                        // thread to apply it, so moves can never queue up behind a
                        // busy event loop and replay late. Only the position
                        // changes — no activation, no z-order, no resize.
                        unsafe {
                            let _ = SetWindowPos(
                                hwnd,
                                None,
                                x,
                                y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
                            );
                        }
                    }
                }
                std::thread::sleep(DRAG_TICK);
            }
        });
        Ok(())
    }
}
