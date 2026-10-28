use moli_animation::{
    AdvanceMode, AnimationCommand as Command, AnimationEngine, AnimationEvent, AnimationId,
    AnimationState, FinishedPromiseUpdate as Finished, PlaybackNotificationKind,
    ReadyPromiseUpdate as Ready,
};
use std::time::{Duration, Instant};

fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::new(now);
    state.has_effect = true;
    state.duration = Duration::from_secs(1);
    state
}

#[test]
fn each_cancellation_has_one_shared_settlement_identity_and_its_own_event() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state.command(Command::Pause, now).unwrap();
    let first = state.command(Command::Cancel, now).unwrap();
    let first_event = state
        .advance(now, AdvanceMode::Observe)
        .into_iter()
        .find_map(|e| {
            if let AnimationEvent::Playback(n) = e {
                n.promise_update().map(|update| (n, update))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(first_event.1, first);
    let _ = state.command(Command::Play, now).unwrap();
    state.cancel(now);
    let second_event = state
        .advance(now, AdvanceMode::Observe)
        .into_iter()
        .find_map(|e| {
            if let AnimationEvent::Playback(n) = e {
                n.promise_update().map(|update| (n, update))
            } else {
                None
            }
        })
        .unwrap();
    assert_ne!(second_event.1.cancellation, first.cancellation);
    assert_eq!(second_event.1.ready, Ready::Reject);
    assert_eq!(second_event.1.finished, Finished::RejectAndRenew);
    assert!(!state.accepts(first_event.0));
    assert!(state.accepts(second_event.0));
    assert!(first_event.0.event_is_queued());
    assert!(second_event.0.event_is_queued());
}

#[test]
fn short_command_sequences_preserve_or_replace_ready_by_pending_operation() {
    let now = Instant::now();
    let mut state = effect(now);
    // The expectations follow the play/pause/seek/cancel algorithms, including
    // switching between pending tasks without replacing the ready promise.
    for (command, ready, finished, pending, play_state) in [
        (
            Command::Cancel,
            Ready::Unchanged,
            Finished::Unchanged,
            false,
            "idle",
        ),
        (
            Command::Pause,
            Ready::Pending,
            Finished::Unchanged,
            true,
            "paused",
        ),
        (
            Command::Pause,
            Ready::Unchanged,
            Finished::Unchanged,
            true,
            "paused",
        ),
        (
            Command::Play,
            Ready::Unchanged,
            Finished::Unchanged,
            true,
            "running",
        ),
        (
            Command::Pause,
            Ready::Unchanged,
            Finished::Unchanged,
            true,
            "paused",
        ),
        (
            Command::Seek(0.5),
            Ready::Resolve,
            Finished::Unchanged,
            false,
            "paused",
        ),
        (
            Command::Play,
            Ready::Pending,
            Finished::Unchanged,
            true,
            "running",
        ),
        (
            Command::Finish,
            Ready::Resolve,
            Finished::Resolve,
            false,
            "finished",
        ),
        (
            Command::Cancel,
            Ready::Unchanged,
            Finished::RejectAndRenew,
            false,
            "idle",
        ),
        (
            Command::Cancel,
            Ready::Unchanged,
            Finished::Unchanged,
            false,
            "idle",
        ),
    ] {
        let outcome = state.command(command, now).unwrap();
        assert_eq!(
            (outcome.ready, outcome.finished),
            (ready, finished),
            "{command:?}"
        );
        assert_eq!(state.is_pending(), pending, "{command:?}");
        assert_eq!(state.play_state(now), play_state, "{command:?}");
    }
}

#[test]
fn pending_pause_commits_its_hold_once_at_the_service_time() {
    let now = Instant::now();
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    let request = now + Duration::from_millis(200);
    let service = now + Duration::from_millis(300);
    assert_eq!(
        state.command(Command::Pause, request).unwrap().ready,
        Ready::Pending
    );
    assert_eq!(state.play_state(request), "paused");
    assert_eq!(state.current_seconds(service), Some(0.3));
    assert_eq!(state.next_deadline(request), Some(request));
    assert!(!state.advance(service, AdvanceMode::Observe).iter().any(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Ready)));
    assert!(state.is_pending());
    let events = state.advance(service, AdvanceMode::Tick);
    assert_eq!(events.iter().filter(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Ready)).count(), 1);
    assert!(!state.is_pending());
    assert_eq!(
        state.current_seconds(now + Duration::from_secs(100)),
        Some(0.3)
    );
    assert_eq!(state.next_deadline(service), None);
    assert!(state.advance(service, AdvanceMode::Tick).is_empty());
}

#[test]
fn pending_pause_without_a_timeline_waits_until_seek_or_reattachment() {
    for seek in [false, true] {
        let now = Instant::now();
        let mut state = effect(now);
        let _ = state.command(Command::AttachTimeline(false), now).unwrap();
        assert_eq!(
            state.command(Command::Pause, now).unwrap().ready,
            Ready::Pending
        );
        state.advance(now + Duration::from_secs(10), AdvanceMode::Tick);
        assert!(state.is_pending());
        assert_eq!(state.current_seconds(now), Some(0.0));
        assert_eq!(state.next_deadline(now), None);
        if seek {
            assert_eq!(
                state.command(Command::Seek(0.5), now).unwrap().ready,
                Ready::Resolve
            );
            assert!(!state.is_pending());
        } else {
            assert_eq!(
                state
                    .command(Command::AttachTimeline(true), now)
                    .unwrap()
                    .ready,
                Ready::Unchanged
            );
            assert!(state.is_pending());
            state.advance(now, AdvanceMode::Tick);
            assert!(!state.is_pending());
        }
        assert_eq!(state.play_state(now), "paused");
    }
}

#[test]
fn native_cancellation_and_command_cancellation_share_promise_actions() {
    for command in [Command::Play, Command::Pause] {
        let now = Instant::now();
        let mut state = effect(now);
        let _ = state.command(command, now).unwrap();
        let outcome = state.command(Command::Cancel, now).unwrap();
        assert_eq!(outcome.ready, Ready::Reject);
        assert_eq!(outcome.finished, Finished::RejectAndRenew);
        let events = state.advance(now, AdvanceMode::Observe);
        let cancel = events
            .iter()
            .find_map(|event| match event {
                AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Cancel => {
                    Some(n)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(cancel.promise_update(), Some(outcome));
        assert_eq!(
            state.command(Command::Cancel, now).unwrap().finished,
            Finished::Unchanged
        );
        assert!(state.advance(now, AdvanceMode::Tick).is_empty());
    }
}

#[test]
fn engine_does_not_discard_accepted_command_outcomes() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let id = animation.borrow().id();
    assert_eq!(
        engine.command(id, Command::Play, now).unwrap().ready,
        Ready::Pending
    );
    assert_eq!(
        engine.command(id, Command::Pause, now).unwrap().ready,
        Ready::Unchanged
    );
    assert_eq!(
        engine.command(id, Command::Cancel, now).unwrap().finished,
        Finished::RejectAndRenew
    );
    assert!(engine.command(AnimationId(0), Command::Play, now).is_err());
}

#[test]
fn playback_rate_setter_completes_pending_pause_at_the_current_position() {
    let now = Instant::now();
    let mut state = effect(now);
    let _ = state.command(Command::Seek(0.5), now).unwrap();
    let _ = state.command(Command::Play, now).unwrap();
    let _ = state.command(Command::Pause, now).unwrap();
    assert_eq!(
        state.command(Command::SetRate(2.0), now).unwrap().ready,
        Ready::Resolve
    );
    assert!(!state.is_pending());
    assert_eq!(state.play_state(now), "paused");
    assert_eq!(state.current_seconds(now), Some(0.5));
}

#[test]
fn finishing_on_an_inactive_timeline_does_not_complete_pending_tasks() {
    for (operation, play_state) in [(Command::Play, "finished"), (Command::Pause, "paused")] {
        let now = Instant::now();
        let mut state = effect(now);
        let _ = state.command(Command::AttachTimeline(false), now).unwrap();
        let _ = state.command(operation, now).unwrap();
        let outcome = state.command(Command::Finish, now).unwrap();
        assert_eq!(outcome.ready, Ready::Unchanged);
        assert_eq!(outcome.finished, Finished::Unchanged);
        assert!(state.is_pending());
        assert_eq!(state.play_state(now), play_state);
        assert_eq!(state.current_seconds(now), Some(1.0));
        assert!(!state.advance(now, AdvanceMode::Observe).iter().any(|event|
            matches!(event, AnimationEvent::Playback(n) if matches!(n.kind, PlaybackNotificationKind::Ready | PlaybackNotificationKind::Finish))));
    }
}

#[test]
fn replacing_a_pending_pause_preserves_the_existing_running_mapping() {
    let now = Instant::now();
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(10);
    let _ = state
        .command(Command::Pause, now + Duration::from_millis(200))
        .unwrap();
    assert_eq!(
        state
            .command(Command::Play, now + Duration::from_millis(300))
            .unwrap()
            .ready,
        Ready::Unchanged
    );
    let service = now + Duration::from_millis(500);
    assert_eq!(state.current_seconds(service), Some(0.5));
    state.advance(service, AdvanceMode::Tick);
    assert_eq!(state.current_seconds(service), Some(0.5));
    assert_eq!(state.clock().start_millis(now), Some(0.0));
}

#[test]
fn css_pause_replaces_the_operation_without_consuming_ready() {
    let now = Instant::now();
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(10);
    let _ = state.command(Command::Seek(5.0), now).unwrap();
    let _ = state.command(Command::Reverse, now).unwrap();
    state.set_css_paused(true, now);
    assert!(state.is_pending());
    assert_eq!(state.play_state(now), "paused");
    state.advance(now, AdvanceMode::Tick);
    assert!(!state.is_pending());
    assert_eq!(state.play_state(now), "paused");
}
