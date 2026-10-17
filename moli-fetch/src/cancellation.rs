use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use parking_lot::Mutex;

#[derive(Debug, Default)]
struct ResponseProgress {
    declared_body_complete: AtomicBool,
    terminal: AtomicBool,
    timing: Mutex<FetchResponseTiming>,
}

/// Monotonic transport observations for the selected response. Reading these
/// facts later does not turn consumer scheduling delays into network time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FetchResponseTiming {
    request_started_at: Option<Instant>,
    first_interim_response_started_at: Option<Instant>,
    response_started_at: Option<Instant>,
    response_end_at: Option<Instant>,
    body_bytes_received: bool,
}

pub(crate) struct ResponseTimingRecorder {
    progress: Arc<ResponseProgress>,
}

impl ResponseTimingRecorder {
    pub(crate) fn finish(&self) {
        self.progress
            .timing
            .lock()
            .response_end_at
            .get_or_insert_with(Instant::now);
    }
}

impl FetchResponseTiming {
    pub fn body_bytes_received(self) -> bool {
        self.body_bytes_received
    }

    pub fn request_started_at(self) -> Option<Instant> {
        self.request_started_at
    }

    pub fn first_interim_response_started_at(self) -> Option<Instant> {
        self.first_interim_response_started_at
    }

    pub fn response_started_at(self) -> Option<Instant> {
        self.response_started_at
    }

    pub fn response_end_at(self) -> Option<Instant> {
        self.response_end_at
    }
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

    /// Snapshots the accepted response's native header/body observations.
    /// Preflight and discarded redirect children never replace these facts.
    pub fn response_timing(&self) -> FetchResponseTiming {
        *self.state.response_progress.lock().timing.lock()
    }

    pub(crate) fn mark_response_started(&self) {
        let progress = self.state.response_progress.lock();
        progress
            .timing
            .lock()
            .response_started_at
            .get_or_insert_with(Instant::now);
    }

    pub(crate) fn mark_request_started(&self) {
        let progress = self.state.response_progress.lock();
        progress
            .timing
            .lock()
            .request_started_at
            .get_or_insert_with(Instant::now);
    }

    pub(crate) fn mark_response_body_received(&self) {
        self.state
            .response_progress
            .lock()
            .timing
            .lock()
            .body_bytes_received = true;
    }

    pub(crate) fn mark_interim_response_started(&self) {
        let progress = self.state.response_progress.lock();
        progress
            .timing
            .lock()
            .first_interim_response_started_at
            .get_or_insert_with(Instant::now);
    }

    // A complete cache entry commits the response result before its reader has
    // delivered the body. Keep that cancellation fact separate from body time.
    pub(crate) fn mark_cached_response_available(&self) -> ResponseTimingRecorder {
        let progress = self.state.response_progress.lock().clone();
        progress
            .timing
            .lock()
            .response_started_at
            .get_or_insert_with(Instant::now);
        progress.terminal.store(true, Ordering::Release);
        ResponseTimingRecorder { progress }
    }

    /// Starts a new logical transfer without clearing cancellation. Detaching
    /// the previous progress also prevents an old hop's late completion from
    /// committing this new transfer.
    pub fn reset_response_progress(&self) {
        *self.state.response_progress.lock() = Arc::default();
    }

    pub(crate) fn mark_declared_response_body_complete(&self) {
        let progress = self.state.response_progress.lock();
        progress
            .timing
            .lock()
            .response_end_at
            .get_or_insert_with(Instant::now);
        progress
            .declared_body_complete
            .store(true, Ordering::Release);
    }

    pub(crate) fn mark_response_terminal(&self) {
        let progress = self.state.response_progress.lock();
        progress
            .timing
            .lock()
            .response_end_at
            .get_or_insert_with(Instant::now);
        progress.terminal.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::FetchCancelHandle;
    use std::time::Instant;

    #[test]
    fn interim_response_timing_is_first_only_and_isolated_by_response_generation() {
        let parent = FetchCancelHandle::new();
        let early = parent.child_for_subrequest();
        early.mark_request_started();
        let request_start = early.response_timing().request_started_at().unwrap();
        early.mark_request_started();
        assert_eq!(
            early.response_timing().request_started_at(),
            Some(request_start)
        );
        early.mark_interim_response_started();
        early.mark_response_body_received();
        let first = early
            .response_timing()
            .first_interim_response_started_at()
            .unwrap();
        early.mark_interim_response_started();
        early.mark_response_started();
        parent.adopt_response_progress(&early);
        let timing = parent.response_timing();
        assert_eq!(timing.first_interim_response_started_at(), Some(first));
        assert!(timing.body_bytes_received());
        assert!(request_start <= first);
        assert!(timing.response_started_at().unwrap() >= first);

        let final_response = parent.child_for_subrequest();
        final_response.mark_response_started();
        parent.adopt_response_progress(&final_response);
        early.mark_interim_response_started();
        assert!(
            parent
                .response_timing()
                .first_interim_response_started_at()
                .is_none()
        );
        assert!(!parent.response_timing().body_bytes_received());
        assert!(parent.response_timing().request_started_at().is_none());
        parent.reset_response_progress();
        final_response.mark_interim_response_started();
        assert_eq!(parent.response_timing(), Default::default());
    }

    #[test]
    fn response_timing_retains_native_boundaries_across_late_observation() {
        let handle = FetchCancelHandle::new();
        let before_headers = Instant::now();
        handle.mark_response_started();
        let headers = handle.response_timing().response_started_at().unwrap();
        assert!(headers >= before_headers && headers <= Instant::now());
        assert!(handle.response_timing().response_end_at().is_none());
        let before_body = Instant::now();
        handle.mark_declared_response_body_complete();
        let complete = handle.response_timing();
        assert!(complete.response_end_at().unwrap() >= before_body);
        assert!(complete.response_end_at().unwrap() <= Instant::now());
        handle.mark_response_started();
        handle.mark_response_terminal();
        assert_eq!(handle.response_timing(), complete);
        assert_eq!(complete.response_started_at(), Some(headers));
    }

    #[test]
    fn response_timing_tracks_only_the_selected_child_and_generation() {
        let parent = FetchCancelHandle::new();
        let discarded = parent.child_for_subrequest();
        discarded.mark_response_started();
        discarded.mark_response_terminal();
        assert!(parent.response_timing().response_started_at().is_none());
        for complete_before_selection in [false, true] {
            let accepted = parent.child_for_subrequest();
            accepted.mark_response_started();
            if complete_before_selection {
                accepted.mark_response_terminal();
            }
            parent.adopt_response_progress(&accepted);
            assert_eq!(parent.response_timing(), accepted.response_timing());
            accepted.mark_response_terminal();
            let complete = parent.response_timing();
            assert!(complete.response_started_at().is_some());
            assert!(complete.response_end_at().is_some());
            discarded.mark_response_terminal();
            assert_eq!(parent.response_timing(), complete);
            parent.reset_response_progress();
            accepted.mark_response_terminal();
            assert!(parent.response_timing().response_started_at().is_none());
            assert!(parent.response_timing().response_end_at().is_none());
        }
    }

    #[test]
    fn failed_response_timing_does_not_invent_a_header_boundary() {
        let handle = FetchCancelHandle::new();
        handle.mark_response_terminal();
        let timing = handle.response_timing();
        assert!(timing.response_started_at().is_none());
        assert!(timing.response_end_at().is_some());
    }

    #[test]
    fn cached_response_body_timing_is_separate_and_cannot_finish_a_new_generation() {
        for reset_before_body_end in [false, true] {
            let parent = FetchCancelHandle::new();
            let response = parent.child_for_subrequest();
            let recorder = response.mark_cached_response_available();
            parent.adopt_response_progress(&response);
            assert!(parent.response_completion_is_committed());
            assert!(parent.response_timing().response_started_at().is_some());
            assert!(parent.response_timing().response_end_at().is_none());
            if reset_before_body_end {
                parent.reset_response_progress();
            }
            recorder.finish();
            if reset_before_body_end {
                assert!(parent.response_timing().response_started_at().is_none());
                assert!(parent.response_timing().response_end_at().is_none());
            } else {
                let timing = parent.response_timing();
                assert!(timing.response_end_at().unwrap() >= timing.response_started_at().unwrap());
            }
        }
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
