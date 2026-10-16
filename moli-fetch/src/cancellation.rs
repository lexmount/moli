use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

const RESPONSE_PENDING: u8 = 0;
const RESPONSE_COMMITTED: u8 = 1;
const RESPONSE_CANCELLED: u8 = 2;

/// A writer token for one transfer. Replacing a handle's selected progress
/// never redirects an existing writer to a different transfer.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponseProgress {
    state: Arc<AtomicU8>,
}

impl ResponseProgress {
    pub(crate) fn commit(&self) {
        let _ = self.state.compare_exchange(
            RESPONSE_PENDING,
            RESPONSE_COMMITTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn is_committed(&self) -> bool {
        self.state.load(Ordering::Acquire) == RESPONSE_COMMITTED
    }

    fn cancel_if_pending(&self) -> bool {
        self.state.compare_exchange(
            RESPONSE_PENDING,
            RESPONSE_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) != Err(RESPONSE_COMMITTED)
    }
}

#[derive(Debug, Default)]
struct FetchLifecycleState {
    cancellation: CancellationToken,
    // This lock protects selection, while the selected token arbitrates
    // transport completion and consumer cancellation without taking the lock.
    response_progress: Mutex<ResponseProgress>,
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

    /// Cancels only if this response's transport result has not committed.
    /// Completion and this cancellation compete on the same atomic state, so
    /// a completion arriving between a separate check and cancel cannot lose.
    pub fn cancel_if_response_pending(&self) -> bool {
        let progress = self.state.response_progress.lock();
        if !progress.cancel_if_pending() {
            return false;
        }
        self.cancel();
        true
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancellation.is_cancelled()
    }

    /// Creates cancellation authority for one preflight or redirect hop.
    /// Parent cancellation propagates to this child; cancelling the child does
    /// not cancel its parent or siblings. Completion facts remain independent.
    pub fn child_for_subrequest(&self) -> Self {
        Self {
            state: Arc::new(FetchLifecycleState {
                cancellation: self.state.cancellation.child_token(),
                ..FetchLifecycleState::default()
            }),
        }
    }

    /// Selects the accepted final response's transport progress. This shares
    /// already recorded and future completion facts, without retaining the
    /// child's cancellation authority. Call only after the accepted response's
    /// headers arrive: selection is a snapshot, and starting another transfer
    /// on the child does not change this choice.
    /// Once cancellation wins, later selection cannot replace its terminal
    /// progress with a pending or already committed response.
    pub fn adopt_response_progress(&self, response: &Self) {
        let progress = response.state.response_progress.lock().clone();
        let mut selected = self.state.response_progress.lock();
        // Serialize this check with cancel_if_response_pending(), which sets
        // cancellation while holding the same selection lock.
        if !self.is_cancelled() {
            *selected = progress;
        }
    }

    /// Returns whether transport facts already determine this response's
    /// terminal result, so a later consumer cancellation must not replace it.
    pub fn response_completion_is_committed(&self) -> bool {
        self.state.response_progress.lock().is_committed()
    }

    /// Starts a transfer without clearing cancellation and returns its writer.
    /// Every callback must retain this token instead of looking up whichever
    /// progress happens to be selected when the callback eventually runs.
    pub(crate) fn begin_response_progress(&self) -> ResponseProgress {
        let progress = ResponseProgress::default();
        let mut selected = self.state.response_progress.lock();
        if self.is_cancelled() {
            progress.cancel_if_pending();
        }
        *selected = progress.clone();
        progress
    }

    pub(crate) fn response_progress(&self) -> ResponseProgress {
        self.state.response_progress.lock().clone()
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

        let first = handle.response_progress();
        first.commit();
        assert!(observer.response_completion_is_committed());

        let second = handle.begin_response_progress();
        assert!(!observer.response_completion_is_committed());

        first.commit();
        assert!(!observer.response_completion_is_committed());
        second.commit();
        assert!(observer.response_completion_is_committed());
    }

    #[test]
    fn subrequest_cancellation_is_isolated_and_inherits_parent_abort() {
        let parent = FetchCancelHandle::new();
        let first = parent.child_for_subrequest();
        let second = parent.child_for_subrequest();
        first.clone().cancel();
        assert!(first.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!second.is_cancelled());
        parent.cancel();
        assert!(second.is_cancelled());
        assert!(parent.child_for_subrequest().is_cancelled());
        assert!(second.child_for_subrequest().is_cancelled());
    }

    #[test]
    fn cancellation_reaches_descendants_after_an_intermediate_handle_is_dropped() {
        let parent = FetchCancelHandle::new();
        let child = parent.child_for_subrequest();
        let grandchild = child.child_for_subrequest();
        drop(child);

        assert!(!grandchild.is_cancelled());
        parent.cancel();
        assert!(grandchild.is_cancelled());
        assert!(grandchild.child_for_subrequest().is_cancelled());
    }

    #[test]
    fn concurrent_child_creation_inherits_parent_cancellation() {
        for _ in 0..64 {
            let parent = FetchCancelHandle::new();
            let ready = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                let cancellation = scope.spawn(|| {
                    ready.wait();
                    parent.cancel();
                });
                ready.wait();
                let child = parent.child_for_subrequest();
                let grandchild = child.child_for_subrequest();
                cancellation.join().unwrap();

                // CancellationToken guarantees propagation to every child
                // once cancel() returns, including concurrently created hops.
                assert!(child.is_cancelled());
                assert!(grandchild.is_cancelled());
            });
        }
    }

    #[test]
    fn only_selected_response_progress_commits_the_parent() {
        for complete_before_selection in [false, true] {
            let parent = FetchCancelHandle::new();
            let discarded = parent.child_for_subrequest();
            let discarded_writer = discarded.begin_response_progress();
            discarded_writer.commit();
            assert!(!parent.response_completion_is_committed());
            let response = parent.child_for_subrequest();
            let writer = response.begin_response_progress();
            if complete_before_selection {
                writer.commit();
            }
            parent.adopt_response_progress(&response);
            assert_eq!(
                parent.response_completion_is_committed(),
                complete_before_selection
            );
            discarded_writer.commit();
            writer.commit();
            assert!(parent.response_completion_is_committed());
            parent.begin_response_progress();
            writer.commit();
            assert!(!parent.response_completion_is_committed());
            assert!(!parent.is_cancelled());
        }
    }

    #[test]
    fn selected_response_keeps_its_writer_when_child_starts_another_transfer() {
        let parent = FetchCancelHandle::new();
        let child = parent.child_for_subrequest();
        let selected = child.begin_response_progress();
        parent.adopt_response_progress(&child);
        let next = child.begin_response_progress();

        next.commit();
        assert!(!parent.response_completion_is_committed());
        selected.commit();
        assert!(parent.response_completion_is_committed());
    }

    #[test]
    fn committed_response_wins_later_consumer_cancellation() {
        let parent = FetchCancelHandle::new();
        let child = parent.child_for_subrequest();
        let writer = child.begin_response_progress();
        parent.adopt_response_progress(&child);
        writer.commit();

        assert!(!parent.cancel_if_response_pending());
        assert!(!parent.is_cancelled());
        assert!(!child.is_cancelled());
    }

    #[test]
    fn consumer_cancellation_wins_later_completion() {
        let parent = FetchCancelHandle::new();
        let child = parent.child_for_subrequest();
        let writer = child.begin_response_progress();
        parent.adopt_response_progress(&child);

        assert!(parent.cancel_if_response_pending());
        writer.commit();
        assert!(!parent.response_completion_is_committed());
        assert!(parent.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[test]
    fn adoption_after_cancellation_cannot_replace_cancelled_progress() {
        for complete_before_selection in [false, true] {
            let parent = FetchCancelHandle::new();
            let child = parent.child_for_subrequest();
            let writer = child.begin_response_progress();

            assert!(parent.cancel_if_response_pending());
            if complete_before_selection {
                writer.commit();
            }
            parent.adopt_response_progress(&child);
            writer.commit();

            assert!(parent.is_cancelled());
            assert!(!parent.response_completion_is_committed());
        }
    }

    #[test]
    fn adoption_respects_direct_and_inherited_cancellation() {
        let root = FetchCancelHandle::new();
        let parent = root.child_for_subrequest();
        let response = parent.child_for_subrequest();
        let writer = response.begin_response_progress();
        root.cancel();
        root.adopt_response_progress(&response);
        parent.adopt_response_progress(&response);
        writer.commit();

        assert!(!root.response_completion_is_committed());
        assert!(parent.is_cancelled());
        assert!(!parent.response_completion_is_committed());
    }

    #[test]
    fn starting_transfer_after_cancellation_cannot_revive_completion() {
        let parent = FetchCancelHandle::new();
        let child = parent.child_for_subrequest();
        assert!(parent.cancel_if_response_pending());

        parent.begin_response_progress().commit();
        child.begin_response_progress().commit();

        assert!(parent.is_cancelled());
        assert!(!parent.response_completion_is_committed());
        assert!(child.is_cancelled());
        assert!(!child.response_completion_is_committed());
    }

    #[test]
    fn concurrent_adoption_completion_and_cancellation_have_one_winner() {
        for _ in 0..64 {
            let parent = FetchCancelHandle::new();
            let child = parent.child_for_subrequest();
            let writer = child.begin_response_progress();
            let ready = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                let selection = scope.spawn(|| {
                    ready.wait();
                    parent.adopt_response_progress(&child);
                    writer.commit();
                });
                ready.wait();
                let cancelled = parent.cancel_if_response_pending();
                selection.join().unwrap();
                assert_eq!(parent.response_completion_is_committed(), !cancelled);
                assert_eq!(parent.is_cancelled(), cancelled);
            });
        }
    }

    #[test]
    fn completion_and_consumer_cancellation_have_one_winner() {
        for _ in 0..64 {
            let handle = FetchCancelHandle::new();
            let writer = handle.begin_response_progress();
            let ready = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                let completion = scope.spawn(|| {
                    ready.wait();
                    writer.commit();
                });
                ready.wait();
                let cancelled = handle.cancel_if_response_pending();
                completion.join().unwrap();
                assert_eq!(handle.response_completion_is_committed(), !cancelled);
                assert_eq!(handle.is_cancelled(), cancelled);
            });
        }
    }

    #[test]
    fn late_completion_during_selection_cannot_commit_a_new_transfer() {
        let parent = FetchCancelHandle::new();
        let child = parent.child_for_subrequest();
        let old_writer = child.begin_response_progress();
        let current_writer = child.begin_response_progress();
        std::thread::scope(|scope| {
            let completion = scope.spawn(move || old_writer.commit());
            parent.adopt_response_progress(&child);
            completion.join().unwrap();
        });
        assert!(!parent.response_completion_is_committed());
        current_writer.commit();
        assert!(parent.response_completion_is_committed());
    }

    #[test]
    fn concurrent_selection_and_transfer_start_keep_a_stable_snapshot() {
        for _ in 0..32 {
            let parent = FetchCancelHandle::new();
            let child = parent.child_for_subrequest();
            let first = child.begin_response_progress();
            let ready = std::sync::Barrier::new(2);
            let second = std::thread::scope(|scope| {
                let restart = scope.spawn(|| {
                    ready.wait();
                    child.begin_response_progress()
                });
                ready.wait();
                parent.adopt_response_progress(&child);
                restart.join().unwrap()
            });
            // Selection may observe either transfer. A subsequent restart
            // must leave that snapshot and both existing writers intact.
            child.begin_response_progress();
            first.commit();
            second.commit();
            assert!(parent.response_completion_is_committed());
            assert!(!child.response_completion_is_committed());
        }
    }
}
