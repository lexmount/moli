use moli_animation::{
    AdvanceMode, AnimationCommand, AnimationEvent, AnimationState, CssTime, PhaseEvent,
};
use std::time::{Duration, Instant};
fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    state
}
#[test]
fn negative_end_delay_clips_both_css_phase_endpoints() {
    let now = Instant::now();
    let mut state = effect(now);
    state.end_delay = CssTime::from_seconds(-0.5).unwrap();
    state.advance(now, AdvanceMode::Tick);
    let end = state.advance(now + Duration::from_millis(500), AdvanceMode::Tick);
    assert!(
        end.iter().any(
            |e| matches!(e,AnimationEvent::Phase{kind:PhaseEvent::End,elapsed} if *elapsed==0.5)
        )
    );
    state
        .command(
            AnimationCommand::SetRate(-1.0),
            now + Duration::from_millis(500),
        )
        .unwrap();
    let back = state.advance(now + Duration::from_millis(600), AdvanceMode::Tick);
    assert!(back.iter().any(
        |e| matches!(e,AnimationEvent::Phase{kind:PhaseEvent::Start,elapsed} if *elapsed==0.5)
    ));
}
