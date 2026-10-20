//! Windows clipboard transaction (see `paste_tx`).
//!
//! The dictation goes on the clipboard as a *delayed-render* format
//! (`SetClipboardData(CF_UNICODETEXT, NULL)`) owned by a hidden message-only
//! window. Windows sends that window `WM_RENDERFORMAT` when a consumer asks
//! for the data, which is the read receipt. The previous clipboard, every
//! format that can be copied byte for byte plus a bitmap, is put back once the
//! reads go quiet (`paste_tx::evaluate`), guarded by the clipboard sequence
//! number so a newer copy by the user is never overwritten.
//!
//! The same history and cloud-sync opt-outs as
//! `clipboard::write_transient_clipboard_text` go on with the promise and with
//! the restored content, so a dictation adds nothing to Win+V history and
//! never leaves the machine through clipboard sync. Monitoring stays on, for
//! the same reason it does there: remote-desktop and VM clients forward the
//! clipboard into their session.
//!
//! Threading: clipboard ownership and delayed rendering belong to the thread
//! that owns the window and need a message pump, so the whole transaction
//! lives on its own worker thread. That is also why a read is answered while
//! the paste still holds the app's main thread. The calling thread only sends
//! the keystroke once the worker reports the dictation published, and
//! returns; the wait, the restore and the auto-submit finish on the worker.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Once};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, SetLastError, ERROR_SUCCESS, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardOwner, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyImage, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, KillTimer, PostQuitMessage, RegisterClassW, SetTimer, SetWindowLongPtrW,
    GWLP_USERDATA, HWND_MESSAGE, IMAGE_BITMAP, LR_CREATEDIBSECTION, MSG, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WM_TIMER, WNDCLASSW,
};

use super::{
    blind_submit_due, evaluate, send_auto_submit_later, send_chord, settle_reason, settlement,
    ClipboardSettle, PasteRequest, PasteTxError, TxState, WaitDecision, PUBLISH_TIMEOUT,
};
use crate::settings::AutoSubmitKey;

const CLASS_NAME: PCWSTR = w!("SpeakoFlowPasteTxWindow");
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 25;
/// Clipboard formats larger than this are not snapshotted.
const MAX_FORMAT_BYTES: usize = 64 * 1024 * 1024;
/// The snapshot stops reading formats past this total size...
const MAX_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;
/// ...or this long. Well under `PUBLISH_TIMEOUT`, after which the paste gives
/// up on the transaction and takes the plain path while this snapshot could
/// still have the clipboard open.
const SNAPSHOT_BUDGET: Duration = Duration::from_millis(600);
/// `OpenClipboard` fails while another window has the clipboard open, which
/// clipboard managers do for a moment after every change.
const OPEN_ATTEMPTS: u32 = 10;
const OPEN_RETRY: Duration = Duration::from_millis(10);

// Standard clipboard formats (winuser.h). Named here rather than taken from
// `Win32_System_Ole`, which would be a whole feature for a dozen numbers.
const CF_BITMAP: u32 = 2;
const CF_METAFILEPICT: u32 = 3;
const CF_PALETTE: u32 = 9;
const CF_UNICODETEXT: u32 = 13;
const CF_ENHMETAFILE: u32 = 14;
const CF_OWNERDISPLAY: u32 = 0x0080;
const CF_DSPTEXT: u32 = 0x0081;
const CF_DSPBITMAP: u32 = 0x0082;
const CF_DSPMETAFILEPICT: u32 = 0x0083;
const CF_DSPENHMETAFILE: u32 = 0x008E;
const CF_PRIVATEFIRST: u32 = 0x0200;
const CF_PRIVATELAST: u32 = 0x02FF;
const CF_GDIOBJFIRST: u32 = 0x0300;
const CF_GDIOBJLAST: u32 = 0x03FF;

/// The history and cloud-sync opt-outs, each a DWORD 0. The same pair
/// `arboard`'s `exclude_from_history().exclude_from_cloud()` writes for
/// `clipboard::write_transient_clipboard_text`.
const EXCLUSION_FORMATS: [&str; 2] = ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"];

/// Whether a format's handle is plain global memory that can be copied byte
/// for byte and handed back later.
///
/// Bitmaps, palettes and metafiles are GDI objects; a metafile picture holds
/// a metafile handle that dies with the clipboard contents; the owner-display
/// formats and the private range belong to the owner's own process; and the
/// GDI-object range is freed with `DeleteObject`. Copying any of those would
/// put a dangling handle back. The bitmap is duplicated separately instead.
fn is_byte_copyable(format: u32) -> bool {
    !matches!(
        format,
        CF_BITMAP
            | CF_METAFILEPICT
            | CF_PALETTE
            | CF_ENHMETAFILE
            | CF_OWNERDISPLAY
            | CF_DSPTEXT
            | CF_DSPBITMAP
            | CF_DSPMETAFILEPICT
            | CF_DSPENHMETAFILE
            | CF_PRIVATEFIRST..=CF_PRIVATELAST
            | CF_GDIOBJFIRST..=CF_GDIOBJLAST
    )
}

struct SavedFormat {
    format: u32,
    data: Vec<u8>,
}

/// The caller's side of publishing: whether the caller gave up waiting (the
/// worker must then not publish) and whether the worker did publish (the
/// caller must then not fall back to the plain paste). One lock, so the two
/// can never both happen.
#[derive(Default)]
struct Handshake {
    abandoned: bool,
    published: bool,
}

struct WinTx {
    state: Mutex<TxState>,
    /// The dictation, UTF-16 with its terminating NUL.
    text: Vec<u16>,
    /// "Copy to clipboard": left as ordinary text when settling.
    leave_copy: Option<Vec<u16>>,
    snapshot: Mutex<Vec<SavedFormat>>,
    /// A duplicated `HBITMAP` (as a raw value), restored as `CF_BITMAP`.
    saved_bitmap: Mutex<Option<usize>>,
    sequence: AtomicU32,
    /// Settling happens exactly once, from the timer or from a newer paste.
    settled: AtomicBool,
    auto_submit: Option<AutoSubmitKey>,
    handshake: Mutex<Handshake>,
    /// Sends the auto-submit Enter off the paste's thread (see
    /// `paste_tx::send_auto_submit_later`). A callback rather than the
    /// `AppHandle` itself, so the manual test below builds without the app.
    submit_later: Option<SubmitLater>,
}

/// How a settling worker sends an owed auto-submit Enter.
type SubmitLater = Box<dyn Fn(AutoSubmitKey) + Send + Sync>;

impl Drop for WinTx {
    fn drop(&mut self) {
        // A bitmap duplicated for a restore that never happened (the user
        // copied something else) is ours to free.
        if let Some(raw) = lock(&self.saved_bitmap).take() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(raw as *mut _));
            }
        }
    }
}

/// What the worker reports once publishing is decided: an auto-submit owed by
/// a previous paste it settled (the caller holds the input lock, so it sends
/// that Enter), and whether this paste was published.
type Ready = (Option<AutoSubmitKey>, Result<(), String>);

/// The transaction holding the clipboard right now, if any. A new one settles
/// it first, so its snapshot is the user's content, not the last dictation.
static PENDING: Mutex<Option<Arc<WinTx>>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn tx_ptr(hwnd: HWND) -> *const WinTx {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WinTx
}

unsafe fn open_clipboard(owner: Option<HWND>) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..OPEN_ATTEMPTS {
        match OpenClipboard(owner) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            thread::sleep(OPEN_RETRY);
        }
    }
    Err(format!("OpenClipboard failed: {last}"))
}

/// Put `bytes` on the open clipboard as `format`.
unsafe fn set_global(format: u32, bytes: &[u8]) -> bool {
    let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
        return false;
    };
    let ptr = GlobalLock(hg) as *mut u8;
    if ptr.is_null() {
        let _ = GlobalFree(Some(hg));
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    let _ = GlobalUnlock(hg);
    // SetClipboardData owns the memory once it succeeds.
    if SetClipboardData(format, Some(HANDLE(hg.0))).is_err() {
        let _ = GlobalFree(Some(hg));
        return false;
    }
    true
}

/// Put NUL-terminated UTF-16 on the open clipboard as `CF_UNICODETEXT`.
unsafe fn set_text(text: &[u16]) -> bool {
    let bytes = std::slice::from_raw_parts(text.as_ptr() as *const u8, std::mem::size_of_val(text));
    set_global(CF_UNICODETEXT, bytes)
}

/// Add the history and cloud-sync opt-outs to the open clipboard, except any
/// format in `already` (a restored snapshot that carried its own).
unsafe fn set_exclusions(already: &[u32]) {
    for name in EXCLUSION_FORMATS {
        let name = wide(name);
        let format = RegisterClipboardFormatW(PCWSTR(name.as_ptr()));
        if format == 0 || already.contains(&format) {
            continue;
        }
        if !set_global(format, &0u32.to_ne_bytes()) {
            debug!("Paste: could not mark the clipboard as excluded from history");
        }
    }
}

unsafe extern "system" fn paste_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let tx = tx_ptr(hwnd);
    match msg {
        WM_RENDERFORMAT => {
            // The system opened the clipboard for us; render into it.
            if let Some(tx) = tx.as_ref() {
                lock(&tx.state).record_receipt(Instant::now());
                if wparam.0 as u32 == CF_UNICODETEXT {
                    set_text(&tx.text);
                }
            }
            LRESULT(0)
        }
        WM_RENDERALLFORMATS => {
            // The window is going away with a promise still out: not a read,
            // so no receipt. Here the clipboard is not opened for us, and it
            // must still be ours to render into.
            if let Some(tx) = tx.as_ref() {
                if OpenClipboard(Some(hwnd)).is_ok() {
                    if GetClipboardOwner().is_ok_and(|owner| owner == hwnd) {
                        set_text(&tx.text);
                    }
                    let _ = CloseClipboard();
                }
            }
            LRESULT(0)
        }
        WM_DESTROYCLIPBOARD => {
            if let Some(tx) = tx.as_ref() {
                lock(&tx.state).ownership_lost = true;
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(tx) = tx.as_ref() {
                on_timer(tx);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn ensure_window_class(hinstance: HINSTANCE) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(paste_wnd_proc),
            hInstance: hinstance,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&wc);
        }
    });
}

/// Settle `tx` once: restore the user's clipboard (or leave the copy) while it
/// is still ours. Returns the auto-submit Enter to send, if one is owed; the
/// caller sends it, because only it knows whether it holds the input lock.
unsafe fn settle(tx: &WinTx) -> Option<AutoSubmitKey> {
    if tx.settled.swap(true, Ordering::SeqCst) {
        return None;
    }
    let still_ours = GetClipboardSequenceNumber() == tx.sequence.load(Ordering::SeqCst);
    let (decision, reason) = {
        let state = lock(&tx.state);
        (
            settlement(
                &state,
                still_ours,
                tx.auto_submit.is_some(),
                tx.leave_copy.is_some(),
            ),
            settle_reason(&state),
        )
    };
    info!("Paste: settling, {reason}");
    match decision.clipboard {
        ClipboardSettle::Restore => restore_snapshot(tx),
        ClipboardSettle::LeaveCopy => leave_copy(tx),
        ClipboardSettle::Untouched => {
            info!("Paste: the clipboard changed since the paste; leaving it as it is")
        }
    }
    if decision.submit {
        tx.auto_submit
    } else {
        None
    }
}

/// "Copy to clipboard": replace the promise with the dictation as ordinary
/// text, which history and clipboard managers keep, as the user asked.
unsafe fn leave_copy(tx: &WinTx) {
    let Some(copy) = tx.leave_copy.as_ref() else {
        return;
    };
    if let Err(e) = open_clipboard(None) {
        warn!("Paste: could not leave the dictation on the clipboard: {e}");
        return;
    }
    let _ = EmptyClipboard();
    set_text(copy);
    let _ = CloseClipboard();
}

/// Put the snapshot back. Opened without an owner window on purpose: emptying
/// the clipboard from our own window would send that window
/// `WM_DESTROYCLIPBOARD`, and a newer transaction's window must never read
/// that as having lost the clipboard.
unsafe fn restore_snapshot(tx: &WinTx) {
    if let Err(e) = open_clipboard(None) {
        warn!("Paste: could not restore the clipboard: {e}");
        return;
    }
    let _ = EmptyClipboard();
    let formats = std::mem::take(&mut *lock(&tx.snapshot));
    for saved in formats.iter().filter(|saved| !saved.data.is_empty()) {
        set_global(saved.format, &saved.data);
    }
    if let Some(raw) = lock(&tx.saved_bitmap).take() {
        let bitmap = HANDLE(raw as *mut _);
        if SetClipboardData(CF_BITMAP, Some(bitmap)).is_err() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
    }
    // Putting the user's content back is not a new copy: keep it out of
    // history, as `write_transient_clipboard_text` does for the plain paste.
    let restored: Vec<u32> = formats.iter().map(|saved| saved.format).collect();
    set_exclusions(&restored);
    let _ = CloseClipboard();
    debug!("Paste: restored {} clipboard formats", formats.len());
}

unsafe fn snapshot_clipboard(hwnd: HWND, tx: &WinTx) -> Result<(), String> {
    open_clipboard(Some(hwnd))?;
    // Listing is cheap; reading is not, because a format the copying app
    // promised is rendered by that app the moment it is read, while the
    // paste (and the app's main thread) waits for this snapshot. So the text
    // is read first, OLE's own bookkeeping is never read, and the rest is
    // read only within a time and size budget. The plain paste kept only the
    // text (or an image), so a snapshot cut short still restores more.
    let mut listed: Vec<(usize, u32)> = Vec::new();
    let mut format = 0u32;
    loop {
        format = EnumClipboardFormats(format);
        if format == 0 {
            break;
        }
        listed.push((listed.len(), format));
    }
    // Read the text first, but remember where each format was: apps pick the
    // first format they understand in clipboard order, so the restore has to
    // keep the order (rich text ahead of plain text) the copying app chose.
    if let Some(text_at) = listed.iter().position(|&(_, f)| f == CF_UNICODETEXT) {
        listed[..=text_at].rotate_right(1);
    }

    let started = Instant::now();
    let mut total = 0usize;
    let mut formats: Vec<(usize, SavedFormat)> = Vec::new();
    for (index, &(order, format)) in listed.iter().enumerate() {
        // The text always; everything else only while the budget lasts.
        if index > 0 && (started.elapsed() >= SNAPSHOT_BUDGET || total >= MAX_SNAPSHOT_BYTES) {
            info!(
                "Paste: clipboard snapshot stopped after {} of {} formats ({} ms, {} KB); \
                 the rest will not be restored",
                formats.len(),
                listed.len(),
                started.elapsed().as_millis(),
                total / 1024
            );
            break;
        }
        if format == CF_BITMAP {
            // A GDI object, not global memory: duplicate it.
            if let Ok(handle) = GetClipboardData(CF_BITMAP) {
                if let Ok(copy) = CopyImage(handle, IMAGE_BITMAP, 0, 0, LR_CREATEDIBSECTION) {
                    *lock(&tx.saved_bitmap) = Some(copy.0 as usize);
                }
            }
            continue;
        }
        if !is_byte_copyable(format) || is_ole_bookkeeping(format) {
            continue;
        }
        let Ok(handle) = GetClipboardData(format) else {
            continue;
        };
        let hg = HGLOBAL(handle.0);
        let size = GlobalSize(hg);
        if size == 0 || size > MAX_FORMAT_BYTES {
            continue;
        }
        let ptr = GlobalLock(hg) as *const u8;
        if ptr.is_null() {
            continue;
        }
        let data = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(hg);
        total += size;
        formats.push((order, SavedFormat { format, data }));
    }
    let _ = CloseClipboard();
    formats.sort_by_key(|(order, _)| *order);
    *lock(&tx.snapshot) = formats.into_iter().map(|(_, saved)| saved).collect();
    Ok(())
}

/// Registered formats that are OLE's bookkeeping rather than data: they
/// describe, or point at, a live data object inside the app that copied.
/// Their bytes mean nothing once that copy is replaced (put back, they would
/// send an OLE-aware app looking for an object that is gone), and reading
/// them makes the copying app (Office, typically) render embedded objects
/// while the paste waits.
fn is_ole_bookkeeping(format: u32) -> bool {
    const OLE_FORMATS: [&str; 10] = [
        "DataObject",
        "Ole Private Data",
        "Object Descriptor",
        "Link Source Descriptor",
        "Embed Source",
        "Embedded Object",
        "Link Source",
        "Native",
        "OwnerLink",
        "ObjectLink",
    ];
    // Registered formats start at 0xC000; below that are the standard ones.
    if format < 0xC000 {
        return false;
    }
    let mut name = [0u16; 64];
    // SAFETY: the buffer is valid for its length, which the call is given.
    let len = unsafe { GetClipboardFormatNameW(format, &mut name) };
    if len <= 0 {
        return false;
    }
    let name = String::from_utf16_lossy(&name[..len as usize]);
    OLE_FORMATS.iter().any(|ole| name.eq_ignore_ascii_case(ole))
}

/// Publish the dictation as a promise. Returns the new sequence number.
unsafe fn publish(hwnd: HWND) -> Result<u32, String> {
    open_clipboard(Some(hwnd))?;
    let published = publish_formats();
    let closed = CloseClipboard();
    published?;
    closed.map_err(|e| format!("CloseClipboard failed: {e}"))?;
    Ok(GetClipboardSequenceNumber())
}

/// Everything `publish` does while the clipboard is open, split out so the
/// clipboard is closed on every path.
unsafe fn publish_formats() -> Result<(), String> {
    EmptyClipboard().map_err(|e| format!("EmptyClipboard failed: {e}"))?;
    set_exclusions(&[]);
    // NULL handle = delayed rendering. SetClipboardData returns the handle it
    // was given, so success is also NULL and the windows crate reports it as
    // an `Err` carrying GetLastError(). Only a nonzero thread error is a real
    // failure, and it is cleared first so a stale one cannot pose as it
    // (cjpais/Handy #1847).
    SetLastError(ERROR_SUCCESS);
    if let Err(e) = SetClipboardData(CF_UNICODETEXT, None) {
        if e.code().is_err() {
            return Err(format!("SetClipboardData failed: {e}"));
        }
    }
    Ok(())
}

fn on_timer(tx: &WinTx) {
    // A blind paste's Enter goes ahead of the restore (`BLIND_SUBMIT_DELAY`).
    if let (Some(key), Some(submit)) = (tx.auto_submit, tx.submit_later.as_ref()) {
        // `settled` is read under the state lock, which `settle` holds while it
        // decides its own Enter: either this sends it and `settle` sees
        // `submitted`, or `settle` got there first and this sees `settled`.
        let due = {
            let mut state = lock(&tx.state);
            let due =
                !tx.settled.load(Ordering::SeqCst) && blind_submit_due(&state, Instant::now());
            if due {
                state.submitted = true;
            }
            due
        };
        if due {
            debug!("Paste: sending auto-submit; the target's read cannot be seen");
            submit(key);
        }
    }
    let finish = tx.settled.load(Ordering::SeqCst)
        || evaluate(&lock(&tx.state), Instant::now()) == WaitDecision::Finish;
    if !finish {
        return;
    }
    if let (Some(key), Some(submit)) = (unsafe { settle(tx) }, tx.submit_later.as_ref()) {
        submit(key);
    }
    let mut pending = lock(&PENDING);
    if pending
        .as_ref()
        .is_some_and(|current| std::ptr::eq(Arc::as_ptr(current), tx))
    {
        *pending = None;
    }
    drop(pending);
    unsafe { PostQuitMessage(0) };
}

/// Settle the previous transaction, if one still holds the clipboard, so the
/// next snapshot is the user's content. Its worker sees it settled on its next
/// tick and exits.
fn flush_pending() -> Option<AutoSubmitKey> {
    let previous = lock(&PENDING).take()?;
    lock(&previous.state).cancelled = true;
    unsafe { settle(&previous) }
}

/// [`flush_pending`] for an ask's selection harvest, which runs while the ask
/// keys are held: an Enter still owed would reach the target with them (see
/// `paste_tx::settle_pending`), so it is dropped.
pub(super) fn settle_pending(_app_handle: &tauri::AppHandle) {
    if flush_pending().is_some() {
        warn!(
            "Paste: skipped the previous dictation's auto-submit; an ask started \
             before its target had read it"
        );
    }
}

/// [`flush_pending`] before the app exits; an owed Enter still goes.
pub(super) fn settle_before_exit(app_handle: &tauri::AppHandle) {
    if let Some(key) = flush_pending() {
        send_auto_submit_later(app_handle, key);
    }
}

unsafe fn destroy_window(hwnd: HWND) {
    let ptr = tx_ptr(hwnd);
    let _ = DestroyWindow(hwnd);
    if !ptr.is_null() {
        drop(Arc::from_raw(ptr));
    }
}

fn pump_thread(tx: Arc<WinTx>, ready: Sender<Ready>) {
    let owed = flush_pending();
    let fail = |e: String| {
        let _ = ready.send((owed, Err(e)));
    };
    unsafe {
        let hinstance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(module) => HINSTANCE(module.0),
            Err(e) => return fail(format!("GetModuleHandle failed: {e}")),
        };
        ensure_window_class(hinstance);
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS_NAME,
            w!("SpeakoFlowPasteTx"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(e) => return fail(format!("CreateWindowEx failed: {e}")),
        };
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Arc::into_raw(tx.clone()) as isize);

        if let Err(e) = snapshot_clipboard(hwnd, &tx) {
            destroy_window(hwnd);
            return fail(e);
        }
        let published = {
            let mut handshake = lock(&tx.handshake);
            if handshake.abandoned {
                Err("the paste stopped waiting".to_string())
            } else {
                let result = publish(hwnd);
                match &result {
                    Ok(_) => handshake.published = true,
                    // Publishing may have emptied the clipboard before it
                    // failed: put the user's content back before the plain
                    // paste snapshots it.
                    Err(_) => restore_snapshot(&tx),
                }
                result
            }
        };
        let sequence = match published {
            Ok(sequence) => sequence,
            Err(e) => {
                destroy_window(hwnd);
                return fail(e);
            }
        };
        tx.sequence.store(sequence, Ordering::SeqCst);
        lock(&tx.state).published_at = Instant::now();
        *lock(&PENDING) = Some(tx.clone());
        let _ = SetTimer(Some(hwnd), TIMER_ID, TIMER_INTERVAL_MS, None);
        let _ = ready.send((owed, Ok(())));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            DispatchMessageW(&msg);
        }

        let _ = KillTimer(Some(hwnd), TIMER_ID);
        destroy_window(hwnd);
    }
}

pub(super) fn run(
    app_handle: &tauri::AppHandle,
    enigo: &mut enigo::Enigo,
    request: PasteRequest<'_>,
) -> Result<(), PasteTxError> {
    let app = app_handle.clone();
    let submit: SubmitLater = Box::new(move |key| send_auto_submit_later(&app, key));
    run_with(Some(submit), enigo, request)
}

fn run_with(
    submit_later: Option<SubmitLater>,
    enigo: &mut enigo::Enigo,
    request: PasteRequest<'_>,
) -> Result<(), PasteTxError> {
    let tx = Arc::new(WinTx {
        state: Mutex::new(TxState::new()),
        text: wide(request.text),
        leave_copy: request.leave_on_clipboard.as_deref().map(wide),
        snapshot: Mutex::new(Vec::new()),
        saved_bitmap: Mutex::new(None),
        sequence: AtomicU32::new(0),
        settled: AtomicBool::new(false),
        auto_submit: request.auto_submit,
        handshake: Mutex::new(Handshake::default()),
        submit_later,
    });

    let (ready_tx, ready_rx) = mpsc::channel();
    let for_pump = tx.clone();
    thread::Builder::new()
        .name("paste-tx".into())
        .spawn(move || pump_thread(for_pump, ready_tx))
        .map_err(|e| PasteTxError::Unavailable(format!("could not start the paste worker: {e}")))?;

    match ready_rx.recv_timeout(PUBLISH_TIMEOUT) {
        Ok((owed, result)) => {
            // An Enter the previous paste still owed goes before this paste,
            // in the order the user dictated them.
            if let Some(key) = owed {
                if let Err(e) = crate::clipboard::send_return_key(enigo, key) {
                    warn!("Paste: could not send the previous auto-submit key: {e}");
                }
            }
            result.map_err(PasteTxError::Unavailable)?;
        }
        Err(_) => {
            let mut handshake = lock(&tx.handshake);
            if !handshake.published {
                handshake.abandoned = true;
                return Err(PasteTxError::Unavailable(
                    "publishing the dictation took too long".to_string(),
                ));
            }
        }
    }
    debug!("Paste: dictation published for delayed rendering");

    if !request.paste_delay.is_zero() {
        thread::sleep(request.paste_delay);
    }

    // Marked before sending: the keystroke is held ~100 ms, and a fast target
    // may read while it is still down.
    lock(&tx.state).injected_at = Some(Instant::now());
    if let Err(e) = send_chord(enigo, request.paste_method) {
        // The worker restores the clipboard after the short timeout.
        lock(&tx.state).injection_failed = true;
        error!("Paste: could not send the paste keystroke: {e}");
        return Err(PasteTxError::Keystroke(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_global_memory_formats_are_copied_byte_for_byte() {
        for format in [1, 7, 8, 13, 15, 16, 17, 0xC000, 0xC123] {
            assert!(is_byte_copyable(format), "format {format:#x}");
        }
        for format in [
            CF_BITMAP,
            CF_METAFILEPICT,
            CF_PALETTE,
            CF_ENHMETAFILE,
            CF_OWNERDISPLAY,
            CF_DSPTEXT,
            CF_DSPENHMETAFILE,
            0x0200,
            0x02FF,
            0x0300,
            0x03FF,
        ] {
            assert!(!is_byte_copyable(format), "format {format:#x}");
        }
    }

    #[test]
    fn text_is_handed_over_with_its_terminator() {
        let text = wide("héllo");
        assert_eq!(text.last(), Some(&0));
        assert_eq!(text.len(), "héllo".encode_utf16().count() + 1);
    }

    /// Manual end-to-end check of the delayed-render paste, for a person at
    /// the keyboard: focus a text field (Notepad), then run
    /// `cargo test --lib paste_tx::windows::tests::manual -- --ignored --nocapture`
    /// and switch back to the field within three seconds. It writes the
    /// system clipboard and sends a real Ctrl+V, so it is never run in CI.
    /// Expected: "SpeakoFlow paste test" is pasted, and the clipboard holds
    /// what it held before, a second or so later.
    #[test]
    #[ignore = "writes the system clipboard and sends a real Ctrl+V"]
    fn manual_delayed_render_paste() {
        let mut enigo = enigo::Enigo::new(&enigo::Settings::default()).expect("enigo");
        thread::sleep(Duration::from_secs(3));
        let result = run_with(
            None,
            &mut enigo,
            PasteRequest {
                text: "SpeakoFlow paste test",
                paste_method: crate::settings::PasteMethod::CtrlV,
                paste_delay: Duration::from_millis(60),
                auto_submit: None,
                leave_on_clipboard: None,
            },
        );
        assert!(result.is_ok());
        thread::sleep(super::super::RESTORE_TIMEOUT + Duration::from_secs(1));
    }
}
