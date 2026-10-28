use moli_animation::{
    AdvanceMode, AnimationCommand as Command, AnimationEvent, AnimationState, CssTime, FillMode,
    PhaseEvent, ReadyPromiseUpdate, SamplingMode,
};
use std::time::{Duration, Instant};

fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    state
}

#[test]
fn zero_rate_preserves_explicit_start_and_hold_together() {
    let now = Instant::now();
    let mut state = AnimationState::new(now);
    let _ = state.command(Command::Seek(0.5), now).unwrap();
    let _ = state.command(Command::SetRate(0.0), now).unwrap();
    let _ = state
        .command(
            Command::SetStartTime {
                origin: now,
                seconds: Some(0.123),
            },
            now,
        )
        .unwrap();
    assert_eq!(state.clock().start_millis(now), Some(123.0));
    assert_eq!(
        state.current_seconds(now + Duration::from_secs(10)),
        Some(0.5)
    );
    assert_eq!(state.play_state(now), "running");
}

#[test]
fn pending_cancel_rejects_ready_but_idle_cancel_has_no_notification() {
    let now = Instant::now();
    let mut state = AnimationState::new(now);
    assert_eq!(
        state
            .command(Command::Cancel, now)
            .map(|outcome| outcome.ready),
        Ok(ReadyPromiseUpdate::Unchanged)
    );
    assert!(state.advance(now, AdvanceMode::Tick).is_empty());
    assert_eq!(
        state
            .command(Command::Play, now)
            .map(|outcome| outcome.ready),
        Ok(ReadyPromiseUpdate::Pending)
    );
    assert_eq!(
        state
            .command(Command::Cancel, now)
            .map(|outcome| outcome.ready),
        Ok(ReadyPromiseUpdate::Reject)
    );
    state.advance(now, AdvanceMode::Tick);
    let _ = state.command(Command::Cancel, now).unwrap();
    assert!(state.advance(now, AdvanceMode::Tick).is_empty());
}

#[test]
fn idle_reverse_pause_starts_at_the_end_and_rejects_an_infinite_end() {
    let now = Instant::now();
    for infinite in [false, true] {
        let mut state = AnimationState::new(now);
        state.has_effect = true;
        state.duration = Duration::from_secs(1);
        state.iterations = if infinite { f64::INFINITY } else { 1.0 };
        let _ = state.command(Command::SetRate(-1.0), now).unwrap();
        assert_eq!(state.command(Command::Pause, now).is_err(), infinite);
        if !infinite {
            assert_eq!(state.current_seconds(now), Some(1.0));
        }
    }
}

#[test]
fn a_reverse_infinite_animation_can_finish_at_zero() {
    let now = Instant::now();
    let mut state = effect(now);
    state.iterations = f64::INFINITY;
    let _ = state.command(Command::SetRate(-1.0), now).unwrap();
    let _ = state.command(Command::Seek(0.5), now).unwrap();
    let _ = state.command(Command::Finish, now).unwrap();
    assert_eq!(state.current_seconds(now), Some(0.0));
    assert_eq!(state.play_state(now), "finished");
    state.advance(now, AdvanceMode::Tick);
    assert_eq!(state.next_deadline(now + Duration::from_secs(10)), None);
}

#[test]
fn effect_endpoints_are_included_only_for_commit_sampling() {
    let now = Instant::now();
    let mut state = effect(now);
    state.pause(now);
    for (time, display, commit) in [
        (0.999, true, true),
        (1.0, false, true),
        (1.001, false, false),
    ] {
        let _ = state.command(Command::Seek(time), now).unwrap();
        assert_eq!(state.progress(now).is_some(), display);
        assert_eq!(
            state
                .progress_for_sample(now, SamplingMode::CommitStyles)
                .is_some(),
            commit
        );
    }
    let _ = state.command(Command::Seek(1.0), now).unwrap();
    let end = state
        .progress_for_sample(now, SamplingMode::CommitStyles)
        .unwrap();
    assert_eq!(end.fraction, 1.0);
    assert!(!end.before);
    let _ = state.command(Command::SetRate(-1.0), now).unwrap();
    let _ = state.command(Command::Seek(0.0), now).unwrap();
    let start = state
        .progress_for_sample(now, SamplingMode::CommitStyles)
        .unwrap();
    assert_eq!(start.fraction, 0.0);
    assert!(!start.before);
}

#[test]
fn zero_duration_before_phase_uses_iteration_start_not_final_iteration() {
    let now = Instant::now();
    for (iterations, direction) in [(0.5, "normal"), (1.0, "alternate")] {
        let mut state = effect(now);
        state.duration = Duration::ZERO;
        state.delay = CssTime::from_seconds(1.0).unwrap();
        state.iterations = iterations;
        state.direction = direction.into();
        state.fill = FillMode::Both;
        assert_eq!(state.progress(now).unwrap().fraction, 0.0);
    }
}

#[test]
fn the_steps_before_flag_follows_the_iteration_direction() {
    let now = Instant::now();
    let mut state = effect(now);
    state.delay = CssTime::from_seconds(1.0).unwrap();
    state.fill = FillMode::Both;
    for direction in ["reverse", "alternate-reverse"] {
        state.direction = direction.into();
        assert!(!state.progress(now).unwrap().before);
        assert!(state.progress(now + Duration::from_secs(3)).unwrap().before);
    }
}

#[test]
fn css_unresolved_and_replay_transitions_have_run_start_and_cancel() {
    let now = Instant::now();
    let mut state = effect(now);
    let first = state.advance(now, AdvanceMode::Tick);
    assert!(
        first
            .iter()
            .any(|e| matches!(e, AnimationEvent::Run { .. }))
    );
    let _ = state.command(Command::AttachTimeline(false), now).unwrap();
    let detached = state.advance(now, AdvanceMode::Tick);
    assert!(detached.iter().any(|e| matches!(
        e,
        AnimationEvent::Phase {
            kind: PhaseEvent::Cancel,
            ..
        }
    )));
    let _ = state.command(Command::AttachTimeline(true), now).unwrap();
    let attached = state.advance(now, AdvanceMode::Tick);
    assert!(
        attached
            .iter()
            .any(|e| matches!(e, AnimationEvent::Run { .. }))
    );
    assert!(attached.iter().any(|e| matches!(
        e,
        AnimationEvent::Phase {
            kind: PhaseEvent::Start,
            ..
        }
    )));
    let _ = state.command(Command::Cancel, now).unwrap();
    state.advance(now, AdvanceMode::Tick);
    let _ = state.command(Command::Play, now).unwrap();
    let replay = state.advance(now, AdvanceMode::Tick);
    assert!(
        replay
            .iter()
            .any(|e| matches!(e, AnimationEvent::Run { .. }))
    );
    assert!(replay.iter().any(|e| matches!(
        e,
        AnimationEvent::Phase {
            kind: PhaseEvent::Start,
            ..
        }
    )));
}

#[test]
fn shrinking_timing_preserves_the_observable_position() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state.command(Command::Seek(0.8), now).unwrap();
    state.preserve_current_time_on_timing_change(now);
    state.duration = Duration::from_millis(500);
    state.advance(now, AdvanceMode::Observe);
    assert_eq!(state.current_seconds(now), Some(0.8));
    assert_eq!(state.play_state(now), "finished");
}
