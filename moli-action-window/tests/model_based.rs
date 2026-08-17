use std::time::{Duration, Instant};

use moli_action_window::{
    ActionBarrier, ActionBatch, ActionBatchCause, ActionWindow, ClickAction, MouseButton,
    PlannedAction, Point, ScrollAction, WindowAction,
};

#[derive(Clone, Debug, PartialEq)]
enum FlatKind {
    Scroll(u64),
    Click(u64),
    Ordered(u64),
}

#[derive(Clone, Debug, PartialEq)]
struct FlatAction {
    scope: u8,
    sequence: u64,
    kind: FlatKind,
}

#[derive(Clone, Copy)]
enum Choice {
    Scroll(u8),
    Click(u8),
    Ordered(u8),
}

const CHOICES: [Choice; 6] = [
    Choice::Scroll(0),
    Choice::Scroll(1),
    Choice::Click(0),
    Choice::Click(1),
    Choice::Ordered(0),
    Choice::Ordered(1),
];

fn action_for(choice: Choice, marker: u64) -> (u8, WindowAction<u64>, FlatKind) {
    match choice {
        Choice::Scroll(scope) => (
            scope,
            WindowAction::Scroll(ScrollAction::pixels(
                Point::new(marker as f64, scope.into()),
                0.0,
                marker as f64,
            )),
            FlatKind::Scroll(marker),
        ),
        Choice::Click(scope) => (
            scope,
            WindowAction::Click(ClickAction::new(
                Point::new(marker as f64, scope.into()),
                MouseButton::Left,
                1,
            )),
            FlatKind::Click(marker),
        ),
        Choice::Ordered(scope) => (
            scope,
            WindowAction::Ordered(marker),
            FlatKind::Ordered(marker),
        ),
    }
}

fn reference_admit(actions: &mut Vec<FlatAction>, action: FlatAction) {
    if matches!(action.kind, FlatKind::Click(_)) {
        actions.retain(|existing| {
            existing.scope != action.scope || !matches!(existing.kind, FlatKind::Click(_))
        });
    }
    actions.push(action);
}

fn expected_planned_count(actions: &[FlatAction]) -> usize {
    let mut count = 0;
    let mut previous_scroll_scope = None;
    for action in actions {
        match action.kind {
            FlatKind::Scroll(_) if previous_scroll_scope == Some(action.scope) => {}
            FlatKind::Scroll(_) => {
                count += 1;
                previous_scroll_scope = Some(action.scope);
            }
            FlatKind::Click(_) | FlatKind::Ordered(_) => {
                count += 1;
                previous_scroll_scope = None;
            }
        }
    }
    count
}

fn flatten(batch: &ActionBatch<u8, u64>) -> Vec<FlatAction> {
    let mut flattened = Vec::new();
    for action in batch.actions() {
        match action {
            PlannedAction::Scroll { scope, run } => {
                for step in run.steps() {
                    flattened.push(FlatAction {
                        scope: *scope,
                        sequence: step.sequence().get(),
                        kind: FlatKind::Scroll(step.value().delta_y as u64),
                    });
                }
            }
            PlannedAction::Click { scope, click } => flattened.push(FlatAction {
                scope: *scope,
                sequence: click.sequence().get(),
                kind: FlatKind::Click(click.value().position.x as u64),
            }),
            PlannedAction::Ordered { scope, action } => flattened.push(FlatAction {
                scope: *scope,
                sequence: action.sequence().get(),
                kind: FlatKind::Ordered(*action.value()),
            }),
        }
    }
    flattened
}

fn assert_matches_reference(
    batch: &ActionBatch<u8, u64>,
    reference: &[FlatAction],
    admitted_count: usize,
) {
    assert_eq!(batch.admitted_action_count(), admitted_count);
    assert_eq!(batch.retained_action_count(), reference.len());
    assert_eq!(
        batch.planned_action_count(),
        expected_planned_count(reference)
    );
    assert_eq!(flatten(batch), reference);

    let sequences: Vec<_> = reference.iter().map(|action| action.sequence).collect();
    assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));

    for pair in batch.actions().windows(2) {
        assert!(
            !matches!(
                pair,
                [
                    PlannedAction::Scroll {
                        scope: left_scope,
                        ..
                    },
                    PlannedAction::Scroll {
                        scope: right_scope,
                        ..
                    }
                ] if left_scope == right_scope
            ),
            "adjacent same-scope scroll runs must be normalized"
        );
    }
}

#[test]
fn exhaustive_sequences_up_to_five_actions_match_reference_compactor() {
    let base = Instant::now();
    let mut checked_cases = 0;

    for length in 0..=5_u32 {
        for encoded in 0..CHOICES.len().pow(length) {
            let mut digits = encoded;
            let mut window = ActionWindow::<u8, u64>::default();
            let mut reference = Vec::new();

            for index in 0..length as usize {
                let choice = CHOICES[digits % CHOICES.len()];
                digits /= CHOICES.len();
                let marker = index as u64 + 1;
                let (scope, action, kind) = action_for(choice, marker);
                window.push(scope, action, base + Duration::from_micros(index as u64));
                reference_admit(
                    &mut reference,
                    FlatAction {
                        scope,
                        sequence: marker,
                        kind,
                    },
                );
            }

            if length == 0 {
                assert!(window.flush(ActionBarrier::Explicit, base).is_none());
            } else {
                let batch = window
                    .flush(ActionBarrier::Explicit, base + Duration::from_millis(1))
                    .expect("non-empty sequence must flush");
                assert_matches_reference(&batch, &reference, length as usize);
            }
            checked_cases += 1;
        }
    }

    assert_eq!(checked_cases, 9_331);
}

#[test]
fn long_mixed_stream_never_rotates_before_deadline_and_matches_reference() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, u64>::new();
    let mut reference = Vec::new();

    let mut random = 0x4d59_5df4_d0f3_3173_u64;
    for index in 0..1_000_u64 {
        random = random
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let choice = CHOICES[(random as usize) % CHOICES.len()];
        let marker = index + 1;
        let (scope, action, kind) = action_for(choice, marker);

        let admission = window.push(scope, action, base + Duration::from_micros(index));
        assert!(admission.ready_batch().is_none());
        assert_eq!(admission.deadline(), base + Duration::from_secs(1));

        reference_admit(
            &mut reference,
            FlatAction {
                scope,
                sequence: marker,
                kind,
            },
        );
    }

    assert!(window.take_due(base + Duration::from_millis(999)).is_none());
    let final_batch = window
        .flush(ActionBarrier::Explicit, base + Duration::from_millis(999))
        .expect("stream should leave a final batch");
    assert_eq!(
        final_batch.cause(),
        ActionBatchCause::Barrier(ActionBarrier::Explicit)
    );
    assert_matches_reference(&final_batch, &reference, 1_000);
}

#[test]
fn every_retained_sequence_is_globally_unique_across_deadline_and_barrier_batches() {
    let base = Instant::now();
    let mut window = ActionWindow::<u8, u64>::new();
    let mut emitted_sequences = Vec::new();
    let mut causes = Vec::new();

    for (index, choice) in [Choice::Ordered(0), Choice::Ordered(1), Choice::Scroll(0)]
        .into_iter()
        .enumerate()
    {
        let marker = index as u64 + 1;
        let (scope, action, _) = action_for(choice, marker);
        let admission = window.push(scope, action, base + Duration::from_millis(marker));
        assert!(admission.ready_batch().is_none());
    }

    // Additional retained actions remain in the same fixed window.
    let (scope, action, _) = action_for(Choice::Scroll(1), 4);
    let admission = window.push(scope, action, base + Duration::from_millis(4));
    assert!(admission.ready_batch().is_none());

    // This action arrives at the original window's fixed 1001ms deadline.
    let (scope, action, _) = action_for(Choice::Click(0), 5);
    let deadline_batch = window
        .push(scope, action, base + Duration::from_millis(1_001))
        .into_ready_batch()
        .expect("late action must rotate the deadline batch");
    causes.push(deadline_batch.cause());
    emitted_sequences.extend(
        flatten(&deadline_batch)
            .into_iter()
            .map(|action| action.sequence),
    );

    let (scope, action, _) = action_for(Choice::Ordered(0), 6);
    window.push(scope, action, base + Duration::from_millis(1_002));
    let barrier_batch = window
        .flush(
            ActionBarrier::Screenshot,
            base + Duration::from_millis(1_003),
        )
        .expect("barrier must release final window");
    causes.push(barrier_batch.cause());
    emitted_sequences.extend(
        flatten(&barrier_batch)
            .into_iter()
            .map(|action| action.sequence),
    );

    let mut sorted = emitted_sequences.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), emitted_sequences.len());
    assert_eq!(sorted, (1..=6).collect::<Vec<_>>());
    assert_eq!(
        causes,
        vec![
            ActionBatchCause::Deadline,
            ActionBatchCause::Barrier(ActionBarrier::Screenshot),
        ]
    );
}
