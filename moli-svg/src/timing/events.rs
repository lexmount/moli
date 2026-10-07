//! Animation timing events are separate from value sampling. Normal clock
//! progress visits interval boundaries; a seek only changes active membership.

use super::{SvgAnimationInstanceTimes, SvgAnimationInterval, SvgAnimationTiming};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SvgAnimationEventKind {
    Begin,
    Repeat(f64),
    End,
}

impl SvgAnimationEventKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Begin => "beginEvent",
            Self::Repeat(_) => "repeatEvent",
            Self::End => "endEvent",
        }
    }

    pub fn order(self) -> u8 {
        match self {
            Self::End => 0,
            Self::Begin => 1,
            Self::Repeat(_) => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgAnimationEvent {
    pub time: f64,
    pub kind: SvgAnimationEventKind,
}

/// An owned, lazy interval traversal. A renderer can yield between events
/// without allocating a vector proportional to the repeat count.
#[derive(Debug)]
pub struct SvgAnimationEvents {
    intervals: std::vec::IntoIter<SvgAnimationInterval>,
    current: Option<SvgAnimationInterval>,
    duration: f64,
    after: f64,
    through: f64,
    phase: u8,
    repetition: f64,
}

impl SvgAnimationTiming {
    pub fn active_interval(
        &self,
        instances: &SvgAnimationInstanceTimes,
        time: f64,
    ) -> Option<SvgAnimationInterval> {
        self.intervals(instances)
            .into_iter()
            .find(|interval| interval.begin <= time && time < interval.end)
    }

    pub fn events_between(
        &self,
        instances: &SvgAnimationInstanceTimes,
        after: f64,
        through: f64,
    ) -> SvgAnimationEvents {
        SvgAnimationEvents {
            intervals: self.intervals(instances).into_iter(),
            current: None,
            duration: self.simple_duration,
            after,
            through,
            phase: 0,
            repetition: 1.0,
        }
    }
}

impl SvgAnimationEvent {
    pub fn active_transition(
        before: Option<SvgAnimationInterval>,
        after: Option<SvgAnimationInterval>,
        time: f64,
    ) -> impl Iterator<Item = Self> {
        let changed = before.map(|interval| interval.begin) != after.map(|interval| interval.begin);
        [
            (changed && before.is_some()).then_some(Self {
                time,
                kind: SvgAnimationEventKind::End,
            }),
            (changed && after.is_some()).then_some(Self {
                time,
                kind: SvgAnimationEventKind::Begin,
            }),
        ]
        .into_iter()
        .flatten()
    }
}

impl SvgAnimationEvents {
    /// The next boundary that can run begin/end handlers. This permits a
    /// renderer to skip unobserved repeat events only up to the next point
    /// where author code could install a repeat listener.
    pub fn next_non_repeat_time(&self) -> Option<f64> {
        let current = self.current.into_iter().filter_map(|interval| {
            if self.phase == 0 && self.contains(interval.begin.max(0.0)) {
                Some(interval.begin.max(0.0))
            } else {
                self.contains(interval.end).then_some(interval.end)
            }
        });
        current
            .chain(self.intervals.as_slice().iter().filter_map(|interval| {
                if self.contains(interval.begin.max(0.0)) {
                    Some(interval.begin.max(0.0))
                } else {
                    self.contains(interval.end).then_some(interval.end)
                }
            }))
            .min_by(f64::total_cmp)
    }

    pub fn skip_repeats_before(&mut self, time: f64) {
        if self.phase == 1
            && self.duration.is_finite()
            && self.duration > 0.0
            && let Some(interval) = self.current
        {
            self.repetition = self
                .repetition
                .max(((time - interval.begin) / self.duration).ceil());
        }
    }

    fn contains(&self, time: f64) -> bool {
        time.is_finite() && time >= 0.0 && self.after < time && time <= self.through
    }
}

impl Iterator for SvgAnimationEvents {
    type Item = SvgAnimationEvent;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let interval = if let Some(interval) = self.current {
                interval
            } else {
                let interval = self.intervals.next()?;
                self.current = Some(interval);
                self.phase = 0;
                self.repetition = ((self.after.max(0.0) - interval.begin) / self.duration)
                    .floor()
                    .max(0.0)
                    + 1.0;
                interval
            };
            let (time, kind) = match self.phase {
                0 => {
                    self.phase = 1;
                    (interval.begin.max(0.0), SvgAnimationEventKind::Begin)
                }
                1 if self.duration.is_finite() && self.duration > 0.0 => {
                    let time = interval.begin + self.repetition * self.duration;
                    if !time.is_finite() || time >= interval.end || time > self.through {
                        self.phase = 2;
                        continue;
                    }
                    let repetition = self.repetition;
                    self.repetition = (repetition + 1.0).max(repetition.next_up());
                    (time, SvgAnimationEventKind::Repeat(repetition))
                }
                1 => {
                    self.phase = 2;
                    continue;
                }
                _ => {
                    self.current = None;
                    (interval.end, SvgAnimationEventKind::End)
                }
            };
            if self.contains(time) && (interval.end > 0.0 || interval.begin >= 0.0) {
                return Some(SvgAnimationEvent { time, kind });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SvgAnimationRestart;

    fn timing(duration: f64, count: f64) -> SvgAnimationTiming {
        SvgAnimationTiming {
            begins: vec![0.0],
            ends: vec![],
            simple_duration: duration,
            repeat_count: count,
            repeat_duration: f64::INFINITY,
            freeze: false,
            restart: SvgAnimationRestart::Always,
        }
    }

    #[test]
    fn normal_progress_includes_begin_repeats_and_terminal_end_once() {
        let timing = timing(1.0, 3.0);
        let instances = SvgAnimationInstanceTimes::default();
        let rows: Vec<_> = timing.events_between(&instances, -1.0, 3.0).collect();
        assert_eq!(
            rows.iter().map(|event| event.time).collect::<Vec<_>>(),
            [0.0, 1.0, 2.0, 3.0]
        );
        assert_eq!(
            rows.iter().map(|event| event.kind).collect::<Vec<_>>(),
            [
                SvgAnimationEventKind::Begin,
                SvgAnimationEventKind::Repeat(1.0),
                SvgAnimationEventKind::Repeat(2.0),
                SvgAnimationEventKind::End
            ]
        );
        assert!(timing.events_between(&instances, 3.0, 4.0).next().is_none());
    }

    #[test]
    fn seeking_changes_active_membership_without_skipped_repeat_events() {
        let timing = timing(1.0, 3.0);
        let instances = SvgAnimationInstanceTimes::default();
        let first = timing.active_interval(&instances, 0.5);
        assert!(
            SvgAnimationEvent::active_transition(
                first,
                timing.active_interval(&instances, 2.5),
                2.5
            )
            .next()
            .is_none()
        );
        let rows: Vec<_> = SvgAnimationEvent::active_transition(first, None, 5.0).collect();
        assert_eq!(
            rows,
            [SvgAnimationEvent {
                time: 5.0,
                kind: SvgAnimationEventKind::End
            }]
        );
        assert_eq!(
            SvgAnimationEvent::active_transition(None, first, 0.5)
                .next()
                .unwrap()
                .kind,
            SvgAnimationEventKind::Begin
        );
    }

    #[test]
    fn restarted_intervals_end_before_the_replacement_begins() {
        let mut timing = timing(3.0, 1.0);
        timing.begins.push(1.0);
        let rows: Vec<_> = timing
            .events_between(&SvgAnimationInstanceTimes::default(), -1.0, 4.0)
            .collect();
        assert_eq!(
            rows.iter()
                .map(|event| (event.time, event.kind))
                .collect::<Vec<_>>(),
            [
                (0.0, SvgAnimationEventKind::Begin),
                (1.0, SvgAnimationEventKind::End),
                (1.0, SvgAnimationEventKind::Begin),
                (4.0, SvgAnimationEventKind::End)
            ]
        );
    }

    #[test]
    fn repeat_duration_and_fractional_counts_preserve_the_final_partial_iteration() {
        let mut timing = timing(2.0, 2.5);
        timing.repeat_duration = 4.5;
        let rows: Vec<_> = timing
            .events_between(&SvgAnimationInstanceTimes::default(), -1.0, 5.0)
            .collect();
        assert_eq!(
            rows.iter().map(|event| event.time).collect::<Vec<_>>(),
            [0.0, 2.0, 4.0, 4.5]
        );
    }

    #[test]
    fn enormous_repeat_counts_are_lazy_and_unobserved_repeats_can_be_skipped() {
        let timing = timing(0.001, 1e30);
        let mut events = timing.events_between(&SvgAnimationInstanceTimes::default(), -1.0, 1e20);
        assert_eq!(events.next().unwrap().kind, SvgAnimationEventKind::Begin);
        assert_eq!(
            events.next().unwrap().kind,
            SvgAnimationEventKind::Repeat(1.0)
        );
        events.skip_repeats_before(1e20);
        let next = events.next();
        assert!(next.is_none() || next.unwrap().time >= 1e20);
    }

    #[test]
    fn zero_duration_and_intervals_before_document_begin_do_not_loop() {
        let instances = SvgAnimationInstanceTimes::default();
        for count in [3.0, f64::INFINITY] {
            let zero_duration = timing(0.0, count);
            let rows: Vec<_> = zero_duration
                .events_between(&instances, -1.0, 1.0)
                .collect();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].kind, SvgAnimationEventKind::Begin);
            assert_eq!(rows[1].kind, SvgAnimationEventKind::End);
            assert_eq!(rows[1].time, 0.0);
        }
        let mut ended = timing(1.0, 1.0);
        for begin in [-5.0, -1.0] {
            ended.begins = vec![begin];
            assert!(ended.events_between(&instances, -1.0, 1.0).next().is_none());
        }
    }
}
