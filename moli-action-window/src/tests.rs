use std::time::{Duration, Instant};

use crate::{
    ActionBarrier, ActionBatchCause, ActionCompaction, ActionWindow, AdmissionState, ClickAction,
    InputModifiers, MouseButton, PlannedAction, Point, ScrollAction, WindowAction,
};

const SCOPE_A: u8 = 1;
const SCOPE_B: u8 = 2;

fn at(base: Instant, milliseconds: u64) -> Instant {
    base + Duration::from_millis(milliseconds)
}

fn scroll(delta_y: f64) -> WindowAction<&'static str> {
    WindowAction::Scroll(ScrollAction::pixels(Point::new(10.0, 20.0), 0.0, delta_y))
}

fn click(x: f64) -> WindowAction<&'static str> {
    WindowAction::Click(ClickAction::new(Point::new(x, 20.0), MouseButton::Left, 1))
}

fn ordered(value: &'static str) -> WindowAction<&'static str> {
    WindowAction::Ordered(value)
}

fn default_window() -> ActionWindow<u8, &'static str> {
    ActionWindow::default()
}

fn scroll_deltas(action: &PlannedAction<u8, &'static str>) -> Vec<f64> {
    let PlannedAction::Scroll { run, .. } = action else {
        panic!("expected scroll run");
    };
    run.steps()
        .iter()
        .map(|step| step.value().delta_y)
        .collect()
}

#[test]
fn new_constructor_uses_the_fixed_one_second_window() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, &'static str>::new();

    let admission = window.push(SCOPE_A, scroll(1.0), base);

    assert_eq!(admission.deadline(), base + Duration::from_secs(1));
}

#[test]
fn idle_window_has_no_timer_or_pending_work() {
    let base = Instant::now();
    let mut window = default_window();

    assert!(window.is_idle());
    assert_eq!(window.next_deadline(), None);
    assert_eq!(window.pending_admitted_action_count(), 0);
    assert_eq!(window.pending_retained_action_count(), 0);
    assert_eq!(window.pending_planned_action_count(), 0);
    assert!(window.take_due(at(base, 10_000)).is_none());
}

#[test]
fn first_action_opens_one_shot_window() {
    let base = Instant::now();
    let mut window = default_window();

    let admission = window.push(SCOPE_A, scroll(10.0), base);

    assert_eq!(admission.state(), AdmissionState::Opened);
    assert_eq!(admission.batch_id().get(), 1);
    assert_eq!(admission.deadline(), at(base, 1_000));
    assert_eq!(admission.compaction(), ActionCompaction::Added);
    assert!(admission.ready_batch().is_none());
    assert_eq!(window.next_deadline(), Some(at(base, 1_000)));
}

#[test]
fn later_actions_join_without_moving_fixed_deadline() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);

    let second = window.push(SCOPE_A, scroll(20.0), at(base, 400));
    let third = window.push(SCOPE_A, scroll(30.0), at(base, 900));

    assert_eq!(second.state(), AdmissionState::Joined);
    assert_eq!(third.state(), AdmissionState::Joined);
    assert_eq!(second.deadline(), at(base, 1_000));
    assert_eq!(third.deadline(), at(base, 1_000));
    assert_eq!(window.next_deadline(), Some(at(base, 1_000)));
}

#[test]
fn batch_is_not_due_before_deadline_and_is_due_at_deadline() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);

    assert!(window.take_due(at(base, 999)).is_none());
    let batch = window
        .take_due(at(base, 1_000))
        .expect("batch should be due at its deadline");

    assert_eq!(batch.cause(), ActionBatchCause::Deadline);
    assert_eq!(batch.opened_at(), base);
    assert_eq!(batch.deadline(), at(base, 1_000));
    assert_eq!(batch.released_at(), at(base, 1_000));
    assert!(window.is_idle());
    assert_eq!(window.next_deadline(), None);
}

#[test]
fn action_after_idle_period_starts_fresh_window_from_its_arrival() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.take_due(at(base, 1_000)).expect("first batch");

    let admission = window.push(SCOPE_A, scroll(20.0), at(base, 1_600));

    assert_eq!(admission.state(), AdmissionState::Opened);
    assert_eq!(admission.batch_id().get(), 2);
    assert_eq!(admission.deadline(), at(base, 2_600));
}

#[test]
fn screenshot_flushes_immediately_and_resets_deadline() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_A, scroll(20.0), at(base, 400));

    let batch = window
        .flush(ActionBarrier::Screenshot, at(base, 600))
        .expect("screenshot should flush pending work");

    assert_eq!(
        batch.cause(),
        ActionBatchCause::Barrier(ActionBarrier::Screenshot)
    );
    assert_eq!(batch.deadline(), at(base, 1_000));
    assert_eq!(batch.released_at(), at(base, 600));
    assert!(window.is_idle());
    assert_eq!(window.next_deadline(), None);

    let next = window.push(SCOPE_A, scroll(30.0), at(base, 700));
    assert_eq!(next.state(), AdmissionState::Opened);
    assert_eq!(next.deadline(), at(base, 1_700));

    // A timer wake already queued for the canceled 1.0s deadline is harmless.
    assert!(window.take_due(at(base, 1_000)).is_none());
    assert_eq!(window.next_deadline(), Some(at(base, 1_700)));
}

#[test]
fn screencast_flush_uses_its_own_barrier_cause() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);

    let batch = window
        .flush(ActionBarrier::Screencast, at(base, 200))
        .expect("screencast should flush pending work");

    assert_eq!(
        batch.cause(),
        ActionBatchCause::Barrier(ActionBarrier::Screencast)
    );
}

#[test]
fn barrier_while_idle_does_not_create_a_window() {
    let base = Instant::now();
    let mut window = default_window();

    assert!(window.flush(ActionBarrier::Screenshot, base).is_none());
    assert!(window.is_idle());
    assert_eq!(window.next_deadline(), None);
}

#[test]
fn late_action_rotates_due_batch_and_opens_fresh_window() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);

    let admission = window.push(SCOPE_A, scroll(20.0), at(base, 1_200));
    let ready = admission
        .ready_batch()
        .expect("late admission should return old batch");

    assert_eq!(admission.state(), AdmissionState::Rotated);
    assert_eq!(ready.id().get(), 1);
    assert_eq!(ready.cause(), ActionBatchCause::Deadline);
    assert_eq!(ready.released_at(), at(base, 1_200));
    assert_eq!(admission.batch_id().get(), 2);
    assert_eq!(admission.deadline(), at(base, 2_200));
    assert_eq!(window.pending_retained_action_count(), 1);
}

#[test]
fn action_exactly_at_deadline_belongs_to_next_window() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);

    let admission = window.push(SCOPE_A, scroll(20.0), at(base, 1_000));

    assert_eq!(admission.state(), AdmissionState::Rotated);
    assert_eq!(
        scroll_deltas(&admission.ready_batch().expect("old batch").actions()[0]),
        vec![10.0]
    );
    assert_eq!(admission.deadline(), at(base, 2_000));
}

#[test]
fn adjacent_scrolls_form_one_ordered_run_without_summing() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    let second = window.push(SCOPE_A, scroll(-3.0), at(base, 100));
    let third = window.push(SCOPE_A, scroll(20.0), at(base, 200));

    assert_eq!(second.compaction(), ActionCompaction::AppendedToScrollRun);
    assert_eq!(third.compaction(), ActionCompaction::AppendedToScrollRun);
    let batch = window.take_due(at(base, 1_000)).expect("batch");
    assert_eq!(batch.admitted_action_count(), 3);
    assert_eq!(batch.retained_action_count(), 3);
    assert_eq!(batch.planned_action_count(), 1);
    assert_eq!(scroll_deltas(&batch.actions()[0]), vec![10.0, -3.0, 20.0]);

    let PlannedAction::Scroll { run, .. } = &batch.actions()[0] else {
        panic!("expected scroll run");
    };
    assert_eq!(run.steps()[0].sequence().get(), 1);
    assert_eq!(run.steps()[1].sequence().get(), 2);
    assert_eq!(run.steps()[2].sequence().get(), 3);
    assert_eq!(run.steps()[1].admitted_at(), at(base, 100));
}

#[test]
fn scroll_run_retains_each_steps_mode_position_and_modifiers() {
    let base = Instant::now();
    let mut window = default_window();
    let first = ScrollAction::pixels(Point::new(1.0, 2.0), 3.0, 4.0);
    let second = ScrollAction {
        position: Point::new(5.0, 6.0),
        delta_x: 7.0,
        delta_y: 8.0,
        delta_mode: crate::ScrollDeltaMode::Line,
        modifiers: InputModifiers::CONTROL | InputModifiers::SHIFT,
    };
    window.push(SCOPE_A, WindowAction::Scroll(first.clone()), base);
    window.push(SCOPE_A, WindowAction::Scroll(second.clone()), at(base, 100));

    let batch = window.take_due(at(base, 1_000)).expect("batch");
    let PlannedAction::Scroll { run, .. } = &batch.actions()[0] else {
        panic!("expected scroll run");
    };

    assert_eq!(run.steps()[0].value(), &first);
    assert_eq!(run.steps()[1].value(), &second);
}

#[test]
fn scrolls_in_different_scopes_are_separate_runs() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_B, scroll(20.0), at(base, 100));
    window.push(SCOPE_A, scroll(30.0), at(base, 200));

    let batch = window.take_due(at(base, 1_000)).expect("batch");

    assert_eq!(batch.planned_action_count(), 3);
    assert_eq!(*batch.actions()[0].scope(), SCOPE_A);
    assert_eq!(*batch.actions()[1].scope(), SCOPE_B);
    assert_eq!(*batch.actions()[2].scope(), SCOPE_A);
}

#[test]
fn latest_click_replaces_older_click_in_same_scope() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, click(10.0), base);
    let replacement = window.push(SCOPE_A, click(30.0), at(base, 300));

    assert_eq!(replacement.compaction(), ActionCompaction::ReplacedClick);
    let batch = window.take_due(at(base, 1_000)).expect("batch");
    assert_eq!(batch.admitted_action_count(), 2);
    assert_eq!(batch.retained_action_count(), 1);
    assert_eq!(batch.planned_action_count(), 1);
    let PlannedAction::Click { click, .. } = &batch.actions()[0] else {
        panic!("expected click");
    };
    assert_eq!(click.value().position.x, 30.0);
    assert_eq!(click.sequence().get(), 2);
    assert_eq!(click.admitted_at(), at(base, 300));
}

#[test]
fn clicks_in_different_scopes_do_not_replace_each_other() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, click(10.0), base);
    window.push(SCOPE_B, click(20.0), at(base, 100));

    let batch = window.take_due(at(base, 1_000)).expect("batch");

    assert_eq!(batch.retained_action_count(), 2);
    assert_eq!(batch.planned_action_count(), 2);
    assert_eq!(*batch.actions()[0].scope(), SCOPE_A);
    assert_eq!(*batch.actions()[1].scope(), SCOPE_B);
}

#[test]
fn surviving_click_keeps_scroll_runs_separated() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_A, click(10.0), at(base, 100));
    window.push(SCOPE_A, scroll(20.0), at(base, 200));

    let batch = window.take_due(at(base, 1_000)).expect("batch");

    assert_eq!(batch.planned_action_count(), 3);
    assert!(matches!(batch.actions()[0], PlannedAction::Scroll { .. }));
    assert!(matches!(batch.actions()[1], PlannedAction::Click { .. }));
    assert!(matches!(batch.actions()[2], PlannedAction::Scroll { .. }));
}

#[test]
fn replacing_click_reorders_it_and_joins_newly_adjacent_scroll_runs() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_A, click(10.0), at(base, 100));
    window.push(SCOPE_A, scroll(20.0), at(base, 200));
    window.push(SCOPE_A, click(40.0), at(base, 300));

    let batch = window.take_due(at(base, 1_000)).expect("batch");

    assert_eq!(batch.admitted_action_count(), 4);
    assert_eq!(batch.retained_action_count(), 3);
    assert_eq!(batch.planned_action_count(), 2);
    assert_eq!(scroll_deltas(&batch.actions()[0]), vec![10.0, 20.0]);
    let PlannedAction::Click { click, .. } = &batch.actions()[1] else {
        panic!("expected latest click last");
    };
    assert_eq!(click.value().position.x, 40.0);
}

#[test]
fn ordered_actions_are_never_compacted_and_preserve_cross_type_order() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_A, ordered("key-a"), at(base, 100));
    window.push(SCOPE_A, scroll(20.0), at(base, 200));

    let batch = window.take_due(at(base, 1_000)).expect("batch");

    assert_eq!(batch.planned_action_count(), 3);
    assert!(matches!(batch.actions()[0], PlannedAction::Scroll { .. }));
    let PlannedAction::Ordered { action, .. } = &batch.actions()[1] else {
        panic!("expected ordered action");
    };
    assert_eq!(*action.value(), "key-a");
    assert!(matches!(batch.actions()[2], PlannedAction::Scroll { .. }));
}

#[test]
fn modifiers_are_a_composable_bit_set() {
    let modifiers = InputModifiers::CONTROL | InputModifiers::SHIFT;

    assert!(modifiers.contains(InputModifiers::CONTROL));
    assert!(modifiers.contains(InputModifiers::SHIFT));
    assert!(!modifiers.contains(InputModifiers::ALT));
    assert_eq!(modifiers.bits(), 0b1010);
}

#[test]
fn large_action_count_stays_in_same_window_until_deadline() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, &'static str>::new();
    for index in 0..10_000 {
        let admission = window.push(SCOPE_A, scroll(index as f64), base);
        assert_eq!(
            admission.state(),
            if index == 0 {
                AdmissionState::Opened
            } else {
                AdmissionState::Joined
            }
        );
        assert!(admission.ready_batch().is_none());
        assert_eq!(admission.deadline(), at(base, 1_000));
    }

    let batch = window.take_due(at(base, 1_000)).expect("deadline batch");
    assert_eq!(batch.cause(), ActionBatchCause::Deadline);
    assert_eq!(batch.admitted_action_count(), 10_000);
    assert_eq!(batch.retained_action_count(), 10_000);
    assert_eq!(batch.planned_action_count(), 1);
}

#[test]
fn click_replacement_never_rotates_window() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, &'static str>::new();
    window.push(SCOPE_A, click(10.0), base);

    let admission = window.push(SCOPE_A, click(20.0), at(base, 100));

    assert_eq!(admission.state(), AdmissionState::Joined);
    assert_eq!(admission.compaction(), ActionCompaction::ReplacedClick);
    assert!(admission.ready_batch().is_none());
    assert_eq!(window.pending_admitted_action_count(), 2);
    assert_eq!(window.pending_retained_action_count(), 1);
}

#[test]
fn deadline_is_the_only_automatic_rotation_cause() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, &'static str>::new();
    window.push(SCOPE_A, scroll(10.0), base);

    let admission = window.push(SCOPE_A, scroll(20.0), at(base, 1_000));

    assert_eq!(
        admission.ready_batch().expect("old batch").cause(),
        ActionBatchCause::Deadline
    );
}

#[test]
fn clear_drops_work_and_cancels_deadline() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_A, click(10.0), at(base, 100));

    assert_eq!(window.clear(), 2);
    assert_eq!(window.clear(), 0);
    assert!(window.is_idle());
    assert_eq!(window.next_deadline(), None);
}

#[test]
fn cancel_scope_keeps_other_scope_and_original_deadline() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_A, click(10.0), at(base, 100));
    window.push(SCOPE_B, scroll(20.0), at(base, 200));

    assert_eq!(window.cancel_scope(&SCOPE_A).len(), 2);
    assert_eq!(window.next_deadline(), Some(at(base, 1_000)));
    assert_eq!(window.pending_retained_action_count(), 1);
    let batch = window.take_due(at(base, 1_000)).expect("scope B batch");
    assert_eq!(*batch.actions()[0].scope(), SCOPE_B);
}

#[test]
fn canceling_last_scope_returns_to_idle() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);

    assert_eq!(window.cancel_scope(&SCOPE_A).len(), 1);
    assert!(window.cancel_scope(&SCOPE_A).is_empty());
    assert!(window.is_idle());
    assert_eq!(window.next_deadline(), None);
}

#[test]
fn canceling_an_intervening_scope_normalizes_scroll_runs() {
    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_B, ordered("separator"), at(base, 100));
    window.push(SCOPE_A, scroll(20.0), at(base, 200));

    assert_eq!(window.cancel_scope(&SCOPE_B).len(), 1);
    assert_eq!(window.pending_planned_action_count(), 1);
    let batch = window.take_due(at(base, 1_000)).expect("batch");
    assert_eq!(scroll_deltas(&batch.actions()[0]), vec![10.0, 20.0]);
}

#[test]
fn batch_ids_and_action_sequences_continue_across_resets() {
    let base = Instant::now();
    let mut window = default_window();
    let first = window.push(SCOPE_A, scroll(10.0), base);
    let first_batch = window
        .flush(ActionBarrier::Explicit, at(base, 100))
        .expect("first batch");
    let second = window.push(SCOPE_A, scroll(20.0), at(base, 200));
    let second_batch = window.take_due(at(base, 1_200)).expect("second batch");

    assert_eq!(first.batch_id().get(), 1);
    assert_eq!(first_batch.id().get(), 1);
    assert_eq!(second.batch_id().get(), 2);
    assert_eq!(second_batch.id().get(), 2);
    let PlannedAction::Scroll { run, .. } = &second_batch.actions()[0] else {
        panic!("expected scroll");
    };
    assert_eq!(run.steps()[0].sequence().get(), 2);
}

#[test]
fn one_batch_maps_to_many_actions_and_one_derived_work_commit() {
    #[derive(Default)]
    struct FakeRenderer {
        applied_scrolls: Vec<f64>,
        clicks: Vec<f64>,
        derived_work_commits: usize,
    }

    let base = Instant::now();
    let mut window = default_window();
    window.push(SCOPE_A, scroll(10.0), base);
    window.push(SCOPE_A, scroll(20.0), at(base, 100));
    window.push(SCOPE_A, click(30.0), at(base, 200));
    let batch = window.take_due(at(base, 1_000)).expect("batch");
    let mut renderer = FakeRenderer::default();

    for action in batch.actions() {
        match action {
            PlannedAction::Scroll { run, .. } => renderer
                .applied_scrolls
                .extend(run.steps().iter().map(|step| step.value().delta_y)),
            PlannedAction::Click { click, .. } => {
                renderer.clicks.push(click.value().position.x);
            }
            PlannedAction::Ordered { .. } => {}
        }
    }
    renderer.derived_work_commits += 1;

    assert_eq!(renderer.applied_scrolls, vec![10.0, 20.0]);
    assert_eq!(renderer.clicks, vec![30.0]);
    assert_eq!(renderer.derived_work_commits, 1);
}
