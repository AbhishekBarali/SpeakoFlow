use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

/// One overlay lifetime. A newer show cancels old waits; hover parks them without
/// polling. Shared by the native window and its completed-transcript card.
pub(crate) struct OverlayLifecycle {
    epoch: AtomicU64,
    hovered: AtomicBool,
    wake: Notify,
}

impl OverlayLifecycle {
    pub const fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            hovered: AtomicBool::new(false),
            wake: Notify::const_new(),
        }
    }
    pub fn advance(&self) -> u64 {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.wake.notify_waiters();
        epoch
    }
    pub fn current(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
    pub fn is_current(&self, epoch: u64) -> bool {
        self.current() == epoch
    }
    pub fn set_hovered(&self, hovered: bool) {
        if self.hovered.swap(hovered, Ordering::SeqCst) != hovered {
            self.wake.notify_waiters();
        }
    }
    /// Returns true once the current card may be hidden; false when superseded.
    /// The callback starts or reverses the fade without dropping the transcript.
    pub async fn wait_for_dismissal(
        &self,
        epoch: u64,
        linger: Duration,
        fade: Duration,
        on_fade: impl Fn(bool),
    ) -> bool {
        'linger: loop {
            let mut deadline = Instant::now() + linger;
            loop {
                let wake = self.wake.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                if !self.is_current(epoch) {
                    return false;
                }
                if self.hovered.load(Ordering::SeqCst) {
                    wake.await;
                    deadline = Instant::now() + linger;
                    continue;
                }
                tokio::select! {
                    _ = wake => { deadline = Instant::now() + linger; }
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
            if !self.is_current(epoch) {
                return false;
            }
            on_fade(true);
            let deadline = Instant::now() + fade;
            loop {
                let wake = self.wake.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                if !self.is_current(epoch) {
                    return false;
                }
                if self.hovered.load(Ordering::SeqCst) {
                    on_fade(false);
                    continue 'linger;
                }
                tokio::select! {
                    _ = wake => {},
                    _ = tokio::time::sleep_until(deadline) => return self.is_current(epoch),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    const LINGER: Duration = Duration::from_millis(40);
    const FADE: Duration = Duration::from_millis(60);

    fn start(
        state: Arc<OverlayLifecycle>,
    ) -> (tokio::task::JoinHandle<bool>, mpsc::UnboundedReceiver<bool>) {
        let epoch = state.advance();
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            state
                .wait_for_dismissal(epoch, LINGER, FADE, |fading| {
                    let _ = tx.send(fading);
                })
                .await
        });
        (task, rx)
    }
    #[tokio::test]
    async fn finished_text_lingers_then_fades() {
        let state = Arc::new(OverlayLifecycle::new());
        let began = Instant::now();
        let (task, mut events) = start(state);
        assert_eq!(events.recv().await, Some(true));
        assert!(began.elapsed() >= LINGER);
        assert!(task.await.unwrap());
        assert!(began.elapsed() >= LINGER + FADE);
    }
    #[tokio::test]
    async fn hover_holds_until_leave_and_gives_a_fresh_reading_delay() {
        let state = Arc::new(OverlayLifecycle::new());
        state.set_hovered(true);
        let (task, mut events) = start(state.clone());
        tokio::time::sleep(LINGER + FADE + LINGER).await;
        assert!(events.try_recv().is_err());
        assert!(!task.is_finished());
        let left = Instant::now();
        state.set_hovered(false);
        assert_eq!(events.recv().await, Some(true));
        assert!(left.elapsed() >= LINGER);
        assert!(task.await.unwrap());
    }
    #[tokio::test]
    async fn entering_during_fade_restores_the_card() {
        let state = Arc::new(OverlayLifecycle::new());
        let (task, mut events) = start(state.clone());
        assert_eq!(events.recv().await, Some(true));
        state.set_hovered(true);
        assert_eq!(events.recv().await, Some(false));
        tokio::time::sleep(LINGER + FADE).await;
        assert!(!task.is_finished());
        state.set_hovered(false);
        assert_eq!(events.recv().await, Some(true));
        assert!(task.await.unwrap());
    }
    #[tokio::test]
    async fn new_recording_wakes_and_cancels_an_old_hover_hold() {
        let state = Arc::new(OverlayLifecycle::new());
        state.set_hovered(true);
        let (task, mut events) = start(state.clone());
        tokio::task::yield_now().await;
        state.advance();
        assert!(!tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap());
        assert_eq!(events.recv().await, None);
    }
}
