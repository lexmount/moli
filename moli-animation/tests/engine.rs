use moli_animation::{
    AnimationCommand, AnimationEngine, AnimationEvent, AnimationPhase, AnimationState, CssTime,
    FillMode, PhaseEvent, PlaybackNotificationKind,
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

fn effect(now: Instant) -> AnimationState {
    let mut state = AnimationState::css(now);
    state.duration = Duration::from_secs(1);
    state.fill = FillMode::Both;
    state
}

fn phases(records: &[AnimationEvent]) -> Vec<(PhaseEvent, f64)> {
    records
        .iter()
        .filter_map(|event| match event {
            AnimationEvent::Phase { kind, elapsed } => Some((*kind, *elapsed)),
            _ => None,
        })
        .collect()
}

#[test]
fn sampling_finished_state_does_not_consume_overdue_notifications() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let id = animation.borrow().id();
    let later = now + Duration::from_secs(2);
    for _ in 0..3 {
        let sample = engine.sample(id, later).unwrap();
        assert_eq!(sample.phase, AnimationPhase::After);
        assert_eq!(sample.local_time, Some(1.0));
        assert_eq!(sample.progress.unwrap().fraction, 1.0);
    }
    let batch = engine.advance(later);
    assert_eq!(batch.updates.len(), 1);
    assert_eq!(
        phases(&batch.updates[0].events),
        [(PhaseEvent::Start, 0.0), (PhaseEvent::End, 1.0)]
    );
    assert!(batch.updates[0].events.iter().any(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Finish)));
    assert_eq!(batch.next_wake, None);
    assert!(engine.advance(later).updates.is_empty());
    assert_eq!(engine.sample(id, later).unwrap().play_state, "finished");
}

#[test]
fn every_instance_is_committed_before_the_host_receives_a_batch() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let a = engine.create(effect(now));
    let b = engine.create(effect(now));
    let b_id = b.borrow().id();
    engine.advance(now);
    let later = now + Duration::from_secs(1);
    let batch = engine.advance(later);
    assert_eq!(batch.updates[0].animation.borrow().id(), a.borrow().id());
    assert_eq!(b.borrow().committed_phase(), AnimationPhase::After);
    // Simulate A's end callback changing B. The committed batch remains intact.
    assert!(
        engine
            .command(b_id, AnimationCommand::Seek(0.5), later)
            .is_ok()
    );
    assert_eq!(phases(&batch.updates[1].events), [(PhaseEvent::End, 1.0)]);
    let next = engine.advance(later);
    assert_eq!(phases(&next.updates[0].events), [(PhaseEvent::Start, 1.0)]);
    assert!(next.updates[0].events.iter().any(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Reset)));
}

#[test]
fn repeated_admission_and_advance_do_not_duplicate_start_or_cancel() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let id = animation.borrow().id();
    for _ in 0..3 {
        engine.activate(&animation);
    }
    let batch = engine.advance(now);
    assert_eq!(batch.updates.len(), 1);
    assert_eq!(phases(&batch.updates[0].events), [(PhaseEvent::Start, 0.0)]);
    assert!(engine.advance(now).updates[0].events.is_empty());
    assert!(engine.command(id, AnimationCommand::Cancel, now).is_ok());
    let batch = engine.advance(now);
    assert_eq!(
        phases(&batch.updates[0].events),
        [(PhaseEvent::Cancel, 0.0)]
    );
    assert!(!engine.is_active());
    assert!(engine.command(id, AnimationCommand::Cancel, now).is_ok());
    assert!(engine.advance(now).updates[0].events.is_empty());
}

#[test]
fn pause_seek_and_resume_rearm_the_same_id_without_sampling_side_effects() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let id = animation.borrow().id();
    engine.advance(now);
    let paused = now + Duration::from_millis(200);
    assert!(engine.command(id, AnimationCommand::Pause, paused).is_ok());
    assert_eq!(engine.advance(paused).next_wake, None);
    assert!(!engine.is_active());
    let later = now + Duration::from_secs(100);
    assert_eq!(engine.sample(id, later).unwrap().local_time, Some(0.2));
    assert!(
        engine
            .command(id, AnimationCommand::Seek(0.4), later)
            .is_ok()
    );
    assert!(engine.command(id, AnimationCommand::Play, later).is_ok());
    assert!(engine.sample(id, later).unwrap().pending);
    let resumed = engine.advance(later);
    assert!(resumed.updates[0].events.iter().any(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Ready)));
    assert_eq!(resumed.next_wake, Some(later + Duration::from_millis(600)));
    assert_eq!(
        engine
            .sample(id, later + Duration::from_millis(100))
            .unwrap()
            .local_time,
        Some(0.5)
    );
}

#[test]
fn infinite_playback_is_active_without_a_periodic_wake() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let mut state = effect(now);
    state.iterations = f64::INFINITY;
    let animation = engine.create(state);
    let id = animation.borrow().id();
    assert_eq!(engine.advance(now).next_wake, None);
    assert!(engine.is_active());
    let later = now + Duration::from_millis(100_250);
    assert_eq!(
        engine.sample(id, later).unwrap().progress.unwrap().fraction,
        0.25
    );
    assert!(engine.advance(later).updates[0].events.is_empty());
}

#[test]
fn registry_does_not_keep_dormant_instances_or_host_data_alive() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let id = animation.borrow().id();
    let weak = Rc::downgrade(&animation);
    assert!(engine.command(id, AnimationCommand::Pause, now).is_ok());
    drop(engine.advance(now));
    drop(animation);
    assert!(weak.upgrade().is_none());
    assert!(engine.sample(id, now).is_none());
    assert!(engine.command(id, AnimationCommand::Play, now).is_err());
    engine.prune();
    assert!(engine.into_instances().next().is_none());
}

#[test]
fn active_instances_survive_until_their_notification_batch_is_released() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let weak = Rc::downgrade(&animation);
    drop(animation);
    assert!(weak.upgrade().is_some());
    let batch = engine.advance(now + Duration::from_secs(2));
    assert!(!engine.is_active());
    assert!(weak.upgrade().is_some());
    drop(batch);
    assert!(weak.upgrade().is_none());
}

#[test]
fn cancellation_generations_reject_stale_ready_and_finished_records() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let animation = engine.create(effect(now));
    let id = animation.borrow().id();
    let finished = engine.advance(now + Duration::from_secs(2));
    let notification = finished.updates[0]
        .events
        .iter()
        .find_map(|event| match event {
            AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Finish => Some(*n),
            _ => None,
        })
        .unwrap();
    assert!(animation.borrow().accepts(notification));
    assert!(
        engine
            .command(id, AnimationCommand::Play, now + Duration::from_secs(2))
            .is_ok()
    );
    assert!(!animation.borrow().accepts(notification));
}

#[test]
fn a_delayed_tick_crosses_signed_delay_boundaries_without_clock_reads() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let mut state = effect(now);
    state.delay = CssTime::from_seconds(-0.5).unwrap();
    engine.create(state);
    let batch = engine.advance(now + Duration::from_secs(1));
    assert_eq!(
        phases(&batch.updates[0].events),
        [(PhaseEvent::Start, 0.5), (PhaseEvent::End, 1.0)]
    );
    assert_eq!(batch.next_wake, None);
}

#[test]
fn detached_timeline_keeps_pending_play_until_reattachment() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let mut state = effect(now);
    let _ = state.command(AnimationCommand::Pause, now).unwrap();
    let _ = state.command(AnimationCommand::Seek(0.25), now).unwrap();
    let _ = state
        .command(
            AnimationCommand::SetTimeline(moli_animation::TimelineState::None),
            now,
        )
        .unwrap();
    let animation = engine.create(state);
    let id = animation.borrow().id();
    assert!(engine.command(id, AnimationCommand::Play, now).is_ok());
    assert_eq!(engine.advance(now).next_wake, None);
    let later = now + Duration::from_secs(100);
    assert_eq!(engine.sample(id, later).unwrap().local_time, Some(0.25));
    assert!(
        engine
            .command(
                id,
                AnimationCommand::SetTimeline(moli_animation::TimelineState::Active),
                later
            )
            .is_ok()
    );
    let batch = engine.advance(later);
    assert!(batch.updates[0].events.iter().any(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Ready)));
    assert_eq!(batch.next_wake, Some(later + Duration::from_millis(750)));
}

#[test]
fn observation_preserves_pending_play_and_defers_css_phase_commit_until_tick() {
    use moli_animation::AdvanceMode;
    let now = Instant::now();
    let mut animation = effect(now);
    let _ = animation.command(AnimationCommand::Pause, now).unwrap();
    let _ = animation
        .command(AnimationCommand::Seek(0.25), now)
        .unwrap();
    let _ = animation.command(AnimationCommand::Play, now).unwrap();
    let observed = animation.advance(now, AdvanceMode::Observe);
    assert!(animation.sample(now).pending);
    assert!(phases(&observed).is_empty());
    assert!(!observed.iter().any(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Ready)));
    assert!(animation.advance(now, AdvanceMode::Observe).is_empty());
    let tick = animation.advance(now, AdvanceMode::Tick);
    assert!(!animation.sample(now).pending);
    assert_eq!(phases(&tick), [(PhaseEvent::Start, 0.0)]);
    assert_eq!(tick.len(), 3);
    assert!(
        tick.iter()
            .any(|event| matches!(event, AnimationEvent::Run { elapsed: 0.0 }))
    );
    assert!(tick.iter().any(|event|
        matches!(event, AnimationEvent::Playback(n) if n.kind == PlaybackNotificationKind::Ready))
    );
}

#[test]
fn retirement_enumerates_active_and_dormant_instances_once() {
    let now = Instant::now();
    let mut engine = AnimationEngine::default();
    let active = engine.create(effect(now));
    let dormant = engine.create(effect(now));
    let id = dormant.borrow().id();
    let _ = engine.command(id, AnimationCommand::Pause, now).unwrap();
    engine.advance(now);
    let instances = engine.into_instances().collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    assert!(
        instances
            .iter()
            .any(|instance| Rc::ptr_eq(instance, &active))
    );
    assert!(
        instances
            .iter()
            .any(|instance| Rc::ptr_eq(instance, &dormant))
    );
    for instance in instances {
        let mut state = instance.borrow_mut();
        state.retire(now);
        assert!(state.is_idle());
        assert!(!state.timeline_attached());
        assert!(
            state
                .advance(now, moli_animation::AdvanceMode::Tick)
                .is_empty()
        );
    }
}
