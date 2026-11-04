//! Presentation clocks and interval evaluation for SVG declarative animation.
//! Wall time is supplied by the owner, so seeking never rewrites DOM attributes
//! and paused clocks can be sampled deterministically.

#[derive(Debug, Default)]
pub struct SvgPresentationClock {
    started: bool,
    paused: bool,
    position: f64,
    anchor: f64,
}

impl SvgPresentationClock {
    pub fn start(&mut self, now: f64) {
        if !self.started {
            self.started = true;
            self.anchor = now;
        }
    }

    pub fn current_time(&self, now: f64) -> f64 {
        if !self.started {
            0.0
        } else if self.paused {
            self.position
        } else {
            self.position + (now - self.anchor).max(0.0)
        }
    }

    pub fn seek(&mut self, now: f64, seconds: f64) {
        self.position = seconds.max(0.0);
        self.anchor = now;
    }

    pub fn pause(&mut self, now: f64) {
        if !self.paused {
            if self.started {
                self.position = self.current_time(now);
            }
            self.anchor = now;
            self.paused = true;
        }
    }

    pub fn unpause(&mut self, now: f64) {
        if self.paused {
            self.anchor = now;
            self.paused = false;
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn has_started(&self) -> bool {
        self.started
    }
}

/// SMIL clock values. Offsets may have a sign; simple durations may not.
/// Reject non-grammar Rust float spellings such as NaN, infinity and exponents.
pub fn parse_clock_value(raw: &str) -> Option<f64> {
    let raw = raw.trim_matches(|c: char| c.is_ascii_whitespace());
    let (sign, raw) = if let Some(raw) = raw.strip_prefix('-') {
        (-1.0, raw)
    } else {
        (1.0, raw.strip_prefix('+').unwrap_or(raw))
    };
    fn decimal(raw: &str) -> Option<f64> {
        let mut dots = 0;
        let mut digits = 0;
        for c in raw.bytes() {
            if c == b'.' {
                dots += 1;
            } else if c.is_ascii_digit() {
                digits += 1;
            } else {
                return None;
            }
        }
        if digits == 0 || dots > 1 {
            return None;
        }
        raw.parse::<f64>().ok().filter(|v| v.is_finite())
    }
    let parts: Vec<_> = raw.split(':').collect();
    let seconds = match parts.as_slice() {
        [offset] => {
            let (number, scale) = if let Some(n) = offset.strip_suffix("ms") {
                (n, 0.001)
            } else if let Some(n) = offset.strip_suffix("min") {
                (n, 60.0)
            } else if let Some(n) = offset.strip_suffix('h') {
                (n, 3600.0)
            } else {
                (offset.strip_suffix('s').unwrap_or(offset), 1.0)
            };
            decimal(number)? * scale
        }
        [minutes, seconds] | [_, minutes, seconds] => {
            if minutes.len() != 2 || !minutes.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let whole_seconds = seconds.split('.').next()?;
            if whole_seconds.len() != 2 || !whole_seconds.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let minutes = decimal(minutes)?;
            let seconds = decimal(seconds)?;
            if minutes >= 60.0 || seconds >= 60.0 {
                return None;
            }
            let hours = if parts.len() == 3 {
                if parts[0].len() < 2 || !parts[0].bytes().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                decimal(parts[0])?
            } else {
                0.0
            };
            hours * 3600.0 + minutes * 60.0 + seconds
        }
        _ => return None,
    };
    seconds.is_finite().then_some(sign * seconds)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SvgAnimationRestart {
    #[default]
    Always,
    WhenNotActive,
    Never,
}

#[derive(Clone, Debug, Default)]
pub struct SvgAnimationInstanceTimes {
    pub begins: Vec<f64>,
    pub ends: Vec<f64>,
}

#[derive(Debug)]
pub struct SvgAnimationTiming {
    pub begins: Vec<f64>,
    pub ends: Vec<f64>,
    pub simple_duration: f64,
    pub repeat_count: f64,
    pub repeat_duration: f64,
    pub freeze: bool,
    pub restart: SvgAnimationRestart,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgAnimationInterval {
    pub begin: f64,
    pub end: f64,
}

impl SvgAnimationTiming {
    pub fn intervals(&self, instances: &SvgAnimationInstanceTimes) -> Vec<SvgAnimationInterval> {
        let mut begins = self.begins.clone();
        begins.extend_from_slice(&instances.begins);
        begins.sort_by(f64::total_cmp);
        begins.dedup();
        let mut ends = self.ends.clone();
        ends.extend_from_slice(&instances.ends);
        ends.sort_by(f64::total_cmp);
        let duration = (self.simple_duration * self.repeat_count).min(self.repeat_duration);
        let mut intervals: Vec<SvgAnimationInterval> = Vec::new();
        for begin in begins {
            if let Some(previous) = intervals.last_mut() {
                match self.restart {
                    SvgAnimationRestart::Never => break,
                    SvgAnimationRestart::WhenNotActive if begin < previous.end => continue,
                    _ => previous.end = previous.end.min(begin),
                }
            }
            let end = ends
                .iter()
                .copied()
                .find(|end| *end >= begin)
                .unwrap_or(f64::INFINITY)
                .min(begin + duration);
            intervals.push(SvgAnimationInterval { begin, end });
        }
        intervals
    }

    pub fn current_interval(
        &self,
        instances: &SvgAnimationInstanceTimes,
        time: f64,
    ) -> Option<SvgAnimationInterval> {
        self.intervals(instances)
            .into_iter()
            .find(|interval| interval.end > time)
    }

    pub fn sample_progress(&self, instances: &SvgAnimationInstanceTimes, time: f64) -> Option<f64> {
        let interval = self
            .intervals(instances)
            .into_iter()
            .rev()
            .find(|interval| interval.begin <= time)?;
        let finished = time >= interval.end;
        if finished && !self.freeze {
            return None;
        }
        let elapsed = time.min(interval.end) - interval.begin;
        if self.simple_duration == 0.0 {
            return Some(1.0);
        }
        if !self.simple_duration.is_finite() {
            return Some(0.0);
        }
        let fraction = (elapsed / self.simple_duration).fract();
        Some(if finished && fraction == 0.0 && elapsed > 0.0 {
            1.0
        } else {
            fraction
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_clock_preserves_pending_seek_and_pause_continuity() {
        let mut clock = SvgPresentationClock::default();
        clock.seek(2.0, 7.0);
        clock.pause(3.0);
        assert_eq!(clock.current_time(4.0), 0.0);
        clock.start(10.0);
        assert_eq!(clock.current_time(20.0), 7.0);
        clock.unpause(20.0);
        assert_eq!(clock.current_time(21.5), 8.5);
        clock.pause(22.0);
        clock.pause(30.0);
        assert_eq!(clock.current_time(100.0), 9.0);
        clock.seek(100.0, -5.0);
        assert_eq!(clock.current_time(200.0), 0.0);
    }

    #[test]
    fn clock_values_validate_grammar_and_units() {
        for (raw, seconds) in [
            ("250ms", 0.25),
            ("2min", 120.0),
            ("1h", 3600.0),
            ("01:02:03.5", 3723.5),
            ("02:03.5", 123.5),
            (" +3.5s ", 3.5),
            ("-0.5s", -0.5),
            (".5", 0.5),
        ] {
            assert_eq!(parse_clock_value(raw), Some(seconds), "{raw}");
        }
        for raw in [
            "", "NaN", "Infinity", "1e2", "1 2", "1:02", "01:60", "01:00:60", "1:00:00", "1ss",
            "1..2",
        ] {
            assert_eq!(parse_clock_value(raw), None, "{raw}");
        }
    }

    fn timing() -> SvgAnimationTiming {
        SvgAnimationTiming {
            begins: vec![0.0],
            ends: vec![],
            simple_duration: 10.0,
            repeat_count: 2.5,
            repeat_duration: f64::INFINITY,
            freeze: true,
            restart: SvgAnimationRestart::Always,
        }
    }

    #[test]
    fn repeating_intervals_freeze_at_the_fractional_final_sample() {
        let timing = timing();
        let instances = SvgAnimationInstanceTimes::default();
        for (time, progress) in [
            (5.0, 0.5),
            (10.0, 0.0),
            (15.0, 0.5),
            (25.0, 0.5),
            (50.0, 0.5),
        ] {
            assert_eq!(timing.sample_progress(&instances, time), Some(progress));
        }
        let mut timing = timing;
        timing.repeat_count = 2.0;
        assert_eq!(timing.sample_progress(&instances, 20.0), Some(1.0));
        timing.freeze = false;
        assert_eq!(timing.sample_progress(&instances, 20.0), None);
    }

    #[test]
    fn instance_times_obey_restart_and_end_constraints() {
        let mut timing = timing();
        let instances = SvgAnimationInstanceTimes {
            begins: vec![5.0, 30.0],
            ends: vec![33.0],
        };
        assert_eq!(timing.current_interval(&instances, 6.0).unwrap().begin, 5.0);
        assert_eq!(timing.sample_progress(&instances, 34.0), Some(0.3));
        timing.restart = SvgAnimationRestart::WhenNotActive;
        assert_eq!(timing.current_interval(&instances, 6.0).unwrap().begin, 0.0);
        timing.restart = SvgAnimationRestart::Never;
        assert_eq!(timing.current_interval(&instances, 30.0), None);
    }
}
