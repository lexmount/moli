use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use thin_cancellation_token::CancellationToken;

#[derive(Debug, Default)]
struct FetchLifecycleState {
    cancellation: CancellationToken,
    declared_body_complete: AtomicBool,
    terminal: AtomicBool,
}

#[derive(Debug, Clone, Default)]
pub struct FetchCancelHandle {
    state: Arc<FetchLifecycleState>,
}

impl FetchCancelHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.state.cancellation.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancellation.is_cancelled()
    }

    /// Waits for this consumer's cancellation, including an earlier cancel.
    /// Multiple observers are woken without polling the transport.
    pub async fn cancelled(&self) {
        self.state.cancellation.cancelled().await;
    }

    /// Returns whether transport facts already determine this response's
    /// terminal result, so a later consumer cancellation must not replace it.
    pub fn response_completion_is_committed(&self) -> bool {
        self.state.declared_body_complete.load(Ordering::Acquire)
            || self.state.terminal.load(Ordering::Acquire)
    }

    pub(crate) fn reset_response_progress(&self) {
        self.state
            .declared_body_complete
            .store(false, Ordering::Release);
        self.state.terminal.store(false, Ordering::Release);
    }

    pub(crate) fn mark_declared_response_body_complete(&self) {
        self.state
            .declared_body_complete
            .store(true, Ordering::Release);
    }

    pub(crate) fn mark_response_terminal(&self) {
        self.state.terminal.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Wake, Waker},
    };

    use super::FetchCancelHandle;

    #[tokio::test]
    async fn cancellation_wakes_all_observers_and_remembers_earlier_cancel() {
        #[derive(Default)]
        struct WakeCount(AtomicUsize);
        impl Wake for WakeCount {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let handle = FetchCancelHandle::new();
        let first = handle.cancelled();
        let second = handle.cancelled();
        tokio::pin!(first, second);
        let wakes = Arc::new(WakeCount::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert!(second.as_mut().poll(&mut cx).is_pending());
        handle.clone().cancel();
        assert_eq!(wakes.0.load(Ordering::Relaxed), 2);
        assert!(first.as_mut().poll(&mut cx).is_ready());
        assert!(second.as_mut().poll(&mut cx).is_ready());
        assert!(std::pin::pin!(handle.cancelled()).poll(&mut cx).is_ready());
    }

    #[test]
    fn response_completion_commit_is_shared_and_resettable() {
        let handle = FetchCancelHandle::new();
        let observer = handle.clone();
        assert!(!observer.response_completion_is_committed());

        handle.mark_declared_response_body_complete();
        assert!(observer.response_completion_is_committed());

        handle.reset_response_progress();
        assert!(!observer.response_completion_is_committed());

        handle.mark_response_terminal();
        assert!(observer.response_completion_is_committed());

        handle.cancel();
        assert!(observer.is_cancelled());
        assert!(observer.response_completion_is_committed());

        handle.reset_response_progress();
        assert!(!observer.response_completion_is_committed());
        assert!(observer.is_cancelled());
    }
}
