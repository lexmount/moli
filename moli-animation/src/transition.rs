use crate::{CssTime, duration};
use std::time::Duration;

/// Timing of a reversed transition; value matching and typed interpolation stay
/// with the host, while shortening arithmetic is independent of style or DOM.
#[derive(Clone, Copy, Debug)]
pub struct ReversalTiming {
    pub shortening_factor: f64,
    pub duration: Duration,
    pub delay: CssTime,
}

pub fn reverse_transition_timing(
    duration_value: Duration,
    delay: CssTime,
    progress: f64,
    previous_shortening: f64,
) -> ReversalTiming {
    let shortening_factor = (progress * previous_shortening + (1.0 - previous_shortening))
        .abs()
        .clamp(0.0, 1.0);
    ReversalTiming {
        shortening_factor,
        duration: duration(duration_value.as_secs_f64() * shortening_factor),
        delay: if delay.is_positive() {
            delay
        } else {
            CssTime::from_seconds(delay.seconds() * shortening_factor).unwrap_or_default()
        },
    }
}
