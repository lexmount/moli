use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use parking_lot::Mutex;

#[derive(Debug, Default)]
struct ResponseProgress {
    declared_body_complete: AtomicBool,
    terminal: AtomicBool,
}

#[derive(Debug, Default)]
struct FetchLifecycleState {
    cancel_requested: AtomicBool,
    parent: Option<Arc<FetchLifecycleState>>,
    response_progress: Mutex<Arc<ResponseProgress>>,
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
        let mut state = self.state.as_ref();
        loop {
            if state.cancel_requested.load(Ordering::SeqCst) {
                return true;
            }
            match state.parent.as_deref() {
                Some(parent) => state = parent,
                None => return false,
            }
        }
    }

    /// Creates cancellation authority for one preflight or redirect hop.
    /// Parent cancellation propagates to this child; cancelling the child does
    /// not cancel its parent or siblings. Completion facts remain independent.
    pub fn child_for_subrequest(&self) -> Self {
        Self {
            state: Arc::new(FetchLifecycleState {
                parent: Some(self.state.clone()),
                ..FetchLifecycleState::default()
            }),
        }
    }

    /// Selects the accepted final response's transport progress. This shares
    /// already recorded and future completion facts, without retaining the
    /// child's cancellation state (which itself retains the parent).
    pub fn adopt_response_progress(&self, response: &Self) {
        let progress = response.state.response_progress.lock().clone();
        *self.state.response_progress.lock() = progress;
    }

    /// Returns whether transport facts already determine this response's
    /// terminal result, so a later consumer cancellation must not replace it.
    pub fn response_completion_is_committed(&self) -> bool {
        let progress = self.state.response_progress.lock();
        progress.declared_body_complete.load(Ordering::Acquire)
            || progress.terminal.load(Ordering::Acquire)
    }

    /// Starts a new logical transfer without clearing cancellation. Detaching
    /// the previous progress also prevents an old hop's late completion from
    /// committing this new transfer.
    pub fn reset_response_progress(&self) {
        *self.state.response_progress.lock() = Arc::default();
    }

    pub(crate) fn mark_declared_response_body_complete(&self) {
        self.state
            .response_progress
            .lock()
            .declared_body_complete
            .store(true, Ordering::Release);
    }

    pub(crate) fn mark_response_terminal(&self) {
        self.state
            .response_progress
            .lock()
            .terminal
            .store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::FetchCancelHandle;

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

    #[test]
    fn subrequest_cancellation_is_isolated_and_inherits_parent_abort() {
        let parent = FetchCancelHandle::new();
        let first = parent.child_for_subrequest();
        let second = parent.child_for_subrequest();
        first.cancel();
        assert!(first.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!second.is_cancelled());
        parent.cancel();
        assert!(second.is_cancelled());
        assert!(second.child_for_subrequest().is_cancelled());
    }

    #[test]
    fn only_selected_response_progress_commits_the_parent() {
        for complete_before_selection in [false, true] {
            for declared_body in [false, true] {
                let parent = FetchCancelHandle::new();
                let discarded = parent.child_for_subrequest();
                discarded.mark_response_terminal();
                assert!(!parent.response_completion_is_committed());
                let response = parent.child_for_subrequest();
                let complete = || {
                    if declared_body {
                        response.mark_declared_response_body_complete();
                    } else {
                        response.mark_response_terminal();
                    }
                };
                if complete_before_selection {
                    complete();
                }
                parent.adopt_response_progress(&response);
                assert_eq!(
                    parent.response_completion_is_committed(),
                    complete_before_selection
                );
                complete();
                assert!(parent.response_completion_is_committed());
                parent.reset_response_progress();
                response.mark_response_terminal();
                assert!(!parent.response_completion_is_committed());
                assert!(!parent.is_cancelled());
            }
        }
    }
}
