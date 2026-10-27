use moli_animation::{CssTime, reverse_transition_timing};
use std::time::Duration;

#[test]
fn reversal_shortens_duration_and_negative_delay() {
    let timing = reverse_transition_timing(
        Duration::from_secs(1),
        CssTime::from_seconds(-0.1).unwrap(),
        0.4,
        1.0,
    );
    assert_eq!(timing.shortening_factor, 0.4);
    assert_eq!(timing.duration, Duration::from_millis(400));
    assert!((timing.delay.seconds() + 0.04).abs() < 1e-9);
}

#[test]
fn reversal_keeps_positive_delay_and_composes_previous_shortening() {
    let delay = CssTime::from_seconds(0.15).unwrap();
    let timing = reverse_transition_timing(Duration::from_secs(1), delay, 0.5, 0.4);
    assert_eq!(timing.duration, Duration::from_millis(800));
    assert_eq!(timing.delay, delay);
    assert_eq!(timing.shortening_factor, 0.8);
}

#[test]
fn reversal_bounds_easing_overshoot() {
    let timing = reverse_transition_timing(Duration::from_secs(1), CssTime::default(), 1.5, 1.0);
    assert_eq!(timing.shortening_factor, 1.0);
    assert_eq!(timing.duration, Duration::from_secs(1));
}
