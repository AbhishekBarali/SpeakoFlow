//! Windows low-level keyboard hook implementation

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    HANDLE, HWND, LPARAM, LRESULT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    MsgWaitForMultipleObjects, PeekMessageW, RegisterClassW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HHOOK, HWND_MESSAGE, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, MSG,
    MSLLHOOKSTRUCT, PM_REMOVE, QS_ALLINPUT, WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW,
};

use crate::error::{Error, Result};
use crate::platform::state::BlockingHotkeys;
use crate::types::{Key, KeyEvent, Modifiers};

use super::keycode::{vk_to_key, vk_to_modifier};

// WTS session notification plumbing not exposed by the `windows` crate bindings.
const NOTIFY_FOR_THIS_SESSION: u32 = 0;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
// WM_WTSSESSION_CHANGE wParam values (wtsapi32.h).
const WTS_CONSOLE_CONNECT: usize = 0x1;
const WTS_REMOTE_CONNECT: usize = 0x3;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;

/// The side-specific modifier keys we track, paired with their virtual-key codes.
const MODIFIER_KEYS: [(u16, Modifiers); 8] = [
    (0x5B, Modifiers::CMD_LEFT),    // VK_LWIN
    (0x5C, Modifiers::CMD_RIGHT),   // VK_RWIN
    (0xA0, Modifiers::SHIFT_LEFT),  // VK_LSHIFT
    (0xA1, Modifiers::SHIFT_RIGHT), // VK_RSHIFT
    (0xA2, Modifiers::CTRL_LEFT),   // VK_LCONTROL
    (0xA3, Modifiers::CTRL_RIGHT),  // VK_RCONTROL
    (0xA4, Modifiers::OPT_LEFT),    // VK_LMENU
    (0xA5, Modifiers::OPT_RIGHT),   // VK_RMENU
];

/// Every modifier bit reconciliation may touch. FN is excluded: Windows never
/// reports it, so reconciliation must not clear it.
const RECONCILABLE: Modifiers = Modifiers::CMD
    .union(Modifiers::SHIFT)
    .union(Modifiers::CTRL)
    .union(Modifiers::OPT);

/// `dwExtraInfo` stamped on every keystroke this hook injects itself, so the
/// hook can let its own replays straight through. ("HKEY" in ASCII.)
const OWN_INJECTION_MARKER: usize = 0x484B_4559;

/// An unassigned virtual-key code, tapped to "mask" a Win or Alt press so that
/// releasing it does not open the Start menu or activate a menu bar. The same
/// default AutoHotkey uses (`#MenuMaskKey vkE8`), chosen because no layout or
/// application assigns it.
const VK_MENU_MASK: u16 = 0xE8;

/// Win and Alt: the modifiers that do something on their own when released
/// with nothing pressed in between.
const MENU_MODIFIERS: Modifiers = Modifiers::CMD.union(Modifiers::OPT);

/// Modifier virtual-key codes that need the extended-key flag when injected.
fn is_extended_modifier(vk: u16) -> bool {
    matches!(vk, 0x5B | 0x5C | 0xA3 | 0xA5) // LWIN, RWIN, RCONTROL, RMENU
}

fn key_input(vk: u16, scan: u16, extended: bool, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OWN_INJECTION_MARKER,
            },
        },
    }
}

fn inject(inputs: &[INPUT]) {
    // Queued behind the event being handled, and delivered back through this
    // hook later, where the marker lets them pass untouched.
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Press a modifier on the OS's behalf, the one this hook kept from it.
fn modifier_press(vk: u16) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    key_input(vk, scan, is_extended_modifier(vk), false)
}

/// A modifier-only hotkey's last modifier was kept from Windows, and then a
/// key nobody registered went down on top of it: the user was typing another
/// shortcut (`Ctrl+Alt+↑`, `Ctrl+Win+→`, or `Ctrl+Alt+Q` for `@` on a German
/// layout). Give Windows the modifiers it never saw, then the key, so the
/// focused application receives exactly what was pressed. Without this it got
/// `Ctrl+↑` — or, for the `@`, a `Ctrl+Q` that closes windows.
fn replay_chord(withheld: &[u16], vk: u16, scan: u16, extended: bool) {
    let mut inputs: Vec<INPUT> = withheld.iter().map(|&m| modifier_press(m)).collect();
    inputs.push(key_input(vk, scan, extended, false));
    inject(&inputs);
}

/// Tap the mask key, so a Win or Alt the OS saw go down does not open the
/// Start menu or a menu bar when it comes back up.
fn send_menu_mask() {
    inject(&[
        key_input(VK_MENU_MASK, 0, false, false),
        key_input(VK_MENU_MASK, 0, false, true),
    ]);
}

/// The modifiers Windows knows are down: tracked, minus the ones withheld.
fn modifiers_seen_by_os(ctx: &HookContext) -> Modifiers {
    ctx.current_modifiers & !blocked_modifiers(&ctx.blocked_keys)
}

/// Thread-local state for the keyboard hook callback.
///
/// Windows low-level hooks require a callback function with a specific signature,
/// so we use thread-local storage to access our state from within the callback.
struct HookContext {
    event_sender: Sender<KeyEvent>,
    current_modifiers: Modifiers,
    blocking_hotkeys: Option<BlockingHotkeys>,
    /// Virtual-key codes of keys that are physically held right now (as seen
    /// by this hook). Used to tell an initial key-down apart from auto-repeat.
    physically_down: std::collections::HashSet<u16>,
    /// Virtual-key codes whose key-DOWN we blocked from reaching the OS.
    ///
    /// Blocking must be *symmetric per physical key press*: if the OS saw the
    /// key go down, it must also see it go up (and vice versa). Re-evaluating
    /// the hotkey match on every event — the old behaviour — desyncs the two
    /// whenever the modifier state changes between a key's down and its up
    /// (sloppy press order, tap-to-lock Shift taps, hotkeys registered
    /// mid-hold, …) and leaves keys stuck "pressed forever" at the OS level.
    blocked_keys: std::collections::HashSet<u16>,
}

thread_local! {
    static HOOK_CONTEXT: std::cell::RefCell<Option<HookContext>> = const { std::cell::RefCell::new(None) };
}

/// What draining the thread message queue observed.
#[derive(Default)]
struct DrainOutcome {
    /// WM_QUIT received -- exit the message loop.
    quit: bool,
    /// A session change (lock/unlock/connect) occurred -- reset modifier
    /// state, since key-ups on the secure desktop never reach the hook.
    session_change: bool,
    /// The interactive desktop came back (unlock or console/remote connect) --
    /// re-install hooks in case Windows silently removed them.
    reinstall_hooks: bool,
}

/// Drain all pending thread messages.
fn drain_thread_messages(msg: &mut MSG) -> DrainOutcome {
    let mut outcome = DrainOutcome::default();
    unsafe {
        while PeekMessageW(msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                outcome.quit = true;
                return outcome;
            }
            if msg.message == WM_WTSSESSION_CHANGE {
                match msg.wParam.0 {
                    WTS_SESSION_LOCK => outcome.session_change = true,
                    WTS_SESSION_UNLOCK | WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
                        outcome.session_change = true;
                        outcome.reinstall_hooks = true;
                    }
                    _ => {}
                }
            }
            let _ = TranslateMessage(msg);
            DispatchMessageW(msg);
        }
    }
    outcome
}

/// Whether Windows reports virtual key `vk` as held right now.
///
/// If the calling thread's desktop is not active (the lock screen's secure
/// desktop is up), every key reads as up -- the right answer here: treat
/// everything as released.
fn async_key_down(vk: u16) -> bool {
    // High bit set = key currently down.
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

/// Which tracked modifier keys Windows reports as physically held.
fn physical_modifiers() -> Modifiers {
    let mut held = Modifiers::empty();
    for (vk, modifier) in MODIFIER_KEYS {
        if async_key_down(vk) {
            held |= modifier;
        }
    }
    held
}

/// Modifiers whose key-down this hook *blocked*. Windows never saw those go
/// down, so `GetAsyncKeyState` reports them as up while the user is holding
/// them (the blind spot `injected.rs` documents). Reconciliation must trust our
/// own record for these, or holding Ctrl+Win and clicking the mouse would
/// release the hotkey mid-recording.
fn blocked_modifiers(blocked_keys: &std::collections::HashSet<u16>) -> Modifiers {
    blocked_keys
        .iter()
        .filter_map(|vk| vk_to_modifier(*vk))
        .fold(Modifiers::empty(), |acc, m| acc | m)
}

/// Modifiers we track as held that are no longer physically held.
fn stale_modifiers(tracked: Modifiers, physical: Modifiers) -> Modifiers {
    (tracked & RECONCILABLE) & !physical
}

/// Build the synthetic release events that clear `stale` from `tracked`, in
/// MODIFIER_KEYS order. Each event carries the modifier set as it shrinks,
/// exactly as if the keys had been released one by one.
fn release_events(tracked: Modifiers, stale: Modifiers) -> Vec<KeyEvent> {
    let mut modifiers = tracked;
    let mut events = Vec::new();
    for (_, modifier) in MODIFIER_KEYS {
        if stale.contains(modifier) {
            modifiers &= !modifier;
            events.push(KeyEvent {
                modifiers,
                key: None,
                is_key_down: false,
                changed_modifier: Some(modifier),
            });
        }
    }
    events
}

/// Reconcile tracked modifiers against the physical keyboard state.
///
/// Corrects drift from missed events: the secure desktop swallows key-ups.
/// Win+L delivers the Win key DOWN to this hook but its UP happens on the lock
/// screen, so Win stayed "held" here after unlocking — and with the default
/// `Ctrl+Win` dictation hotkey, pressing Ctrl alone then started a phantom
/// dictation (and was blocked from reaching the focused app) until Win was
/// tapped again. Sleep/wake loses key-ups the same way.
///
/// Stale modifiers are cleared with synthetic release events so the manager's
/// press/release tracking recovers; missed presses are adopted silently and
/// ride along on the next real event. Port of upstream handy-keys #24, adapted
/// to this fork's symmetric blocking (see [`blocked_modifiers`]).
///
/// `hard` is the session-change case: every record — including blocked keys
/// and auto-repeat tracking — is dropped unless Windows says the key is down,
/// because nothing typed on the other desktop reached us.
fn reconcile_modifiers(ctx: &mut HookContext, hard: bool) {
    // Our own synthetic keystrokes are in flight: the async state momentarily
    // includes injected modifiers the user is not holding. The next real event
    // reconciles instead.
    if crate::injected::ignoring_injected_input() {
        return;
    }
    if hard {
        ctx.physically_down.retain(|vk| async_key_down(*vk));
        ctx.blocked_keys.retain(|vk| async_key_down(*vk));
    }
    let physical = physical_modifiers() | blocked_modifiers(&ctx.blocked_keys);
    let stale = stale_modifiers(ctx.current_modifiers, physical);
    if !stale.is_empty() {
        // A stale modifier's key is not down, so its auto-repeat record is
        // stale too: without clearing it the next real press would be read as
        // a repeat and skip the hotkey/blocking decision.
        ctx.physically_down
            .retain(|vk| vk_to_modifier(*vk).map_or(true, |m| !stale.contains(m)));
    }
    for event in release_events(ctx.current_modifiers, stale) {
        ctx.current_modifiers = event.modifiers;
        let _ = ctx.event_sender.send(event);
    }
    ctx.current_modifiers |= physical & RECONCILABLE & !ctx.current_modifiers;
}

/// Reconcile from the hook thread's message loop (session change, where no
/// input event accompanies the state change).
fn reconcile_modifiers_in_context(hard: bool) {
    HOOK_CONTEXT.with(|ctx_cell| {
        if let Ok(mut ctx_ref) = ctx_cell.try_borrow_mut() {
            if let Some(ctx) = ctx_ref.as_mut() {
                reconcile_modifiers(ctx, hard);
            }
        }
    });
}

/// Wndproc for the session notification window. (The `windows` crate's
/// DefWindowProcW is a generic Rust wrapper, so it cannot be used as
/// lpfnWndProc directly.)
///
/// Reconciles here as well as in the drain loop: the drain loop covers posted
/// delivery of WM_WTSSESSION_CHANGE, while this path covers builds that deliver
/// it via SendMessage, which bypasses the message queue. Double reconciliation
/// on the posted path is harmless — the second pass finds nothing stale.
unsafe extern "system" fn session_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_WTSSESSION_CHANGE {
        match wparam.0 {
            WTS_SESSION_LOCK | WTS_SESSION_UNLOCK | WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
                reconcile_modifiers_in_context(true);
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Create a message-only window registered for WTS session notifications.
///
/// Returns None on failure. Non-fatal: hooks still work, and stale modifiers
/// are still corrected lazily by `reconcile_modifiers` on the next key event.
unsafe fn create_session_notification_window() -> Option<HWND> {
    let class_name: Vec<u16> = "HandyKeysSessionWatcher\0".encode_utf16().collect();
    let wnd_class = WNDCLASSW {
        lpfnWndProc: Some(session_wndproc),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    // May fail with ERROR_CLASS_ALREADY_EXISTS (a second listener in the same
    // process); CreateWindowExW still succeeds against the existing class.
    RegisterClassW(&wnd_class);

    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        PCWSTR(class_name.as_ptr()),
        PCWSTR::null(),
        WINDOW_STYLE::default(),
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        None,
        None,
        None,
    )
    .ok()?;

    if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).is_err() {
        let _ = DestroyWindow(hwnd);
        return None;
    }

    Some(hwnd)
}

/// Clean up the session notification window. Must run on the creating thread.
unsafe fn destroy_session_notification_window(hwnd: HWND) {
    let _ = WTSUnRegisterSessionNotification(hwnd);
    let _ = DestroyWindow(hwnd);
}

/// Re-install the low-level hooks, defensively: Windows silently removes an LL
/// hook whose callback exceeds its timeout budget, and returning to the
/// interactive desktop is a common moment for that to surface (Handy #1620:
/// hotkeys dead after sleep). The replacements are installed before the old
/// hooks are removed, so a failure never leaves us hook-less, and no messages
/// are pumped in between, so no event is delivered twice.
unsafe fn reinstall_hooks(kb_hook: &mut HHOOK, mouse_hook: &mut HHOOK) -> bool {
    let new_kb = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0) {
        Ok(h) => h,
        Err(_) => return false,
    };
    let new_mouse = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0) {
        Ok(h) => h,
        Err(_) => {
            let _ = UnhookWindowsHookEx(new_kb);
            return false;
        }
    };
    let _ = UnhookWindowsHookEx(*kb_hook);
    let _ = UnhookWindowsHookEx(*mouse_hook);
    *kb_hook = new_kb;
    *mouse_hook = new_mouse;
    true
}

/// Sleep until Windows delivers input or the listener is explicitly stopped.
/// A manual-reset event remembers an early stop, including one that arrives
/// before the message loop starts, so shutdown never depends on polling.
fn wait_for_message_or_shutdown(shutdown: &OwnedHandle) -> bool {
    let result = unsafe {
        MsgWaitForMultipleObjects(
            Some(&[HANDLE(shutdown.as_raw_handle())]),
            false,
            INFINITE,
            QS_ALLINPUT,
        )
    };
    result != WAIT_OBJECT_0 && result != WAIT_FAILED
}

/// Internal listener state returned to KeyboardListener
pub(crate) struct WindowsListenerState {
    pub event_receiver: mpsc::Receiver<KeyEvent>,
    pub thread_handle: Option<JoinHandle<()>>,
    pub running: Arc<AtomicBool>,
    pub blocking_hotkeys: Option<BlockingHotkeys>,
    pub shutdown_event: Arc<OwnedHandle>,
}

/// Spawn a Windows low-level keyboard hook listener
pub(crate) fn spawn(blocking_hotkeys: Option<BlockingHotkeys>) -> Result<WindowsListenerState> {
    let event = unsafe { CreateEventW(None, true, false, None) }
        .map_err(|e| Error::Platform(format!("Failed to create keyboard shutdown event: {e}")))?;
    // Transfer ownership once; both threads share it until the listener exits.
    let shutdown_event = Arc::new(unsafe { OwnedHandle::from_raw_handle(event.0) });
    let thread_shutdown = shutdown_event.clone();
    let (tx, rx) = mpsc::channel();
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&running);
    let thread_blocking = blocking_hotkeys.clone();

    let handle = thread::spawn(move || {
        // Initialize thread-local hook context
        HOOK_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = Some(HookContext {
                event_sender: tx,
                current_modifiers: Modifiers::empty(),
                blocking_hotkeys: thread_blocking,
                physically_down: std::collections::HashSet::new(),
                blocked_keys: std::collections::HashSet::new(),
            });
        });

        // Install the low-level keyboard hook
        let kb_hook =
            unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0) };

        let mut kb_hook = match kb_hook {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Failed to install keyboard hook: {:?}", e);
                return;
            }
        };

        // Install the low-level mouse hook
        let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0) };

        let mut mouse_hook = match mouse_hook {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Failed to install mouse hook: {:?}", e);
                // Clean up keyboard hook before returning
                unsafe {
                    let _ = UnhookWindowsHookEx(kb_hook);
                }
                return;
            }
        };

        // Watch for session changes (Win+L lock/unlock, RDP connect): the
        // secure desktop swallows key-up events, so modifier state must be
        // reset when the session comes back.
        let session_hwnd = unsafe { create_session_notification_window() };

        // Message loop - required for low-level hooks to function.
        // Input and shutdown both wake the loop immediately, with no idle timer.
        let mut msg = MSG::default();
        loop {
            // Check if we should stop
            if !thread_running.load(Ordering::SeqCst) {
                break;
            }

            // Process all pending messages
            let outcome = drain_thread_messages(&mut msg);
            if outcome.quit {
                break;
            }
            if outcome.session_change {
                reconcile_modifiers_in_context(true);
            }
            if outcome.reinstall_hooks {
                unsafe {
                    if !reinstall_hooks(&mut kb_hook, &mut mouse_hook) {
                        // Keep the old hooks: they usually still work (the
                        // reinstall is defensive hardening, not a repair).
                        eprintln!("handy-keys: failed to re-install hooks after session change");
                    }
                }
            }

            if !wait_for_message_or_shutdown(&thread_shutdown) {
                break;
            }
        }

        // Clean up the session notification window, then the hooks
        if let Some(hwnd) = session_hwnd {
            unsafe {
                destroy_session_notification_window(hwnd);
            }
        }
        unsafe {
            let _ = UnhookWindowsHookEx(kb_hook);
            let _ = UnhookWindowsHookEx(mouse_hook);
        }

        // Clear thread-local state
        HOOK_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = None;
        });
    });

    Ok(WindowsListenerState {
        event_receiver: rx,
        thread_handle: Some(handle),
        running,
        blocking_hotkeys,
        shutdown_event,
    })
}

/// Low-level keyboard hook callback
///
/// This function is called by Windows for every keyboard event system-wide.
/// It must return quickly to avoid input lag.
unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // If code < 0, we must pass to next hook without processing
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    // Our own replays and mask taps: already accounted for, pass straight on.
    // Checked before touching the context, so a hook call that arrived while
    // this one was still running could never contend for it.
    if (*(lparam.0 as *const KBDLLHOOKSTRUCT)).dwExtraInfo == OWN_INJECTION_MARKER {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let mut should_block = false;
    HOOK_CONTEXT.with(|ctx_cell| {
        let mut ctx_ref = ctx_cell.borrow_mut();
        if let Some(ctx) = ctx_ref.as_mut() {
            // Extract key information from KBDLLHOOKSTRUCT
            let kb_struct = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let vk_code = kb_struct.vkCode as u16;
            let is_extended = (kb_struct.flags.0 & LLKHF_EXTENDED.0) != 0;
            let is_injected = (kb_struct.flags.0 & LLKHF_INJECTED.0) != 0;

            // The host is synthesizing keystrokes right now, so this event is its
            // own and must not be mistaken for the user's. Returning here leaves
            // `should_block` false, so the injected key still reaches the target
            // application — it is only withheld from hotkey matching. Modifier
            // tracking is skipped too: our injected Ctrl release says nothing
            // about whether the user is still holding Ctrl. See `injected.rs`.
            if crate::injected::should_ignore_event(is_injected) {
                return;
            }

            let is_key_down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);

            // Check if this is a modifier key
            if let Some(modifier) = vk_to_modifier(vk_code) {
                let prev_modifiers = ctx.current_modifiers;

                // Update modifier state
                if is_key_down {
                    ctx.current_modifiers |= modifier;
                } else {
                    ctx.current_modifiers &= !modifier;
                }

                // Symmetric blocking: decide only on the *initial* down;
                // repeats and the release simply follow that decision.
                if is_key_down {
                    let is_repeat = !ctx.physically_down.insert(vk_code);
                    if is_repeat {
                        should_block = ctx.blocked_keys.contains(&vk_code);
                    } else {
                        should_block =
                            should_block_hotkey(&ctx.blocking_hotkeys, ctx.current_modifiers, None);
                        if should_block {
                            ctx.blocked_keys.insert(vk_code);
                            // A Win or Alt that went down first reached
                            // Windows; released with nothing in between it
                            // would open the Start menu or a menu bar.
                            if modifiers_seen_by_os(ctx).intersects(MENU_MODIFIERS) {
                                send_menu_mask();
                            }
                        }
                    }
                } else {
                    ctx.physically_down.remove(&vk_code);
                    should_block = ctx.blocked_keys.remove(&vk_code);
                }

                // Only emit event if modifiers actually changed
                if ctx.current_modifiers != prev_modifiers {
                    let _ = ctx.event_sender.send(KeyEvent {
                        modifiers: ctx.current_modifiers,
                        key: None,
                        is_key_down,
                        changed_modifier: Some(modifier),
                    });
                }
            } else if let Some(key) = vk_to_key(vk_code, is_extended) {
                // Non-modifier key: reconcile tracked modifiers against the
                // physical keyboard first, so a key-up missed during a secure
                // desktop transition can't stick a modifier onto this event.
                // (Not done for modifier events: inside a low-level hook the
                // async key state does not yet include the in-flight change,
                // so reconciling there would fight the toggle logic above.)
                reconcile_modifiers(ctx, false);

                // Regular key event. Same symmetric-blocking rule: evaluate
                // the hotkey match only on the initial key-down; auto-repeats
                // and the key-up mirror that decision so the OS always sees a
                // consistent down/up pair (or neither).
                if is_key_down {
                    let is_repeat = !ctx.physically_down.insert(vk_code);
                    if is_repeat {
                        should_block = ctx.blocked_keys.contains(&vk_code);
                    } else {
                        should_block = should_block_hotkey(
                            &ctx.blocking_hotkeys,
                            ctx.current_modifiers,
                            Some(key),
                        );
                        if should_block {
                            ctx.blocked_keys.insert(vk_code);
                        } else {
                            let withheld: Vec<u16> = ctx
                                .blocked_keys
                                .iter()
                                .copied()
                                .filter(|vk| vk_to_modifier(*vk).is_some())
                                .collect();
                            if !withheld.is_empty() {
                                // Windows sees the modifiers from now on, so
                                // their real key-ups must reach it too.
                                for vk in &withheld {
                                    ctx.blocked_keys.remove(vk);
                                }
                                replay_chord(
                                    &withheld,
                                    vk_code,
                                    kb_struct.scanCode as u16,
                                    is_extended,
                                );
                                // The replay stands in for this key-down. Its
                                // repeats and key-up pass as usual.
                                should_block = true;
                            }
                        }
                    }
                } else {
                    ctx.physically_down.remove(&vk_code);
                    should_block = ctx.blocked_keys.remove(&vk_code);
                }

                let _ = ctx.event_sender.send(KeyEvent {
                    modifiers: ctx.current_modifiers,
                    key: Some(key),
                    is_key_down,
                    changed_modifier: None,
                });
            }
        }
    });

    if should_block {
        // Return non-zero to block the event from propagating
        LRESULT(1)
    } else {
        // Pass to next hook in chain
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Low-level mouse hook callback
///
/// This function is called by Windows for every mouse event system-wide.
unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // If code < 0, we must pass to next hook without processing
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    // Only button transitions matter; bail out early for moves and wheel
    // events so the hot path stays free of state access.
    if !matches!(
        wparam.0 as u32,
        WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_MBUTTONDOWN
            | WM_MBUTTONUP
            | WM_XBUTTONDOWN
            | WM_XBUTTONUP
    ) {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    // Process the mouse event
    HOOK_CONTEXT.with(|ctx_cell| {
        let mut ctx_ref = ctx_cell.borrow_mut();
        if let Some(ctx) = ctx_ref.as_mut() {
            let mouse_struct = &*(lparam.0 as *const MSLLHOOKSTRUCT);

            // Stale modifiers must not gate button reporting (or decorate the
            // event), so reconcile before reading them.
            reconcile_modifiers(ctx, false);

            // Only report left/right clicks when modifiers are held (to avoid noise)
            let has_modifiers = !ctx.current_modifiers.is_empty();

            let (key, is_down) = match wparam.0 as u32 {
                WM_LBUTTONDOWN if has_modifiers => (Some(Key::MouseLeft), true),
                WM_LBUTTONUP if has_modifiers => (Some(Key::MouseLeft), false),
                WM_RBUTTONDOWN if has_modifiers => (Some(Key::MouseRight), true),
                WM_RBUTTONUP if has_modifiers => (Some(Key::MouseRight), false),
                // Middle and X buttons always reported
                WM_MBUTTONDOWN => (Some(Key::MouseMiddle), true),
                WM_MBUTTONUP => (Some(Key::MouseMiddle), false),
                WM_XBUTTONDOWN => {
                    // High word of mouseData contains which X button (1 or 2)
                    let xbutton = (mouse_struct.mouseData >> 16) & 0xFFFF;
                    let key = if xbutton == 1 {
                        Some(Key::MouseX1)
                    } else if xbutton == 2 {
                        Some(Key::MouseX2)
                    } else {
                        None
                    };
                    (key, true)
                }
                WM_XBUTTONUP => {
                    let xbutton = (mouse_struct.mouseData >> 16) & 0xFFFF;
                    let key = if xbutton == 1 {
                        Some(Key::MouseX1)
                    } else if xbutton == 2 {
                        Some(Key::MouseX2)
                    } else {
                        None
                    };
                    (key, false)
                }
                _ => (None, false),
            };

            if let Some(key) = key {
                let _ = ctx.event_sender.send(KeyEvent {
                    modifiers: ctx.current_modifiers,
                    key: Some(key),
                    is_key_down: is_down,
                    changed_modifier: None,
                });
            }
        }
    });

    // Always pass mouse events through (no blocking for mouse)
    CallNextHookEx(None, code, wparam, lparam)
}

/// Check if a hotkey combination should be blocked
fn should_block_hotkey(
    blocking_hotkeys: &Option<BlockingHotkeys>,
    modifiers: Modifiers,
    key: Option<Key>,
) -> bool {
    if let Some(ref hotkeys) = blocking_hotkeys {
        if let Ok(set) = hotkeys.lock() {
            return set
                .iter()
                .any(|h| h.modifiers.matches(modifiers) && h.key == key);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use windows::Win32::System::Threading::SetEvent;
    use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

    fn clear_message_queue() {
        let mut msg = MSG::default();
        unsafe { while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {} }
    }

    #[test]
    fn wait_stays_asleep_until_shutdown() {
        let event = unsafe { CreateEventW(None, true, false, None) }.unwrap();
        let shutdown = Arc::new(unsafe { OwnedHandle::from_raw_handle(event.0) });
        let worker_shutdown = shutdown.clone();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            clear_message_queue();
            tx.send(wait_for_message_or_shutdown(&worker_shutdown))
                .unwrap();
        });
        let idle_result = rx.recv_timeout(Duration::from_millis(100));
        unsafe { SetEvent(HANDLE(shutdown.as_raw_handle())) }.unwrap();
        assert!(matches!(idle_result, Err(mpsc::RecvTimeoutError::Timeout)));
        assert!(!rx.recv_timeout(Duration::from_secs(2)).unwrap());
        worker.join().unwrap();
    }

    #[test]
    fn shutdown_before_wait_is_not_lost() {
        clear_message_queue();
        let event = unsafe { CreateEventW(None, true, true, None) }.unwrap();
        let shutdown = unsafe { OwnedHandle::from_raw_handle(event.0) };
        assert!(!wait_for_message_or_shutdown(&shutdown));
    }

    #[test]
    fn wait_returns_immediately_when_message_is_pending() {
        clear_message_queue();
        unsafe {
            PostQuitMessage(0);
        }
        let event = unsafe { CreateEventW(None, true, false, None) }.unwrap();
        let shutdown = unsafe { OwnedHandle::from_raw_handle(event.0) };
        assert!(wait_for_message_or_shutdown(&shutdown));
        clear_message_queue();
    }

    #[test]
    fn drain_messages_stops_on_wm_quit() {
        clear_message_queue();
        unsafe {
            PostQuitMessage(0);
        }
        let mut msg = MSG::default();
        assert!(drain_thread_messages(&mut msg).quit);
        clear_message_queue();
    }

    fn post_session_change(wparam: usize) {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        unsafe {
            PostThreadMessageW(
                GetCurrentThreadId(),
                WM_WTSSESSION_CHANGE,
                WPARAM(wparam),
                LPARAM(0),
            )
            .unwrap();
        }
    }

    #[test]
    fn drain_reports_session_unlock_with_reinstall() {
        clear_message_queue();
        post_session_change(WTS_SESSION_UNLOCK);
        let mut msg = MSG::default();
        let outcome = drain_thread_messages(&mut msg);
        assert!(outcome.session_change);
        assert!(outcome.reinstall_hooks);
        assert!(!outcome.quit);
        clear_message_queue();
    }

    #[test]
    fn drain_reports_session_lock_without_reinstall() {
        clear_message_queue();
        post_session_change(WTS_SESSION_LOCK);
        let mut msg = MSG::default();
        let outcome = drain_thread_messages(&mut msg);
        assert!(outcome.session_change);
        assert!(!outcome.reinstall_hooks);
        clear_message_queue();
    }

    #[test]
    fn drain_ignores_unrelated_session_events() {
        clear_message_queue();
        // WTS_SESSION_LOGOFF (0x6) is not a state we react to.
        post_session_change(0x6);
        let mut msg = MSG::default();
        let outcome = drain_thread_messages(&mut msg);
        assert!(!outcome.session_change);
        assert!(!outcome.reinstall_hooks);
        clear_message_queue();
    }

    #[test]
    fn stale_modifiers_flags_released_keys() {
        // Tracked Win+Ctrl, but only Ctrl still physically held: Win is stale.
        let stale = stale_modifiers(
            Modifiers::CMD_LEFT | Modifiers::CTRL_LEFT,
            Modifiers::CTRL_LEFT,
        );
        assert_eq!(stale, Modifiers::CMD_LEFT);
    }

    #[test]
    fn stale_modifiers_empty_when_state_matches() {
        let tracked = Modifiers::SHIFT_LEFT | Modifiers::OPT_RIGHT;
        assert_eq!(stale_modifiers(tracked, tracked), Modifiers::empty());
        assert_eq!(
            stale_modifiers(Modifiers::empty(), Modifiers::CTRL_LEFT),
            Modifiers::empty()
        );
    }

    #[test]
    fn stale_modifiers_never_touches_fn() {
        let stale = stale_modifiers(Modifiers::FN | Modifiers::CMD_LEFT, Modifiers::empty());
        assert_eq!(stale, Modifiers::CMD_LEFT);
    }

    #[test]
    fn a_blocked_modifier_is_never_reported_stale() {
        // Ctrl+Alt held for the assistant: this hook blocked the Alt key-down,
        // so Windows reports Alt as up. Our own record must keep it held.
        let blocked: std::collections::HashSet<u16> = [0xA4].into_iter().collect();
        let physical = Modifiers::CTRL_LEFT | blocked_modifiers(&blocked);
        let stale = stale_modifiers(Modifiers::CTRL_LEFT | Modifiers::OPT_LEFT, physical);
        assert!(stale.is_empty(), "{stale:?}");
    }

    #[test]
    fn release_events_shrink_modifiers_one_key_at_a_time() {
        let tracked = Modifiers::CMD_LEFT | Modifiers::CTRL_LEFT | Modifiers::FN;
        let stale = Modifiers::CMD_LEFT | Modifiers::CTRL_LEFT;
        let events = release_events(tracked, stale);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].changed_modifier, Some(Modifiers::CMD_LEFT));
        assert_eq!(events[0].modifiers, Modifiers::CTRL_LEFT | Modifiers::FN);
        assert!(!events[0].is_key_down);
        assert_eq!(events[0].key, None);
        assert_eq!(events[1].changed_modifier, Some(Modifiers::CTRL_LEFT));
        assert_eq!(events[1].modifiers, Modifiers::FN);
        assert!(!events[1].is_key_down);
    }

    #[test]
    fn release_events_empty_when_nothing_stale() {
        assert!(release_events(Modifiers::CMD_LEFT, Modifiers::empty()).is_empty());
    }

    #[test]
    fn injected_keys_carry_the_marker_and_their_flags() {
        let win_down = modifier_press(0x5B);
        let ki = unsafe { win_down.Anonymous.ki };
        assert_eq!(ki.wVk, VIRTUAL_KEY(0x5B));
        assert_eq!(ki.dwExtraInfo, OWN_INJECTION_MARKER);
        assert!(ki.dwFlags.contains(KEYEVENTF_EXTENDEDKEY));
        assert!(!ki.dwFlags.contains(KEYEVENTF_KEYUP));

        let alt_down = unsafe { modifier_press(0xA4).Anonymous.ki };
        assert!(!alt_down.dwFlags.contains(KEYEVENTF_EXTENDEDKEY));

        let mask_up = unsafe { key_input(VK_MENU_MASK, 0, false, true).Anonymous.ki };
        assert!(mask_up.dwFlags.contains(KEYEVENTF_KEYUP));
        assert_eq!(mask_up.dwExtraInfo, OWN_INJECTION_MARKER);
    }
}
