use moli_animation::{AnimationCommand, AnimationState};
use std::time::{Duration, Instant};
fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    state
}
#[test]
fn clearing_start_time_holds_the_previously_exposed_finished_time() {
    let now = Instant::now();
    let mut state = effect(now);
    state.finish(now);
    let later = now + Duration::from_millis(120);
    assert_eq!(state.current_seconds(later), Some(1.0));
    state
        .command(
            AnimationCommand::SetStartTime {
                origin: now,
                seconds: None,
            },
            later,
        )
        .unwrap();
    assert_eq!(state.current_seconds(later), Some(1.0));
    assert_eq!(state.play_state(later), "paused");
    assert_eq!(state.clock().start_millis(now), None);
}
#[test]
fn clearing_an_unresolved_start_mapping_keeps_time_unresolved() {
    let now = Instant::now();
    let mut state = effect(now);
    state
        .command(AnimationCommand::AttachTimeline(false), now)
        .unwrap();
    state
        .command(
            AnimationCommand::SetStartTime {
                origin: now,
                seconds: Some(0.0),
            },
            now,
        )
        .unwrap();
    assert_eq!(state.current_seconds(now), None);
    state
        .command(
            AnimationCommand::SetStartTime {
                origin: now,
                seconds: None,
            },
            now,
        )
        .unwrap();
    assert_eq!(state.current_seconds(now), None);
    assert_eq!(state.play_state(now), "idle");
    assert_eq!(state.next_deadline(now), None);
}
