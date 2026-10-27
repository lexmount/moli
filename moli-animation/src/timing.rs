use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FillMode {
    #[default]
    None,
    Backwards,
    Forwards,
    Both,
}

impl FillMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "backwards" => Self::Backwards,
            "forwards" => Self::Forwards,
            "both" => Self::Both,
            _ => Self::None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Backwards => "backwards",
            Self::Forwards => "forwards",
            Self::Both => "both",
        }
    }
}

pub fn active_duration(duration: Duration, iterations: f64) -> Option<Duration> {
    if duration.is_zero() || iterations == 0.0 {
        return Some(Duration::ZERO);
    }
    if !iterations.is_finite() {
        return None;
    }
    Duration::try_from_secs_f64(duration.as_secs_f64() * iterations.max(0.0)).ok()
}

pub fn duration(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(seconds.max(0.0)).unwrap_or(Duration::MAX)
}

/// Delay and deadline arithmetic retain the sign and saturate only when
/// converting to a platform Duration or Instant.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct CssTime(i128);
impl CssTime {
    pub fn from_seconds(seconds: f64) -> Option<Self> {
        if seconds.is_nan() {
            return None;
        }
        let magnitude = duration(seconds.abs()).as_nanos() as i128;
        Some(Self(if seconds.is_sign_negative() {
            -magnitude
        } else {
            magnitude
        }))
    }
    pub fn seconds(self) -> f64 {
        self.0 as f64 / 1e9
    }
    pub fn is_positive(self) -> bool {
        self.0 > 0
    }
    pub fn add_duration(self, value: Duration) -> Self {
        Self(self.0 + value.as_nanos() as i128)
    }
    #[cfg(test)]
    fn remaining_since(self, local: Duration) -> Option<Duration> {
        Self::to_duration((self.0 - local.as_nanos() as i128).max(0))
    }
    pub fn elapsed_at(self, local: Duration) -> Duration {
        Self::to_duration((local.as_nanos() as i128 - self.0).max(0)).unwrap_or(Duration::MAX)
    }
    fn to_duration(nanos: i128) -> Option<Duration> {
        Some(Duration::new(
            u64::try_from(nanos / 1_000_000_000).ok()?,
            (nanos % 1_000_000_000) as u32,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_duration_has_no_finite_end_for_infinite_or_overflowing_iterations() {
        assert_eq!(active_duration(Duration::from_secs(1), f64::INFINITY), None);
        assert_eq!(active_duration(Duration::MAX, 2.0), None);
        assert_eq!(
            active_duration(Duration::from_secs(2), 1.5),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            active_duration(Duration::ZERO, f64::INFINITY),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn signed_time_keeps_negative_delay_and_checked_overflow() {
        let delay = CssTime::from_seconds(-0.1).unwrap();
        assert!(
            delay
                .elapsed_at(Duration::ZERO)
                .abs_diff(Duration::from_millis(100))
                < Duration::from_nanos(2)
        );
        assert!(
            delay
                .add_duration(Duration::from_millis(200))
                .remaining_since(Duration::ZERO)
                .unwrap()
                .abs_diff(Duration::from_millis(100))
                < Duration::from_nanos(2)
        );
        assert!(
            !CssTime::from_seconds(-0.4)
                .unwrap()
                .add_duration(Duration::from_millis(200))
                .is_positive()
        );
        let huge = CssTime::from_seconds(1e30).unwrap();
        assert_eq!(
            huge.add_duration(Duration::MAX)
                .remaining_since(Duration::ZERO),
            None
        );
        assert!(
            !CssTime::from_seconds(-1e30)
                .unwrap()
                .add_duration(Duration::MAX)
                .is_positive()
        );
    }
}
