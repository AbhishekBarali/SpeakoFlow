//! Capturing the text the user has selected in whatever application has focus.
//!
//! This is what lets someone highlight a paragraph, press the assistant shortcut,
//! say "translate this", and get an answer about the selection rather than about
//! nothing. The result is optional context: a turn with no selection is a
//! perfectly ordinary question, so every failure path here answers "no selection"
//! and the turn proceeds without it.
//!
//! # The one rule
//!
//! **Never report clipboard contents we cannot prove arrived from this capture.**
//!
//! The mechanism on most platforms is a synthetic Ctrl+C, and a synthetic Ctrl+C
//! frequently does nothing at all — nothing was selected, or the target app does
//! not implement copy, or it was busy. Reading the clipboard regardless returns
//! whatever the user copied earlier, which is plausibly a password or an unrelated
//! document, and that text would then be sent to whichever model provider is
//! configured, possibly a cloud one. A stale read is therefore not a cosmetic bug
//! but a way to leak data the user never offered.
//!
//! So [`capture_selection`] proves freshness before it trusts anything:
//!
//! * **Wayland** reads the compositor's primary selection through the
//!   data-control protocol (KDE Plasma, wlroots compositors). It is the same
//!   thing as X11's `PRIMARY` below, for native Wayland apps, and it is tried
//!   first because on a Wayland desktop XWayland's `PRIMARY` does not hold their
//!   selection (see `read_wayland_primary_selection`).
//! * **X11** reads the `PRIMARY` selection directly. Selected text is already
//!   there by definition, so there is no keystroke, no timing race, no clipboard
//!   to clobber and nothing to prove. This is the best path on an X11 desktop.
//! * **Windows** compares `GetClipboardSequenceNumber` before and after. If the
//!   counter did not move, the copy did not happen, and we say so.
//! * **Anywhere the counter is unavailable** (macOS, Wayland), a sentinel is
//!   written to the clipboard first and the read only counts once the contents
//!   differ from it. That costs us the ability to preserve a non-text clipboard,
//!   which is exactly why a non-text clipboard makes the capture bail out instead.
//!
//! `Ok(None)` means "asked, and there was genuinely no selection". `Err` means
//! "could not tell", which must never be presented to the user as an empty
//! selection.

use crate::clipboard::{self, ClipboardSnapshot};
use crate::input::{self, EnigoState};
use log::{debug, warn};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// How long to wait for the target application to service the copy.
///
/// This has to be a poll rather than the fixed sleeps the paste path uses. Those
/// run in the outbound direction (we write, the app reads) and 60 ms is plenty.
/// A harvest is inbound: the target has to pump its message loop, run its copy
/// handler and publish to the clipboard.
///
/// Kept deliberately short. This runs on every assistant press, and when nothing
/// is selected — the common case — the full budget is always spent, because "no
/// selection" and "the app is being slow" look identical from here. A generous
/// budget therefore buys a rare success at the cost of holding a background thread
/// and the synthetic-input lock on every single invocation. 250 ms covers ordinary
/// native and Electron apps; something slower than that loses its selection this
/// time and the user can ask again.
const CAPTURE_BUDGET: Duration = Duration::from_millis(250);

/// Gap between clipboard checks while waiting for the copy to land.
const POLL_INTERVAL: Duration = Duration::from_millis(15);

/// Selections longer than this are truncated. A whole document pasted into a
/// prompt buys nothing and can push a small local model past its context window.
const MAX_SELECTION_CHARS: usize = 8000;

/// Sentinel written to the clipboard when no change counter is available, so a
/// read can be told apart from the user's previous clipboard contents. Chosen to
/// be something no human would ever have copied.
const SENTINEL: &str = "\u{200B}speakoflow-selection-probe\u{200B}";

/// Where a captured selection came from. Kept because the two routes have
/// genuinely different reliability, and the logs are much easier to read when a
/// bug report says which one was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    /// X11 `PRIMARY`: the selection itself, no keystroke involved.
    ///
    /// Only ever constructed on Linux, so every other target sees it as dead —
    /// hence the allow, rather than a `cfg` that would make the enum's shape differ
    /// per platform and drag `cfg` noise into every match on it.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    PrimarySelection,
    /// The Wayland primary selection, read through the compositor's
    /// data-control protocol: no keystroke involved. Linux only, like the
    /// variant above.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    WaylandPrimarySelection,
    /// A synthetic Ctrl+C / Cmd+C, verified to have changed the clipboard.
    SyntheticCopy,
}

#[derive(Debug, Clone)]
pub struct CapturedSelection {
    pub text: String,
    pub source: SelectionSource,
}

/// Harvest the focused application's current text selection.
///
/// * `Ok(Some(_))` — there was a selection and this is it.
/// * `Ok(None)` — the application answered and there was nothing selected.
/// * `Err(_)` — we could not establish either way. Callers must treat this as
///   "unknown", never as "nothing selected".
pub fn capture_selection(app: &AppHandle) -> Result<Option<CapturedSelection>, String> {
    // Wayland first, where the compositor offers it: the same no-keystroke read
    // as X11's PRIMARY below, from the one place a native Wayland app's
    // selection actually is.
    #[cfg(target_os = "linux")]
    if let Some(result) = read_wayland_primary_selection() {
        return result.map(|text| {
            normalize(text).map(|text| CapturedSelection {
                text,
                source: SelectionSource::WaylandPrimarySelection,
            })
        });
    }

    // Then X11: the selection is already in PRIMARY, so this needs no keystroke,
    // cannot collide with a held hotkey, and leaves the clipboard alone.
    #[cfg(target_os = "linux")]
    if let Some(text) = read_primary_selection() {
        let text = text.trim().to_string();
        return Ok(normalize(text).map(|text| CapturedSelection {
            text,
            source: SelectionSource::PrimarySelection,
        }));
    }

    // A modifier the user is still physically holding turns our Ctrl+C into some
    // other combo entirely, and clearing it afterwards would desync the OS from
    // their actual keyboard. Refuse rather than corrupt.
    if input::conflicting_modifier_held() {
        return Err(
            "a modifier key is being held, so a synthetic copy would be misread".to_string(),
        );
    }

    let snapshot = clipboard::snapshot_clipboard(app);
    // A clipboard holding an image or a file list cannot be put back afterwards,
    // and the sentinel path would have to overwrite it to work at all. Someone who
    // copied an image and then asks about some selected text must not lose it.
    if matches!(snapshot, ClipboardSnapshot::NotText) && clipboard::clipboard_sequence().is_none() {
        return Err(
            "the clipboard holds something we cannot restore, so the copy was skipped".to_string(),
        );
    }

    let result = synthetic_copy_capture(app, &snapshot);
    clipboard::restore_clipboard(app, &snapshot);
    result
}

/// The verified synthetic-copy path. Split out so the clipboard is restored on
/// every exit, including the error ones.
fn synthetic_copy_capture(
    app: &AppHandle,
    snapshot: &ClipboardSnapshot,
) -> Result<Option<CapturedSelection>, String> {
    let before = clipboard::clipboard_sequence();

    // With no change counter, plant a sentinel so a read can be distinguished
    // from the user's prior clipboard contents.
    if before.is_none() {
        clipboard::write_clipboard_text(app, SENTINEL)
            .map_err(|e| format!("could not prepare the clipboard for a selection capture: {e}"))?;
    }

    {
        let enigo_state = app
            .try_state::<EnigoState>()
            .ok_or_else(|| "synthetic input is not available".to_string())?;
        let mut enigo = enigo_state
            .0
            .lock()
            .map_err(|_| "the synthetic input lock is poisoned".to_string())?;
        input::send_copy_combo(&mut enigo)?;
    }

    // Poll until the clipboard demonstrably changes, or the budget runs out. A
    // timeout is the "no selection" answer: the app was asked and published
    // nothing.
    let deadline = Instant::now() + CAPTURE_BUDGET;
    while Instant::now() < deadline {
        std::thread::sleep(POLL_INTERVAL);

        if let (Some(before), Some(now)) = (before, clipboard::clipboard_sequence()) {
            if now == before {
                continue; // Nothing has landed yet.
            }
        }

        let Ok(text) = app.clipboard_text() else {
            continue;
        };

        // The sentinel path: unchanged means the copy has not landed.
        if before.is_none() && text == SENTINEL {
            continue;
        }

        // The counter moved (or the sentinel was replaced), so this text is ours.
        // One more check: an app that "copies" an unchanged selection can leave
        // the clipboard byte-identical to what was there before, which is
        // indistinguishable from a stale read. Treating it as no selection is the
        // safe way to be wrong — the user simply asks again or types the text.
        if let ClipboardSnapshot::Text(previous) = snapshot {
            if before.is_none() && &text == previous {
                continue;
            }
        }

        debug!(
            "captured a {} character selection via synthetic copy",
            text.chars().count()
        );
        return Ok(normalize(text).map(|text| CapturedSelection {
            text,
            source: SelectionSource::SyntheticCopy,
        }));
    }

    debug!("no selection: the copy produced no clipboard change within the budget");
    Ok(None)
}

/// Trim, reject empty, and cap the length.
fn normalize(text: String) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= MAX_SELECTION_CHARS {
        return Some(trimmed.to_string());
    }
    let truncated: String = trimmed.chars().take(MAX_SELECTION_CHARS).collect();
    warn!(
        "selection truncated from {} to {} characters",
        trimmed.chars().count(),
        MAX_SELECTION_CHARS
    );
    Some(truncated)
}

/// How long the application that owns the Wayland primary selection gets to
/// hand it over.
///
/// Nothing waits on this: the capture has its own thread and runs while the user
/// is still speaking. "Nothing selected" is answered by the compositor at once,
/// so this only bounds an owner that has stopped responding, which would
/// otherwise hold the capture thread for as long as it stays stuck.
#[cfg(target_os = "linux")]
const WAYLAND_PRIMARY_BUDGET: Duration = Duration::from_millis(500);

/// Bytes read from the Wayland primary selection before stopping. Enough for
/// [`MAX_SELECTION_CHARS`] of any script (UTF-8 is at most four bytes per
/// character), so `normalize` still makes the cut, and selecting a huge document
/// does not mean reading all of it into memory.
#[cfg(target_os = "linux")]
const WAYLAND_PRIMARY_MAX_BYTES: usize = MAX_SELECTION_CHARS * 4;

/// Read the Wayland primary selection through the compositor's data-control
/// protocol, which holds selected text with no copy action at all.
///
/// * `None`: this route is not available here (not a Wayland session, or the
///   compositor has no data-control or no primary selection), so the caller
///   tries the X11 route next, exactly as before this existed.
/// * `Some(Ok(text))`: the selection, empty when nothing is selected.
/// * `Some(Err(_))`: something is selected but could not be read.
///
/// This comes before the X11 read because on a Wayland desktop XWayland's
/// `PRIMARY` is not where a native app's selection lives. KWin only shows it to
/// X11 while an X11 window is active, so with the user in Kate or Dolphin,
/// `PRIMARY` had no owner: `xclip` failed, `xsel` printed nothing (read as
/// "nothing selected"), and the synthetic copy behind them sent XTest
/// keystrokes, which only reach X11 windows. The selection was never seen.
///
/// The connection here is our own, separate from GTK's, so this works the same
/// whether GTK is on Wayland or XWayland. The AppImage is always on XWayland,
/// because its GTK hook exports `GDK_BACKEND=x11`.
#[cfg(target_os = "linux")]
fn read_wayland_primary_selection() -> Option<Result<String, String>> {
    use wl_clipboard_rs::paste::{get_contents, ClipboardType, Error, MimeType, Seat};

    if !crate::utils::is_wayland() {
        return None;
    }

    match get_contents(ClipboardType::Primary, Seat::Unspecified, MimeType::Text) {
        Ok((pipe, _mime_type)) => {
            let deadline = Instant::now() + WAYLAND_PRIMARY_BUDGET;
            let result =
                read_pipe_until(pipe, deadline, WAYLAND_PRIMARY_MAX_BYTES).map(decode_selection);
            if result.is_ok() {
                debug!("read a selection from the Wayland primary selection");
            }
            Some(result)
        }
        // The compositor answered: nothing is selected, or the selection is not
        // text. Don't fall through to X11, which would only find the same.
        Err(Error::ClipboardEmpty | Error::NoMimeType) => Some(Ok(String::new())),
        // No data-control, no primary selection, or no compositor to ask.
        Err(e) => {
            debug!("Wayland primary selection not readable ({e}); trying X11");
            None
        }
    }
}

/// Read `pipe` to its end, giving up at `deadline` or once `limit` bytes are in.
///
/// A plain `read_to_end` waits for as long as the writing application takes, and
/// one that has stopped responding never finishes. So each read waits in `poll`
/// first, which is bounded by the deadline.
#[cfg(target_os = "linux")]
fn read_pipe_until(
    mut pipe: impl std::io::Read + std::os::fd::AsRawFd,
    deadline: Instant,
    limit: usize,
) -> Result<Vec<u8>, String> {
    use std::io::ErrorKind;

    let mut contents = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("the application holding the selection did not send it in time".into());
        }
        let mut fd = libc::pollfd {
            fd: pipe.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        // SAFETY: `fd` is one valid `pollfd` that lives across the call, and the
        // count passed is 1.
        let ready = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("could not wait for the selection: {err}"));
        }
        if ready == 0 {
            continue; // Timed out; the check at the top reports it.
        }
        // Readable, or the writer hung up: either way this read cannot block.
        match pipe.read(&mut chunk) {
            Ok(0) => return Ok(contents),
            Ok(n) => {
                contents.extend_from_slice(&chunk[..n]);
                if contents.len() >= limit {
                    return Ok(contents);
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("could not read the selection: {e}")),
        }
    }
}

/// Selection bytes as text. A read stopped at the byte limit can end partway
/// through a character, so an incomplete sequence at the very end is dropped
/// rather than turned into a replacement character.
#[cfg(target_os = "linux")]
fn decode_selection(mut bytes: Vec<u8>) -> String {
    if let Err(e) = std::str::from_utf8(&bytes) {
        if e.error_len().is_none() {
            bytes.truncate(e.valid_up_to());
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Read the X11 `PRIMARY` selection, which holds selected text with no copy
/// action at all. Returns `None` on Wayland, when no tool is present, or when
/// nothing is selected.
///
/// Note that GNOME/Wayland sessions relaunch this app under XWayland (see the
/// overlay's stacking requirements), so the most common Linux desktop reaches
/// this path too. On any Wayland desktop it only runs when the compositor's own
/// primary selection could not be read (`read_wayland_primary_selection`).
#[cfg(target_os = "linux")]
fn read_primary_selection() -> Option<String> {
    use std::process::Command;

    if crate::utils::is_wayland() && !std::env::var("DISPLAY").is_ok() {
        return None;
    }

    for (program, args) in [
        ("xclip", vec!["-o", "-selection", "primary"]),
        ("xsel", vec!["--primary", "--output"]),
    ] {
        let Ok(output) = Command::new(program).args(&args).output() else {
            continue; // Tool not installed.
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        if !text.trim().is_empty() {
            debug!("read a selection from the X11 PRIMARY selection via {program}");
            return Some(text);
        }
        // The tool ran and PRIMARY was empty: genuinely nothing selected. Don't
        // fall through to a synthetic copy, which would only find the same.
        return Some(String::new());
    }
    None
}

/// Small helper so the polling loop reads cleanly.
trait ClipboardText {
    fn clipboard_text(&self) -> Result<String, String>;
}

impl ClipboardText for AppHandle {
    fn clipboard_text(&self) -> Result<String, String> {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        self.clipboard()
            .read_text()
            .map_err(|e| format!("clipboard read failed: {e}"))
    }
}

// === Overlapping the capture with the recording ===========================
//
// The capture has to happen at recording *start*, while the user's selection and
// focus are still where they were when they pressed the shortcut. By the time the
// turn runs, they may have clicked elsewhere.
//
// It must not happen on the caller's thread, though. `Action::start` runs on the
// transcription coordinator's single thread, which also handles the release event
// and the debounce window; blocking it for a clipboard round trip delays the
// microphone opening (so the user's first words are lost) and delays hold-to-talk
// release handling. So the capture is spawned, and the turn collects it later.
//
// Spawning is safe because none of our own windows can take the foreground first:
// the overlay is built non-focusable, and on Windows it also carries
// `WS_EX_NOACTIVATE`.

/// The most recent capture, with the recording generation it belongs to.
///
/// A generation counter rather than a plain slot: a capture from an abandoned
/// recording must never surface in a later turn, and "the previous answer's
/// selection" appearing in front of a fresh question is exactly the kind of bug
/// that is impossible to reproduce on demand.
static PENDING: std::sync::Mutex<Option<PendingCapture>> = std::sync::Mutex::new(None);
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct PendingCapture {
    generation: u64,
    captured_at: Instant,
    result: Result<Option<CapturedSelection>, String>,
}

/// A capture parked for the turn that will consume it. Expires so an abandoned
/// recording cannot donate its selection to a much later question.
const PENDING_TTL: Duration = Duration::from_secs(120);

/// Begin capturing the selection for a new recording, off the calling thread.
///
/// Returns the generation to hand to [`take_selection`]. Call at recording start.
pub fn begin_capture(app: &AppHandle) -> u64 {
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    // Drop anything the previous recording left behind: it is answered for, or it
    // was abandoned, and either way it must not reach this turn.
    if let Ok(mut pending) = PENDING.lock() {
        *pending = None;
    }

    let app = app.clone();
    std::thread::Builder::new()
        .name("selection-capture".into())
        .spawn(move || {
            let result = capture_selection(&app);
            match &result {
                Ok(Some(selection)) => debug!(
                    "selection captured for generation {generation} ({} chars, {:?})",
                    selection.text.chars().count(),
                    selection.source
                ),
                Ok(None) => debug!("no selection for generation {generation}"),
                // Not an error the user needs to see: a turn without a selection
                // is an ordinary question. But it must be in the log, because
                // "why didn't it see my selection" is otherwise unanswerable.
                Err(e) => debug!("selection capture for generation {generation} failed: {e}"),
            }
            let chars = match &result {
                Ok(Some(selection)) => selection.text.chars().count(),
                _ => 0,
            };
            if let Ok(mut pending) = PENDING.lock() {
                *pending = Some(PendingCapture {
                    generation,
                    captured_at: Instant::now(),
                    result,
                });
            }
            // Say so while the user is still speaking, so the quick ask can show
            // that it has their selection before the answer arrives. A newer
            // recording has already moved on, so a stale capture stays quiet.
            if GENERATION.load(std::sync::atomic::Ordering::SeqCst) == generation {
                use tauri::Emitter;
                let _ = app.emit("assistant-selection-captured", chars);
            }
        })
        .map_err(|e| warn!("could not spawn the selection capture thread: {e}"))
        .ok();

    generation
}

/// Collect the selection captured for `generation`, if it is still relevant.
///
/// Answers `None` for every uninteresting case — no capture, a stale generation,
/// an expired capture, or a capture that failed — because all of them mean the
/// same thing to the turn: carry on without a selection.
pub fn take_selection(generation: u64) -> Option<CapturedSelection> {
    let mut guard = PENDING.lock().ok()?;
    let pending = guard.take()?;
    if pending.generation != generation {
        debug!(
            "discarding a selection from generation {} (wanted {generation})",
            pending.generation
        );
        return None;
    }
    if pending.captured_at.elapsed() > PENDING_TTL {
        debug!(
            "discarding a selection captured {:?} ago",
            pending.captured_at.elapsed()
        );
        return None;
    }
    pending.result.ok().flatten()
}

/// Forget any parked capture. Used when a recording is cancelled, so an abandoned
/// selection cannot appear in front of the next question.
pub fn clear_pending() {
    if let Ok(mut pending) = PENDING.lock() {
        *pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_whitespace_selections_are_not_selections() {
        assert_eq!(normalize(String::new()), None);
        assert_eq!(normalize("   \n\t ".to_string()), None);
    }

    #[test]
    fn a_selection_is_trimmed_but_otherwise_preserved() {
        assert_eq!(
            normalize("  hello world \n".to_string()),
            Some("hello world".to_string())
        );
        // Interior whitespace and newlines are content, not padding.
        assert_eq!(
            normalize("first line\n\nsecond line".to_string()),
            Some("first line\n\nsecond line".to_string())
        );
    }

    /// A whole document in the prompt buys nothing and can push a small local
    /// model past its context window.
    #[test]
    fn an_enormous_selection_is_capped_rather_than_refused() {
        let huge = "x".repeat(MAX_SELECTION_CHARS * 3);
        let capped = normalize(huge).expect("a long selection is still a selection");
        assert_eq!(capped.chars().count(), MAX_SELECTION_CHARS);
    }

    /// The cap counts characters, not bytes, so multi-byte text cannot be cut
    /// mid-character.
    #[test]
    fn the_cap_counts_characters_so_multibyte_text_is_not_split() {
        let huge = "日".repeat(MAX_SELECTION_CHARS * 2);
        let capped = normalize(huge).expect("a long selection is still a selection");
        assert_eq!(capped.chars().count(), MAX_SELECTION_CHARS);
        // Round-trips as valid UTF-8 with every character intact.
        assert!(capped.chars().all(|c| c == '日'));
    }

    /// The sentinel must be something no person would plausibly have copied, or
    /// a real clipboard could be mistaken for "the copy has not landed yet".
    #[test]
    fn the_sentinel_is_not_something_a_user_could_type() {
        assert!(SENTINEL.contains('\u{200B}'));
        assert!(normalize(SENTINEL.to_string()).is_some());
        assert_ne!(SENTINEL.trim(), "");
    }

    fn park(generation: u64, captured_at: Instant, text: &str) {
        *PENDING.lock().unwrap() = Some(PendingCapture {
            generation,
            captured_at,
            result: Ok(Some(CapturedSelection {
                text: text.to_string(),
                source: SelectionSource::SyntheticCopy,
            })),
        });
    }

    /// The happy path: a capture parked for this recording reaches its own turn.
    #[test]
    fn a_capture_reaches_the_turn_it_belongs_to() {
        park(7, Instant::now(), "hello");
        let taken = take_selection(7).expect("the matching generation collects it");
        assert_eq!(taken.text, "hello");
        // Collected exactly once: a second turn must not reuse it.
        assert!(take_selection(7).is_none());
    }

    /// The bug this guards against is the worst kind — the previous question's
    /// selection silently prefixed to a new, unrelated one.
    #[test]
    fn a_capture_from_another_recording_is_discarded() {
        park(3, Instant::now(), "text from an abandoned recording");
        assert!(
            take_selection(4).is_none(),
            "a newer turn must not inherit an older recording's selection"
        );
    }

    /// A recording abandoned at mute or cancel leaves a capture behind. It must
    /// not resurface in front of a question asked much later.
    #[test]
    fn a_stale_capture_expires_rather_than_waiting_forever() {
        let long_ago = Instant::now() - (PENDING_TTL + Duration::from_secs(1));
        park(9, long_ago, "yesterday's selection");
        assert!(take_selection(9).is_none());
    }

    /// Cancelling has to actually forget, not merely mark.
    #[test]
    fn clearing_discards_a_parked_capture() {
        park(11, Instant::now(), "cancelled");
        clear_pending();
        assert!(take_selection(11).is_none());
    }

    /// A failed capture and an absent one are the same thing to the turn: ask the
    /// question without a selection. Neither may be reported as empty text.
    #[test]
    fn a_failed_capture_yields_no_selection_rather_than_empty_text() {
        *PENDING.lock().unwrap() = Some(PendingCapture {
            generation: 5,
            captured_at: Instant::now(),
            result: Err("a modifier key is being held".to_string()),
        });
        assert!(take_selection(5).is_none());
    }
}
