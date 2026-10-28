use moli_animation::{
    AdvanceMode, AnimationCommand, AnimationState, FillMode, KeyframeValue, PropertyKeyframes,
};
use std::time::{Duration, Instant};

fn effect(now: Instant) -> AnimationState {
    let mut animation = AnimationState::new(now);
    animation.has_effect = true;
    animation.duration = Duration::from_secs(1);
    animation.fill = FillMode::Both;
    animation
}

#[test]
fn pause_seek_resume_and_rate_share_one_local_clock() {
    let now = Instant::now();
    let mut animation = effect(now);
    assert!(animation.play(now));
    assert_eq!(animation.next_deadline(now), Some(now));
    assert!(animation.commit_pending(now));
    animation.pause(now + Duration::from_millis(200));
    assert!(animation.is_pending());
    assert!(animation.commit_pending(now + Duration::from_millis(200)));
    let resumed = now + Duration::from_secs(10);
    assert_eq!(animation.current_seconds(resumed), Some(0.2));
    assert_eq!(animation.next_deadline(resumed), Some(resumed));
    animation.advance(resumed, AdvanceMode::Tick);
    assert_eq!(animation.next_deadline(resumed), None);

    let _ = animation
        .command(AnimationCommand::Seek(0.4), resumed)
        .unwrap();
    assert!(animation.play(resumed));
    assert!(animation.commit_pending(resumed));
    let later = resumed + Duration::from_millis(100);
    assert_eq!(animation.current_seconds(later), Some(0.5));
    let _ = animation
        .command(AnimationCommand::SetRate(-2.0), later)
        .unwrap();
    assert!(
        (animation
            .current_seconds(later + Duration::from_millis(100))
            .unwrap()
            - 0.3)
            .abs()
            < 1e-9
    );
}

#[test]
fn transferring_effect_configuration_preserves_identity_and_playback() {
    let now = Instant::now();
    let mut animation = effect(now);
    assert!(animation.play(now));
    assert!(animation.commit_pending(now));
    let id = animation.id();
    let mut source = effect(now);
    source.duration = Duration::from_secs(2);
    source.frames.push(PropertyKeyframes {
        property: "opacity".into(),
        values: vec![
            KeyframeValue::new(0.0, "0".into()),
            KeyframeValue::new(1.0, "1".into()),
        ],
    });
    animation.copy_effect_from(&source);
    let current = now + Duration::from_millis(200);
    assert_eq!(animation.id(), id);
    assert_ne!(animation.id(), source.id());
    assert_eq!(animation.current_seconds(current), Some(0.2));
    assert_eq!(animation.progress(current).unwrap().fraction, 0.1);
    assert_eq!(
        animation
            .sample_interval("opacity", 0.1)
            .unwrap()
            .to
            .as_deref(),
        Some("1")
    );
}

#[test]
fn interval_selection_accepts_host_easing_and_preserves_overshoot() {
    let now = Instant::now();
    let mut animation = effect(now);
    animation.frames.push(PropertyKeyframes {
        property: "value".into(),
        values: vec![
            KeyframeValue::new(0.0, "start".into()),
            KeyframeValue::new(0.4, "middle".into()),
            KeyframeValue::new(1.0, "end".into()),
        ],
    });
    let sample = animation.sample_interval("value", 0.25).unwrap();
    assert_eq!(
        (sample.from.as_deref(), sample.to.as_deref()),
        (Some("start"), Some("middle"))
    );
    assert_eq!(sample.progress, 0.625);
    assert!(animation.sample_interval("value", -0.1).unwrap().progress < 0.0);
    assert!(animation.sample_interval("value", 1.1).unwrap().progress > 1.0);
    assert!(animation.sample_interval("missing", 0.5).is_none());
}

#[test]
fn infinite_effects_need_sampling_without_repeated_timer_wakes() {
    let now = Instant::now();
    let mut animation = effect(now);
    animation.iterations = f64::INFINITY;
    assert!(animation.play(now));
    assert!(animation.commit_pending(now));
    animation.advance(now, AdvanceMode::Tick);
    let later = now + Duration::from_secs(100);
    assert!(animation.needs_timeline_service(later));
    assert_eq!(animation.next_deadline(later), None);
    animation.pause(later);
    assert!(animation.needs_timeline_service(later));
    assert!(animation.commit_pending(later));
    assert!(!animation.needs_timeline_service(later));
}
