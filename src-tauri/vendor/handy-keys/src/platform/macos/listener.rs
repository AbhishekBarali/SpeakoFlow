//! macOS keyboard listener using CGEventTap

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, CFRunLoopSource};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventMask, CGEventSource, CGEventSourceStateID,
    CGEventTapCallBack, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};

use crate::error::{Error, Result};
use crate::platform::state::{BlockingHotkeys, ListenerState};
use crate::types::{Key, KeyEvent, Modifiers};

use super::keycode::{flags_have_alpha_shift, flags_have_fn, keycode_to_key, keycode_to_modifier};
use super::permissions::check_accessibility;

/// The tap thread's run loop, handed back to the public listener so Drop can
/// stop it. CFRunLoop is one of the documented thread-safe CF types; objc2
/// doesn't mark it Send, hence the wrapper.
pub(crate) struct TapRunLoop(CFRetained<CFRunLoop>);

unsafe impl Send for TapRunLoop {}

impl TapRunLoop {
    /// Stop the tap thread's run loop. Callable from any thread.
    ///
    /// A bare CFRunLoopStop only affects a loop that is currently between
    /// entry and exit — the stop flag lives in per-run data that is reset on
    /// every entry — so a stop landing in the gap between the `running` check
    /// and CFRunLoopRun entering would be lost and the tap thread would park
    /// forever. Enqueue the stop as a run-loop block instead: the block queue
    /// persists across entries, so the request executes either right away
    /// (after the wake-up) or at the next entry.
    pub(crate) fn stop(&self) {
        let block = block2::StackBlock::new(|| {
            if let Some(rl) = CFRunLoop::current() {
                rl.stop();
            }
        });
        unsafe {
            self.0.perform_block(
                objc2_core_foundation::kCFRunLoopCommonModes.map(|m| m.as_ref()),
                Some(&block),
            );
        }
        self.0.wake_up();
    }
}

/// Internal listener state returned to KeyboardListener
pub(crate) struct MacOSListenerState {
    pub event_receiver: Receiver<KeyEvent>,
    pub thread_handle: Option<JoinHandle<()>>,
    pub running: Arc<AtomicBool>,
    pub blocking_hotkeys: Option<BlockingHotkeys>,
    pub run_loop: TapRunLoop,
}

/// Spawn a macOS keyboard listener using CGEventTap
pub(crate) fn spawn(blocking_hotkeys: Option<BlockingHotkeys>) -> Result<MacOSListenerState> {
    if !check_accessibility() {
        return Err(Error::AccessibilityNotGranted);
    }

    let (tx, rx) = mpsc::channel();
    let state = Arc::new(Mutex::new(ListenerState::new(tx, blocking_hotkeys.clone())));
    let running = Arc::new(AtomicBool::new(true));

    // Channel reporting event tap creation: the tap thread's run loop on
    // success (needed to stop it later), or an error message.
    let (init_tx, init_rx) = mpsc::channel::<std::result::Result<TapRunLoop, String>>();

    let thread_state = Arc::clone(&state);
    let thread_running = Arc::clone(&running);

    let handle = thread::spawn(move || {
        run_event_tap(thread_state, thread_running, init_tx);
    });

    // Wait for the event tap to be created
    let run_loop = match init_rx.recv() {
        Ok(Ok(run_loop)) => run_loop,
        Ok(Err(msg)) => {
            return Err(Error::EventTapCreationFailed(msg));
        }
        Err(_) => {
            return Err(Error::EventTapCreationFailed(
                "Event tap thread terminated unexpectedly".to_string(),
            ));
        }
    };

    Ok(MacOSListenerState {
        event_receiver: rx,
        thread_handle: Some(handle),
        running,
        blocking_hotkeys,
        run_loop,
    })
}

/// Reconcile internally tracked modifiers against the actual CGEventFlags from the OS.
///
/// This corrects drift caused by missed events (e.g., tap disabled by timeout, system
/// interruptions like Mission Control or screen lock). Should only be called for
/// non-FlagsChanged events, where flags reflect the current state with no change pending.
fn reconcile_modifiers(current: &mut Modifiers, flags: CGEventFlags) {
    // If OS says a modifier group is NOT held, clear our tracked bits.
    // This fixes "stuck modifier" from missed release events.
    if !flags.contains(CGEventFlags::MaskControl) {
        current.remove(Modifiers::CTRL_LEFT | Modifiers::CTRL_RIGHT);
    }
    if !flags.contains(CGEventFlags::MaskShift) {
        current.remove(Modifiers::SHIFT_LEFT | Modifiers::SHIFT_RIGHT);
    }
    if !flags.contains(CGEventFlags::MaskCommand) {
        current.remove(Modifiers::CMD_LEFT | Modifiers::CMD_RIGHT);
    }
    if !flags.contains(CGEventFlags::MaskAlternate) {
        current.remove(Modifiers::OPT_LEFT | Modifiers::OPT_RIGHT);
    }

    // If OS says a modifier group IS held but we have no bits for it,
    // we missed a press event. Default to left side as fallback.
    if flags.contains(CGEventFlags::MaskControl) && !current.intersects(Modifiers::CTRL) {
        current.insert(Modifiers::CTRL_LEFT);
    }
    if flags.contains(CGEventFlags::MaskShift) && !current.intersects(Modifiers::SHIFT) {
        current.insert(Modifiers::SHIFT_LEFT);
    }
    if flags.contains(CGEventFlags::MaskCommand) && !current.intersects(Modifiers::CMD) {
        current.insert(Modifiers::CMD_LEFT);
    }
    if flags.contains(CGEventFlags::MaskAlternate) && !current.intersects(Modifiers::OPT) {
        current.insert(Modifiers::OPT_LEFT);
    }
}

/// Context handed to the event tap callback through the refcon pointer.
///
/// `tap` is filled in right after `CGEventTapCreate` returns. The callback and
/// `run_event_tap` execute on the same thread (callbacks only fire while that
/// thread runs its run loop), so a plain `Cell` is sound.
struct CallbackCtx {
    state: Arc<Mutex<ListenerState>>,
    tap: Cell<Option<NonNull<CFMachPort>>>,
}

/// Whether this event was posted by our own process.
///
/// The macOS counterpart of Windows' `LLKHF_INJECTED`, but narrower: it names
/// *this* process rather than "anything synthetic", so a macro tool or an
/// accessibility app that fires a registered hotkey is never mistaken for us.
fn is_own_synthetic_event(event: &CGEvent) -> bool {
    let source_pid =
        CGEvent::integer_value_field(Some(event), CGEventField::EventSourceUnixProcessID);
    source_pid == i64::from(std::process::id())
}

/// The `EventSourceUserData` value enigo stamps on every event it posts
/// (`enigo::EVENT_MARKER`; the host creates enigo with default settings). This
/// crate does not depend on enigo, hence the copy.
const ENIGO_EVENT_MARKER: i64 = 100;

/// Whether an event carries enigo's marker. Any process using enigo stamps the
/// same value, so this is only trusted inside the host's own injection window.
fn has_enigo_marker(event: &CGEvent) -> bool {
    CGEvent::integer_value_field(Some(event), CGEventField::EventSourceUserData)
        == ENIGO_EVENT_MARKER
}

/// Whether an event must bypass hotkey matching *and* modifier tracking.
///
/// The host synthesizes Cmd+C to harvest a selection the moment an assistant
/// recording starts, while the user is still holding the hotkey. enigo stamps
/// those events with only the modifiers *it* pressed, so the synthetic C key-down
/// arrives with Cmd set and Option/Control clear. `reconcile_modifiers` trusted
/// that and dropped the user's held Option+Control from the tracked state, and
/// the manager then reported the held `option+ctrl+space` binding as released
/// about 30 ms into the recording — the same failure `injected.rs` documents for
/// the Windows hook.
///
/// Unlike Windows this does not wait for the host's `ignore_injected_input`
/// window. `CGEventPost` is asynchronous, so the synthetic Cmd key-up can reach
/// this tap after the host has already closed the window; it would then be read
/// as a Cmd *press* (the down was skipped) and leave Cmd stuck in the tracked
/// state. The window exists on Windows to keep other tools' injected hotkeys
/// working, and a process-id match already excludes those, so nothing is lost.
///
/// enigo's marker is the fallback in case an event ever reaches the tap without
/// our process id, and is honoured only inside that window, where the host is
/// the one synthesizing.
fn should_skip_own_event(event: &CGEvent) -> bool {
    is_own_synthetic_event(event) || crate::injected::should_ignore_event(has_enigo_marker(event))
}

/// The callback function for the event tap
///
/// Returns NULL to block the event, or the event pointer to pass it through.
unsafe extern "C-unwind" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    // Safety: user_info is the CallbackCtx owned by run_event_tap
    let ctx = &*(user_info as *const CallbackCtx);

    // Handle tap-disable pseudo-events before touching the state lock: macOS
    // stops delivering real events until the tap is re-enabled, so recovery
    // must not depend on the mutex being healthy. Re-enabling from the
    // callback (rather than polling CGEventTapIsEnabled from the run loop) is
    // the Apple-documented pattern and avoids recurring WindowServer RPCs —
    // those leak kernel IPC vouchers and eventually kernel-panic the machine
    // (cjpais/Handy#1827). Port of upstream handy-keys #31.
    if matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    ) {
        if let Some(tap) = ctx.tap.get() {
            CGEvent::tap_enable(tap.as_ref(), true);
        }
        // Reconcile modifiers: we may have missed events while disabled.
        if let Ok(mut state) = ctx.state.lock() {
            reconcile_modifiers(
                &mut state.current_modifiers,
                CGEventSource::flags_state(CGEventSourceStateID::CombinedSessionState),
            );
        }
        return event.as_ptr();
    }

    let cg_event = event.as_ref();

    // Our own synthetic keystrokes (the selection harvest's Cmd+C) pass through
    // to the focused app untouched, but must not move the tracked modifiers or
    // reach hotkey matching. See `should_skip_own_event`.
    if should_skip_own_event(cg_event) {
        return event.as_ptr();
    }

    let flags = CGEvent::flags(Some(cg_event));

    let mut should_block = false;

    if let Ok(mut state) = ctx.state.lock() {
        // Reconcile tracked modifiers against OS flags for non-FlagsChanged events.
        // On FlagsChanged, flags reflect the state *after* the current change, so
        // reconciling would fight with the toggle logic.
        if event_type != CGEventType::FlagsChanged {
            reconcile_modifiers(&mut state.current_modifiers, flags);
        }

        // Build side-specific modifiers from internally tracked state + FN from flags
        let modifiers = if flags_have_fn(flags) {
            state.current_modifiers | Modifiers::FN
        } else {
            state.current_modifiers & !Modifiers::FN
        };

        match event_type {
            CGEventType::KeyDown => {
                let keycode = CGEvent::integer_value_field(
                    Some(cg_event),
                    CGEventField::KeyboardEventKeycode,
                ) as u16;

                let key = keycode_to_key(keycode);

                // Skip special function key events (e.g., F3 triggering Mission Control).
                // These have MaskSecondaryFn set but use special keycodes (like 0xA0)
                // that we don't recognize. Without this check, they'd be reported as
                // "Fn pressed" with no key.
                if key.is_none() && flags_have_fn(flags) {
                    return event.as_ptr();
                }

                // Check if this should be blocked
                should_block = state.should_block(modifiers, key);

                let _ = state.event_sender.send(KeyEvent {
                    modifiers,
                    key,
                    is_key_down: true,
                    changed_modifier: None,
                });
            }
            CGEventType::KeyUp => {
                let keycode = CGEvent::integer_value_field(
                    Some(cg_event),
                    CGEventField::KeyboardEventKeycode,
                ) as u16;

                let key = keycode_to_key(keycode);

                // Skip special function key events (same as KeyDown)
                if key.is_none() && flags_have_fn(flags) {
                    return event.as_ptr();
                }

                // Block key up if we blocked key down (to be consistent)
                should_block = state.should_block(modifiers, key);

                let _ = state.event_sender.send(KeyEvent {
                    modifiers,
                    key,
                    is_key_down: false,
                    changed_modifier: None,
                });
            }
            CGEventType::FlagsChanged => {
                let keycode = CGEvent::integer_value_field(
                    Some(cg_event),
                    CGEventField::KeyboardEventKeycode,
                ) as u16;

                let changed_modifier = keycode_to_modifier(keycode);

                // Check if this is a lock key (e.g., Caps Lock) which comes through
                // as FlagsChanged but isn't a traditional modifier
                let lock_key = keycode_to_key(keycode);

                // Handle lock keys specially - they come through FlagsChanged
                // but don't change our tracked modifier state
                if let Some(key) = lock_key {
                    let is_key_down = flags_have_alpha_shift(flags);

                    should_block = state.should_block(modifiers, Some(key));

                    let _ = state.event_sender.send(KeyEvent {
                        modifiers,
                        key: Some(key),
                        is_key_down,
                        changed_modifier: None,
                    });
                } else if let Some(modifier_bit) = changed_modifier {
                    // Regular modifier key — use keycode to toggle the specific bit
                    let was_set = state.current_modifiers.contains(modifier_bit);
                    let is_key_down = !was_set;

                    if is_key_down {
                        state.current_modifiers |= modifier_bit;
                    } else {
                        state.current_modifiers &= !modifier_bit;
                    }

                    // Re-derive modifiers after update (include FN from flags)
                    let new_modifiers = if flags_have_fn(flags) {
                        state.current_modifiers | Modifiers::FN
                    } else {
                        state.current_modifiers & !Modifiers::FN
                    };

                    // Check if this modifier-only combo should be blocked
                    if is_key_down {
                        should_block = state.should_block(new_modifiers, None);
                    }

                    let _ = state.event_sender.send(KeyEvent {
                        modifiers: new_modifiers,
                        key: None,
                        is_key_down,
                        changed_modifier,
                    });
                } else if keycode == 0x3F {
                    // FN key itself — tracked via flags, not keycode state
                    let had_fn = modifiers.contains(Modifiers::FN);
                    let has_fn = flags_have_fn(flags);
                    if had_fn != has_fn {
                        let new_modifiers = if has_fn {
                            state.current_modifiers | Modifiers::FN
                        } else {
                            state.current_modifiers & !Modifiers::FN
                        };

                        if has_fn {
                            should_block = state.should_block(new_modifiers, None);
                        }

                        let _ = state.event_sender.send(KeyEvent {
                            modifiers: new_modifiers,
                            key: None,
                            is_key_down: has_fn,
                            changed_modifier: Some(Modifiers::FN),
                        });
                    }
                }
            }
            // Mouse button events
            // Only report left/right clicks when modifiers are held (to avoid noise)
            CGEventType::LeftMouseDown if !modifiers.is_empty() => {
                let _ = state.event_sender.send(KeyEvent {
                    modifiers,
                    key: Some(Key::MouseLeft),
                    is_key_down: true,
                    changed_modifier: None,
                });
            }
            CGEventType::LeftMouseUp if !modifiers.is_empty() => {
                let _ = state.event_sender.send(KeyEvent {
                    modifiers,
                    key: Some(Key::MouseLeft),
                    is_key_down: false,
                    changed_modifier: None,
                });
            }
            CGEventType::RightMouseDown if !modifiers.is_empty() => {
                let _ = state.event_sender.send(KeyEvent {
                    modifiers,
                    key: Some(Key::MouseRight),
                    is_key_down: true,
                    changed_modifier: None,
                });
            }
            CGEventType::RightMouseUp if !modifiers.is_empty() => {
                let _ = state.event_sender.send(KeyEvent {
                    modifiers,
                    key: Some(Key::MouseRight),
                    is_key_down: false,
                    changed_modifier: None,
                });
            }
            // Pass through unmodified left/right clicks
            CGEventType::LeftMouseDown
            | CGEventType::LeftMouseUp
            | CGEventType::RightMouseDown
            | CGEventType::RightMouseUp => {}
            CGEventType::OtherMouseDown => {
                let button_number = CGEvent::integer_value_field(
                    Some(cg_event),
                    CGEventField::MouseEventButtonNumber,
                );
                let key = match button_number {
                    2 => Some(Key::MouseMiddle),
                    3 => Some(Key::MouseX1),
                    4 => Some(Key::MouseX2),
                    _ => None, // Unknown button
                };
                if let Some(key) = key {
                    let _ = state.event_sender.send(KeyEvent {
                        modifiers,
                        key: Some(key),
                        is_key_down: true,
                        changed_modifier: None,
                    });
                }
            }
            CGEventType::OtherMouseUp => {
                let button_number = CGEvent::integer_value_field(
                    Some(cg_event),
                    CGEventField::MouseEventButtonNumber,
                );
                let key = match button_number {
                    2 => Some(Key::MouseMiddle),
                    3 => Some(Key::MouseX1),
                    4 => Some(Key::MouseX2),
                    _ => None,
                };
                if let Some(key) = key {
                    let _ = state.event_sender.send(KeyEvent {
                        modifiers,
                        key: Some(key),
                        is_key_down: false,
                        changed_modifier: None,
                    });
                }
            }
            // TapDisabledByTimeout/TapDisabledByUserInput are handled (and
            // returned from) before the state lock above.
            _ => {}
        }
    }

    if should_block {
        // Block the event from reaching other applications
        std::ptr::null_mut()
    } else {
        // Pass the event through unchanged
        event.as_ptr()
    }
}

/// Run the event tap in a dedicated thread
fn run_event_tap(
    state: Arc<Mutex<ListenerState>>,
    running: Arc<AtomicBool>,
    init_tx: Sender<std::result::Result<TapRunLoop, String>>,
) {
    // Event types we want to monitor
    let event_mask: CGEventMask = (1 << CGEventType::KeyDown.0)
        | (1 << CGEventType::KeyUp.0)
        | (1 << CGEventType::FlagsChanged.0)
        // Mouse buttons
        | (1 << CGEventType::LeftMouseDown.0)
        | (1 << CGEventType::LeftMouseUp.0)
        | (1 << CGEventType::RightMouseDown.0)
        | (1 << CGEventType::RightMouseUp.0)
        | (1 << CGEventType::OtherMouseDown.0)
        | (1 << CGEventType::OtherMouseUp.0);

    // Callback context, handed to the tap as a raw refcon pointer
    let ctx_ptr = Box::into_raw(Box::new(CallbackCtx {
        state: Arc::clone(&state),
        tap: Cell::new(None),
    }));

    let callback: CGEventTapCallBack = Some(event_tap_callback);

    // Use Default mode (not ListenOnly) to enable optional event blocking
    let tap: Option<CFRetained<CFMachPort>> = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            event_mask,
            callback,
            ctx_ptr as *mut c_void,
        )
    };

    let tap = match tap {
        Some(t) => t,
        None => {
            // Cleanup
            unsafe {
                drop(Box::from_raw(ctx_ptr));
            }
            let _ = init_tx.send(Err(
                "Failed to create event tap. Your terminal app may need accessibility permission in System Settings > Privacy & Security > Accessibility".to_string()
            ));
            return;
        }
    };

    // Give the callback the tap port so it can re-enable on TapDisabledBy*
    // pseudo-events. No race: callbacks only run on this thread's run loop,
    // which hasn't started yet.
    unsafe { (*ctx_ptr).tap.set(Some(NonNull::from(&*tap))) };

    // Create run loop source
    let source: Option<CFRetained<CFRunLoopSource>> =
        CFMachPort::new_run_loop_source(None, Some(&tap), 0);

    let source = match source {
        Some(s) => s,
        None => {
            unsafe {
                CFMachPort::invalidate(&tap);
                drop(Box::from_raw(ctx_ptr));
            }
            let _ = init_tx.send(Err("Failed to create run loop source".to_string()));
            return;
        }
    };

    // Get the current run loop and add the source
    let run_loop = CFRunLoop::current();

    // Unwrap the Option<CFRetained<CFRunLoop>> - current() should always succeed on a valid thread
    let run_loop = match run_loop {
        Some(rl) => rl,
        None => {
            unsafe {
                CFMachPort::invalidate(&tap);
                drop(Box::from_raw(ctx_ptr));
            }
            let _ = init_tx.send(Err("Failed to get current run loop".to_string()));
            return;
        }
    };

    run_loop.add_source(Some(&source), unsafe {
        objc2_core_foundation::kCFRunLoopCommonModes
    });
    CGEvent::tap_enable(&tap, true);

    // Signal successful initialization, handing back this thread's run loop
    // so KeyboardListener::drop can stop it.
    let _ = init_tx.send(Ok(TapRunLoop(run_loop.clone())));

    // Park in the run loop: no periodic wakeups, no watchdog. The previous
    // loop cycled CFRunLoopRunInMode every 100ms and polled
    // CGEventTapIsEnabled on each pass; that RPC leaked kernel IPC vouchers
    // until macOS panicked with "Cannot grow ipc space beyond
    // IVAC_ENTRIES_MAX" (cjpais/Handy#1827). Tap recovery is event-driven in
    // event_tap_callback; shutdown goes through TapRunLoop::stop. Guarded by
    // `running` in case the loop returns without a stop request, and bails out
    // if the tap's port died (e.g. accessibility permission revoked) so it
    // doesn't spin on a dead source.
    while running.load(std::sync::atomic::Ordering::SeqCst) {
        CFRunLoop::run();

        if !CFMachPort::is_valid(&tap) {
            break;
        }
    }

    // Cleanup
    run_loop.remove_source(Some(&source), unsafe {
        objc2_core_foundation::kCFRunLoopCommonModes
    });
    CGEvent::tap_enable(&tap, false);
    CFMachPort::invalidate(&tap);
    unsafe {
        drop(Box::from_raw(ctx_ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use objc2_core_foundation::CFRunLoopTimer;

    unsafe extern "C-unwind" fn noop_timer(_timer: *mut CFRunLoopTimer, _info: *mut c_void) {}

    /// The stop request must survive being issued before the run loop has been
    /// entered. A bare CFRunLoopStop is lost in that window, which would park
    /// the tap thread forever and hang KeyboardListener::drop. Needs no
    /// accessibility permission, so it runs in CI. Port of upstream #31.
    #[test]
    fn stop_requested_before_run_entry_is_not_lost() {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let rl = CFRunLoop::current().expect("thread has a run loop");

            // A far-future timer keeps the mode non-empty so run_in_mode
            // actually blocks instead of returning Finished immediately.
            let timer = unsafe {
                CFRunLoopTimer::new(
                    None,
                    objc2_core_foundation::CFAbsoluteTimeGetCurrent() + 3600.0,
                    0.0,
                    0,
                    0,
                    Some(noop_timer),
                    std::ptr::null_mut(),
                )
            }
            .expect("create timer");
            rl.add_timer(Some(&timer), unsafe {
                objc2_core_foundation::kCFRunLoopDefaultMode
            });

            tx.send(TapRunLoop(rl.clone())).unwrap();

            // Guarantee the main thread's stop lands while this thread is
            // provably outside the run loop.
            thread::sleep(Duration::from_millis(200));

            let started = Instant::now();
            CFRunLoop::run_in_mode(
                unsafe { objc2_core_foundation::kCFRunLoopDefaultMode },
                5.0,
                false,
            );
            started.elapsed()
        });

        let run_loop = rx.recv().expect("run loop handle");
        run_loop.stop(); // lands before the thread enters the run loop

        let elapsed = handle.join().expect("tap thread panicked");
        assert!(
            elapsed < Duration::from_secs(2),
            "stop issued before run-loop entry was lost; loop ran for {elapsed:?}"
        );
    }
}
