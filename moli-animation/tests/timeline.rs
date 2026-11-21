use moli_animation::{AnimationCommand, AnimationState};
use std::time::{Duration, Instant};

#[test]
fn rate_setter_distinguishes_missing_and_inactive_monotonic_timelines() {
    use moli_animation::{ReadyPromiseUpdate, TimelineState};
    let now = Instant::now();
    for timeline in [
        TimelineState::None,
        TimelineState::Inactive,
        TimelineState::Active,
    ] {
        let mut state = AnimationState::new(now);
        state.duration = Duration::from_secs(1);
        let _ = state
            .command(AnimationCommand::SetTimeline(timeline), now)
            .unwrap();
        let _ = state.command(AnimationCommand::Pause, now).unwrap();
        assert!(state.is_pending());
        assert_eq!(state.current_seconds(now), Some(0.0));
        let outcome = state.command(AnimationCommand::SetRate(2.0), now).unwrap();
        assert_eq!(state.is_pending(), timeline == TimelineState::None);
        assert_eq!(
            outcome.ready,
            if timeline == TimelineState::None {
                ReadyPromiseUpdate::Unchanged
            } else {
                ReadyPromiseUpdate::Resolve
            }
        );
        assert_eq!(state.current_seconds(now), Some(0.0));
        assert_eq!(state.play_state(now), "paused");
    }
}
fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    state
}
#[test]
fn replacing_a_timeline_clears_only_a_hold_with_a_resolved_start_mapping() {
    let now = Instant::now();
    for paused in [false, true] {
        let mut state = effect(now);
        if paused {
            state.pause(now);
        }
        let _ = state.command(AnimationCommand::Seek(2.0), now).unwrap();
        let _ = state
            .command(
                AnimationCommand::SetTimeline(moli_animation::TimelineState::None),
                now,
            )
            .unwrap();
        assert_eq!(
            state.current_seconds(now),
            if paused { Some(2.0) } else { None }
        );
        let _ = state
            .command(
                AnimationCommand::SetTimeline(moli_animation::TimelineState::Active),
                now,
            )
            .unwrap();
        assert_eq!(
            state.current_seconds(now),
            Some(if paused { 2.0 } else { 1.0 })
        );
    }
}

#[test]
fn changing_timeline_origin_preserves_numeric_start_time_and_pending_ready() {
    let now = Instant::now();
    let next = now + Duration::from_millis(200);
    let mut state = effect(now);
    let _ = state.command(AnimationCommand::Seek(0.5), now).unwrap();
    let previous_start = state.clock().start_millis(now);
    state.rebase_timeline(now, next, now);
    assert_eq!(state.clock().start_millis(next), previous_start);
    assert!((state.current_seconds(now).unwrap() - 0.3).abs() < 1e-9);
    let _ = state.command(AnimationCommand::Reverse, now).unwrap();
    state.rebase_timeline(next, now, now);
    assert!(state.is_pending());
}
