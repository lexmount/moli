use crate::CssTime;
use std::time::{Duration, Instant};

/// A monotonic timeline anchor and a hold time, in signed local seconds.
/// Pausing, seeking and changing speed all preserve the sampled local time.
#[derive(Debug)]
pub struct AnimationClock {
    sampled_at: Instant,
    // Start time is relative to sampled_at, not derived by dividing local time
    // by the playback rate. In particular a zero-rate animation can have both
    // an explicit start time and a hold time.
    start: Option<f64>,
    hold: Option<f64>,
    pub rate: f64,
}

impl AnimationClock {
    pub fn new(now: Instant, paused: bool) -> Self {
        Self {
            sampled_at: now,
            start: (!paused).then_some(0.0),
            hold: paused.then_some(0.0),
            rate: 1.0,
        }
    }
    pub fn resolved_seconds_at(&self, now: Instant) -> Option<f64> {
        self.hold.or_else(|| {
            self.start.map(|start| {
                // A delayed CSS update can carry a timestamp preceding this
                // anchor. Do not undo the latest playback command. Explicit
                // future start times still produce negative local times.
                (now.saturating_duration_since(self.sampled_at).as_secs_f64() - start) * self.rate
            })
        })
    }
    pub fn seconds_at(&self, now: Instant) -> f64 {
        self.resolved_seconds_at(now).unwrap_or(0.0)
    }
    /// Clearing a start time preserves its previously exposed current time,
    /// including an unresolved time, rather than sampling the raw anchor again.
    pub fn hold(&mut self, now: Instant, time: Option<f64>) {
        self.sampled_at = now;
        self.start = None;
        self.hold = time;
    }
    pub fn update_play_state(&mut self, now: Instant, paused: bool) {
        let now = now.max(self.sampled_at);
        let time = self.resolved_seconds_at(now);
        self.sampled_at = now;
        self.start = if paused {
            None
        } else {
            time.map(|time| {
                if self.rate == 0.0 {
                    0.0
                } else {
                    -time / self.rate
                }
            })
        };
        self.hold = time.filter(|_| paused || self.rate == 0.0);
    }
    pub fn seek(&mut self, now: Instant, seconds: f64) {
        if self.start.is_none() || self.rate == 0.0 {
            self.hold = Some(seconds);
        } else {
            self.sampled_at = now;
            self.start = Some(-seconds / self.rate);
            self.hold = None;
        }
    }
    pub fn set_rate(&mut self, now: Instant, rate: f64) {
        let paused = self.is_paused();
        let local = self.resolved_seconds_at(now);
        self.rate = rate;
        if let Some(local) = local {
            if rate == 0.0 {
                self.hold = Some(local);
            } else if !paused {
                self.sampled_at = now;
                self.start = Some(-local / rate);
                self.hold = None;
            }
        }
    }
    pub fn has_start_time(&self) -> bool {
        self.start.is_some()
    }
    pub fn start_millis(&self, origin: Instant) -> Option<f64> {
        Some((signed_seconds(self.sampled_at, origin) + self.start?) * 1000.0)
    }
    pub fn set_start_time(
        &mut self,
        now: Instant,
        origin: Instant,
        start: Option<f64>,
        previous: Option<f64>,
    ) {
        self.sampled_at = now;
        self.start = start.map(|start| signed_seconds(origin, now) + start);
        self.hold = previous.filter(|_| start.is_none() || self.rate == 0.0);
    }

    pub fn is_paused(&self) -> bool {
        self.start.is_none() && self.hold.is_some()
    }
    pub fn deadline(&self, local_time: CssTime) -> Option<Instant> {
        if !self.has_start_time() || self.rate == 0.0 {
            return None;
        }
        let wait = (local_time.seconds() - self.resolved_seconds_at(self.sampled_at)?) / self.rate;
        self.sampled_at
            .checked_add(Duration::try_from_secs_f64(wait.max(0.0)).ok()?)
    }
}
fn signed_seconds(left: Instant, right: Instant) -> f64 {
    if left >= right {
        left.duration_since(right).as_secs_f64()
    } else {
        -right.duration_since(left).as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration;
    #[test]
    fn an_earlier_sample_does_not_rewind_the_latest_clock_anchor() {
        let now = Instant::now();
        let mut clock = AnimationClock::new(now, false);
        let earlier = now - Duration::from_millis(20);
        assert_eq!(clock.resolved_seconds_at(earlier), Some(0.0));
        clock.set_start_time(now, now, Some(1.0), None);
        assert_eq!(clock.resolved_seconds_at(now), Some(-1.0));
        clock.seek(now, 0.5);
        assert_eq!(clock.resolved_seconds_at(earlier), Some(0.5));
    }
    #[test]
    fn playback_clock_excludes_paused_time_and_resumes_from_its_hold_time() {
        let now = Instant::now();
        let mut clock = AnimationClock::new(now, false);
        clock.update_play_state(now + Duration::from_millis(20), true);
        assert_eq!(
            duration(clock.seconds_at(now + Duration::from_secs(10))),
            Duration::from_millis(20)
        );
        assert_eq!(clock.deadline(CssTime::from_seconds(0.04).unwrap()), None);
        clock.update_play_state(now + Duration::from_secs(10), false);
        assert_eq!(
            duration(clock.seconds_at(now + Duration::from_millis(10030))),
            Duration::from_millis(50)
        );
    }
}
