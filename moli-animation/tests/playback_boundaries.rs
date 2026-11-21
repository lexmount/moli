use moli_animation::{
    AnimationCommand, AnimationEngine, AnimationEvent, AnimationState, CssTime, FillMode,
    PlaybackNotificationKind,
};
use std::time::{Duration, Instant};
fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_millis(600);
    state.fill = FillMode::Both;
    state
}
#[test]
fn reverse_completion_retires_time_service_but_preserves_fill_sampling() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let id = animation.borrow().id();
    engine.advance(now + Duration::from_secs(1));
    assert!(
        engine
            .command(id, AnimationCommand::Reverse, now + Duration::from_secs(1))
            .is_ok()
    );
    engine.advance(now + Duration::from_secs(1));
    let end = now + Duration::from_millis(1600);
    let batch = engine.advance(end);
    assert!(batch.updates[0].events.iter().any(
        |e| matches!(e,AnimationEvent::Playback(n) if n.kind==PlaybackNotificationKind::Finish)
    ));
    assert_eq!(batch.next_wake, None);
    assert!(!engine.is_active());
    let state = engine.sample(id, end + Duration::from_secs(10)).unwrap();
    assert_eq!(state.local_time, Some(0.0));
    assert_eq!(state.play_state, "finished");
    assert_eq!(state.progress.unwrap().fraction, 0.0);
    assert!(
        engine
            .advance(end + Duration::from_secs(10))
            .updates
            .is_empty()
    );
}
#[test]
fn changing_rate_after_completion_preserves_the_bounded_current_time() {
    let now = Instant::now();
    for command in [AnimationCommand::Reverse, AnimationCommand::SetRate(-1.0)] {
        let mut state = effect(now);
        state.advance(
            now + Duration::from_millis(600),
            moli_animation::AdvanceMode::Tick,
        );
        let later = now + Duration::from_secs(10);
        assert_eq!(state.current_seconds(later), Some(0.6));
        assert!(state.command(command, later).is_ok());
        assert_eq!(state.current_seconds(later), Some(0.6));
        assert!(
            (state
                .current_seconds(later + Duration::from_millis(25))
                .unwrap()
                - 0.575)
                .abs()
                < 1e-9
        );
    }
}
#[test]
fn negative_end_delay_wakes_at_the_effective_end_not_the_full_duration() {
    let now = Instant::now();
    let mut state = effect(now);
    state.end_delay = CssTime::from_seconds(-0.5).unwrap();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(state);
    let id = animation.borrow().id();
    let started = engine.advance(now);
    assert_eq!(started.next_wake, Some(now + Duration::from_millis(100)));
    let finished = engine.advance(now + Duration::from_millis(100));
    assert!(finished.updates[0].events.iter().any(
        |e| matches!(e,AnimationEvent::Playback(n) if n.kind==PlaybackNotificationKind::Finish)
    ));
    assert_eq!(finished.next_wake, None);
    assert_eq!(
        engine
            .sample(id, now + Duration::from_millis(250))
            .unwrap()
            .play_state,
        "finished"
    );
}
#[test]
fn reverse_crosses_phase_boundaries_before_reaching_the_playback_end() {
    let now = Instant::now();
    let mut state = effect(now);
    state.delay = CssTime::from_seconds(0.2).unwrap();
    state.end_delay = CssTime::from_seconds(0.1).unwrap();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(state);
    let id = animation.borrow().id();
    engine.advance(now + Duration::from_secs(1));
    let _ = engine
        .command(id, AnimationCommand::Reverse, now + Duration::from_secs(1))
        .unwrap();
    let reversed = engine.advance(now + Duration::from_secs(1));
    assert_eq!(reversed.next_wake, Some(now + Duration::from_millis(1100)));
    assert_eq!(
        engine.advance(now + Duration::from_millis(1100)).next_wake,
        Some(now + Duration::from_millis(1700))
    );
    assert_eq!(
        engine.advance(now + Duration::from_millis(1700)).next_wake,
        Some(now + Duration::from_millis(1900))
    );
    assert_eq!(
        engine.advance(now + Duration::from_millis(1900)).next_wake,
        None
    );
    assert!(!engine.is_active());
}
