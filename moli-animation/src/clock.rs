use crate::CssTime;
use std::time::{Duration, Instant};

/// A monotonic timeline anchor and a hold time, in signed local seconds.
/// Pausing, seeking and changing speed all preserve the sampled local time.
#[derive(Debug)]
pub struct AnimationClock {
    sampled_at: Instant,
    position: ClockPosition,
    pub rate: f64,
}

#[derive(Debug)]
enum ClockPosition {
    Unresolved,
    Hold(f64),
    Started(f64),
}

impl AnimationClock {
    pub fn new(now: Instant, paused: bool) -> Self {
        Self {
            sampled_at: now,
            position: if paused {
                ClockPosition::Hold(0.0)
            } else {
                ClockPosition::Started(0.0)
            },
            rate: 1.0,
        }
    }
    pub fn resolved_seconds_at(&self, now: Instant) -> Option<f64> {
        match self.position {
            ClockPosition::Unresolved => None,
            ClockPosition::Hold(time) => Some(time),
            ClockPosition::Started(time) => Some(
                time + now.saturating_duration_since(self.sampled_at).as_secs_f64() * self.rate,
            ),
        }
    }
    pub fn seconds_at(&self, now: Instant) -> f64 {
        self.resolved_seconds_at(now).unwrap_or(0.0)
    }
    /// Clearing a start time preserves its previously exposed current time,
    /// including an unresolved time, rather than sampling the raw anchor again.
    pub fn hold(&mut self, now: Instant, time: Option<f64>) {
        self.sampled_at = now;
        self.position = time.map_or(ClockPosition::Unresolved, ClockPosition::Hold);
    }
    pub fn update_play_state(&mut self, now: Instant, paused: bool) {
        let now = now.max(self.sampled_at);
        self.position = match self.resolved_seconds_at(now) {
            None => ClockPosition::Unresolved,
            Some(time) if paused => ClockPosition::Hold(time),
            Some(time) => ClockPosition::Started(time),
        };
        self.sampled_at = now;
    }
    pub fn seek(&mut self, now: Instant, seconds: f64) {
        self.position = if self.is_paused() {
            ClockPosition::Hold(seconds)
        } else {
            ClockPosition::Started(seconds)
        };
        self.sampled_at = now;
    }
    pub fn set_rate(&mut self, now: Instant, rate: f64) {
        self.update_play_state(now, self.is_paused());
        self.rate = rate;
    }
    pub fn has_start_time(&self) -> bool {
        matches!(self.position, ClockPosition::Started(_))
    }
    pub fn start_millis(&self, origin: Instant) -> Option<f64> {
        if !self.has_start_time() || self.rate == 0.0 {
            return None;
        }
        let elapsed = if self.sampled_at >= origin {
            self.sampled_at.duration_since(origin).as_secs_f64()
        } else {
            -origin.duration_since(self.sampled_at).as_secs_f64()
        };
        Some((elapsed - self.resolved_seconds_at(self.sampled_at)? / self.rate) * 1000.0)
    }

    pub fn is_paused(&self) -> bool {
        matches!(self.position, ClockPosition::Hold(_))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration;
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
