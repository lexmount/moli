use moli_animation::{AdvanceMode, AnimationCommand, AnimationEvent, AnimationState};
use std::time::{Duration, Instant};
fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    state
}
#[test]
fn observations_do_not_commit_transient_css_phase_crossings() {
    let now = Instant::now();
    let mut state = effect(now);
    state.pause(now);
    state.command(AnimationCommand::Seek(0.5), now).unwrap();
    state.advance(now, AdvanceMode::Tick);
    for t in [1.0, 0.5] {
        state.command(AnimationCommand::Seek(t), now).unwrap();
        assert!(
            !state
                .advance(now, AdvanceMode::Observe)
                .iter()
                .any(|e| matches!(e, AnimationEvent::Phase { .. }))
        );
    }
    assert!(
        !state
            .advance(now, AdvanceMode::Tick)
            .iter()
            .any(|e| matches!(e, AnimationEvent::Phase { .. }))
    );
}

#[test]
fn a_paused_dirty_phase_is_committed_once_without_polling() {
    let now = Instant::now();
    let mut state = effect(now);
    state.pause(now);
    assert!(state.advance(now, AdvanceMode::Observe).is_empty());
    assert!(state.needs_timeline_service(now));
    assert_eq!(state.next_deadline(now), Some(now));
    assert!(
        state
            .advance(now, AdvanceMode::Tick)
            .iter()
            .any(|event| matches!(event, AnimationEvent::Phase { .. }))
    );
    assert_eq!(state.next_deadline(now), None);
    assert!(!state.needs_timeline_service(now));
}
