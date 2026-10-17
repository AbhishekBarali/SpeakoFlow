//! Linux keyboard listener: subscribers of the process's one evdev engine.
//!
//! The engine (`engine.rs`) reads the input devices and takes keyboards over;
//! a [`KeyboardListener`](crate::KeyboardListener) only subscribes to it. One
//! engine serves every listener (the hotkey manager and the Settings recorder
//! beside it) because a keyboard can be taken over only once: a second engine
//! would find every keyboard busy. It starts with the first listener and hands
//! every device back when the last one leaves.

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::{debug, error, warn};

use crate::error::Result;
use crate::platform::state::BlockingHotkeys;
use crate::types::KeyEvent;

use super::engine::{Config, Engine};
use super::hub::{Hub, Subscriber};

/// How long a failed engine waits before starting again.
const RESTART_DELAY: Duration = Duration::from_secs(2);

/// Internal listener state returned to KeyboardListener
pub(crate) struct LinuxListenerState {
    pub event_receiver: Receiver<KeyEvent>,
    /// Always `None`: the engine thread belongs to the process, not to a
    /// listener.
    pub thread_handle: Option<JoinHandle<()>>,
    pub running: Arc<AtomicBool>,
    pub blocking_hotkeys: Option<BlockingHotkeys>,
}

static HUB: OnceLock<Arc<Hub>> = OnceLock::new();

fn hub() -> io::Result<Arc<Hub>> {
    if let Some(hub) = HUB.get() {
        return Ok(Arc::clone(hub));
    }
    let hub = Arc::new(Hub::new()?);
    Ok(Arc::clone(HUB.get_or_init(|| hub)))
}

/// Subscribe a listener to the engine, starting the engine if needed.
pub(crate) fn spawn(blocking_hotkeys: Option<BlockingHotkeys>) -> Result<LinuxListenerState> {
    let hub = hub()?;
    let (sender, event_receiver) = mpsc::channel();
    let running = Arc::new(AtomicBool::new(true));
    hub.subscribe(Subscriber {
        sender,
        running: Arc::clone(&running),
        blocking: blocking_hotkeys.clone(),
    });
    if let Err(error) = ensure_engine(&hub) {
        running.store(false, Ordering::SeqCst);
        return Err(error.into());
    }
    Ok(LinuxListenerState {
        event_receiver,
        thread_handle: None,
        running,
        blocking_hotkeys,
    })
}

/// A listener went away: let the engine see whether any is left.
pub(crate) fn wake() {
    if let Some(hub) = HUB.get() {
        hub.wake();
    }
}

fn ensure_engine(hub: &Arc<Hub>) -> io::Result<()> {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(());
    }
    let hub = Arc::clone(hub);
    if let Err(error) = thread::Builder::new()
        .name("handy-keys-evdev".into())
        .spawn(move || run_engine(&hub))
    {
        STARTED.store(false, Ordering::SeqCst);
        return Err(error);
    }
    Ok(())
}

fn run_engine(hub: &Arc<Hub>) {
    let mut failures = 0u32;
    loop {
        hub.wait_for_subscriber();
        // The engine is built on this thread and dropped here, by returning
        // or by unwinding, and dropping it hands every keyboard back.
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            Engine::new(Config::system(), Arc::clone(hub))?.run()
        }));
        match outcome {
            Ok(Ok(())) => {
                failures = 0;
                debug!("handy-keys: no listener left; every input device handed back");
                continue;
            }
            Ok(Err(error)) => {
                failures += 1;
                if failures <= 3 {
                    warn!("handy-keys: keyboard capture stopped ({error}); starting again in {RESTART_DELAY:?}");
                }
            }
            Err(_) => {
                failures += 1;
                if failures <= 3 {
                    error!("handy-keys: keyboard capture panicked; starting again in {RESTART_DELAY:?}");
                }
            }
        }
        thread::sleep(RESTART_DELAY);
    }
}
