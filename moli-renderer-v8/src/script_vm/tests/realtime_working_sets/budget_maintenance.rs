use super::budget::managed_fixture;
use super::*;
use crate::layout_working_set_budget::{BudgetLimits, TrimStatus};

#[derive(Debug, PartialEq, Eq)]
struct LayoutObservation {
    builds: u64,
    snapshot: (
        u64,
        u64,
        u64,
        Option<(DomHandle, moli_layout::LayoutTreeRetentionMetrics)>,
    ),
    inputs: Option<String>,
    frame: Option<String>,
}

fn observe(vm: &StandaloneScriptVmHarness) -> LayoutObservation {
    let inputs = vm
        ._context_host
        .borrow()
        .realtime_layout_input_snapshot_for_test();
    let frame = vm._context_host.borrow().realtime_frame_snapshot_for_test();
    LayoutObservation {
        builds: vm.layout_pass_observability_for_test().1,
        snapshot: vm.layout_snapshot_cache_observability_for_test(),
        inputs,
        frame,
    }
}

const READ_GEOMETRY: &str = "JSON.stringify([changing.offsetWidth,stable.offsetHeight,\
    frame.offsetWidth,child.document.getElementById('target').offsetWidth])";

#[test]
fn owner_native_maintenance_releases_root_and_child_work_without_invalidating_layout() {
    let mut fixture = managed_fixture(BudgetLimits::default());
    let child = install_child(&mut fixture.vm);
    fixture
        .vm
        .eval("changing.style.width='150px';changing.offsetWidth")
        .unwrap();
    let geometry = fixture.vm.eval(READ_GEOMETRY).unwrap();
    assert_eq!(count(&fixture.vm), 2);
    let charged = fixture.pool.snapshot().charged_bytes;
    let root_bytes = main_stats(&fixture.vm).unwrap().estimated_retained_bytes;
    let child_bytes = fixture
        .vm
        ._context_host
        .borrow()
        .realtime_layout_working_set_stats_for_test(child)
        .unwrap()
        .estimated_retained_bytes;
    assert_eq!(charged, root_bytes + child_bytes);
    assert_eq!(fixture.pool.snapshot().live_leases, 2);
    assert!(charged > 1);
    let before = observe(&fixture.vm);

    // Positive pressure respects the existing grant's protection interval.
    // The clock controls ledger time, not V8 GC or an artificial frame loop.
    fixture.pool.set_pressure_target(charged - 1).unwrap();
    fixture.clock.at_millis(9);
    assert_eq!(
        fixture.vm.maintain_realtime_layout_working_set_budget(),
        None
    );
    assert_eq!(count(&fixture.vm), 2);
    assert_eq!(fixture.pool.snapshot().charged_bytes, charged);
    assert_eq!(observe(&fixture.vm), before);

    fixture.clock.at_millis(11);
    assert_eq!(
        fixture.vm.maintain_realtime_layout_working_set_budget(),
        Some(TrimStatus::Complete),
    );
    assert_eq!(count(&fixture.vm), 0);
    assert_eq!(fixture.pool.snapshot().charged_bytes, 0);
    assert_eq!(fixture.pool.snapshot().live_leases, 0);
    assert_eq!(fixture.pool.snapshot().open_pages, 1);
    assert_eq!(
        fixture
            .vm
            ._context_host
            .borrow()
            .realtime_layout_working_set_ticket_count_for_test(),
        2
    );
    assert_eq!(observe(&fixture.vm), before);
    assert_eq!(
        fixture.vm.maintain_realtime_layout_working_set_budget(),
        None
    );

    // Frozen screen geometry is still current; a getter does not build layout.
    assert_eq!(fixture.vm.eval(READ_GEOMETRY).unwrap(), geometry);
    assert_eq!(
        fixture.vm.layout_pass_observability_for_test().1,
        before.builds
    );
    fixture
        .pool
        .set_pressure_target(fixture.pool.limits().browser_bytes)
        .unwrap();
    assert_eq!(
        count(&fixture.vm),
        0,
        "recovery alone does not construct a cache"
    );
    fixture
        .vm
        .eval("changing.style.width='160px';changing.offsetWidth")
        .unwrap();
    assert_eq!(count(&fixture.vm), 2);
    assert_eq!(main_stats(&fixture.vm).unwrap().reused_box_slots, 0);
}

#[test]
fn owner_native_maintenance_keeps_checked_out_publication_charged_until_drop() {
    let mut fixture = managed_fixture(BudgetLimits::default());
    install_child(&mut fixture.vm);
    let geometry = fixture.vm.eval(READ_GEOMETRY).unwrap();
    assert_eq!(count(&fixture.vm), 2);
    let charged = fixture.pool.snapshot().charged_bytes;
    assert!(charged > 0);
    let publication = {
        let host = fixture.vm._context_host.borrow();
        let document = host.document_handle();
        host.prepare_layout_publication(
            document,
            host.layout_viewport_for_document(document),
            LayoutPublicationRequest::Screen,
        )
        .unwrap()
    };
    assert_eq!(
        count(&fixture.vm),
        0,
        "the transaction has borrowed the payloads"
    );
    assert_eq!(fixture.pool.snapshot().charged_bytes, charged);
    let before = observe(&fixture.vm);

    fixture.pool.set_pressure_target(0).unwrap();
    assert_eq!(
        fixture.vm.maintain_realtime_layout_working_set_budget(),
        Some(TrimStatus::PendingRelease),
    );
    assert_eq!(fixture.pool.snapshot().charged_bytes, charged);
    assert_eq!(fixture.pool.snapshot().live_leases, 2);
    assert_eq!(observe(&fixture.vm), before);

    fixture.clock.at_millis(1_000);
    assert_eq!(
        fixture.vm.maintain_realtime_layout_working_set_budget(),
        Some(TrimStatus::PendingRelease),
        "PendingRelease must not mark this trim epoch as applied",
    );
    assert_eq!(fixture.pool.snapshot().charged_bytes, charged);
    assert_eq!(observe(&fixture.vm), before);

    // This is the real native transaction's Drop, not a forged ledger refund.
    drop(publication);
    assert_eq!(fixture.pool.snapshot().charged_bytes, 0);
    assert_eq!(fixture.pool.snapshot().live_leases, 0);
    assert_eq!(fixture.pool.snapshot().open_pages, 1);
    assert_eq!(
        fixture.vm.maintain_realtime_layout_working_set_budget(),
        None
    );
    assert_eq!(observe(&fixture.vm), before);
    assert_eq!(fixture.vm.eval(READ_GEOMETRY).unwrap(), geometry);
    assert_eq!(
        fixture.vm.layout_pass_observability_for_test().1,
        before.builds
    );

    // Pressure zero remains disabled even after an explicit source change.
    assert_eq!(
        fixture
            .vm
            .eval("changing.style.width='120px';String(changing.offsetWidth)")
            .unwrap(),
        "120"
    );
    assert_eq!(count(&fixture.vm), 0);
    assert_eq!(fixture.pool.snapshot().charged_bytes, 0);
    assert_eq!(fixture.pool.snapshot().live_leases, 0);
}
