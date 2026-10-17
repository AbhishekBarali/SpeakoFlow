//! Smart spacing: fit each dictation onto the text already in the field.
//!
//! Without it, two dictations into the same box run together ("hello" +
//! "there" pastes "hellothere"), because a paste knows nothing about what is
//! already there. With it on, SpeakoFlow looks at the few characters around the
//! caret of the field it is about to paste into, and the rules in [`rules`]
//! decide on a space before, a space after, the case of the first word, and a
//! stray period of its own from the previous dictation.
//!
//! # When the field is read
//!
//! Twice per dictation, both while the user is still talking or the speech is
//! still being transcribed, so a paste never waits on another app:
//!
//! * when recording starts ([`begin`]), before any window of ours is in front;
//! * when it stops ([`refresh`]), in case the caret moved while they spoke.
//!
//! The newest reading that learned something wins. At paste time ([`prepare`])
//! the result is taken, waiting at most [`PASTE_WAIT`] for a read still in
//! flight. Reading at paste time instead is what made OpenWhispr drop its
//! version: an Accessibility round trip on the paste path is felt on every
//! dictation.
//!
//! # When the field cannot be read
//!
//! Some apps hide their text from every outside tool. There, the last-paste
//! memory takes over: if SpeakoFlow pasted into the same window or app a moment
//! ago, it knows what it left there and adds the space a continuation needs.
//! It never changes case or removes anything on that basis. With neither, the
//! dictation is pasted exactly as before this feature existed.
//!
//! # What is never done
//!
//! Password fields are never read. Terminals are never read (their text is the
//! shell's). No text from another app is ever logged or stored beyond the one
//! decision it is read for: the log carries kinds and lengths, the memory keeps
//! only the tail of SpeakoFlow's own last paste.

mod reading;
mod rules;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use reading::{Reading, Snapshot};
pub use rules::Fitted;

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::{debug, info};
use tauri::AppHandle;

use crate::settings::get_settings;

/// How long one read may take before it is given up as unknown. It runs while
/// the user talks, so this bounds wasted work, not felt latency. Leaves room
/// for the one retry of an app that was asleep (`windows::RETRY_AFTER`).
const READ_BUDGET: Duration = Duration::from_millis(500);

/// How long a paste waits for a read still in flight before using the earlier
/// one (or the memory). Felt by the user only when an app is slow to answer.
const PASTE_WAIT: Duration = Duration::from_millis(100);

/// How long SpeakoFlow's own last paste is trusted as "the text before the
/// caret" for a field that cannot be read.
const REMEMBER_FOR: Duration = Duration::from_secs(45);

/// How long after a paste its stray period may still be removed. The field is
/// read again either way; this only bounds how stale "ours" may be.
const JOIN_FOR: Duration = Duration::from_secs(90);

/// A reading older than this is not used: a dictation recovered and pasted
/// much later may no longer be aimed at the same caret.
const READING_FRESH_FOR: Duration = Duration::from_secs(120);

/// How much of the last paste is kept. Enough to recognise it at the end of the
/// field and to know how it ended; nothing more of the user's text is kept.
const REMEMBERED_CHARS: usize = 48;

struct LastPaste {
    target: String,
    text: String,
    at: Instant,
}

struct State {
    /// Bumped by every dictation, so a late read from an earlier one is ignored.
    generation: u64,
    /// Reads started for the current generation and not yet stored.
    pending: u32,
    /// Sequence of the read that produced `snapshot`, so a slower earlier read
    /// cannot overwrite a newer one.
    snapshot_seq: u64,
    next_seq: u64,
    snapshot: Option<Snapshot>,
    /// When `snapshot` was stored.
    snapshot_at: Option<Instant>,
    /// Where the dictation being pasted was read from, for [`record_paste`].
    pasting_into: Option<String>,
    last_paste: Option<LastPaste>,
}

static STATE: Mutex<State> = Mutex::new(State {
    generation: 0,
    pending: 0,
    snapshot_seq: 0,
    next_seq: 0,
    snapshot: None,
    snapshot_at: None,
    pasting_into: None,
    last_paste: None,
});
static SETTLED: Condvar = Condvar::new();

fn state() -> MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A dictation started: read the field it is aimed at, in the background.
///
/// Called at recording start, after the paste target is remembered and before
/// any window of ours can take the foreground. Returns immediately.
pub fn begin(app: &AppHandle) {
    let enabled = get_settings(app).smart_spacing;
    let (generation, seq) = {
        let mut state = state();
        state.generation += 1;
        state.pending = 0;
        state.snapshot = None;
        state.snapshot_at = None;
        state.snapshot_seq = 0;
        state.pasting_into = None;
        if !enabled {
            return;
        }
        state.pending += 1;
        state.next_seq += 1;
        (state.generation, state.next_seq)
    };
    spawn_read(generation, seq);
}

/// The recording stopped: read the field again, in case the caret moved while
/// the user spoke. Returns immediately.
pub fn refresh(app: &AppHandle) {
    if !get_settings(app).smart_spacing {
        return;
    }
    let (generation, seq) = {
        let mut state = state();
        if state.generation == 0 {
            return;
        }
        state.pending += 1;
        state.next_seq += 1;
        (state.generation, state.next_seq)
    };
    spawn_read(generation, seq);
}

fn spawn_read(generation: u64, seq: u64) {
    let spawned = std::thread::Builder::new()
        .name("smart-spacing-read".into())
        .spawn(move || {
            let started = Instant::now();
            let snapshot = platform_read();
            debug!(
                "Smart spacing: read the field as {} in {} ms",
                snapshot.reading.kind(),
                started.elapsed().as_millis()
            );
            store(generation, seq, snapshot);
        });
    if spawned.is_err() {
        store(generation, seq, Snapshot::unknown(None));
    }
}

fn store(generation: u64, seq: u64, snapshot: Snapshot) {
    let mut state = state();
    if generation != state.generation {
        return;
    }
    state.pending = state.pending.saturating_sub(1);
    let replace = match &state.snapshot {
        None => true,
        Some(current) => {
            if snapshot.reading.is_informative() {
                seq > state.snapshot_seq || !current.reading.is_informative()
            } else {
                // Learned nothing: keep whatever the other read found.
                false
            }
        }
    };
    if replace {
        state.snapshot = Some(snapshot);
        state.snapshot_seq = seq;
        state.snapshot_at = Some(Instant::now());
    }
    SETTLED.notify_all();
}

fn platform_read() -> Snapshot {
    #[cfg(target_os = "windows")]
    {
        windows::read(READ_BUDGET)
    }
    #[cfg(target_os = "macos")]
    {
        macos::read(READ_BUDGET)
    }
    #[cfg(target_os = "linux")]
    {
        linux::read(READ_BUDGET)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Snapshot::unknown(None)
    }
}

/// Fit a finished dictation onto the field, or `None` to paste it as before.
///
/// `can_erase` says whether the paste method can send the Backspace that
/// removes SpeakoFlow's own stray period; without it that rule is skipped.
/// Consumes this dictation's reading: a second call (another paste of other
/// text) gets `None`.
pub fn prepare(app: &AppHandle, transcript: &str, can_erase: bool) -> Option<Fitted> {
    let settings = get_settings(app);
    if !settings.smart_spacing {
        return None;
    }

    let mut state = state();
    let deadline = Instant::now() + PASTE_WAIT;
    while state.pending > 0 {
        let now = Instant::now();
        if now >= deadline {
            debug!("Smart spacing: a read was still running at paste time; using the earlier one");
            break;
        }
        state = SETTLED
            .wait_timeout(state, deadline - now)
            .map(|(guard, _)| guard)
            .unwrap_or_else(|poisoned| poisoned.into_inner().0);
    }
    let snapshot = state.snapshot.take()?;
    let stored_at = state.snapshot_at.take();
    // Any read still running belongs to this dictation and is no longer wanted.
    state.generation += 1;
    state.pending = 0;
    if stored_at.map_or(true, |at| at.elapsed() > READING_FRESH_FOR) {
        debug!("Smart spacing: the field was read too long ago; pasting as is");
        return None;
    }

    let now = Instant::now();
    let remembered = state
        .last_paste
        .as_ref()
        .filter(|last| snapshot.target.as_deref() == Some(last.target.as_str()))
        .map(|last| (last.text.clone(), now.duration_since(last.at)));

    let previous_is_ours = match (&snapshot.reading, &remembered) {
        (Reading::Text { before, .. }, Some((last, age))) => {
            *age <= JOIN_FOR && rules::ends_with_pasted(before, last)
        }
        _ => false,
    };

    let context = match &snapshot.reading {
        Reading::Text {
            before,
            after,
            selected,
        } => rules::Context::Field {
            before,
            after,
            selected,
        },
        Reading::Empty => rules::Context::EmptyField,
        Reading::Unknown | Reading::Terminal => match &remembered {
            Some((last, age)) if *age <= REMEMBER_FOR => rules::Context::Remembered { last },
            _ => rules::Context::Nothing,
        },
        Reading::Secret => rules::Context::Nothing,
    };

    let options = rules::Options {
        fix_case: settings.smart_spacing_fix_case,
        join_sentences: settings.smart_spacing_join_sentences && can_erase,
        previous_is_ours,
        language: &settings.selected_language,
        dictionary: &settings.custom_words,
    };
    let mut fitted = rules::fit(transcript, context, &options);
    fitted.type_leading_space = fitted.decision.leading_space && snapshot.trims_pasted_spaces;

    let source = match context {
        rules::Context::Field { .. } => "field",
        rules::Context::EmptyField => "empty field",
        rules::Context::Remembered { .. } => "last paste",
        rules::Context::Nothing => "nothing",
    };
    let d = fitted.decision;
    info!(
        "Smart spacing: from {} ({}): space before {}{}, space after {}, lowercased {}, \
         selection case {}, final mark dropped {}, own period removed {}",
        source,
        snapshot.reading.kind(),
        d.leading_space,
        if fitted.type_leading_space {
            " (typed)"
        } else {
            ""
        },
        d.trailing_space,
        d.lowercased,
        d.matched_selection_case,
        d.dropped_final_mark,
        d.erased_own_period
    );

    state.pasting_into = match snapshot.reading {
        Reading::Secret => None,
        _ => snapshot.target,
    };
    Some(fitted)
}

/// Whether the window in front is still the one this dictation's field was
/// read from. The Backspace that removes SpeakoFlow's own stray period goes
/// wherever focus is now, and the paste respects a switch to another app made
/// while the speech was transcribed (`input::restore_paste_target`), so after
/// such a switch it would delete a character of the user's own text there.
/// Where the window in front cannot be named, the reading is trusted as before.
pub fn still_aimed_at_reading() -> bool {
    let Some(read_from) = state().pasting_into.clone() else {
        return false;
    };
    match current_target() {
        Some(now) => now == read_from,
        None => true,
    }
}

fn current_target() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        windows::current_target()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// The fitted dictation landed. Remember its tail for the next one.
///
/// `submitted` is true when SpeakoFlow also pressed Enter after it: the field
/// was sent and is empty now, so there is nothing to continue.
pub fn record_paste(pasted: &str, submitted: bool) {
    let mut state = state();
    let target = state.pasting_into.take();
    state.last_paste = match target {
        Some(target) if !submitted && !pasted.trim().is_empty() => Some(LastPaste {
            target,
            text: reading::tail_chars(pasted, REMEMBERED_CHARS),
            at: Instant::now(),
        }),
        _ => None,
    };
}

/// Forget the last paste, for a dictation that went anywhere unusual.
pub fn forget_last_paste() {
    let mut state = state();
    state.last_paste = None;
    state.pasting_into = None;
}
