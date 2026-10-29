use moli_animation::{AdvanceMode, AnimationCommand, AnimationState};
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
    let _ = state
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
    let _ = state
        .command(
            AnimationCommand::SetTimeline(moli_animation::TimelineState::None),
            now,
        )
        .unwrap();
    let _ = state
        .command(
            AnimationCommand::SetStartTime {
                origin: now,
                seconds: Some(0.0),
            },
            now,
        )
        .unwrap();
    assert_eq!(state.current_seconds(now), None);
    let _ = state
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

#[test]
fn equivalent_current_and_start_time_seeks_keep_the_same_position_and_notifications() {
    let origin = Instant::now();
    let now = origin + Duration::from_secs(3);
    for rate in [1.0, 2.0, -1.0, -2.0] {
        for local in [-0.5, 0.0, 0.5, 1.0, 2.0] {
            let mut seek = effect(origin);
            let mut start = effect(origin);
            for state in [&mut seek, &mut start] {
                state.advance(origin, AdvanceMode::Tick);
                let _ = state.command(AnimationCommand::SetRate(rate), now).unwrap();
            }
            let _ = seek.command(AnimationCommand::Seek(local), now).unwrap();
            let _ = start
                .command(
                    AnimationCommand::SetStartTime {
                        origin,
                        seconds: Some(3.0 - local / rate),
                    },
                    now,
                )
                .unwrap();
            assert_eq!(
                start.current_seconds(now),
                Some(local),
                "rate={rate}, local={local}"
            );
            assert_eq!(
                start.clock().start_millis(origin),
                seek.clock().start_millis(origin)
            );
            assert_eq!(start.play_state(now), seek.play_state(now));
            assert_eq!(
                start.advance(now, AdvanceMode::Observe),
                seek.advance(now, AdvanceMode::Observe)
            );
            let later = now + Duration::from_millis(125);
            assert_eq!(start.current_seconds(later), seek.current_seconds(later));
            assert_eq!(
                start.advance(later, AdvanceMode::Tick),
                seek.advance(later, AdvanceMode::Tick)
            );
            assert_eq!(start.next_deadline(later), seek.next_deadline(later));
        }
    }
}

#[test]
fn natural_completion_stops_at_the_boundary_while_explicit_start_time_keeps_overrun() {
    let origin = Instant::now();
    let now = origin + Duration::from_secs(2);
    let mut natural = effect(origin);
    let mut explicit = effect(origin);
    let _ = explicit
        .command(
            AnimationCommand::SetStartTime {
                origin,
                seconds: Some(0.0),
            },
            now,
        )
        .unwrap();
    natural.advance(now, AdvanceMode::Tick);
    explicit.advance(now, AdvanceMode::Tick);
    for time in [now, now + Duration::from_secs(10)] {
        assert_eq!(natural.current_seconds(time), Some(1.0));
        assert_eq!(explicit.current_seconds(time), Some(2.0));
        assert_eq!(natural.next_deadline(time), None);
        assert_eq!(explicit.next_deadline(time), None);
    }
}
