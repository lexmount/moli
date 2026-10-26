use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashSet},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TimerId(u32);

impl TimerId {
    pub fn new(id: u32) -> Option<Self> {
        (id != 0).then_some(Self(id))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

/// A scheduling-sequence boundary from which an exact timer range can be
/// recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerScheduleSnapshot {
    next_sequence: u64,
}

/// A half-open scheduling-sequence range for timers queued during one owner
/// operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerScheduleRange {
    inclusive_sequence: u64,
    exclusive_sequence: u64,
}

impl TimerScheduleRange {
    pub fn is_empty(self) -> bool {
        self.inclusive_sequence == self.exclusive_sequence
    }

    fn contains(self, sequence: u64) -> bool {
        self.inclusive_sequence <= sequence && sequence < self.exclusive_sequence
    }
}

#[derive(Debug)]
pub struct ReadyTimer<T> {
    pub id: TimerId,
    pub delay_ms: u64,
    pub payload: T,
}

#[derive(Debug)]
struct ScheduledTimer<T> {
    id: TimerId,
    sequence: u64,
    run_at: Instant,
    delay_ms: u64,
    payload: T,
}

impl<T> ScheduledTimer<T> {
    fn is_ready_at(&self, now: Instant) -> bool {
        self.run_at <= now
    }
}

impl<T> Ord for ScheduledTimer<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.run_at.cmp(&other.run_at).reverse() {
            Ordering::Equal => self.sequence.cmp(&other.sequence).reverse(),
            ordering => ordering,
        }
    }
}

impl<T> PartialOrd for ScheduledTimer<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Eq for ScheduledTimer<T> {}

impl<T> PartialEq for ScheduledTimer<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.sequence == other.sequence
    }
}

/// Timers become runnable only at or after their stored monotonic deadline.
#[derive(Debug)]
pub struct TimerScheduler<T> {
    pending: BinaryHeap<ScheduledTimer<T>>,
    active: HashSet<TimerId>,
    running: HashSet<TimerId>,
    cancelled_running: HashSet<TimerId>,
    next_id: u32,
    next_sequence: u64,
}

impl<T> Default for TimerScheduler<T> {
    fn default() -> Self {
        Self {
            pending: BinaryHeap::new(),
            active: HashSet::new(),
            running: HashSet::new(),
            cancelled_running: HashSet::new(),
            next_id: 1,
            next_sequence: 0,
        }
    }
}

impl<T> TimerScheduler<T> {
    // Internal timeouts such as AbortSignal.timeout accept 64-bit delays.
    // HTML timers apply their own argument conversion before scheduling.
    pub fn schedule_after(&mut self, payload: T, delay_ms: u64, now: Instant) -> TimerId {
        let id = self.allocate_id();
        self.schedule_existing_after(id, payload, delay_ms, now);
        id
    }

    pub fn cancel(&mut self, id: TimerId) -> bool {
        if self.active.remove(&id) {
            return true;
        }
        if self.running.contains(&id) {
            return self.cancelled_running.insert(id);
        }
        false
    }

    pub fn cancel_matching<F>(&mut self, mut predicate: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let mut cancelled = 0;
        for timer in &self.pending {
            if self.active.contains(&timer.id) && predicate(&timer.payload) {
                self.active.remove(&timer.id);
                cancelled += 1;
            }
        }
        cancelled
    }

    pub fn active_payload(&self, id: TimerId) -> Option<&T> {
        self.active
            .contains(&id)
            .then(|| {
                self.pending
                    .iter()
                    .find(|timer| timer.id == id)
                    .map(|timer| &timer.payload)
            })
            .flatten()
    }

    pub fn take_next_ready(&mut self, now: Instant) -> Option<ReadyTimer<T>> {
        loop {
            let timer = self.pending.peek()?;
            if !self.active.contains(&timer.id) {
                let _ = self.pending.pop();
                continue;
            }
            if !timer.is_ready_at(now) {
                return None;
            }

            let Some(timer) = self.pending.pop() else {
                break None;
            };
            self.active.remove(&timer.id);
            self.running.insert(timer.id);
            return Some(ReadyTimer {
                id: timer.id,
                delay_ms: timer.delay_ms,
                payload: timer.payload,
            });
        }
    }

    pub fn take_next_ready_matching<F>(
        &mut self,
        now: Instant,
        mut predicate: F,
    ) -> Option<ReadyTimer<T>>
    where
        F: FnMut(&T) -> bool,
    {
        self.take_next_ready_matching_scheduled(now, |timer| predicate(&timer.payload))
    }

    /// Takes the next ready timer scheduled inside one of `ranges`.
    ///
    /// Timers scheduled by a callback that runs while draining the ranges are
    /// outside those closed ranges and remain pending for a later task turn.
    pub fn take_next_ready_from_schedule_ranges(
        &mut self,
        ranges: &[TimerScheduleRange],
        now: Instant,
    ) -> Option<ReadyTimer<T>> {
        self.take_next_ready_matching_scheduled(now, |timer| {
            schedule_ranges_contain(ranges, timer.sequence)
        })
    }

    fn take_next_ready_matching_scheduled<P>(
        &mut self,
        now: Instant,
        mut predicate: P,
    ) -> Option<ReadyTimer<T>>
    where
        P: FnMut(&ScheduledTimer<T>) -> bool,
    {
        let mut selected = None;
        for timer in &self.pending {
            if !self.active.contains(&timer.id) {
                continue;
            }
            if !timer.is_ready_at(now) {
                continue;
            }
            if predicate(timer) && selected.is_none_or(|current| timer_precedes(timer, current)) {
                selected = Some(timer);
            }
        }

        let selected = selected?;
        let selected_id = selected.id;
        let selected_sequence = selected.sequence;

        let timers = std::mem::take(&mut self.pending).into_vec();
        let mut retained = Vec::with_capacity(timers.len().saturating_sub(1));
        let mut selected = None;
        for timer in timers {
            if selected.is_none() && timer.id == selected_id && timer.sequence == selected_sequence
            {
                selected = Some(timer);
                continue;
            }
            if self.active.contains(&timer.id) {
                retained.push(timer);
            }
        }
        self.pending = BinaryHeap::from(retained);

        let timer = selected?;
        self.active.remove(&timer.id);
        self.running.insert(timer.id);
        Some(ReadyTimer {
            id: timer.id,
            delay_ms: timer.delay_ms,
            payload: timer.payload,
        })
    }

    pub fn has_ready_matching<F>(&self, now: Instant, mut predicate: F) -> bool
    where
        F: FnMut(&T) -> bool,
    {
        self.pending.iter().any(|timer| {
            self.active.contains(&timer.id) && timer.is_ready_at(now) && predicate(&timer.payload)
        })
    }

    pub fn finish_running(&mut self, id: TimerId) {
        self.running.remove(&id);
        self.cancelled_running.remove(&id);
    }

    pub fn reschedule_running_after(
        &mut self,
        id: TimerId,
        payload: T,
        delay_ms: u64,
        now: Instant,
    ) -> bool {
        self.running.remove(&id);
        if self.cancelled_running.remove(&id) {
            return false;
        }
        self.schedule_existing_after(id, payload, delay_ms, now);
        true
    }

    pub fn has_ready_timer(&self, now: Instant) -> bool {
        self.pending
            .iter()
            .any(|timer| self.active.contains(&timer.id) && timer.is_ready_at(now))
    }

    pub fn next_ready_deadline_matching<F>(&self, now: Instant, predicate: F) -> Option<Instant>
    where
        F: FnMut(&T) -> bool,
    {
        self.next_ready_matching_timer(now, predicate)
            .map(|timer| timer.run_at)
    }

    pub fn has_ready_from_schedule_ranges(
        &self,
        ranges: &[TimerScheduleRange],
        now: Instant,
    ) -> bool {
        self.pending.iter().any(|timer| {
            self.active.contains(&timer.id)
                && schedule_ranges_contain(ranges, timer.sequence)
                && timer.is_ready_at(now)
        })
    }

    pub fn schedule_snapshot(&self) -> TimerScheduleSnapshot {
        TimerScheduleSnapshot {
            next_sequence: self.next_sequence,
        }
    }

    pub fn schedule_range_since(&self, start: TimerScheduleSnapshot) -> TimerScheduleRange {
        TimerScheduleRange {
            inclusive_sequence: start.next_sequence,
            exclusive_sequence: self.next_sequence,
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .iter()
            .filter(|timer| self.active.contains(&timer.id))
            .map(|timer| timer.run_at)
            .min()
    }

    pub fn ms_to_next(&self, now: Instant) -> Option<u64> {
        self.next_deadline().map(|deadline| {
            if deadline <= now {
                0
            } else {
                let duration = deadline.duration_since(now);
                let millis = duration.as_millis() as u64;
                millis.max(1)
            }
        })
    }

    pub fn pending_count(&self) -> usize {
        self.active.len()
    }

    fn schedule_existing_after(&mut self, id: TimerId, payload: T, delay_ms: u64, now: Instant) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.active.insert(id);
        self.pending.push(ScheduledTimer {
            id,
            sequence,
            run_at: now + Duration::from_millis(delay_ms),
            delay_ms,
            payload,
        });
    }

    fn allocate_id(&mut self) -> TimerId {
        loop {
            let id = TimerId(self.next_id.max(1));
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if !self.active.contains(&id) && !self.running.contains(&id) {
                return id;
            }
        }
    }

    fn next_ready_matching_timer<F>(
        &self,
        now: Instant,
        mut predicate: F,
    ) -> Option<&ScheduledTimer<T>>
    where
        F: FnMut(&T) -> bool,
    {
        let mut selected = None;
        for timer in &self.pending {
            if !self.active.contains(&timer.id) {
                continue;
            }
            if !timer.is_ready_at(now) {
                continue;
            }
            if predicate(&timer.payload)
                && selected.is_none_or(|current| timer_precedes(timer, current))
            {
                selected = Some(timer);
            }
        }

        selected
    }
}

fn schedule_ranges_contain(ranges: &[TimerScheduleRange], sequence: u64) -> bool {
    ranges.iter().any(|range| range.contains(sequence))
}

fn timer_precedes<T>(left: &ScheduledTimer<T>, right: &ScheduledTimer<T>) -> bool {
    match left.run_at.cmp(&right.run_at) {
        Ordering::Less => true,
        Ordering::Equal => left.sequence < right.sequence,
        Ordering::Greater => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_wait_until_their_deadline_in_every_ready_query() {
        let now = Instant::now();
        for delay_ms in [1, 2, 100] {
            for consume_matching in [false, true] {
                let mut scheduler = TimerScheduler::default();
                let id = scheduler.schedule_after("timer", delay_ms, now);
                let deadline = now + Duration::from_millis(delay_ms);

                for before_deadline in [now, deadline - Duration::from_nanos(1)] {
                    assert!(!scheduler.has_ready_timer(before_deadline));
                    assert!(!scheduler.has_ready_matching(before_deadline, |_| true));
                    assert_eq!(
                        scheduler.next_ready_deadline_matching(before_deadline, |_| true),
                        None
                    );
                    assert!(scheduler.take_next_ready(before_deadline).is_none());
                    assert!(
                        scheduler
                            .take_next_ready_matching(before_deadline, |_| true)
                            .is_none()
                    );
                    assert_eq!(scheduler.next_deadline(), Some(deadline));
                    assert!(scheduler.ms_to_next(before_deadline).unwrap() > 0);
                    assert_eq!(scheduler.pending_count(), 1);
                }

                assert!(scheduler.has_ready_timer(deadline));
                assert!(scheduler.has_ready_matching(deadline, |_| true));
                assert_eq!(
                    scheduler.next_ready_deadline_matching(deadline, |_| true),
                    Some(deadline)
                );
                assert_eq!(scheduler.ms_to_next(deadline), Some(0));
                let ready = if consume_matching {
                    scheduler.take_next_ready_matching(deadline, |_| true)
                } else {
                    scheduler.take_next_ready(deadline)
                }
                .unwrap();
                assert_eq!(ready.id, id);
                assert_eq!(ready.payload, "timer");
                scheduler.finish_running(id);
                assert_eq!(scheduler.pending_count(), 0);
            }
        }
    }

    #[test]
    fn ready_timers_fire_by_deadline_then_sequence() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let slow = scheduler.schedule_after("slow", 20, now);
        let first = scheduler.schedule_after("first", 10, now);
        let second = scheduler.schedule_after("second", 10, now);

        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(10))
            .expect("first timer should be ready");
        assert_eq!(ready.id, first);
        assert_eq!(ready.payload, "first");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(10))
            .expect("second timer should be ready");
        assert_eq!(ready.id, second);
        assert_eq!(ready.payload, "second");
        scheduler.finish_running(ready.id);

        assert!(
            scheduler
                .take_next_ready(now + Duration::from_millis(10))
                .is_none()
        );

        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(20))
            .expect("slow timer should be ready");
        assert_eq!(ready.id, slow);
        assert_eq!(ready.payload, "slow");
        scheduler.finish_running(ready.id);
    }

    #[test]
    fn long_timeouts_preserve_their_full_deadline() {
        let now = Instant::now();
        for delay_ms in [u64::from(u32::MAX) + 1, 9_007_199_254_740_991] {
            let mut scheduler = TimerScheduler::default();
            let id = scheduler.schedule_after("long", delay_ms, now);
            let deadline = now + Duration::from_millis(delay_ms);
            assert_eq!(scheduler.next_deadline(), Some(deadline));
            assert!(
                scheduler
                    .take_next_ready(deadline - Duration::from_millis(1))
                    .is_none()
            );
            let ready = scheduler.take_next_ready(deadline).unwrap();
            assert_eq!(ready.id, id);
            assert_eq!(ready.delay_ms, delay_ms);
            assert!(scheduler.reschedule_running_after(id, ready.payload, delay_ms, deadline));
            assert_eq!(
                scheduler.next_deadline(),
                Some(deadline + Duration::from_millis(delay_ms))
            );
            assert!(scheduler.cancel(id));
            assert_eq!(scheduler.pending_count(), 0);
        }
    }

    #[test]
    fn cancellation_skips_pending_timers() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let cancelled = scheduler.schedule_after("cancelled", 0, now);
        let kept = scheduler.schedule_after("kept", 0, now);

        scheduler.cancel(cancelled);

        let ready = scheduler
            .take_next_ready(now)
            .expect("kept timer should be ready");
        assert_eq!(ready.id, kept);
        assert_eq!(ready.payload, "kept");
        scheduler.finish_running(ready.id);
        assert!(scheduler.take_next_ready(now).is_none());
    }

    #[test]
    fn cancel_while_running_prevents_interval_reschedule() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let interval = scheduler.schedule_after("tick", 0, now);

        let ready = scheduler
            .take_next_ready(now)
            .expect("interval should be ready");
        scheduler.cancel(interval);
        assert!(!scheduler.reschedule_running_after(
            ready.id,
            ready.payload,
            ready.delay_ms.max(1),
            now
        ));
        assert_eq!(scheduler.pending_count(), 0);
    }

    #[test]
    fn active_deadline_queries_ignore_cancelled_timers() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let cancelled = scheduler.schedule_after("cancelled", 5, now);
        let kept = scheduler.schedule_after("kept", 12, now);

        scheduler.cancel(cancelled);

        assert_eq!(
            scheduler.next_deadline(),
            Some(now + Duration::from_millis(12))
        );
        assert_eq!(
            scheduler.ms_to_next(now + Duration::from_millis(8)),
            Some(4)
        );
        assert_eq!(scheduler.pending_count(), 1);

        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(12))
            .expect("kept timer should become ready");
        assert_eq!(ready.id, kept);
        scheduler.finish_running(ready.id);
    }

    #[test]
    fn matching_ready_deadline_observes_selection_and_cancelled_timers() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let cancelled = scheduler.schedule_after("cancelled", 0, now);
        scheduler.schedule_after("skipped", 0, now);
        scheduler.schedule_after("selected", 1, now);
        scheduler.cancel(cancelled);

        assert_eq!(
            scheduler.next_ready_deadline_matching(now, |payload| *payload == "selected",),
            None
        );
        assert_eq!(
            scheduler
                .next_ready_deadline_matching(now + Duration::from_millis(1), |payload| *payload
                    == "selected",),
            Some(now + Duration::from_millis(1))
        );
    }

    #[test]
    fn matching_ready_deadline_does_not_admit_a_future_timer() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        scheduler.schedule_after("earlier", 2, now);
        scheduler.schedule_after("selected", 1, now + Duration::from_micros(1_500));
        let before_selected = now + Duration::from_micros(2_499);
        assert!(scheduler.has_ready_timer(before_selected));
        assert_eq!(
            scheduler
                .next_ready_deadline_matching(before_selected, |payload| *payload == "selected"),
            None
        );
        assert!(
            scheduler
                .take_next_ready_matching(before_selected, |payload| *payload == "selected")
                .is_none()
        );
        assert_eq!(
            scheduler.next_ready_deadline_matching(now + Duration::from_micros(2_500), |payload| {
                *payload == "selected"
            },),
            Some(now + Duration::from_micros(2_500))
        );
    }

    #[test]
    fn ms_to_next_rounds_future_submillisecond_deadline_up() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        scheduler.schedule_after("soon", 1, now);

        assert_eq!(
            scheduler.ms_to_next(now + Duration::from_micros(500)),
            Some(1),
            "future timer deadlines must not be reported as immediate"
        );
        assert_eq!(
            scheduler.ms_to_next(now + Duration::from_millis(1)),
            Some(0)
        );
    }

    #[test]
    fn rescheduled_short_timer_waits_for_its_new_deadline() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let id = scheduler.schedule_after("tick", 1, now);
        let first_deadline = now + Duration::from_millis(1);
        let ready = scheduler.take_next_ready(first_deadline).unwrap();
        assert!(scheduler.reschedule_running_after(id, ready.payload, 1, first_deadline));

        let next_deadline = first_deadline + Duration::from_millis(1);
        for before_deadline in [first_deadline, next_deadline - Duration::from_nanos(1)] {
            assert!(!scheduler.has_ready_timer(before_deadline));
            assert!(scheduler.take_next_ready(before_deadline).is_none());
            assert_eq!(scheduler.next_deadline(), Some(next_deadline));
        }
        let ready = scheduler.take_next_ready(next_deadline).unwrap();
        assert_eq!(ready.id, id);
        assert_eq!(ready.payload, "tick");
        scheduler.finish_running(id);
        assert_eq!(scheduler.pending_count(), 0);
    }

    #[test]
    fn matching_ready_timer_preserves_other_ready_timers() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let first = scheduler.schedule_after("first", 0, now);
        let selected = scheduler.schedule_after("selected", 0, now);
        let second = scheduler.schedule_after("second", 0, now);

        assert!(scheduler.has_ready_matching(now, |payload| { *payload == "selected" }));
        let ready = scheduler
            .take_next_ready_matching(now, |payload| *payload == "selected")
            .expect("selected timer should be ready");
        assert_eq!(ready.id, selected);
        assert_eq!(ready.payload, "selected");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now)
            .expect("first timer should remain pending");
        assert_eq!(ready.id, first);
        assert_eq!(ready.payload, "first");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now)
            .expect("second timer should remain pending");
        assert_eq!(ready.id, second);
        assert_eq!(ready.payload, "second");
        scheduler.finish_running(ready.id);
    }

    #[test]
    fn matching_ready_timer_skips_many_without_reordering_remaining_timers() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let first = scheduler.schedule_after("first", 0, now);
        let skipped = (0..64)
            .map(|_| scheduler.schedule_after("skipped", 0, now))
            .collect::<Vec<_>>();
        let selected = scheduler.schedule_after("selected", 0, now);

        let ready = scheduler
            .take_next_ready_matching(now, |payload| *payload == "selected")
            .expect("selected timer should be ready after skipped timers");
        assert_eq!(ready.id, selected);
        assert_eq!(ready.payload, "selected");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now)
            .expect("first timer should remain first after matching drain");
        assert_eq!(ready.id, first);
        assert_eq!(ready.payload, "first");
        scheduler.finish_running(ready.id);

        for skipped_id in skipped {
            let ready = scheduler
                .take_next_ready(now)
                .expect("skipped timer should remain pending");
            assert_eq!(ready.id, skipped_id);
            assert_eq!(ready.payload, "skipped");
            scheduler.finish_running(ready.id);
        }
    }

    #[test]
    fn schedule_ranges_select_only_timers_queued_inside_owner_operations() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let earlier = scheduler.schedule_after("earlier", 0, now);
        let first_start = scheduler.schedule_snapshot();
        let first = scheduler.schedule_after("first", 0, now);
        let first_range = scheduler.schedule_range_since(first_start);
        let between = scheduler.schedule_after("between", 0, now);
        let second_start = scheduler.schedule_snapshot();
        let second = scheduler.schedule_after("second", 0, now);
        let second_range = scheduler.schedule_range_since(second_start);
        let later = scheduler.schedule_after("later", 0, now);
        let ranges = [first_range, second_range];

        assert!(scheduler.has_ready_from_schedule_ranges(&ranges, now));
        for (expected_id, expected_payload) in [(first, "first"), (second, "second")] {
            let ready = scheduler
                .take_next_ready_from_schedule_ranges(&ranges, now)
                .expect("timer scheduled inside an owner range should be ready");
            assert_eq!(ready.id, expected_id);
            assert_eq!(ready.payload, expected_payload);
            scheduler.finish_running(ready.id);
        }

        assert!(!scheduler.has_ready_from_schedule_ranges(&ranges, now));
        assert!(
            scheduler
                .take_next_ready_from_schedule_ranges(&ranges, now)
                .is_none(),
            "timers outside the owner ranges must remain for later task turns"
        );
        for (expected_id, expected_payload) in
            [(earlier, "earlier"), (between, "between"), (later, "later")]
        {
            let ready = scheduler
                .take_next_ready(now)
                .expect("timer outside the ranges should remain in the ordinary queue");
            assert_eq!(ready.id, expected_id);
            assert_eq!(ready.payload, expected_payload);
            scheduler.finish_running(ready.id);
        }
    }

    #[test]
    fn interval_rescheduled_after_range_is_not_drained_twice() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let start = scheduler.schedule_snapshot();
        let interval = scheduler.schedule_after("tick", 0, now);
        let range = scheduler.schedule_range_since(start);

        let ready = scheduler
            .take_next_ready_from_schedule_ranges(&[range], now)
            .expect("initial interval task should belong to the range");
        assert_eq!(ready.id, interval);
        assert!(scheduler.reschedule_running_after(ready.id, ready.payload, 1, now));

        assert!(
            scheduler
                .take_next_ready_from_schedule_ranges(&[range], now + Duration::from_millis(1),)
                .is_none(),
            "an interval's newly scheduled task must not re-enter the closed range"
        );
        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(1))
            .expect("rescheduled interval should remain in the ordinary queue");
        assert_eq!(ready.id, interval);
        scheduler.finish_running(ready.id);
    }
}
