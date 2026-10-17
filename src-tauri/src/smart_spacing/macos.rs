//! macOS: the text around the caret, through the Accessibility API.
//!
//! Every AX call is a synchronous message answered by the target app's main
//! thread, so all of them run on one dedicated thread with short messaging
//! timeouts. Never on our main thread: a stalled AX call there would freeze the
//! app, and asking our own windows would wait on ourselves.
//!
//! What the read does, and the bug each step avoids:
//!
//! * Needs the Accessibility permission SpeakoFlow already asks for to paste;
//!   without it nothing is read.
//! * Secure input (a password prompt anywhere) and secure text fields are
//!   never read.
//! * Terminals are never read: their text area is the scrollback.
//! * Chromium browsers and Electron apps only build their accessibility tree
//!   when asked. `AXManualAccessibility` is switched on for them, and back off a
//!   minute after the last read. `AXEnhancedUserInterface` is never written: it
//!   flips an app into screen-reader mode for the rest of its life.
//! * Only a small window around the caret is copied (`AXStringForRange`), never
//!   a whole document; the whole value is a fallback for short fields only.
//! * Offsets are UTF-16 units, so all slicing happens on UTF-16 and only the
//!   small result is decoded.
//! * A Chromium placeholder can arrive as the field's value; an empty web field
//!   is only "empty" when its character count says so.
//! * Chromium reports the caret on an empty last line before the line break
//!   that opened it; that is corrected.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use log::debug;
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFBoolean, CFNumber, CFRange, CFRetained, CFString, CFType};

use super::reading::{self, Reading, Snapshot, AFTER_CHARS, BEFORE_CHARS};

/// Messaging timeout for each AX call, in seconds.
const AX_TIMEOUT_SECS: f32 = 0.2;

/// How long `AXManualAccessibility` stays on after the last read of an app.
const LEASE_FOR: Duration = Duration::from_secs(60);

/// Longest field whose whole value may be copied when a ranged read is not
/// supported.
const MAX_VALUE_UNITS: usize = 200_000;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

struct Job {
    reply: SyncSender<Snapshot>,
}

static BUSY: AtomicBool = AtomicBool::new(false);
static JOBS: OnceLock<Mutex<Option<Sender<Job>>>> = OnceLock::new();

fn jobs() -> Option<Sender<Job>> {
    let slot = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("smart-spacing-ax".into())
            .spawn(move || worker(rx));
        Mutex::new(spawned.ok().map(|_| tx))
    });
    slot.lock().ok().and_then(|guard| guard.clone())
}

/// Read the focused field of the frontmost app, waiting at most `budget`.
pub fn read(budget: Duration) -> Snapshot {
    if BUSY.swap(true, Ordering::SeqCst) {
        debug!("Smart spacing: the previous read is still running; skipping this one");
        return Snapshot::unknown(None);
    }
    let Some(sender) = jobs() else {
        BUSY.store(false, Ordering::SeqCst);
        return Snapshot::unknown(None);
    };
    let (reply, answer) = mpsc::sync_channel(1);
    if sender.send(Job { reply }).is_err() {
        BUSY.store(false, Ordering::SeqCst);
        return Snapshot::unknown(None);
    }
    answer
        .recv_timeout(budget)
        .unwrap_or_else(|_| Snapshot::unknown(None))
}

/// An app whose accessibility tree we switched on, to be switched off again.
struct Lease {
    app: CFRetained<AXUIElement>,
    last_used: Instant,
}

fn worker(jobs: Receiver<Job>) {
    let mut leases: HashMap<i32, Lease> = HashMap::new();
    loop {
        match jobs.recv_timeout(Duration::from_secs(10)) {
            Ok(job) => {
                let snapshot = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read_frontmost(&mut leases)
                }))
                .unwrap_or_else(|_| Snapshot::unknown(None));
                BUSY.store(false, Ordering::SeqCst);
                let _ = job.reply.try_send(snapshot);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        release_expired(&mut leases);
    }
    for (_, lease) in leases.drain() {
        set_manual_accessibility(&lease.app, false);
    }
}

fn release_expired(leases: &mut HashMap<i32, Lease>) {
    let now = Instant::now();
    leases.retain(|_, lease| {
        if now.duration_since(lease.last_used) < LEASE_FOR {
            return true;
        }
        set_manual_accessibility(&lease.app, false);
        false
    });
}

/* ───────────────────────────── AX helpers ───────────────────────────── */

fn attr(element: &AXUIElement, name: &str) -> Result<CFRetained<CFType>, AXError> {
    let name = CFString::from_str(name);
    let mut value: *const CFType = std::ptr::null();
    let error = unsafe { element.copy_attribute_value(&name, NonNull::from(&mut value)) };
    if error != AXError::Success {
        return Err(error);
    }
    NonNull::new(value as *mut CFType)
        .map(|ptr| unsafe { CFRetained::from_raw(ptr) })
        .ok_or(AXError::NoValue)
}

fn attr_string(element: &AXUIElement, name: &str) -> Option<String> {
    let value = attr(element, name).ok()?;
    value.downcast_ref::<CFString>().map(|s| s.to_string())
}

fn attr_element(element: &AXUIElement, name: &str) -> Option<CFRetained<AXUIElement>> {
    attr(element, name).ok()?.downcast::<AXUIElement>().ok()
}

fn attr_bool(element: &AXUIElement, name: &str) -> Option<bool> {
    let value = attr(element, name).ok()?;
    value.downcast_ref::<CFBoolean>().map(|b| b.as_bool())
}

fn attr_isize(element: &AXUIElement, name: &str) -> Option<isize> {
    let value = attr(element, name).ok()?;
    value
        .downcast_ref::<CFNumber>()
        .and_then(|n| n.as_i64())
        .map(|n| n as isize)
}

fn attr_range(element: &AXUIElement, name: &str) -> Option<CFRange> {
    let value = attr(element, name).ok()?;
    let value = value.downcast_ref::<AXValue>()?;
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    let ok = unsafe {
        value.value(
            AXValueType::CFRange,
            NonNull::from(&mut range).cast::<c_void>(),
        )
    };
    ok.then_some(range)
}

fn string_for_range(element: &AXUIElement, range: CFRange) -> Option<Vec<u16>> {
    let name = CFString::from_str("AXStringForRange");
    let parameter =
        unsafe { AXValue::new(AXValueType::CFRange, NonNull::from(&range).cast::<c_void>()) }?;
    let mut value: *const CFType = std::ptr::null();
    let error = unsafe {
        element.copy_parameterized_attribute_value(
            &name,
            parameter.as_ref(),
            NonNull::from(&mut value),
        )
    };
    if error != AXError::Success {
        return None;
    }
    let value = unsafe { CFRetained::from_raw(NonNull::new(value as *mut CFType)?) };
    let text = value.downcast_ref::<CFString>()?.to_string();
    let units: Vec<u16> = text.encode_utf16().collect();
    // An app that answers with a different length than asked is not trusted.
    (units.len() as isize == range.length).then_some(units)
}

fn element_pid(element: &AXUIElement) -> Option<i32> {
    let mut pid: libc::pid_t = 0;
    let error = unsafe { element.pid(NonNull::from(&mut pid)) };
    (error == AXError::Success && pid > 0).then_some(pid)
}

fn set_timeout(element: &AXUIElement) {
    unsafe {
        let _ = element.set_messaging_timeout(AX_TIMEOUT_SECS);
    }
}

fn set_manual_accessibility(app: &AXUIElement, on: bool) {
    let name = CFString::from_str("AXManualAccessibility");
    let value: &CFType = CFBoolean::new(on).as_ref();
    unsafe {
        let _ = app.set_attribute_value(&name, value);
    }
}

/* ───────────────────────────── the app ───────────────────────────── */

fn app_bundle_path(pid: i32) -> Option<String> {
    let mut buffer = vec![0u8; 4096];
    let len =
        unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr() as *mut c_void, buffer.len() as u32) };
    if len <= 0 {
        return None;
    }
    let path = String::from_utf8_lossy(&buffer[..len as usize]).into_owned();
    // ".../Foo.app/Contents/MacOS/Foo" -> ".../Foo.app"
    let end = path.find(".app/").map(|i| i + 4)?;
    Some(path[..end].to_string())
}

fn bundle_name(bundle: &str) -> String {
    bundle
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".app")
        .to_ascii_lowercase()
}

fn is_terminal(name: &str) -> bool {
    matches!(
        name,
        "terminal"
            | "iterm"
            | "iterm2"
            | "warp"
            | "ghostty"
            | "kitty"
            | "alacritty"
            | "wezterm"
            | "hyper"
            | "tabby"
            | "cmux"
    )
}

/// Apps that only build their accessibility tree on request.
fn builds_tree_on_request(bundle: &str, name: &str) -> bool {
    let chromium = matches!(
        name,
        "google chrome"
            | "google chrome beta"
            | "google chrome canary"
            | "chromium"
            | "arc"
            | "brave browser"
            | "microsoft edge"
            | "vivaldi"
            | "opera"
            | "dia"
            | "comet"
    );
    chromium
        || std::path::Path::new(bundle)
            .join("Contents/Frameworks/Electron Framework.framework")
            .exists()
}

fn read_frontmost(leases: &mut HashMap<i32, Lease>) -> Snapshot {
    if !unsafe { AXIsProcessTrusted() } {
        return Snapshot::unknown(None);
    }
    if unsafe { IsSecureEventInputEnabled() } != 0 {
        return Snapshot::new(Reading::Secret, None);
    }
    let system = unsafe { AXUIElement::new_system_wide() };
    set_timeout(&system);
    let Some(app) = attr_element(&system, "AXFocusedApplication") else {
        return Snapshot::unknown(None);
    };
    let Some(pid) = element_pid(&app) else {
        return Snapshot::unknown(None);
    };
    if pid == std::process::id() as i32 {
        // Our own window is frontmost: nothing of the user's to read.
        return Snapshot::unknown(None);
    }
    let key = Some(format!("pid:{pid}"));
    let bundle = app_bundle_path(pid).unwrap_or_default();
    let name = bundle_name(&bundle);
    if is_terminal(&name) {
        return Snapshot::new(Reading::Terminal, key);
    }
    set_timeout(&app);

    let web_app = builds_tree_on_request(&bundle, &name);
    if web_app {
        match leases.get_mut(&pid) {
            Some(lease) => lease.last_used = Instant::now(),
            None => {
                if attr_bool(&app, "AXManualAccessibility") != Some(true) {
                    set_manual_accessibility(&app, true);
                    leases.insert(
                        pid,
                        Lease {
                            app: app.clone(),
                            last_used: Instant::now(),
                        },
                    );
                }
            }
        }
    }

    let focused = attr_element(&app, "AXFocusedUIElement").or_else(|| {
        attr_element(&system, "AXFocusedUIElement").filter(|el| element_pid(el) == Some(pid))
    });
    let Some(field) = focused else {
        return Snapshot::unknown(key);
    };
    set_timeout(&field);
    Snapshot::new(read_field(&field, web_app), key)
}

fn read_field(field: &AXUIElement, web_app: bool) -> Reading {
    if attr_string(field, "AXSubrole").as_deref() == Some("AXSecureTextField") {
        return Reading::Secret;
    }
    let role = attr_string(field, "AXRole").unwrap_or_default();
    let text_role = matches!(
        role.as_str(),
        "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField" | "AXWebArea"
    );
    if !text_role && attr_bool(field, "AXEditable") != Some(true) {
        return Reading::Unknown;
    }
    let Some(selection) = attr_range(field, "AXSelectedTextRange") else {
        return Reading::Unknown;
    };
    if selection.location < 0 || selection.length < 0 {
        return Reading::Unknown;
    }
    let count = attr_isize(field, "AXNumberOfCharacters");
    if count == Some(0) {
        return Reading::Empty;
    }
    if let Some(count) = count {
        if selection.location + selection.length > count {
            return Reading::Unknown;
        }
    }

    // A window around the selection, in UTF-16 units. Twice the character
    // budget, so surrogate pairs still leave enough characters.
    let low = (selection.location - (BEFORE_CHARS as isize) * 2).max(0);
    let wanted_high = selection.location + selection.length + (AFTER_CHARS as isize) * 2;
    let high = count.map_or(wanted_high, |count| wanted_high.min(count));
    let window = string_for_range(
        field,
        CFRange {
            location: low,
            length: high - low,
        },
    )
    .or_else(|| value_window(field, low, high));
    let Some(window) = window else {
        return Reading::Unknown;
    };

    let caret = (selection.location - low) as usize;
    let selection_end = caret + selection.length as usize;
    if selection_end > window.len() {
        return Reading::Unknown;
    }
    let mut caret_at = caret;
    // Chromium puts the caret of an empty last line before the line break that
    // opened it.
    if web_app
        && selection.length == 0
        && caret + 1 == window.len()
        && window[caret] == b'\n' as u16
    {
        caret_at = caret + 1;
    }
    let before = reading::decode_utf16_edge(&window[..caret_at], true);
    let selected =
        String::from_utf16_lossy(&window[caret_at.min(selection_end)..selection_end.max(caret_at)]);
    let after = reading::decode_utf16_edge(&window[selection_end.max(caret_at)..], false);

    let before = reading::tail_chars(&reading::normalize(&before), BEFORE_CHARS);
    let after = reading::head_chars(&reading::normalize(&after), AFTER_CHARS);
    let selected = reading::normalize(&selected);

    if before.is_empty() && after.is_empty() && selected.is_empty() {
        // A web field can report "" while holding text; only the character
        // count proves it empty, and that was handled above.
        return if web_app {
            Reading::Unknown
        } else {
            Reading::Empty
        };
    }
    // Chromium can hand over the placeholder as the value.
    if selection.location == 0 && selection.length == 0 {
        if let Some(placeholder) = attr_string(field, "AXPlaceholderValue") {
            let shown = format!("{before}{after}");
            if !placeholder.trim().is_empty() && shown.trim() == placeholder.trim() {
                return Reading::Empty;
            }
        }
    }
    Reading::Text {
        before,
        after,
        selected,
    }
}

/// The whole value, sliced to the window, for fields without ranged reads.
fn value_window(field: &AXUIElement, low: isize, high: isize) -> Option<Vec<u16>> {
    let value = attr_string(field, "AXValue")?;
    let units: Vec<u16> = value.encode_utf16().collect();
    if units.len() > MAX_VALUE_UNITS {
        return None;
    }
    let (low, high) = (low as usize, (high as usize).min(units.len()));
    (low <= high).then(|| units[low..high].to_vec())
}
