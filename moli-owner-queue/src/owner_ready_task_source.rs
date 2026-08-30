use std::{collections::VecDeque, fmt, sync::Arc};

use parking_lot::{Mutex, MutexGuard};

#[derive(Debug)]
struct OwnerReadyTaskState<T> {
    incoming: VecDeque<T>,
    ready: bool,
    source_open: bool,
}

impl<T> Default for OwnerReadyTaskState<T> {
    fn default() -> Self {
        Self {
            incoming: VecDeque::new(),
            ready: false,
            source_open: true,
        }
    }
}

#[derive(Debug)]
struct OwnerReadyTaskShared<T, S> {
    state: Mutex<OwnerReadyTaskState<T>>,
    signal: S,
}

/// Fixed notification capability for one owner-ready task source.
///
/// The signal is installed when the source is created and shared by every
/// producer route. Implementations must be nonblocking and non-reentrant: the
/// notification runs while the source readiness boundary is locked and must
/// not inspect or mutate that source.
pub trait OwnerTaskReadySignal: Send + Sync + 'static {
    fn signal_ready(&self);
}

/// Send error returned when an owner-ready source has already closed.
#[derive(Debug, Eq, PartialEq)]
pub struct OwnerReadyTaskRouteClosed<T>(pub T);

impl<T> fmt::Display for OwnerReadyTaskRouteClosed<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("owner-ready task source is closed")
    }
}

impl<T: fmt::Debug> std::error::Error for OwnerReadyTaskRouteClosed<T> {}

/// Cloneable producer route for an owner task source with edge-triggered wakeups.
///
/// The payload enqueue and the empty-to-nonempty wake are serialized with the
/// consumer's dequeue/rearm boundary. This prevents both a lost wake and the
/// inverse race where a consumer removes the payload before its producer can
/// publish the wake.
#[derive(Debug)]
pub struct OwnerReadyTaskRoute<T, S> {
    shared: Arc<OwnerReadyTaskShared<T, S>>,
}

impl<T, S> Clone for OwnerReadyTaskRoute<T, S> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<T, S: OwnerTaskReadySignal> OwnerReadyTaskRoute<T, S> {
    /// Enqueue one task and publish a wake only when the source becomes ready.
    pub fn send_and_signal_if_newly_ready(
        &self,
        task: T,
    ) -> Result<(), OwnerReadyTaskRouteClosed<T>> {
        let mut state = lock_state(&self.shared.state);
        if !state.source_open {
            return Err(OwnerReadyTaskRouteClosed(task));
        }
        state.incoming.push_back(task);
        if !state.ready {
            state.ready = true;
            self.shared.signal.signal_ready();
        }
        Ok(())
    }

    /// Enqueue one producer batch under the same readiness boundary.
    ///
    /// The returned count is zero for an empty iterator. A nonempty batch is
    /// contiguous with respect to the owner consumer and publishes at most one
    /// empty-to-nonempty wake.
    pub fn send_all_and_signal_if_newly_ready(
        &self,
        tasks: impl IntoIterator<Item = T>,
    ) -> Result<usize, OwnerReadyTaskRouteClosed<T>> {
        // Materialize user-provided iterator code before taking the readiness
        // lock. Only queue mutation and the documented notification callback
        // may run inside the critical section.
        let mut tasks = tasks.into_iter().collect::<Vec<_>>().into_iter();
        let enqueued = tasks.len();
        if enqueued == 0 {
            return Ok(0);
        }
        let mut state = lock_state(&self.shared.state);
        if !state.source_open {
            let first = tasks.next().expect("nonempty producer batch");
            // Drop arbitrary remaining payloads outside the shared-state lock;
            // their destructors may call producer code for this source.
            drop(state);
            drop(tasks);
            return Err(OwnerReadyTaskRouteClosed(first));
        }
        state.incoming.extend(tasks);
        if !state.ready {
            state.ready = true;
            self.shared.signal.signal_ready();
        }
        Ok(enqueued)
    }

    pub fn same_source_as(&self, source: &OwnerReadyTaskSource<T, S>) -> bool {
        Arc::ptr_eq(&self.shared, &source.shared)
    }

    pub fn same_route_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

/// Single-consumer task source that coalesces producer wakes by readiness edge.
///
/// Producers may clone [`OwnerReadyTaskRoute`], while this source remains with
/// the unique owner-side scheduler. The source rearms its route only after its
/// final queued task has been removed or the source has been explicitly
/// cleared.
#[derive(Debug)]
pub struct OwnerReadyTaskSource<T, S> {
    tasks: VecDeque<T>,
    shared: Arc<OwnerReadyTaskShared<T, S>>,
}

impl<T, S> Default for OwnerReadyTaskSource<T, S>
where
    S: OwnerTaskReadySignal + Default,
{
    fn default() -> Self {
        Self::new(S::default())
    }
}

impl<T, S: OwnerTaskReadySignal> OwnerReadyTaskSource<T, S> {
    pub fn new(signal: S) -> Self {
        Self {
            tasks: VecDeque::new(),
            shared: Arc::new(OwnerReadyTaskShared {
                state: Mutex::new(OwnerReadyTaskState::default()),
                signal,
            }),
        }
    }

    pub fn route(&self) -> OwnerReadyTaskRoute<T, S> {
        OwnerReadyTaskRoute {
            shared: self.shared.clone(),
        }
    }

    /// Enqueue from the unique owner while it already has execution control.
    ///
    /// This marks the source ready but deliberately publishes no wake. External
    /// producers must use [`OwnerReadyTaskRoute`] instead.
    pub fn enqueue_local(&mut self, task: T) {
        let Self { tasks, shared } = self;
        let mut state = lock_state(&shared.state);
        tasks.push_back(task);
        state.ready = true;
    }

    pub fn front(&mut self) -> Option<&T> {
        let Self { tasks, shared } = self;
        let mut state = lock_state(&shared.state);
        tasks.append(&mut state.incoming);
        let task = tasks.front();
        state.ready = task.is_some();
        task
    }

    pub fn pop_front(&mut self) -> Option<T> {
        let Self { tasks, shared } = self;
        let mut state = lock_state(&shared.state);
        tasks.append(&mut state.incoming);
        let task = tasks.pop_front();
        state.ready = !tasks.is_empty();
        task
    }

    pub fn is_empty(&mut self) -> bool {
        let Self { tasks, shared } = self;
        let mut state = lock_state(&shared.state);
        tasks.append(&mut state.incoming);
        let is_empty = tasks.is_empty();
        state.ready = !is_empty;
        is_empty
    }

    /// Inspect all currently accepted tasks under the same boundary used by
    /// dequeue/rearm. The predicate must be non-reentrant and must not call a
    /// producer route for this source.
    pub fn has_matching_task(&mut self, mut predicate: impl FnMut(&T) -> bool) -> bool {
        let Self { tasks, shared } = self;
        let mut state = lock_state(&shared.state);
        tasks.append(&mut state.incoming);
        let has_match = tasks.iter().any(&mut predicate);
        state.ready = !tasks.is_empty();
        has_match
    }

    /// Mutate all accepted payloads while preserving the producer readiness
    /// edge for as long as the resulting local queue remains nonempty.
    ///
    /// The callback runs under the source readiness lock and must not invoke a
    /// producer route for this source.
    pub fn with_tasks_mut<R>(&mut self, operation: impl FnOnce(&mut VecDeque<T>) -> R) -> R {
        let Self { tasks, shared } = self;
        let mut state = lock_state(&shared.state);
        tasks.append(&mut state.incoming);
        let result = operation(tasks);
        state.ready = !tasks.is_empty();
        result
    }

    /// Inspect only payloads already accepted by the owner. Pending producer
    /// arrivals are deliberately excluded so derived-index assertions do not
    /// mutate source state.
    pub fn with_local_tasks<R>(&self, operation: impl FnOnce(&VecDeque<T>) -> R) -> R {
        operation(&self.tasks)
    }

    /// Drop every accepted task and rearm the producer readiness edge.
    pub fn clear_local(&mut self) {
        let Self { tasks, shared } = self;
        let mut discarded = std::mem::take(tasks);
        {
            let mut state = lock_state(&shared.state);
            discarded.append(&mut state.incoming);
            state.ready = false;
        }
        drop(discarded);
    }
}

impl<T, S> Drop for OwnerReadyTaskSource<T, S> {
    fn drop(&mut self) {
        let incoming = {
            let mut state = lock_state(&self.shared.state);
            state.source_open = false;
            state.ready = false;
            std::mem::take(&mut state.incoming)
        };
        drop(incoming);
    }
}

fn lock_state<T>(state: &Mutex<OwnerReadyTaskState<T>>) -> MutexGuard<'_, OwnerReadyTaskState<T>> {
    state.lock()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{OwnerReadyTaskSource, OwnerTaskReadySignal};

    #[derive(Clone, Debug)]
    struct CountingReadySignal(Arc<AtomicUsize>);

    impl OwnerTaskReadySignal for CountingReadySignal {
        fn signal_ready(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn counting_source<T>() -> (
        OwnerReadyTaskSource<T, CountingReadySignal>,
        Arc<AtomicUsize>,
    ) {
        let wakes = Arc::new(AtomicUsize::new(0));
        (
            OwnerReadyTaskSource::new(CountingReadySignal(wakes.clone())),
            wakes,
        )
    }

    #[test]
    fn consecutive_enqueues_publish_one_wake_until_the_source_drains() {
        let (mut source, wakes) = counting_source();
        let route = source.route();

        route.send_and_signal_if_newly_ready(1).unwrap();
        route.send_and_signal_if_newly_ready(2).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);

        assert_eq!(source.front(), Some(&1));
        route.send_and_signal_if_newly_ready(3).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);

        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));
        assert_eq!(source.pop_front(), Some(3));
        assert!(source.is_empty());

        route.send_and_signal_if_newly_ready(4).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        assert_eq!(source.pop_front(), Some(4));
    }

    #[test]
    fn matching_task_query_accepts_pending_producer_work_without_changing_fifo() {
        let (mut source, wakes) = counting_source();
        let route = source.route();
        route.send_and_signal_if_newly_ready(1).unwrap();
        route.send_and_signal_if_newly_ready(2).unwrap();

        assert!(source.has_matching_task(|task| *task == 2));
        assert!(!source.has_matching_task(|task| *task == 3));
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));
        assert!(source.is_empty());
    }

    #[test]
    fn owner_mutation_preserves_readiness_until_the_resulting_queue_drains() {
        let (mut source, wakes) = counting_source();
        let route = source.route();
        route.send_and_signal_if_newly_ready(2).unwrap();
        route.send_and_signal_if_newly_ready(1).unwrap();

        source.with_tasks_mut(|tasks| tasks.make_contiguous().sort());
        assert_eq!(
            source.with_local_tasks(|tasks| tasks.iter().copied().collect::<Vec<_>>()),
            vec![1, 2]
        );
        route.send_and_signal_if_newly_ready(3).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));
        assert_eq!(source.pop_front(), Some(3));

        route.send_and_signal_if_newly_ready(4).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn clearing_a_live_source_rearms_its_producer_route() {
        let (mut source, wakes) = counting_source();
        let route = source.route();

        route.send_and_signal_if_newly_ready(1).unwrap();
        source.clear_local();
        route.send_and_signal_if_newly_ready(2).unwrap();

        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        assert_eq!(source.pop_front(), Some(2));
    }

    #[test]
    fn producer_batch_is_contiguous_and_shares_one_readiness_wake() {
        let (mut source, wakes) = counting_source();
        let route = source.route();

        assert_eq!(
            route.send_all_and_signal_if_newly_ready([1, 2, 3]).unwrap(),
            3
        );
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_eq!(source.pop_front(), Some(1));

        assert_eq!(route.send_all_and_signal_if_newly_ready([4, 5]).unwrap(), 2);
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_eq!(
            (0..4).map(|_| source.pop_front()).collect::<Vec<_>>(),
            vec![Some(2), Some(3), Some(4), Some(5)]
        );

        assert_eq!(route.send_all_and_signal_if_newly_ready([]).unwrap(), 0);
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_closed_source_rejects_work_without_publishing_a_wake() {
        let (source, wakes) = counting_source();
        let route = source.route();
        drop(source);

        assert!(route.send_and_signal_if_newly_ready(1).is_err());
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_closed_source_rejects_a_batch_at_its_first_payload() {
        let (source, wakes) = counting_source();
        let route = source.route();
        drop(source);

        let error = route
            .send_all_and_signal_if_newly_ready([1, 2, 3])
            .expect_err("closed source must reject a nonempty batch");
        assert_eq!(error.0, 1);
        assert_eq!(
            route.send_all_and_signal_if_newly_ready([]).unwrap(),
            0,
            "an empty batch remains a no-op after source closure"
        );
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn route_identity_uses_the_shared_source() {
        let (source, _wakes) = counting_source::<usize>();
        let first = source.route();
        let clone = first.clone();
        let (other_source, _other_wakes) = counting_source::<usize>();
        let other = other_source.route();

        assert!(first.same_source_as(&source));
        assert!(first.same_route_as(&clone));
        assert!(!first.same_source_as(&other_source));
        assert!(!first.same_route_as(&other));
    }

    #[test]
    fn local_inspection_excludes_unaccepted_producer_arrivals() {
        let (mut source, _wakes) = counting_source();
        let route = source.route();
        source.enqueue_local(1);
        route.send_and_signal_if_newly_ready(2).unwrap();

        assert_eq!(
            source.with_local_tasks(|tasks| tasks.iter().copied().collect::<Vec<_>>()),
            vec![1]
        );
        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));
    }

    #[test]
    fn local_owner_enqueue_participates_in_the_same_readiness_epoch() {
        let (mut source, wakes) = counting_source();
        let route = source.route();
        source.enqueue_local(1);

        route.send_and_signal_if_newly_ready(2).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));

        route.send_and_signal_if_newly_ready(3).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_append_and_final_dequeue_never_lose_the_successor_wake() {
        for _ in 0..128 {
            let (mut source, wakes) = counting_source();
            source.enqueue_local(1);
            let route = source.route();
            let start = Arc::new(Barrier::new(3));

            let consumer_start = start.clone();
            let consumer = std::thread::spawn(move || {
                consumer_start.wait();
                assert_eq!(source.pop_front(), Some(1));
                let observed_empty_after_dequeue = source.is_empty();
                (source, observed_empty_after_dequeue)
            });

            let producer_start = start.clone();
            let producer = std::thread::spawn(move || {
                producer_start.wait();
                route.send_and_signal_if_newly_ready(2).unwrap();
            });

            start.wait();
            let (mut source, observed_empty_after_dequeue) = consumer.join().unwrap();
            producer.join().unwrap();
            let published_wakes = wakes.load(Ordering::Relaxed);

            assert!(published_wakes <= 1);
            if observed_empty_after_dequeue {
                assert_eq!(
                    published_wakes, 1,
                    "a producer arriving after final dequeue must publish the rearmed wake"
                );
            }
            assert_eq!(source.pop_front(), Some(2));
            assert!(source.is_empty());
        }
    }
}
