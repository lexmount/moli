use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Default)]
struct FetchLifecycleState {
    parent: Option<Arc<FetchLifecycleState>>,
    cancel_requested: AtomicBool,
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
        self.state.cancel_requested.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        let mut state = Some(self.state.as_ref());
        while let Some(current) = state {
            if current.cancel_requested.load(Ordering::SeqCst) {
                return true;
            }
            state = current.parent.as_deref();
        }
        false
    }

    /// A physical attempt can be discarded without cancelling its request
    /// admission. Admission cancellation still reaches every live attempt.
    pub fn child(&self) -> Self {
        Self {
            state: Arc::new(FetchLifecycleState {
                parent: Some(self.state.clone()),
                ..Default::default()
            }),
        }
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
    use super::FetchCancelHandle;

    #[test]
    fn cancelling_an_attempt_preserves_admission_and_later_attempts() {
        let admission = FetchCancelHandle::new();
        let challenge = admission.child();
        challenge.cancel();
        assert!(challenge.is_cancelled());
        assert!(!admission.is_cancelled());
        let retry = admission.child();
        let descendant = retry.child();
        assert!(!retry.is_cancelled());
        admission.cancel();
        assert!(retry.is_cancelled());
        assert!(descendant.is_cancelled());
        assert!(admission.child().is_cancelled());
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
    }
}
