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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerReadyAllowance {
    pub max_delay_ms: u32,
    pub allowance: Duration,
}

impl TimerReadyAllowance {
    pub const NONE: Self = Self {
        max_delay_ms: 0,
        allowance: Duration::ZERO,
    };
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

    pub fn take_next_ready(
        &mut self,
        now: Instant,
        allowance: TimerReadyAllowance,
    ) -> Option<ReadyTimer<T>> {
        loop {
            let timer = self.pending.peek()?;
            if !self.active.contains(&timer.id) {
                let _ = self.pending.pop();
                continue;
            }
            if !timer_ready(timer.run_at, timer.delay_ms, now, allowance) {
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
        allowance: TimerReadyAllowance,
        predicate: F,
    ) -> Option<ReadyTimer<T>>
    where
        F: FnMut(&T) -> bool,
    {
        let selected = self.next_ready_matching_timer(now, allowance, predicate)?;
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

    pub fn has_ready_matching<F>(
        &self,
        now: Instant,
        allowance: TimerReadyAllowance,
        mut predicate: F,
    ) -> bool
    where
        F: FnMut(&T) -> bool,
    {
        self.pending.iter().any(|timer| {
            self.active.contains(&timer.id)
                && timer_ready(timer.run_at, timer.delay_ms, now, allowance)
                && predicate(&timer.payload)
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

    pub fn has_ready_timer(&self, now: Instant, allowance: TimerReadyAllowance) -> bool {
        self.pending.iter().any(|timer| {
            self.active.contains(&timer.id)
                && timer_ready(timer.run_at, timer.delay_ms, now, allowance)
        })
    }

    pub fn next_ready_deadline_matching<F>(
        &self,
        now: Instant,
        allowance: TimerReadyAllowance,
        predicate: F,
    ) -> Option<Instant>
    where
        F: FnMut(&T) -> bool,
    {
        self.next_ready_matching_timer(now, allowance, predicate)
            .map(|timer| timer.run_at)
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
        allowance: TimerReadyAllowance,
        mut predicate: F,
    ) -> Option<&ScheduledTimer<T>>
    where
        F: FnMut(&T) -> bool,
    {
        let mut first_non_ready = None;
        let mut selected = None;
        for timer in &self.pending {
            if !self.active.contains(&timer.id) {
                continue;
            }
            if !timer_ready(timer.run_at, timer.delay_ms, now, allowance) {
                if first_non_ready.is_none_or(|current| timer_precedes(timer, current)) {
                    first_non_ready = Some(timer);
                }
                continue;
            }
            if predicate(&timer.payload)
                && selected.is_none_or(|current| timer_precedes(timer, current))
            {
                selected = Some(timer);
            }
        }

        let selected = selected?;
        if first_non_ready.is_some_and(|barrier| timer_precedes(barrier, selected)) {
            return None;
        }
        Some(selected)
    }
}

fn timer_precedes<T>(left: &ScheduledTimer<T>, right: &ScheduledTimer<T>) -> bool {
    match left.run_at.cmp(&right.run_at) {
        Ordering::Less => true,
        Ordering::Equal => left.sequence < right.sequence,
        Ordering::Greater => false,
    }
}

fn timer_ready(
    run_at: Instant,
    delay_ms: u64,
    now: Instant,
    allowance: TimerReadyAllowance,
) -> bool {
    run_at <= now
        || (delay_ms <= u64::from(allowance.max_delay_ms)
            && run_at.duration_since(now).le(&allowance.allowance))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_timers_fire_by_deadline_then_sequence() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let slow = scheduler.schedule_after("slow", 20, now);
        let first = scheduler.schedule_after("first", 10, now);
        let second = scheduler.schedule_after("second", 10, now);

        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(10), TimerReadyAllowance::NONE)
            .expect("first timer should be ready");
        assert_eq!(ready.id, first);
        assert_eq!(ready.payload, "first");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(10), TimerReadyAllowance::NONE)
            .expect("second timer should be ready");
        assert_eq!(ready.id, second);
        assert_eq!(ready.payload, "second");
        scheduler.finish_running(ready.id);

        assert!(
            scheduler
                .take_next_ready(now + Duration::from_millis(10), TimerReadyAllowance::NONE)
                .is_none()
        );

        let ready = scheduler
            .take_next_ready(now + Duration::from_millis(20), TimerReadyAllowance::NONE)
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
                    .take_next_ready(
                        deadline - Duration::from_millis(1),
                        TimerReadyAllowance::NONE
                    )
                    .is_none()
            );
            let ready = scheduler
                .take_next_ready(deadline, TimerReadyAllowance::NONE)
                .unwrap();
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
            .take_next_ready(now, TimerReadyAllowance::NONE)
            .expect("kept timer should be ready");
        assert_eq!(ready.id, kept);
        assert_eq!(ready.payload, "kept");
        scheduler.finish_running(ready.id);
        assert!(
            scheduler
                .take_next_ready(now, TimerReadyAllowance::NONE)
                .is_none()
        );
    }

    #[test]
    fn cancel_while_running_prevents_interval_reschedule() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let interval = scheduler.schedule_after("tick", 0, now);

        let ready = scheduler
            .take_next_ready(now, TimerReadyAllowance::NONE)
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
            .take_next_ready(now + Duration::from_millis(12), TimerReadyAllowance::NONE)
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
            scheduler
                .next_ready_deadline_matching(now, TimerReadyAllowance::NONE, |payload| *payload
                    == "selected",),
            None
        );
        assert_eq!(
            scheduler.next_ready_deadline_matching(
                now + Duration::from_millis(1),
                TimerReadyAllowance::NONE,
                |payload| *payload == "selected",
            ),
            Some(now + Duration::from_millis(1))
        );
    }

    #[test]
    fn matching_ready_deadline_respects_an_earlier_non_ready_barrier() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        scheduler.schedule_after("barrier", 2, now);
        scheduler.schedule_after("selected", 1, now + Duration::from_micros(1_500));
        let allowance = TimerReadyAllowance {
            max_delay_ms: 1,
            allowance: Duration::from_millis(1),
        };

        assert_eq!(
            scheduler.next_ready_deadline_matching(
                now + Duration::from_micros(1_500),
                allowance,
                |payload| *payload == "selected",
            ),
            None,
            "a later timer admitted by early allowance must not overtake the heap head"
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
    fn early_allowance_only_applies_to_short_delays() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let short = scheduler.schedule_after("short", 1, now);
        scheduler.schedule_after("long", 2, now);

        let allowance = TimerReadyAllowance {
            max_delay_ms: 1,
            allowance: Duration::from_millis(1),
        };
        let just_before = now + Duration::from_micros(500);
        assert!(scheduler.has_ready_timer(just_before, allowance));
        let ready = scheduler
            .take_next_ready(just_before, allowance)
            .expect("short timer should be ready within allowance");
        assert_eq!(ready.id, short);
        scheduler.finish_running(ready.id);

        assert!(!scheduler.has_ready_timer(just_before, allowance));
    }

    #[test]
    fn matching_ready_timer_preserves_other_ready_timers() {
        let now = Instant::now();
        let mut scheduler = TimerScheduler::default();
        let first = scheduler.schedule_after("first", 0, now);
        let selected = scheduler.schedule_after("selected", 0, now);
        let second = scheduler.schedule_after("second", 0, now);

        assert!(
            scheduler.has_ready_matching(now, TimerReadyAllowance::NONE, |payload| {
                *payload == "selected"
            })
        );
        let ready = scheduler
            .take_next_ready_matching(now, TimerReadyAllowance::NONE, |payload| {
                *payload == "selected"
            })
            .expect("selected timer should be ready");
        assert_eq!(ready.id, selected);
        assert_eq!(ready.payload, "selected");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now, TimerReadyAllowance::NONE)
            .expect("first timer should remain pending");
        assert_eq!(ready.id, first);
        assert_eq!(ready.payload, "first");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now, TimerReadyAllowance::NONE)
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
            .take_next_ready_matching(now, TimerReadyAllowance::NONE, |payload| {
                *payload == "selected"
            })
            .expect("selected timer should be ready after skipped timers");
        assert_eq!(ready.id, selected);
        assert_eq!(ready.payload, "selected");
        scheduler.finish_running(ready.id);

        let ready = scheduler
            .take_next_ready(now, TimerReadyAllowance::NONE)
            .expect("first timer should remain first after matching drain");
        assert_eq!(ready.id, first);
        assert_eq!(ready.payload, "first");
        scheduler.finish_running(ready.id);

        for skipped_id in skipped {
            let ready = scheduler
                .take_next_ready(now, TimerReadyAllowance::NONE)
                .expect("skipped timer should remain pending");
            assert_eq!(ready.id, skipped_id);
            assert_eq!(ready.payload, "skipped");
            scheduler.finish_running(ready.id);
        }
    }
}
