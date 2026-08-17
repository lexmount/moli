use std::time::{Duration, Instant};

use moli_action_window::{
    ActionBarrier, ActionBatchCause, ActionCompaction, ActionWindow, AdmissionState, ClickAction,
    InputModifiers, MouseButton, PlannedAction, Point, ScrollAction, ScrollDeltaMode, WindowAction,
};

#[test]
fn default_public_surface_starts_idle_with_one_second_policy() {
    let base = Instant::now();
    let mut window = ActionWindow::<String>::default();

    assert!(window.is_idle());
    assert_eq!(window.next_deadline(), None);
    let admission = window.push(
        "document".to_owned(),
        WindowAction::Scroll(ScrollAction::pixels(Point::default(), 0.0, 1.0)),
        base,
    );
    assert_eq!(admission.deadline(), base + Duration::from_secs(1));
}

#[test]
fn admissions_expose_the_sequence_retained_by_the_batch() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, ()>::new();
    let admission = window.push(
        7,
        WindowAction::Scroll(ScrollAction::pixels(Point::new(1.0, 2.0), 3.0, 4.0)),
        base,
    );
    let admitted_sequence = admission.sequence();

    let batch = window
        .flush(ActionBarrier::Explicit, base)
        .expect("admitted action should flush");
    let PlannedAction::Scroll { run, .. } = &batch.actions()[0] else {
        panic!("scroll admission should retain a scroll run");
    };
    assert_eq!(run.steps()[0].sequence(), admitted_sequence);
}

#[test]
fn public_api_accepts_owned_scope_and_non_clone_ordered_payload() {
    #[derive(Debug, PartialEq)]
    struct NonClonePayload(u32);

    let base = Instant::now();
    let mut window = ActionWindow::<String, NonClonePayload>::default();
    window.push(
        "document-a".to_owned(),
        WindowAction::Ordered(NonClonePayload(42)),
        base,
    );

    let batch = window
        .flush(ActionBarrier::Explicit, base)
        .expect("ordered action should flush");
    let mut actions = batch.into_actions();
    assert_eq!(actions.len(), 1);

    let PlannedAction::Ordered { scope, action } = actions.remove(0) else {
        panic!("expected ordered payload");
    };
    assert_eq!(scope, "document-a");
    assert_eq!(action.into_value(), NonClonePayload(42));
}

#[test]
fn late_admission_can_be_consumed_to_execute_deadline_batch() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, &'static str>::new();
    window.push(1, WindowAction::Ordered("first"), base);

    let admission = window.push(
        1,
        WindowAction::Ordered("second"),
        base + Duration::from_secs(1),
    );

    assert_eq!(admission.state(), AdmissionState::Rotated);
    assert_eq!(admission.batch_id().get(), 2);
    let ready = admission
        .into_ready_batch()
        .expect("deadline rotation must return the first batch");
    assert_eq!(ready.id().get(), 1);
    assert_eq!(ready.cause(), ActionBatchCause::Deadline);
    assert_eq!(ready.admitted_action_count(), 1);
    assert_eq!(ready.retained_action_count(), 1);
}

#[test]
fn consuming_batch_preserves_all_scroll_step_metadata() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, ()>::default();
    let scroll = ScrollAction {
        position: Point::new(12.5, 17.5),
        delta_x: -4.0,
        delta_y: 9.0,
        delta_mode: ScrollDeltaMode::Page,
        modifiers: InputModifiers::ALT | InputModifiers::META,
    };
    window.push(7, WindowAction::Scroll(scroll.clone()), base);

    let mut actions = window
        .flush(ActionBarrier::Explicit, base)
        .expect("batch")
        .into_actions();
    let PlannedAction::Scroll { scope, run } = actions.remove(0) else {
        panic!("expected scroll run");
    };

    assert_eq!(scope, 7);
    assert_eq!(run.len(), 1);
    assert!(!run.is_empty());
    assert_eq!(run.steps()[0].sequence().get(), 1);
    assert_eq!(run.steps()[0].admitted_at(), base);
    assert_eq!(run.steps()[0].value(), &scroll);
}

#[test]
fn all_mouse_buttons_and_click_counts_survive_public_model() {
    let buttons = [
        MouseButton::Left,
        MouseButton::Middle,
        MouseButton::Right,
        MouseButton::Back,
        MouseButton::Forward,
    ];

    for (index, button) in buttons.into_iter().enumerate() {
        let base = Instant::now();
        let mut window = ActionWindow::<u8, ()>::default();
        let click = ClickAction {
            position: Point::new(index as f64, 2.0),
            button,
            click_count: (index + 1) as u32,
            modifiers: InputModifiers::CONTROL,
        };
        window.push(1, WindowAction::Click(click.clone()), base);

        let batch = window
            .flush(ActionBarrier::Explicit, base)
            .expect("click batch");
        let PlannedAction::Click {
            click: scheduled, ..
        } = &batch.actions()[0]
        else {
            panic!("expected click");
        };
        assert_eq!(scheduled.value(), &click);
    }
}

#[test]
fn canceling_unknown_scope_is_a_noop_and_preserves_deadline() {
    let base = Instant::now();
    let mut window = ActionWindow::<String, ()>::default();
    window.push(
        "kept".to_owned(),
        WindowAction::Scroll(ScrollAction::pixels(Point::default(), 0.0, 1.0)),
        base,
    );
    let deadline = window.next_deadline();

    assert!(window.cancel_scope(&"missing".to_owned()).is_empty());
    assert_eq!(window.next_deadline(), deadline);
    assert_eq!(window.pending_admitted_action_count(), 1);
    assert_eq!(window.pending_retained_action_count(), 1);
    assert_eq!(window.pending_planned_action_count(), 1);
}

#[test]
fn cancel_scope_returns_retained_sequences() {
    let base = Instant::now();
    let mut window = ActionWindow::<&str, &str>::default();
    let first_scroll = window.push(
        "retired",
        WindowAction::Scroll(ScrollAction::pixels(Point::default(), 0.0, 10.0)),
        base,
    );
    let kept_scroll = window.push(
        "current",
        WindowAction::Scroll(ScrollAction::pixels(Point::default(), 0.0, 20.0)),
        base + Duration::from_millis(10),
    );
    let replaced_click = window.push(
        "retired",
        WindowAction::Click(ClickAction::new(Point::default(), MouseButton::Left, 1)),
        base + Duration::from_millis(20),
    );
    let retained_click = window.push(
        "retired",
        WindowAction::Click(ClickAction::new(Point::default(), MouseButton::Left, 2)),
        base + Duration::from_millis(30),
    );
    let ordered = window.push(
        "retired",
        WindowAction::Ordered("renderer-sidecar"),
        base + Duration::from_millis(40),
    );
    let original_deadline = window.next_deadline();

    assert_eq!(
        window.cancel_scope(&"retired"),
        vec![
            first_scroll.sequence(),
            retained_click.sequence(),
            ordered.sequence(),
        ]
    );
    assert_ne!(replaced_click.sequence(), retained_click.sequence());
    assert_eq!(window.next_deadline(), original_deadline);
    assert_eq!(window.pending_admitted_action_count(), 5);
    assert_eq!(window.pending_retained_action_count(), 1);

    let batch = window
        .flush(ActionBarrier::Explicit, base + Duration::from_millis(50))
        .expect("the other scope should remain pending");
    let PlannedAction::Scroll { scope, run } = &batch.actions()[0] else {
        panic!("the surviving action should be a scroll run");
    };
    assert_eq!(*scope, "current");
    assert_eq!(run.steps()[0].sequence(), kept_scroll.sequence());
}

#[test]
fn click_replacement_reports_public_compaction_without_rotation() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, ()>::new();
    window.push(
        1,
        WindowAction::Click(ClickAction::new(Point::new(1.0, 1.0), MouseButton::Left, 1)),
        base,
    );

    let replacement = window.push(
        1,
        WindowAction::Click(ClickAction::new(Point::new(2.0, 2.0), MouseButton::Left, 2)),
        base + Duration::from_millis(1),
    );

    assert_eq!(replacement.state(), AdmissionState::Joined);
    assert_eq!(replacement.compaction(), ActionCompaction::ReplacedClick);
    assert!(replacement.ready_batch().is_none());
}
