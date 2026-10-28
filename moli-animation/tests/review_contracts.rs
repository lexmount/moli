use moli_animation::{
    AdvanceMode, AnimationCommand, AnimationEvent, AnimationState, CssTime, FillMode,
    PlaybackNotificationKind,
};
use std::time::{Duration, Instant};

fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    state
}

#[test]
fn reversing_running_playback_has_one_ready_completion() {
    let now = Instant::now();
    let mut state = effect(now);
    state.advance(now, AdvanceMode::Tick);
    let half = now + Duration::from_millis(500);
    let _ = state.command(AnimationCommand::Reverse, half).unwrap();
    assert!(state.is_pending());
    let events = state.advance(half, AdvanceMode::Tick);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e,
        AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Ready))
            .count(),
        1
    );
    assert!(!state.is_pending());
    assert!(state.advance(half, AdvanceMode::Tick).is_empty());
}

#[test]
fn reverse_before_first_ready_rewinds_without_replacing_the_pending_operation() {
    use moli_animation::ReadyPromiseUpdate::{Pending, Unchanged};
    let now = Instant::now();
    let mut state = AnimationState::new(now);
    state.has_effect = true;
    state.duration = Duration::from_secs(1);
    assert_eq!(
        state
            .command(AnimationCommand::Play, now)
            .map(|outcome| outcome.ready),
        Ok(Pending)
    );
    assert_eq!(
        state
            .command(AnimationCommand::Reverse, now)
            .map(|outcome| outcome.ready),
        Ok(Unchanged)
    );
    assert_eq!(state.current_seconds(now), Some(1.0));
    let events = state.advance(now, AdvanceMode::Tick);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e,
        AnimationEvent::Playback(n) if n.kind==PlaybackNotificationKind::Ready))
            .count(),
        1
    );
    assert_eq!(state.play_state(now), "running");
    assert_eq!(
        state.current_seconds(now + Duration::from_millis(250)),
        Some(0.75)
    );
}

#[test]
fn reversing_during_end_delay_keeps_the_effect_relevant() {
    let now = Instant::now();
    let mut state = effect(now);
    state.end_delay = CssTime::from_seconds(0.4).unwrap();
    state.finish(now);
    let _ = state.command(AnimationCommand::Reverse, now).unwrap();
    state.advance(now, AdvanceMode::Tick);
    assert_eq!(state.play_state(now), "running");
    assert!(state.is_relevant(now));
    assert!(state.is_relevant(now + Duration::from_millis(500)));
    let end = now + Duration::from_millis(1400);
    state.advance(end, AdvanceMode::Tick);
    assert!(!state.is_relevant(end));
    state.fill = FillMode::Backwards;
    assert!(state.is_relevant(end));
}

#[test]
fn explicit_seek_outside_the_playback_range_is_a_finished_hold() {
    for (rate, seek) in [(1.0, 2.0), (-1.0, -0.5)] {
        let now = Instant::now();
        let mut state = effect(now);
        let _ = state.command(AnimationCommand::SetRate(rate), now).unwrap();
        let _ = state.command(AnimationCommand::Seek(seek), now).unwrap();
        assert_eq!(state.current_seconds(now), Some(seek));
        state.advance(now, AdvanceMode::Tick);
        assert_eq!(state.play_state(now), "finished");
        assert_eq!(
            state.current_seconds(now + Duration::from_secs(10)),
            Some(seek)
        );
        assert!(
            state
                .command(AnimationCommand::Reverse, now + Duration::from_secs(10))
                .is_ok()
        );
        state.advance(now + Duration::from_secs(10), AdvanceMode::Tick);
        assert!(
            (state
                .current_seconds(now + Duration::from_millis(10025))
                .unwrap()
                - (if rate > 0.0 { 1.0 } else { 0.0 } - rate * 0.025))
                .abs()
                < 1e-9
        );
    }
}

#[test]
fn extending_an_effect_preserves_the_explicit_seek_position() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state.command(AnimationCommand::Seek(2.0), now).unwrap();
    state.advance(now, AdvanceMode::Tick);
    state.duration = Duration::from_secs(3);
    state.advance(now, AdvanceMode::Observe);
    assert_eq!(state.current_seconds(now), Some(2.0));
    assert_eq!(state.play_state(now), "running");
    assert_eq!(
        state.current_seconds(now + Duration::from_millis(100)),
        Some(2.1)
    );
}

#[test]
fn seeking_past_the_end_before_ready_preserves_the_authors_hold() {
    let now = Instant::now();
    let mut state = AnimationState::new(now);
    state.has_effect = true;
    state.duration = Duration::from_secs(1);
    let _ = state.command(AnimationCommand::Play, now).unwrap();
    let _ = state.command(AnimationCommand::Seek(2.0), now).unwrap();
    state.advance(now, AdvanceMode::Tick);
    assert_eq!(
        state.current_seconds(now + Duration::from_secs(10)),
        Some(2.0)
    );
    assert_eq!(state.play_state(now), "finished");
}

#[test]
fn extending_a_naturally_finished_effect_keeps_its_start_time_mapping() {
    let now = Instant::now();
    let mut state = effect(now);
    state.advance(now + Duration::from_secs(1), AdvanceMode::Tick);
    assert_eq!(
        state.current_seconds(now + Duration::from_secs(2)),
        Some(1.0)
    );
    state.duration = Duration::from_secs(3);
    state.advance(now + Duration::from_secs(2), AdvanceMode::Observe);
    assert_eq!(
        state.current_seconds(now + Duration::from_secs(2)),
        Some(2.0)
    );
    assert_eq!(state.play_state(now + Duration::from_secs(2)), "running");
}

#[test]
fn ready_promise_updates_follow_pending_operation_transitions() {
    use moli_animation::ReadyPromiseUpdate::{Pending, Reject, Resolve, Unchanged};
    let now = Instant::now();
    let mut state = effect(now);
    state.pause(now);
    assert!(state.commit_pending(now));
    assert_eq!(
        state
            .command(AnimationCommand::Play, now)
            .map(|outcome| outcome.ready),
        Ok(Pending)
    );
    assert_eq!(
        state
            .command(AnimationCommand::Reverse, now)
            .map(|outcome| outcome.ready),
        Ok(Unchanged)
    );
    assert_eq!(
        state
            .command(AnimationCommand::Pause, now)
            .map(|outcome| outcome.ready),
        Ok(Unchanged)
    );
    assert_eq!(
        state
            .command(AnimationCommand::Play, now)
            .map(|outcome| outcome.ready),
        Ok(Unchanged)
    );
    assert_eq!(
        state
            .command(AnimationCommand::Finish, now)
            .map(|outcome| outcome.ready),
        Ok(Resolve)
    );
    assert_eq!(
        state
            .command(AnimationCommand::Reverse, now)
            .map(|outcome| outcome.ready),
        Ok(Pending)
    );
    assert_eq!(
        state
            .command(AnimationCommand::Cancel, now)
            .map(|outcome| outcome.ready),
        Ok(Reject)
    );
}

#[test]
fn zero_rate_keeps_an_out_of_range_seek_without_finishing() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state.command(AnimationCommand::SetRate(0.0), now).unwrap();
    let _ = state.command(AnimationCommand::Seek(2.0), now).unwrap();
    state.advance(now, AdvanceMode::Tick);
    assert_eq!(
        state.current_seconds(now + Duration::from_secs(10)),
        Some(2.0)
    );
    assert_eq!(state.play_state(now), "running");
    assert_eq!(state.next_deadline(now), None);
}

#[test]
fn rejected_reverse_does_not_invalidate_existing_notification_generations() {
    let now = Instant::now();
    let mut state = AnimationState::new(now);
    state.has_effect = true;
    state.duration_infinite = true;
    let before = state.notification(PlaybackNotificationKind::Ready);
    assert!(state.command(AnimationCommand::Reverse, now).is_err());
    assert!(state.accepts(before));
    assert!(!state.is_pending());
    assert!(state.is_idle());
    assert_eq!(state.clock().rate, 1.0);
}

#[test]
fn a_delayed_finish_cannot_settle_a_later_finished_promise() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state.command(AnimationCommand::Finish, now).unwrap();
    let finish = state
        .advance(now, AdvanceMode::Observe)
        .into_iter()
        .find_map(|e| match e {
            AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Finish => Some(n),
            _ => None,
        })
        .unwrap();
    assert!(state.accepts(finish));
    let _ = state.command(AnimationCommand::Pause, now).unwrap();
    state.advance(now, AdvanceMode::Observe);
    let _ = state.command(AnimationCommand::Seek(0.1), now).unwrap();
    state.advance(now, AdvanceMode::Observe);
    assert!(!state.accepts(finish));
    assert_eq!(state.play_state(now), "paused");
    assert_eq!(state.current_seconds(now), Some(0.1));
}

#[test]
fn setting_start_time_completes_pending_play_without_an_active_timeline() {
    use moli_animation::ReadyPromiseUpdate::{Pending, Resolve};
    for seconds in [Some(0.0), None] {
        let now = Instant::now();
        let mut state = effect(now);
        state.pause(now);
        assert!(state.commit_pending(now));
        let _ = state
            .command(
                AnimationCommand::SetTimeline(moli_animation::TimelineState::None),
                now,
            )
            .unwrap();
        assert_eq!(
            state
                .command(AnimationCommand::Play, now)
                .map(|outcome| outcome.ready),
            Ok(Pending)
        );
        assert_eq!(
            state
                .command(
                    AnimationCommand::SetStartTime {
                        origin: now,
                        seconds
                    },
                    now
                )
                .map(|outcome| outcome.ready),
            Ok(Resolve)
        );
        assert!(!state.is_pending());
        assert!(!state.advance(now+Duration::from_secs(10),AdvanceMode::Tick).iter().any(|e|
            matches!(e,AnimationEvent::Playback(n) if n.kind==PlaybackNotificationKind::Ready)));
    }
}

#[test]
fn detaching_a_timeline_preserves_start_mapping_and_authored_pause() {
    let now = Instant::now();
    for paused in [false, true] {
        let mut state = effect(now);
        let _ = state.command(AnimationCommand::Seek(0.5), now).unwrap();
        if paused {
            let _ = state.command(AnimationCommand::Pause, now).unwrap();
            assert!(state.commit_pending(now));
        }
        let start = state.clock().start_millis(now);
        let _ = state
            .command(
                AnimationCommand::SetTimeline(moli_animation::TimelineState::None),
                now,
            )
            .unwrap();
        assert_eq!(
            state.play_state(now),
            if paused { "paused" } else { "running" }
        );
        assert_eq!(state.current_seconds(now), paused.then_some(0.5));
        assert_eq!(state.clock().start_millis(now), start);
        assert_eq!(state.next_deadline(now), None);
        assert!(!state.needs_timeline_service(now));
        let _ = state
            .command(
                AnimationCommand::SetTimeline(moli_animation::TimelineState::Active),
                now + Duration::from_millis(100),
            )
            .unwrap();
        assert_eq!(
            state.current_seconds(now + Duration::from_millis(100)),
            Some(if paused { 0.5 } else { 0.6 })
        );
        assert_eq!(state.clock().start_millis(now), start);
    }
}

#[test]
fn a_start_time_on_an_inactive_timeline_can_be_resolved_without_a_current_time() {
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
    assert_eq!(state.clock().start_millis(now), Some(0.0));
    assert_eq!(state.play_state(now), "running");
    assert_eq!(state.progress(now), None);
    assert_eq!(state.next_deadline(now), None);
}

#[test]
fn explicitly_seeking_an_inactive_timeline_establishes_a_paused_hold() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state
        .command(
            AnimationCommand::SetTimeline(moli_animation::TimelineState::None),
            now,
        )
        .unwrap();
    assert_eq!(state.current_seconds(now), None);
    let _ = state.command(AnimationCommand::Seek(0.3), now).unwrap();
    assert_eq!(state.current_seconds(now), Some(0.3));
    assert_eq!(state.clock().start_millis(now), None);
    assert_eq!(state.play_state(now), "paused");
    let later = now + Duration::from_millis(100);
    let _ = state
        .command(
            AnimationCommand::SetTimeline(moli_animation::TimelineState::Active),
            later,
        )
        .unwrap();
    assert_eq!(state.current_seconds(later), Some(0.3));
    assert_eq!(state.play_state(later), "paused");
}

#[test]
fn synchronous_finish_events_survive_settlement_invalidation_but_pending_finishes_do_not() {
    for synchronous in [false, true] {
        let now = Instant::now();
        let mut state = effect(now);
        let _ = state
            .command(
                if synchronous {
                    AnimationCommand::Finish
                } else {
                    AnimationCommand::Seek(1.0)
                },
                now,
            )
            .unwrap();
        let notification = state
            .advance(now, AdvanceMode::Observe)
            .into_iter()
            .find_map(|event| match event {
                AnimationEvent::Playback(notification)
                    if notification.kind == PlaybackNotificationKind::Finish =>
                {
                    Some(notification)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(notification.event_is_queued(), synchronous);
        let _ = state.command(AnimationCommand::Pause, now).unwrap();
        state.advance(now, AdvanceMode::Observe);
        let _ = state.command(AnimationCommand::Seek(0.1), now).unwrap();
        state.advance(now, AdvanceMode::Observe);
        assert!(!state.accepts(notification));
        assert_eq!(notification.event_is_queued(), synchronous);
    }
}

#[test]
fn finishing_without_a_timeline_establishes_a_hold_without_finish_notification() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state
        .command(
            AnimationCommand::SetTimeline(moli_animation::TimelineState::None),
            now,
        )
        .unwrap();
    let _ = state.command(AnimationCommand::Finish, now).unwrap();
    assert_eq!(state.current_seconds(now), Some(1.0));
    assert_eq!(state.clock().start_millis(now), None);
    assert_eq!(state.play_state(now), "paused");
    assert!(!state.advance(now, AdvanceMode::Observe).iter().any(|event|
        matches!(event, AnimationEvent::Playback(notification) if notification.kind == PlaybackNotificationKind::Finish)));
    assert_eq!(state.next_deadline(now), None);
}
