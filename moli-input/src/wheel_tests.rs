use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Document(u64);

type Controller = WheelInputController<Document, u32, u32>;

const DOCUMENT: Document = Document(1);

fn wheel(timestamp: Instant, delta_y: f64) -> WheelInput {
    WheelInput {
        position: (20.0, 30.0),
        delta: (0.0, delta_y),
        modifiers: 0,
        timestamp,
        phase: WheelPhase::Unspecified,
        source: WheelSource::Platform,
    }
}

#[test]
fn inferred_sequence_refreshes_and_expires_at_its_deadline() {
    let now = Instant::now();
    let mut controller = Controller::default();
    let first = controller.prepare(wheel(now, 10.0), DOCUMENT);
    assert_eq!(first.phase, WheelPhase::Began);
    assert_eq!(
        controller.next_deadline(),
        Some(now + Duration::from_millis(500))
    );
    controller.acknowledge(first, Some(WheelDispatchOutcome::Scrolled), Some(DOCUMENT));

    let next_time = now + Duration::from_millis(499);
    let second = controller.prepare(wheel(next_time, 10.0), DOCUMENT);
    assert_eq!(second.phase, WheelPhase::Changed);
    assert!(!second.cancelable);
    let deadline = next_time + Duration::from_millis(500);
    controller.expire(deadline - Duration::from_millis(1));
    assert_eq!(controller.next_deadline(), Some(deadline));
    controller.expire(deadline);
    assert_eq!(controller.next_deadline(), None);

    let next = controller.prepare(wheel(deadline, 10.0), DOCUMENT);
    assert_eq!(next.phase, WheelPhase::Began);
    assert!(next.cancelable);
}

#[test]
fn motion_slop_and_modifier_changes_bound_inferred_sequences() {
    let now = Instant::now();
    let first_input = wheel(now, 10.0);
    for (x, modifiers, phase) in [
        (29.0, 0, WheelPhase::Changed),
        (30.0, 0, WheelPhase::Began),
        (20.0, crate::InputModifiers::SHIFT.bits(), WheelPhase::Began),
    ] {
        let mut controller = Controller::default();
        let first = controller.prepare(first_input, DOCUMENT);
        controller.acknowledge(first, Some(WheelDispatchOutcome::Scrolled), Some(DOCUMENT));

        let next = controller.prepare(
            WheelInput {
                position: (x, 30.0),
                modifiers,
                ..first_input
            },
            DOCUMENT,
        );
        assert_eq!(next.phase, phase, "x={x}, modifiers={modifiers}");
        assert_eq!(next.cancelable, phase == WheelPhase::Began);
    }
}

#[test]
fn began_ack_commits_mode_and_targets_only_for_the_observed_dispatch() {
    use WheelDispatchOutcome::{Canceled, Scrolled, Unconsumed};

    let now = Instant::now();
    for (delta, outcome, cancelable, event_target, scroll_target) in [
        (10.0, None, true, None, None),
        (10.0, Some(Canceled), true, None, None),
        (0.0, Some(Unconsumed), true, Some(7), None),
        (10.0, Some(Scrolled), false, Some(7), Some(8)),
        (10.0, Some(Unconsumed), false, Some(7), Some(8)),
    ] {
        let mut controller = Controller::default();
        let first = PreparedWheel {
            event_target: Some(7),
            scroll_target: Some(8),
            ..controller.prepare(wheel(now, delta), DOCUMENT)
        };
        assert!(first.cancelable);
        controller.acknowledge(first, outcome, Some(DOCUMENT));

        let next = controller.prepare(wheel(now, 10.0), DOCUMENT);
        assert_eq!(
            (next.cancelable, next.event_target, next.scroll_target),
            (cancelable, event_target, scroll_target),
            "delta={delta}, outcome={outcome:?}"
        );
    }
}

#[test]
fn canceled_update_releases_the_event_target_and_keeps_the_scroll_latch() {
    let now = Instant::now();
    let mut controller = Controller::default();
    let zero = controller.prepare(wheel(now, 0.0), DOCUMENT);
    controller.acknowledge(zero, Some(WheelDispatchOutcome::Unconsumed), Some(DOCUMENT));
    let first_scroll = PreparedWheel {
        event_target: Some(7),
        scroll_target: Some(8),
        ..controller.prepare(wheel(now, 10.0), DOCUMENT)
    };
    controller.acknowledge(
        first_scroll,
        Some(WheelDispatchOutcome::Scrolled),
        Some(DOCUMENT),
    );

    let canceled = controller.prepare(wheel(now, 10.0), DOCUMENT);
    assert!(canceled.cancelable);
    controller.acknowledge(
        canceled,
        Some(WheelDispatchOutcome::Canceled),
        Some(DOCUMENT),
    );

    let next = controller.prepare(wheel(now, 10.0), DOCUMENT);
    assert_eq!(next.event_target, None);
    assert_eq!(next.scroll_target, Some(8));
    assert!(next.cancelable);
}

#[test]
fn reversing_relatches_only_when_the_first_scroll_update_was_unconsumed() {
    let now = Instant::now();
    for (first_outcome, expected_phase) in [
        (WheelDispatchOutcome::Scrolled, WheelPhase::Changed),
        (WheelDispatchOutcome::Unconsumed, WheelPhase::Began),
    ] {
        let mut controller = Controller::default();
        let first = PreparedWheel {
            scroll_target: Some(8),
            ..controller.prepare(wheel(now, 10.0), DOCUMENT)
        };
        controller.acknowledge(first, Some(first_outcome), Some(DOCUMENT));

        let stationary = controller.prepare(wheel(now, 0.0), DOCUMENT);
        assert_eq!(stationary.phase, WheelPhase::Changed);
        controller.acknowledge(
            stationary,
            Some(WheelDispatchOutcome::Unconsumed),
            Some(DOCUMENT),
        );
        let reversed = controller.prepare(wheel(now, -10.0), DOCUMENT);
        assert_eq!(reversed.phase, expected_phase);
        assert_eq!(
            reversed.scroll_target,
            (expected_phase == WheelPhase::Changed).then_some(8)
        );
    }
}

#[test]
fn first_nonzero_update_supplies_the_direction_after_zero_began() {
    let now = Instant::now();
    let mut controller = Controller::default();
    let zero = controller.prepare(wheel(now, 0.0), DOCUMENT);
    controller.acknowledge(zero, Some(WheelDispatchOutcome::Unconsumed), Some(DOCUMENT));

    let first_scroll = controller.prepare(wheel(now, 10.0), DOCUMENT);
    assert_eq!(first_scroll.phase, WheelPhase::Changed);
    controller.acknowledge(
        first_scroll,
        Some(WheelDispatchOutcome::Unconsumed),
        Some(DOCUMENT),
    );
    let forward = controller.prepare(wheel(now, 5.0), DOCUMENT);
    assert_eq!(forward.phase, WheelPhase::Changed);
    let reverse = controller.prepare(wheel(now, -5.0), DOCUMENT);
    assert_eq!(reverse.phase, WheelPhase::Began);
}

#[test]
fn direct_began_preserves_the_current_latch_until_its_end() {
    let now = Instant::now();
    let mut controller = Controller::default();
    let first = PreparedWheel {
        event_target: Some(7),
        scroll_target: Some(8),
        ..controller.prepare(wheel(now, 10.0), DOCUMENT)
    };
    controller.acknowledge(first, Some(WheelDispatchOutcome::Scrolled), Some(DOCUMENT));
    let deadline = controller.next_deadline();

    let direct = controller.prepare(
        WheelInput {
            position: (50.0, 30.0),
            phase: WheelPhase::Began,
            source: WheelSource::Direct,
            ..wheel(now, 10.0)
        },
        DOCUMENT,
    );
    assert!(direct.cancelable);
    assert_eq!(direct.event_target, None);
    assert_eq!(direct.scroll_target, Some(8));
    controller.acknowledge(
        PreparedWheel {
            event_target: Some(9),
            ..direct
        },
        Some(WheelDispatchOutcome::Scrolled),
        Some(DOCUMENT),
    );

    controller.end_transaction();
    assert_eq!(controller.next_deadline(), deadline);
    let resumed = controller.prepare(wheel(now, 10.0), DOCUMENT);
    assert_eq!(resumed.phase, WheelPhase::Changed);
    assert!(!resumed.cancelable);
    assert_eq!(resumed.event_target, None);
    assert_eq!(resumed.scroll_target, None);
}

#[test]
fn platform_end_releases_both_targets_and_the_inferred_deadline() {
    let now = Instant::now();
    let mut controller = Controller::default();
    let first = PreparedWheel {
        event_target: Some(7),
        scroll_target: Some(8),
        ..controller.prepare(wheel(now, 10.0), DOCUMENT)
    };
    controller.acknowledge(first, Some(WheelDispatchOutcome::Scrolled), Some(DOCUMENT));
    controller.end_platform_sequence();

    assert_eq!(controller.next_deadline(), None);
    let next = controller.prepare(
        WheelInput {
            phase: WheelPhase::Changed,
            ..wheel(now, 10.0)
        },
        DOCUMENT,
    );
    assert!(!next.cancelable);
    assert_eq!(next.event_target, None);
    assert_eq!(next.scroll_target, None);
}

#[test]
fn document_retirement_discards_old_targets_and_preserves_the_current_ack() {
    let now = Instant::now();
    let replacement = Document(2);
    for retire_before_ack in [false, true] {
        let mut controller = Controller::default();
        let pending = PreparedWheel {
            event_target: Some(7),
            scroll_target: Some(8),
            ..controller.prepare(wheel(now, 10.0), DOCUMENT)
        };
        let current_document = if retire_before_ack {
            controller.retire_document();
            None
        } else {
            Some(replacement)
        };
        controller.acknowledge(
            pending,
            Some(WheelDispatchOutcome::Scrolled),
            current_document,
        );

        assert_eq!(controller.next_deadline(), None);
        let next = controller.prepare(
            WheelInput {
                phase: WheelPhase::Changed,
                ..wheel(now, 10.0)
            },
            replacement,
        );
        assert!(!next.cancelable);
        assert_eq!(next.event_target, None);
        assert_eq!(next.scroll_target, None);
    }
}

#[test]
fn stale_dispatch_ack_cannot_overwrite_the_newer_mode_or_targets() {
    let now = Instant::now();
    let mut controller = Controller::default();
    let older = PreparedWheel {
        event_target: Some(3),
        scroll_target: Some(4),
        ..controller.prepare(wheel(now, 10.0), DOCUMENT)
    };
    let newer = PreparedWheel {
        event_target: Some(7),
        scroll_target: Some(8),
        ..controller.prepare(wheel(now, 10.0), DOCUMENT)
    };
    assert_eq!(newer.phase, WheelPhase::Changed);
    controller.acknowledge(newer, Some(WheelDispatchOutcome::Scrolled), Some(DOCUMENT));
    controller.acknowledge(older, Some(WheelDispatchOutcome::Scrolled), Some(DOCUMENT));

    let next = controller.prepare(wheel(now, 10.0), DOCUMENT);
    assert!(next.cancelable);
    assert_eq!(next.event_target, Some(7));
    assert_eq!(next.scroll_target, Some(8));
}
