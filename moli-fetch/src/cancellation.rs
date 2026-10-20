use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Instant,
};

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{FetchConnectionTiming, FetchCurlTiming};

const RESPONSE_PENDING: u8 = 0;
const RESPONSE_COMMITTED: u8 = 1;
const RESPONSE_CANCELLED: u8 = 2;

/// A writer token for one transfer. Replacing a handle's selected progress
/// never redirects an existing writer to a different transfer.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponseProgress {
    inner: Arc<ResponseProgressState>,
}

#[derive(Debug, Default)]
struct ResponseProgressState {
    state: AtomicU8,
    timing: Mutex<FetchResponseTiming>,
}

/// Native counts of encoded receipt and decoded representation, excluding framing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FetchResponseBodySizes {
    pub encoded: u64,
    pub decoded: u64,
}

impl ResponseProgress {
    pub(crate) fn mark_request_started(&self) {
        let now = Instant::now();
        let mut timing = self.inner.timing.lock();
        timing.request_started_at.get_or_insert(now);
        timing
            .connection_timing
            .get_or_insert_with(Default::default)
            .request_prepared_at = Some(now);
    }

    pub(crate) fn mark_tls_context_started(&self) {
        let now = Instant::now();
        self.inner
            .timing
            .lock()
            .connection_timing
            .get_or_insert_with(Default::default)
            .tls_context_started_at = Some(now);
    }

    pub(crate) fn record_connection_timing(
        &self,
        dns: Option<moli_curl::CurlDnsTiming>,
        curl: Option<FetchCurlTiming>,
    ) {
        if dns.is_none() && curl.is_none() {
            return;
        }
        let mut timing = self.inner.timing.lock();
        let connection = timing
            .connection_timing
            .get_or_insert_with(Default::default);
        connection.shared_dns = dns;
        connection.curl = curl;
    }

    pub(crate) fn curl_timing(&self) -> Option<FetchCurlTiming> {
        self.inner
            .timing
            .lock()
            .connection_timing
            .and_then(|connection| connection.curl)
    }

    pub(crate) fn mark_interim_response_started(&self) {
        self.inner
            .timing
            .lock()
            .first_interim_response_started_at
            .get_or_insert_with(Instant::now);
    }

    pub(crate) fn mark_response_body_received(&self) {
        self.inner.timing.lock().body_bytes_received = true;
    }

    pub(crate) fn commit(&self) {
        self.finish_timing();
        self.commit_state();
    }

    fn commit_state(&self) {
        let _ = self.inner.state.compare_exchange(
            RESPONSE_PENDING,
            RESPONSE_COMMITTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) fn mark_response_started(&self) {
        let mut timing = self.inner.timing.lock();
        timing.response_started_at.get_or_insert_with(Instant::now);
        timing.body_sizes.get_or_insert_with(Default::default);
    }

    pub(crate) fn reset_response_body_sizes(&self) {
        let mut timing = self.inner.timing.lock();
        timing.body_sizes = Some(FetchResponseBodySizes::default());
        timing.body_bytes_received = false;
    }

    pub(crate) fn record_encoded_response_body_size(&self, bytes: u64) {
        let mut timing = self.inner.timing.lock();
        timing
            .body_sizes
            .get_or_insert_with(Default::default)
            .encoded = bytes;
        timing.body_bytes_received |= bytes > 0;
    }

    pub(crate) fn record_encoded_response_body_chunk(&self, bytes: usize) {
        let mut timing = self.inner.timing.lock();
        let sizes = timing.body_sizes.get_or_insert_with(Default::default);
        sizes.encoded = sizes.encoded.saturating_add(bytes as u64);
        timing.body_bytes_received |= bytes > 0;
    }

    pub(crate) fn record_decoded_response_body_size(&self, bytes: usize) {
        let mut timing = self.inner.timing.lock();
        let sizes = timing.body_sizes.get_or_insert_with(Default::default);
        sizes.decoded = sizes.decoded.saturating_add(bytes as u64);
    }

    fn finish_timing(&self) {
        self.inner
            .timing
            .lock()
            .response_end_at
            .get_or_insert_with(Instant::now);
    }

    pub(crate) fn mark_cached_response_available(
        &self,
        sizes: FetchResponseBodySizes,
    ) -> CachedResponseBodyTiming {
        self.mark_response_started();
        self.inner.timing.lock().body_sizes = Some(sizes);
        self.commit_state();
        CachedResponseBodyTiming(self.clone())
    }

    fn is_committed(&self) -> bool {
        self.inner.state.load(Ordering::Acquire) == RESPONSE_COMMITTED
    }

    fn cancel_if_pending(&self) -> bool {
        self.inner.state.compare_exchange(
            RESPONSE_PENDING,
            RESPONSE_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) != Err(RESPONSE_COMMITTED)
    }
}

/// Native response boundaries shared with observers of the selected transfer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchResponseTiming {
    body_sizes: Option<FetchResponseBodySizes>,
    request_started_at: Option<Instant>,
    first_interim_response_started_at: Option<Instant>,
    body_bytes_received: bool,
    response_started_at: Option<Instant>,
    response_end_at: Option<Instant>,
    connection_timing: Option<FetchConnectionTiming>,
}

impl FetchResponseTiming {
    pub fn body_sizes(self) -> Option<FetchResponseBodySizes> {
        self.body_sizes
    }
    pub fn body_bytes_received(self) -> bool {
        self.body_bytes_received
    }
    /// The transport's pre-request boundary, after connection/TLS setup and
    /// before sending the request. Pure cache hits do not produce this boundary.
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

    /// Observations from the selected transfer only. Callback timestamps may
    /// precede final statistics; streaming consumers await `finish()` for the
    /// terminal result before relying on the completed curl statistics.
    pub fn connection_timing(self) -> Option<FetchConnectionTiming> {
        self.connection_timing
    }
}

pub(crate) struct CachedResponseBodyTiming(ResponseProgress);

impl CachedResponseBodyTiming {
    pub(crate) fn finish(self) {
        self.0.finish_timing();
    }
}

#[derive(Debug, Default)]
struct FetchLifecycleState {
    cancellation: CancellationToken,
    // This lock protects selection, while the selected token arbitrates
    // transport completion and consumer cancellation without taking the lock.
    response_progress: Mutex<ResponseProgress>,
}

/// Cancellation authority and selected response progress are independent.
/// Only pending-only cancellation arbitrates with transport completion;
/// unconditional cancellation can coexist with committed response progress.
#[derive(Debug, Clone, Default)]
pub struct FetchCancelHandle {
    state: Arc<FetchLifecycleState>,
}

impl FetchCancelHandle {
    #[cfg(test)]
    fn mark_request_started(&self) {
        self.response_progress().mark_request_started();
    }

    #[cfg(test)]
    fn mark_interim_response_started(&self) {
        self.response_progress().mark_interim_response_started();
    }

    #[cfg(test)]
    fn mark_response_body_received(&self) {
        self.response_progress().mark_response_body_received();
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Unconditionally requests cancellation of this handle and its descendants.
    /// This does not lock response selection or change the selected progress.
    /// An already committed result or a racing completion can therefore coexist
    /// with cancellation, including when cancellation comes from an ancestor.
    /// Use [`Self::cancel_if_response_pending`] to arbitrate with completion.
    pub fn cancel(&self) {
        self.state.cancellation.cancel();
    }

    /// Cancels only if this response's transport result has not committed.
    /// Completion and this cancellation compete on the same atomic state, so
    /// a completion arriving between a separate check and cancel cannot lose.
    /// Response selection is serialized with this cancellation. A false return
    /// means this call did not request cancellation; independent unconditional
    /// or inherited cancellation may still have cancelled the handle.
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
    /// Selection is skipped if cancellation is already observed. Pending-only
    /// cancellation holds the same selection lock, so once it wins, adoption
    /// cannot replace its terminal progress. Unconditional or inherited
    /// cancellation may race with selection and does not arbitrate completion.
    pub fn adopt_response_progress(&self, response: &Self) {
        let progress = response.state.response_progress.lock().clone();
        let mut selected = self.state.response_progress.lock();
        // Serialize this check with cancel_if_response_pending(), which sets
        // cancellation while holding the same selection lock.
        if !self.is_cancelled() {
            *selected = progress;
        }
    }

    /// Returns whether the selected response's terminal transport result has
    /// committed. A committed result rejects later pending-only cancellation,
    /// but does not imply that unconditional cancellation has not occurred.
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

    /// Restore representation counts without changing this transfer's clock.
    pub fn set_response_body_sizes(&self, sizes: FetchResponseBodySizes) {
        self.response_progress().inner.timing.lock().body_sizes = Some(sizes);
    }

    pub fn response_timing(&self) -> FetchResponseTiming {
        *self.response_progress().inner.timing.lock()
    }

    #[cfg(test)]
    fn mark_response_started(&self) {
        self.response_progress().mark_response_started();
    }

    #[cfg(test)]
    fn mark_declared_response_body_complete(&self) {
        self.response_progress().commit();
    }

    #[cfg(test)]
    fn mark_cached_response_available(
        &self,
        sizes: FetchResponseBodySizes,
    ) -> CachedResponseBodyTiming {
        self.response_progress()
            .mark_cached_response_available(sizes)
    }

    #[cfg(test)]
    fn mark_response_terminal(&self) {
        self.response_progress().commit();
    }

    pub(crate) fn response_progress(&self) -> ResponseProgress {
        self.state.response_progress.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{FetchCancelHandle, FetchResponseBodySizes};
    use crate::FetchCurlTiming;
    use std::time::{Duration, Instant};

    #[test]
    fn connection_observations_cannot_follow_response_selection_or_reset() {
        let selected = FetchCancelHandle::new();
        let first = selected.response_progress();
        let completed_at = Instant::now();
        let dns = moli_curl::CurlDnsTiming {
            started_at: completed_at - Duration::from_millis(1),
            completed_at,
        };
        first.mark_tls_context_started();
        first.mark_request_started();
        let curl = FetchCurlTiming {
            name_lookup: Some(Duration::from_micros(17)),
            connect: Some(Duration::from_micros(213)),
            pretransfer: Some(Duration::from_micros(817)),
            total: Some(Duration::from_micros(1421)),
            new_connections: Some(1),
            ..Default::default()
        };
        first.record_connection_timing(Some(dns), Some(curl));
        let observed = selected.response_timing().connection_timing().unwrap();
        assert_eq!(observed.shared_dns, Some(dns));
        assert_eq!(observed.curl, Some(curl));
        assert!(observed.tls_context_started_at.is_some());
        assert!(observed.request_prepared_at.is_some());

        let second = selected.begin_response_progress();
        assert!(selected.response_timing().connection_timing().is_none());
        first.mark_request_started();
        first.record_connection_timing(Some(dns), Some(curl));
        assert!(selected.response_timing().connection_timing().is_none());

        let tee = FetchCancelHandle::new();
        tee.adopt_response_progress(&selected);
        second.record_connection_timing(None, Some(curl));
        assert_eq!(
            tee.response_timing().connection_timing(),
            selected.response_timing().connection_timing()
        );
        assert!(
            tee.response_timing()
                .connection_timing()
                .unwrap()
                .shared_dns
                .is_none()
        );
        assert_eq!(
            first
                .inner
                .timing
                .lock()
                .connection_timing
                .unwrap()
                .shared_dns,
            Some(dns)
        );
    }

    #[test]
    fn later_preparation_keeps_the_first_request_boundary() {
        let handle = FetchCancelHandle::new();
        let progress = handle.response_progress();
        progress.mark_request_started();
        let first = handle.response_timing().request_started_at().unwrap();
        let second_before = Instant::now();
        progress.mark_request_started();
        let timing = handle.response_timing();
        assert_eq!(timing.request_started_at(), Some(first));
        assert!(
            timing
                .connection_timing()
                .unwrap()
                .request_prepared_at
                .unwrap()
                >= second_before
        );
        assert!(timing.connection_timing().unwrap().curl.is_none());
    }

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
    fn unconditional_cancellation_does_not_arbitrate_response_completion() {
        for cancel_ancestor in [false, true] {
            for complete_before_cancellation in [false, true] {
                let root = FetchCancelHandle::new();
                let parent = root.child_for_subrequest();
                let child = parent.child_for_subrequest();
                let writer = child.begin_response_progress();
                parent.adopt_response_progress(&child);
                if complete_before_cancellation {
                    writer.commit();
                }

                if cancel_ancestor {
                    root.cancel();
                } else {
                    parent.cancel();
                }
                writer.commit();

                assert!(parent.is_cancelled());
                assert!(child.is_cancelled());
                assert!(parent.response_completion_is_committed());
                assert!(!parent.cancel_if_response_pending());
                assert!(parent.is_cancelled());
            }
        }
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
            parent.begin_response_progress();
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
            let recorder = response.mark_cached_response_available(FetchResponseBodySizes {
                encoded: 23,
                decoded: 3,
            });
            parent.adopt_response_progress(&response);
            assert!(parent.response_completion_is_committed());
            assert!(parent.response_timing().response_started_at().is_some());
            assert!(parent.response_timing().response_end_at().is_none());
            if reset_before_body_end {
                parent.begin_response_progress();
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
        parent.begin_response_progress();
        final_response.mark_interim_response_started();
        assert_eq!(parent.response_timing(), Default::default());
    }
}

#[cfg(test)]
mod body_size_generation_tests {
    use super::*;
    #[test]
    fn response_body_sizes_remain_with_their_transfer_writer() {
        let handle = FetchCancelHandle::new();
        let old = handle.begin_response_progress();
        old.mark_response_started();
        old.record_encoded_response_body_size(23);
        old.record_decoded_response_body_size(3);
        assert_eq!(
            handle.response_timing().body_sizes(),
            Some(FetchResponseBodySizes {
                encoded: 23,
                decoded: 3
            })
        );
        let current = handle.begin_response_progress();
        current.mark_response_started();
        current.record_encoded_response_body_size(7);
        current.record_decoded_response_body_size(5);
        old.record_encoded_response_body_size(99);
        old.record_decoded_response_body_size(88);
        old.commit();
        assert_eq!(
            handle.response_timing().body_sizes(),
            Some(FetchResponseBodySizes {
                encoded: 7,
                decoded: 5
            })
        );
        assert!(!handle.response_completion_is_committed());
        current.commit();
        assert!(handle.response_completion_is_committed());
    }
}
