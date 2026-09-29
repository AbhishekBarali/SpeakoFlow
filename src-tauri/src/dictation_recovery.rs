//! Getting a dictation back after it was dismissed or failed.
//!
//! Cancelling used to be final. Esc during a recording threw the audio away,
//! and Esc (or the tray's Cancel) later in the pipeline dropped whatever had
//! already been transcribed, so a slip of the finger cost everything the user
//! had just said. A failed transcription kept its audio in History but said
//! nothing about it where the user was looking.
//!
//! Now both leave an **offer** behind: what is left to do to finish that
//! dictation, held in memory while the recording pill shows a quiet "Undo" or
//! "Try again". Taking the offer runs the rest of the dictation from where it
//! stopped (transcribe, clean up, paste). The pill and the offer live and die
//! together: the offer is keyed by the overlay epoch that showed it, and it is
//! withdrawn when that pill lingers out or anything else replaces it. After
//! that, History is the way back, because every dismissed dictation also keeps
//! its row, marked dismissed, with its audio.
//!
//! This module is the bookkeeping. Running an offer needs the pipeline's
//! internals and lives in `actions.rs` (`start_recovery`); showing one is
//! `overlay::show_recovery_overlay`.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::watch;

const SAMPLE_RATE: usize = 16_000;

/// Less speech than this is not worth offering back. The recorder keeps only
/// what the VAD called speech, so this is half a second of *voice*: below it
/// the recording is almost always a hotkey pressed by mistake and cancelled on
/// purpose, and an Undo pill plus a History row for every one of those would
/// be noise.
pub const MIN_RECOVERABLE_SAMPLES: usize = SAMPLE_RATE / 2;

/// Whether a cancelled recording of `samples` is kept for recovery.
pub fn worth_keeping(samples: usize) -> bool {
    samples >= MIN_RECOVERABLE_SAMPLES
}

/// Why the pill is offering the dictation back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferKind {
    /// The user cancelled it. The pill says so and offers Undo.
    Dismissed,
    /// Transcription or generation failed. The pill offers Try again.
    Failed,
}

impl OfferKind {
    /// The overlay state that presents this kind of offer.
    pub fn overlay_state(self) -> &'static str {
        match self {
            OfferKind::Dismissed => "dismissed",
            OfferKind::Failed => "failed",
        }
    }
}

/// What a dictation needs to be finished the way it would have been.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DictationContext {
    /// AI cleanup runs on it.
    pub post_process: bool,
    /// It came from the dictation shortcut, so "Hey Flow" applies.
    pub flow_eligible: bool,
}

/// Finished text that was never pasted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPaste {
    pub text: String,
    /// The overlay notice the paste would have shown (e.g. cleanup fell back).
    pub notice: Option<&'static str>,
    /// A Flow result, which pastes without a trailing space or auto-submit.
    pub flow: bool,
}

/// How far the dictation got, which is where recovering it resumes.
pub enum Remaining {
    /// Nothing was transcribed: the audio, padded, 16 kHz mono.
    Transcribe(Arc<Vec<f32>>),
    /// Transcribed, but not cleaned up, generated, or pasted.
    Deliver(String),
    /// Everything but the paste.
    Paste(PendingPaste),
}

impl Remaining {
    /// Audio still to transcribe, which sizes the pipeline's stall budget.
    pub fn audio(&self) -> Duration {
        match self {
            Remaining::Transcribe(samples) => {
                Duration::from_secs_f64(samples.len() as f64 / SAMPLE_RATE as f64)
            }
            Remaining::Deliver(_) | Remaining::Paste(_) => Duration::ZERO,
        }
    }
}

/// A dictation the pill can hand back.
pub struct Offer {
    pub kind: OfferKind,
    /// Replaces the pill's generic label when set (an `overlay.notices.*` key).
    pub notice: Option<&'static str>,
    pub remaining: Remaining,
    pub context: DictationContext,
    /// Its History row, which may still be being written.
    pub row: RowHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowState {
    Pending,
    Ready(Option<i64>),
}

/// The History row an offer belongs to.
///
/// A dictation cancelled mid-recording still has its WAV to write and its row
/// to insert when the pill appears, and that work runs off the thread that
/// cancelled it. So the offer is shown at once and learns its row when the
/// write finishes; recovering it waits for the row (briefly) so the result
/// lands in that row instead of a second one.
pub struct RowHandle(watch::Receiver<RowState>);

/// The writing side of a pending [`RowHandle`]. Dropping it unsent means "no
/// row" (the write failed or panicked).
pub struct RowSender(watch::Sender<RowState>);

impl RowSender {
    pub fn send(self, id: Option<i64>) {
        let _ = self.0.send(RowState::Ready(id));
    }
}

impl RowHandle {
    /// A row that is already known (or known not to exist).
    pub fn ready(id: Option<i64>) -> Self {
        let (_sender, receiver) = watch::channel(RowState::Ready(id));
        Self(receiver)
    }

    /// A row still being written.
    pub fn pending() -> (RowSender, Self) {
        let (sender, receiver) = watch::channel(RowState::Pending);
        (RowSender(sender), Self(receiver))
    }

    /// The row, if it has been written.
    pub fn known(&self) -> Option<i64> {
        match *self.0.borrow() {
            RowState::Ready(id) => id,
            RowState::Pending => None,
        }
    }

    /// Wait up to `within` for the row. `None` if there is none, or it never
    /// arrived: the dictation is then recovered without writing History.
    pub async fn resolve(mut self, within: Duration) -> Option<i64> {
        let wait = async move {
            loop {
                let state = *self.0.borrow_and_update();
                if let RowState::Ready(id) = state {
                    return id;
                }
                if self.0.changed().await.is_err() {
                    let state = *self.0.borrow();
                    return match state {
                        RowState::Ready(id) => id,
                        RowState::Pending => None,
                    };
                }
            }
        };
        tokio::time::timeout(within, wait).await.unwrap_or(None)
    }
}

/// The one offer on screen, keyed by the overlay epoch that shows it. There is
/// only ever one pill, so there is only ever one offer: a newer one replaces
/// the old, whose dictation is still in History.
pub struct OfferSlot {
    current: Mutex<Option<(u64, Offer)>>,
}

impl OfferSlot {
    pub const fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(u64, Offer)>> {
        // A poisoned lock only means a panic elsewhere while holding it; the
        // slot itself is always in a valid state.
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn store(&self, epoch: u64, offer: Offer) {
        *self.lock() = Some((epoch, offer));
    }

    /// Take the offer shown at `epoch`. `None` once that pill is gone.
    pub fn take(&self, epoch: u64) -> Option<Offer> {
        let mut current = self.lock();
        if current.as_ref().is_some_and(|(shown, _)| *shown == epoch) {
            current.take().map(|(_, offer)| offer)
        } else {
            None
        }
    }

    /// The pill shown at `epoch` is gone; let its offer go (and its audio).
    /// Returns whether it was still there.
    pub fn withdraw(&self, epoch: u64) -> bool {
        let mut current = self.lock();
        if current.as_ref().is_some_and(|(shown, _)| *shown == epoch) {
            *current = None;
            true
        } else {
            false
        }
    }

    /// The dictation in row `id` is being recovered from History, so the pill
    /// must not also offer it.
    pub fn withdraw_row(&self, id: i64) {
        let mut current = self.lock();
        if current
            .as_ref()
            .is_some_and(|(_, offer)| offer.row.known() == Some(id))
        {
            *current = None;
        }
    }
}

static OFFERS: OfferSlot = OfferSlot::new();

/// Put `offer` on the recording pill. Returns false when it cannot be shown
/// (the overlay is turned off, or on a platform where a clickable overlay
/// could take keyboard focus from the app being dictated into); the offer is
/// then dropped and the caller falls back to its old way of finishing, while
/// History still has the dictation.
pub fn present(app: &AppHandle, offer: Offer) -> bool {
    let state = offer.kind.overlay_state();
    let notice = offer.notice;
    crate::overlay::show_recovery_overlay(app, state, notice, move |epoch| {
        OFFERS.store(epoch, offer)
    })
}

pub fn take(epoch: u64) -> Option<Offer> {
    OFFERS.take(epoch)
}

/// The pill at `epoch` was replaced by something else.
pub fn withdraw(epoch: u64) {
    OFFERS.withdraw(epoch);
}

/// The pill at `epoch` lingered out untouched. Its dictation stays in History;
/// the window it would have pasted into stops being a paste target, which is
/// the rule a cancel follows when it offers nothing (see
/// `utils::cancel_current_operation`).
pub fn expire(epoch: u64) {
    if OFFERS.withdraw(epoch) {
        crate::input::forget_paste_target();
    }
}

pub fn withdraw_row(id: i64) {
    OFFERS.withdraw_row(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(row: RowHandle) -> Offer {
        Offer {
            kind: OfferKind::Dismissed,
            notice: None,
            remaining: Remaining::Deliver("hello".to_string()),
            context: DictationContext {
                post_process: false,
                flow_eligible: true,
            },
            row,
        }
    }

    #[test]
    fn a_stray_press_is_not_worth_an_undo() {
        assert!(!worth_keeping(0));
        assert!(!worth_keeping(MIN_RECOVERABLE_SAMPLES - 1));
        assert!(worth_keeping(MIN_RECOVERABLE_SAMPLES));
        assert!(worth_keeping(SAMPLE_RATE * 60));
    }

    #[test]
    fn an_offer_is_taken_once_and_only_by_the_pill_that_showed_it() {
        let slot = OfferSlot::new();
        slot.store(4, offer(RowHandle::ready(Some(1))));
        assert!(slot.take(3).is_none(), "an older pill's click");
        assert!(slot.take(4).is_some());
        assert!(slot.take(4).is_none(), "a double click");
    }

    #[test]
    fn a_newer_offer_replaces_the_old_and_withdrawing_the_old_keeps_it() {
        let slot = OfferSlot::new();
        slot.store(1, offer(RowHandle::ready(None)));
        slot.store(2, offer(RowHandle::ready(None)));
        // The first pill's linger ending must not take the second offer.
        slot.withdraw(1);
        assert!(slot.take(1).is_none());
        assert!(slot.take(2).is_some());
    }

    #[test]
    fn recovering_from_history_withdraws_only_that_rows_offer() {
        let slot = OfferSlot::new();
        slot.store(7, offer(RowHandle::ready(Some(10))));
        slot.withdraw_row(11);
        assert!(slot.take(7).is_some());

        slot.store(8, offer(RowHandle::ready(Some(10))));
        slot.withdraw_row(10);
        assert!(slot.take(8).is_none());
    }

    #[test]
    fn a_row_still_being_written_is_not_withdrawn_by_id() {
        let slot = OfferSlot::new();
        let (_sender, row) = RowHandle::pending();
        slot.store(1, offer(row));
        slot.withdraw_row(10);
        assert!(slot.take(1).is_some());
    }

    #[test]
    fn only_audio_counts_toward_the_stall_budget() {
        let audio = Remaining::Transcribe(Arc::new(vec![0.0; SAMPLE_RATE * 3]));
        assert_eq!(audio.audio(), Duration::from_secs(3));
        assert_eq!(Remaining::Deliver("x".into()).audio(), Duration::ZERO);
    }

    #[tokio::test]
    async fn a_pending_row_resolves_when_written() {
        let (sender, row) = RowHandle::pending();
        assert_eq!(row.known(), None);
        let waiter = tokio::spawn(row.resolve(Duration::from_secs(5)));
        tokio::task::yield_now().await;
        sender.send(Some(42));
        assert_eq!(waiter.await.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn a_row_whose_write_failed_resolves_to_none() {
        let (sender, row) = RowHandle::pending();
        drop(sender);
        assert_eq!(row.resolve(Duration::from_secs(5)).await, None);
    }

    #[tokio::test]
    async fn a_ready_row_resolves_immediately_and_a_stuck_one_times_out() {
        assert_eq!(
            RowHandle::ready(Some(3))
                .resolve(Duration::from_millis(1))
                .await,
            Some(3)
        );
        let (_sender, stuck) = RowHandle::pending();
        assert_eq!(stuck.resolve(Duration::from_millis(20)).await, None);
    }
}
