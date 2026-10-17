//! Windows: the text around the caret, through UI Automation.
//!
//! One dedicated thread owns the UI Automation client and does every read. COM
//! interface pointers are neither `Send` nor `Sync`, and a cross-process call
//! into a busy or hung app blocks the calling thread, so the caller only ever
//! waits on a channel with a deadline. A read still stuck when the next one is
//! asked for is not queued behind: the new one reports "unknown" straight away.
//!
//! This is a separate thread from the autolearn watcher on purpose. That one
//! polls a whole document for a minute after a paste; this one is on the way to
//! every paste and must never wait behind it.
//!
//! What the read does, and why each step is there (most are fixes other
//! dictation tools shipped after their users hit the bug):
//!
//! * The foreground window decides the target. One of our own windows in front
//!   means there is nothing to read; a terminal is never read; a hung window is
//!   skipped, and a window that just timed out is left alone for a few seconds.
//! * The focused element must belong to the foreground process. UI Automation
//!   focus can lag behind a window switch, which would read another app's field.
//! * Password fields are never read.
//! * The insertion point is the start of the single selection range (a collapsed
//!   range at the caret when nothing is selected). The paste replaces the
//!   selection, so what ends up before the new text is what is before the
//!   selection. `GetCaretRange` is only the fallback, and only when the caret
//!   reports itself active: an inactive caret can be stale.
//! * Both sides are clamped to the control's own document range, so a page's
//!   other text never leaks into "before".
//! * Two empty sides only prove an empty field when the range starts at the
//!   document start, or the control's value says it is empty. A visually empty
//!   Chromium box can report phantom placeholder text, and text equal to the
//!   control's name or help text is taken for a placeholder.
//! * Focus is checked again after the read; if it moved, the read is discarded.
//! * A read that quickly learns nothing is tried once more, 120 ms later.
//!   Chrome switches its accessibility on at the first request it gets and
//!   answers that request with nothing.
//! * Chromium's address bar is flagged: it trims the spaces of pasted text,
//!   so the space in front of a dictation has to be typed there.
//! * Timeouts: 200 ms per UI Automation call (the defaults are 2 s and 20 s),
//!   plus the caller's own deadline, because the per-call timeouts do not bound
//!   every provider.
//!
//! No event handler is ever registered: a subscribed client makes every UI
//! Automation provider on the system do more work.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use log::debug;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, S_OK};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextPattern2, IUIAutomationTextRange, IUIAutomationValuePattern,
    TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
    UIA_ComboBoxControlTypeId, UIA_EditControlTypeId, UIA_TextPattern2Id, UIA_TextPatternId,
    UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId, IsHungAppWindow,
};

use super::reading::{self, Reading, Snapshot, AFTER_CHARS, BEFORE_CHARS};

/// Per-call UI Automation timeout, connection and transaction alike.
const UIA_TIMEOUT_MS: u32 = 200;

/// After a timeout, leave that window alone for this long.
const COOLDOWN: Duration = Duration::from_secs(3);

/// A read that learned nothing this quickly is tried once more, this much
/// later. Measured on Chrome 155: a cold window answers its first request in
/// about 17 ms with nothing, and the next one, 100 ms later, with the text.
const RETRY_AFTER: Duration = Duration::from_millis(120);
const RETRY_IF_FASTER_THAN: Duration = Duration::from_millis(150);

/// The address bar of every Chromium browser (Chrome, Edge, Brave, Opera,
/// Vivaldi). It trims the spaces at both ends of pasted text, so a space that
/// has to go in front of a dictation is typed there instead.
const CHROMIUM_ADDRESS_BAR: &str = "OmniboxViewViews";

/// What one read of the focused control found.
struct Found {
    reading: Reading,
    /// The control drops spaces at the edges of pasted text.
    trims_pasted_spaces: bool,
}

struct Job {
    reply: SyncSender<Snapshot>,
}

/// Set while the worker is busy with a job; a read asked for meanwhile does
/// not queue behind a call that may be stuck in a hung app.
static BUSY: AtomicBool = AtomicBool::new(false);
static JOBS: OnceLock<Mutex<Option<Sender<Job>>>> = OnceLock::new();

fn jobs() -> Option<Sender<Job>> {
    let slot = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("smart-spacing-uia".into())
            .spawn(move || worker(rx));
        Mutex::new(spawned.ok().map(|_| tx))
    });
    slot.lock().ok().and_then(|guard| guard.clone())
}

/// Read the focused field of the foreground window, waiting at most `budget`.
///
/// The window is named before the worker is asked, so a read that is skipped
/// or runs out of time still says where the dictation is going, and the
/// last-paste memory can at least add the space between two dictations. It
/// used to come back with no window at all, which also wiped that memory, and
/// dictations into a slow field ran together.
pub fn read(budget: Duration) -> Snapshot {
    let target = foreground_target();
    if BUSY.swap(true, Ordering::SeqCst) {
        debug!("Smart spacing: the previous read is still running; skipping this one");
        return Snapshot::unknown(target);
    }
    let Some(sender) = jobs() else {
        BUSY.store(false, Ordering::SeqCst);
        return Snapshot::unknown(target);
    };
    let (reply, answer) = mpsc::sync_channel(1);
    if sender.send(Job { reply }).is_err() {
        BUSY.store(false, Ordering::SeqCst);
        return Snapshot::unknown(target);
    }
    // BUSY is cleared by the worker when it finishes, not here: a stuck call
    // must keep later reads from queueing behind it.
    answer.recv_timeout(budget).unwrap_or_else(|_| {
        debug!(
            "Smart spacing: the field did not answer within {} ms",
            budget.as_millis()
        );
        Snapshot::unknown(target)
    })
}

/// The window in front, keyed as a reading's target is; `None` when it is one
/// of ours, which has nothing of the user's to read.
fn foreground_target() -> Option<String> {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut pid)) };
    (pid != 0 && pid != unsafe { GetCurrentProcessId() }).then(|| window_key(foreground))
}

/// Owns this thread's COM apartment. `S_OK` means we initialised COM and owe
/// the matching `CoUninitialize`; `S_FALSE` means somebody else did.
struct Apartment {
    owned: bool,
}

impl Apartment {
    fn new() -> Self {
        let owned = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } == S_OK;
        Self { owned }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}

fn worker(jobs: Receiver<Job>) {
    let _apartment = Apartment::new();
    let client = unsafe {
        CoCreateInstance::<_, IUIAutomation2>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
    }
    .ok()
    .map(|client| {
        unsafe {
            let _ = client.SetConnectionTimeout(UIA_TIMEOUT_MS);
            let _ = client.SetTransactionTimeout(UIA_TIMEOUT_MS);
        }
        client
    });
    if client.is_none() {
        log::warn!("Smart spacing: UI Automation is unavailable; fields cannot be read");
    }
    let mut cooldown: Option<(isize, Instant)> = None;
    while let Ok(job) = jobs.recv() {
        let snapshot = match &client {
            Some(client) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_foreground(client, &mut cooldown)
            }))
            .unwrap_or_else(|_| Snapshot::unknown(None)),
            None => Snapshot::unknown(None),
        };
        BUSY.store(false, Ordering::SeqCst);
        let _ = job.reply.try_send(snapshot);
    }
    // The client is released here, on the thread that created it, before the
    // apartment is torn down.
    drop(client);
}

fn window_key(hwnd: HWND) -> String {
    format!("hwnd:{}", hwnd.0 as isize)
}

/// The window in front now, keyed the way a reading's target is.
pub fn current_target() -> Option<String> {
    let foreground = unsafe { GetForegroundWindow() };
    (!foreground.is_invalid()).then(|| window_key(foreground))
}

fn read_foreground(client: &IUIAutomation2, cooldown: &mut Option<(isize, Instant)>) -> Snapshot {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() {
        return Snapshot::unknown(None);
    }
    let key = Some(window_key(foreground));
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut pid)) };
    if pid == 0 || pid == unsafe { GetCurrentProcessId() } {
        // One of our own windows is in front (the overlay caught a click):
        // nothing of the user's to read, and the earlier reading stands.
        return Snapshot::unknown(None);
    }
    if unsafe { IsHungAppWindow(foreground) }.as_bool() {
        return Snapshot::unknown(key);
    }
    if let Some((hwnd, until)) = *cooldown {
        if hwnd == foreground.0 as isize && Instant::now() < until {
            return Snapshot::unknown(key);
        }
    }
    if is_terminal(foreground, pid) {
        return Snapshot::new(Reading::Terminal, key);
    }

    let started = Instant::now();
    let mut attempt = unsafe { read_focused(client, foreground, pid) };
    if matches!(
        attempt,
        Ok(Found {
            reading: Reading::Unknown,
            ..
        })
    ) && started.elapsed() < RETRY_IF_FASTER_THAN
    {
        // Chrome switches its accessibility on at the first request and
        // answers that one with nothing; a tenth of a second later it reads.
        // A quick "nothing" is worth one more look, which costs no felt time:
        // the read runs while the user is still talking.
        std::thread::sleep(RETRY_AFTER);
        attempt = unsafe { read_focused(client, foreground, pid) };
    }
    match attempt {
        Ok(found) => {
            Snapshot::new(found.reading, key).trimming_pasted_spaces(found.trims_pasted_spaces)
        }
        Err(error) => {
            if is_timeout(error.code().0) {
                *cooldown = Some((foreground.0 as isize, Instant::now() + COOLDOWN));
            }
            debug!(
                "Smart spacing: UI Automation read failed: {:?}",
                error.code()
            );
            Snapshot::unknown(key)
        }
    }
}

fn is_timeout(hresult: i32) -> bool {
    matches!(
        hresult as u32,
        0x8013_1505 /* UIA_E_TIMEOUT */ | 0x8007_05B4 /* ERROR_TIMEOUT */ | 0x8001_011F /* RPC_E_TIMEOUT */
    )
}

/// Terminals expose their whole buffer as text: the "text before the caret"
/// would be the prompt and the last command's output.
fn is_terminal(hwnd: HWND, pid: u32) -> bool {
    let mut class = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    if len > 0 {
        let class = String::from_utf16_lossy(&class[..len as usize]);
        if matches!(
            class.as_str(),
            "ConsoleWindowClass"
                | "CASCADIA_HOSTING_WINDOW_CLASS"
                | "mintty"
                | "VirtualConsoleClass"
                | "PuTTY"
        ) {
            return true;
        }
    }
    let exe = process_exe_name(pid)
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        exe.as_str(),
        "windowsterminal.exe"
            | "openconsole.exe"
            | "conhost.exe"
            | "cmd.exe"
            | "powershell.exe"
            | "pwsh.exe"
            | "mintty.exe"
            | "wezterm-gui.exe"
            | "alacritty.exe"
            | "tabby.exe"
            | "hyper.exe"
            | "putty.exe"
            | "kitty.exe"
            | "warp.exe"
    )
}

fn process_exe_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

unsafe fn read_focused(
    client: &IUIAutomation2,
    foreground: HWND,
    pid: u32,
) -> windows::core::Result<Found> {
    let automation: &IUIAutomation = client;
    let element = automation.GetFocusedElement()?;
    let class = element
        .CurrentClassName()
        .map(|c| c.to_string())
        .unwrap_or_default();
    // What kind of control, never what it holds: enough to tell from a log
    // which app's field misbehaved.
    debug!(
        "Smart spacing: the focused control is {} ({}, {})",
        element
            .CurrentLocalizedControlType()
            .map(|t| t.to_string())
            .unwrap_or_default(),
        element
            .CurrentFrameworkId()
            .map(|f| f.to_string())
            .unwrap_or_default(),
        class
    );
    let reading = read_element(automation, &element, foreground, pid)?;
    Ok(Found {
        reading,
        trims_pasted_spaces: class == CHROMIUM_ADDRESS_BAR,
    })
}

/// A read that learned nothing, with the reason in the log.
fn unknown(why: &str) -> Reading {
    debug!("Smart spacing: nothing learned: {why}");
    Reading::Unknown
}

unsafe fn read_element(
    automation: &IUIAutomation,
    element: &IUIAutomationElement,
    foreground: HWND,
    pid: u32,
) -> windows::core::Result<Reading> {
    if element.CurrentIsPassword()?.as_bool() {
        return Ok(Reading::Secret);
    }
    // Focus can lag behind a window switch; never read another app's field.
    if element.CurrentProcessId()? as u32 != pid {
        return Ok(unknown("the focused control belongs to another process"));
    }

    let Ok(text_pattern) =
        element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
    else {
        return Ok(value_only(element));
    };

    let Some(range) = insertion_range(&text_pattern, element)? else {
        return Ok(unknown("no single caret or selection"));
    };
    let document = text_pattern.DocumentRange()?;

    // Before: from up to BEFORE_CHARS before the selection start, to the start.
    let before = range.Clone()?;
    before.MoveEndpointByRange(
        TextPatternRangeEndpoint_End,
        &range,
        TextPatternRangeEndpoint_Start,
    )?;
    let moved = before.MoveEndpointByUnit(
        TextPatternRangeEndpoint_Start,
        TextUnit_Character,
        -(BEFORE_CHARS as i32),
    )?;
    if moved > 0 {
        return Ok(unknown("the text before the caret moved the wrong way"));
    }
    if before.CompareEndpoints(
        TextPatternRangeEndpoint_Start,
        &document,
        TextPatternRangeEndpoint_Start,
    )? < 0
    {
        before.MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            &document,
            TextPatternRangeEndpoint_Start,
        )?;
    }
    let at_document_start = before.CompareEndpoints(
        TextPatternRangeEndpoint_Start,
        &document,
        TextPatternRangeEndpoint_Start,
    )? == 0;

    // After: from the selection end, up to AFTER_CHARS further.
    let after = range.Clone()?;
    after.MoveEndpointByRange(
        TextPatternRangeEndpoint_Start,
        &range,
        TextPatternRangeEndpoint_End,
    )?;
    let moved = after.MoveEndpointByUnit(
        TextPatternRangeEndpoint_End,
        TextUnit_Character,
        AFTER_CHARS as i32,
    )?;
    if moved < 0 {
        return Ok(unknown("the text after the caret moved the wrong way"));
    }
    if after.CompareEndpoints(
        TextPatternRangeEndpoint_End,
        &document,
        TextPatternRangeEndpoint_End,
    )? > 0
    {
        after.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &document,
            TextPatternRangeEndpoint_End,
        )?;
    }

    // Caps in UTF-16 units: providers count "characters" differently, so the
    // text is trimmed again on our side.
    let before_text = range_text(&before, (BEFORE_CHARS * 2 + 8) as i32, true)?;
    let after_text = range_text(&after, (AFTER_CHARS * 2 + 8) as i32, false)?;
    let selected = range_text(&range, 256, false)?;
    let mut before_text = reading::tail_chars(&reading::normalize(&before_text), BEFORE_CHARS);
    let mut after_text = reading::head_chars(&reading::normalize(&after_text), AFTER_CHARS);
    let selected = reading::normalize(&selected);

    // Placeholder or phantom text in a field that is really empty.
    let value = value_text(element);
    let is_edit = value_is_its_text(element);
    if is_edit && value.as_deref() == Some("") {
        if !before_text.is_empty() || !after_text.is_empty() {
            debug!(
                "Smart spacing: the control's value is empty; its text is taken for a placeholder"
            );
        }
        before_text.clear();
        after_text.clear();
    } else if looks_like_placeholder(element, &before_text, &after_text) {
        debug!("Smart spacing: the text equals the control's name; taken for a placeholder");
        before_text.clear();
        after_text.clear();
    }

    // The focus must still be where it was when the read began.
    let now_focused = automation.GetFocusedElement()?;
    if GetForegroundWindow() != foreground
        || !automation.CompareElements(element, &now_focused)?.as_bool()
    {
        return Ok(unknown("focus moved during the read"));
    }

    if before_text.is_empty() && after_text.is_empty() && selected.is_empty() {
        // Two empty sides are not proof of an empty field on their own.
        let proven_empty = at_document_start || (is_edit && value.as_deref() == Some(""));
        return Ok(if proven_empty {
            Reading::Empty
        } else {
            unknown("no text around the caret, and not at the start of the field")
        });
    }
    if before_text.is_empty() && !at_document_start && !(is_edit && value.as_deref() == Some("")) {
        // Nothing before the caret, yet not at the start of the field: the
        // provider did not hand the text over.
        return Ok(unknown("no text before the caret, yet not at the start"));
    }
    debug!(
        "Smart spacing: {} characters before the caret (the last {}), {} after, {} selected",
        before_text.chars().count(),
        char_kind(before_text.chars().last()),
        after_text.chars().count(),
        selected.chars().count()
    );
    Ok(Reading::Text {
        before: before_text,
        after: after_text,
        selected,
    })
}

/// The range the paste will replace: the single selection, or the caret.
unsafe fn insertion_range(
    text_pattern: &IUIAutomationTextPattern,
    element: &IUIAutomationElement,
) -> windows::core::Result<Option<IUIAutomationTextRange>> {
    if let Ok(selection) = text_pattern.GetSelection() {
        // Several ranges is a multi-caret editor: there is no single place the
        // paste goes, so nothing is assumed.
        match selection.Length()? {
            1 => return Ok(Some(selection.GetElement(0)?)),
            0 => {}
            _ => return Ok(None),
        }
    }
    let Ok(pattern2) = element.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
    else {
        return Ok(None);
    };
    let mut active = BOOL(0);
    let caret = pattern2.GetCaretRange(&mut active)?;
    Ok(active.as_bool().then_some(caret))
}

unsafe fn range_text(
    range: &IUIAutomationTextRange,
    max_len: i32,
    is_before: bool,
) -> windows::core::Result<String> {
    let text = range.GetText(max_len)?;
    Ok(reading::decode_utf16_edge(&text, is_before))
}

fn value_text(element: &IUIAutomationElement) -> Option<String> {
    unsafe {
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()?;
        pattern.CurrentValue().ok().map(|value| value.to_string())
    }
}

/// A control without a text pattern: only an empty plain edit box can be
/// trusted, because the value says nothing about where the caret is.
fn value_only(element: &IUIAutomationElement) -> Reading {
    if value_is_its_text(element) && value_text(element).as_deref() == Some("") {
        Reading::Empty
    } else {
        unknown("the control has no text pattern")
    }
}

/// An edit box, or an editable combo box such as Google's search field: a
/// control whose value is exactly its text, so an empty value proves an empty
/// field whatever the text pattern shows. Google's box, empty, hands over its
/// placeholder as text, which read as a word after the caret.
fn value_is_its_text(element: &IUIAutomationElement) -> bool {
    unsafe { element.CurrentControlType() }
        .map(|t| t == UIA_EditControlTypeId || t == UIA_ComboBoxControlTypeId)
        .unwrap_or(false)
}

/// The kind of a character, for the log: what decides spacing, not the text.
fn char_kind(c: Option<char>) -> &'static str {
    match c {
        None => "none",
        Some(c) if c.is_whitespace() => "a space",
        Some(c) if c.is_alphabetic() => "a letter",
        Some(c) if c.is_numeric() => "a digit",
        Some(_) => "punctuation",
    }
}

/// A composer that shows "Type a message" can hand that text over as if it
/// were the field's content. It usually doubles as the control's name or help
/// text, which is what gives it away.
fn looks_like_placeholder(element: &IUIAutomationElement, before: &str, after: &str) -> bool {
    let shown = format!("{before}{after}");
    let shown = shown.trim();
    if shown.is_empty() {
        return false;
    }
    let matches = |label: windows::core::Result<windows::core::BSTR>| {
        label
            .map(|l| l.to_string().trim() == shown)
            .unwrap_or(false)
    };
    unsafe { matches(element.CurrentName()) || matches(element.CurrentHelpText()) }
}

#[cfg(test)]
mod live_tests {
    use std::time::{Duration, Instant};

    struct Stderr;

    impl log::Log for Stderr {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            if record.target().contains("smart_spacing") {
                eprintln!("  [{}] {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }

    static LOGGER: Stderr = Stderr;

    /// Reads the focused field of the window in front, the way a dictation
    /// does, and prints what came back, text included, so check an app with
    /// text of your own: focus a field in it, then within a few seconds run
    /// `cargo test --lib smart_spacing::windows::live_tests -- --ignored --nocapture`.
    /// `SMART_SPACING_READS` sets how many reads (default 3),
    /// `SMART_SPACING_EVERY_MS` the gap between them (default 1000), and
    /// `SMART_SPACING_ONLY` a window (`hwnd:<n>`) whose reads alone are printed.
    #[test]
    #[ignore]
    fn live_read_the_field_in_front() {
        let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Debug));
        let env = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(default)
        };
        let reads = env("SMART_SPACING_READS", 3);
        let every = Duration::from_millis(env("SMART_SPACING_EVERY_MS", 1000));
        let only = std::env::var("SMART_SPACING_ONLY").ok();
        let first = Instant::now();
        for i in 0..reads {
            let started = Instant::now();
            let snapshot = super::read(super::super::READ_BUDGET);
            let at = started.duration_since(first).as_millis();
            if only.is_some() && snapshot.target != only {
                println!("read {i} at {at} ms: another window is in front; not shown");
            } else {
                println!(
                    "read {i} at {at} ms: {} in {} ms{}: {:?}",
                    snapshot.reading.kind(),
                    started.elapsed().as_millis(),
                    if snapshot.trims_pasted_spaces {
                        ", trims pasted spaces"
                    } else {
                        ""
                    },
                    snapshot.reading
                );
            }
            std::thread::sleep(every);
        }
    }
}
