use super::*;
use crate::{
    PageId,
    layout_working_set_budget::{
        BudgetClock, BudgetLimits, BudgetTiming, LayoutWorkingSetBudget, PageRegistration,
        TrimCause,
    },
    runtime::RendererPageToken,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) struct ManualBudgetClock {
    origin: Instant,
    now: parking_lot::Mutex<Instant>,
}

impl ManualBudgetClock {
    fn new() -> Self {
        let origin = Instant::now();
        Self {
            origin,
            now: parking_lot::Mutex::new(origin),
        }
    }

    pub(super) fn at_millis(&self, milliseconds: u64) {
        *self.now.lock() = self.origin + Duration::from_millis(milliseconds);
    }
}

impl BudgetClock for ManualBudgetClock {
    fn now(&self) -> Instant {
        *self.now.lock()
    }
}

pub(super) struct ManagedFixture {
    // Drop the actual VM payloads before the stable test registration.
    pub(super) vm: StandaloneScriptVmHarness,
    pub(super) _page: PageRegistration,
    pub(super) pool: LayoutWorkingSetBudget,
    pub(super) clock: Arc<ManualBudgetClock>,
}

fn budget_pool(limits: BudgetLimits) -> (LayoutWorkingSetBudget, Arc<ManualBudgetClock>) {
    let clock = Arc::new(ManualBudgetClock::new());
    let pool = LayoutWorkingSetBudget::with_clock(
        limits,
        BudgetTiming {
            protection: Duration::from_millis(10),
            idle_after: Duration::from_millis(30),
            demand_ttl: Duration::from_millis(100),
            claim_ttl: Duration::from_millis(20),
        },
        clock.clone(),
    )
    .unwrap();
    (pool, clock)
}

fn managed_fixture_with(
    pool: LayoutWorkingSetBudget,
    clock: Arc<ManualBudgetClock>,
    page_id: u64,
) -> ManagedFixture {
    let page = pool
        .register_page(RendererPageToken::new_for_testing(PageId::new_for_testing(
            page_id,
        )))
        .unwrap();
    let vm = fixture();
    {
        let host = vm._context_host.borrow();
        host.bind_realtime_layout_working_set_budget(Some(page.handle()))
            .unwrap();
        host.set_realtime_opportunity_clock_for_test(clock.now());
    }
    ManagedFixture {
        vm,
        _page: page,
        pool,
        clock,
    }
}

pub(super) fn managed_fixture(limits: BudgetLimits) -> ManagedFixture {
    let (pool, clock) = budget_pool(limits);
    managed_fixture_with(pool, clock, 1)
}

fn tickets(vm: &StandaloneScriptVmHarness) -> usize {
    vm._context_host
        .borrow()
        .realtime_layout_working_set_ticket_count_for_test()
}

fn charge(vm: &StandaloneScriptVmHarness, document: DomHandle) -> usize {
    vm._context_host
        .borrow()
        .realtime_layout_working_set_stats_for_test(document)
        .map_or(0, |stats| stats.estimated_retained_bytes)
}

fn force_screen_pass(vm: &StandaloneScriptVmHarness, limit: Option<usize>) {
    let host = vm._context_host.borrow();
    let document = host.document_handle();
    let screen = host.layout_viewport_for_document(document);
    let mut publication = host
        .prepare_layout_publication(document, screen, LayoutPublicationRequest::Screen)
        .unwrap();
    if let Some(limit) = limit {
        publication
            .working_sets_for_pass()
            .unwrap()
            .set_retention_limit_for_test(limit);
    }
    let pass = host
        .build_layout_pass_for_document_in_environment(
            document,
            LayoutPassRequest::new(screen, LayoutFlushReason::Test),
            None,
            publication.working_sets_for_pass(),
        )
        .unwrap()
        .unwrap();
    host.finish_layout_publication(publication, pass).unwrap();
}

fn occupy_peer(peer: &mut ManagedFixture) -> usize {
    // A real, larger tree ensures a complete candidate of the smaller fixture
    // fits the target individually, while the occupied pool cannot fit both.
    peer.vm
        .eval(
            "for(let i=0;i<32;i++){const n=document.createElement('div');\
             n.textContent='peer text '+i;document.body.append(n)}changing.offsetWidth",
        )
        .unwrap();
    assert_eq!(count(&peer.vm), 1);
    main_stats(&peer.vm).unwrap().estimated_retained_bytes
}

#[test]
fn managed_working_sets_charge_real_payloads_through_success_and_eviction() {
    let mut managed = managed_fixture(BudgetLimits::default());
    let child = install_child(&mut managed.vm);
    let root = managed.vm._context_host.borrow().document_handle();
    let old_charge = charge(&managed.vm, root) + charge(&managed.vm, child);
    assert!(old_charge > 0);
    assert_eq!(managed.pool.snapshot().charged_bytes, old_charge);
    assert_eq!(managed.pool.snapshot().live_leases, 2);
    assert_eq!(tickets(&managed.vm), 2);

    managed.vm.eval("changing.style.width='150px'").unwrap();
    {
        let host = managed.vm._context_host.borrow();
        let screen = host.layout_viewport_for_document(root);
        let mut publication = host
            .prepare_layout_publication(root, screen, LayoutPublicationRequest::Screen)
            .unwrap();
        assert_eq!(host.realtime_layout_working_set_count_for_test(), 0);
        assert_eq!(managed.pool.snapshot().charged_bytes, old_charge);
        let pass = host
            .build_layout_pass_for_document_in_environment(
                root,
                LayoutPassRequest::new(screen, LayoutFlushReason::Test),
                None,
                publication.working_sets_for_pass(),
            )
            .unwrap()
            .unwrap();
        // Completed candidates are still a transaction. The previous leases
        // remain charged until successful publication resizes them atomically.
        assert_eq!(managed.pool.snapshot().charged_bytes, old_charge);
        assert_eq!(managed.pool.snapshot().live_leases, 2);
        host.finish_layout_publication(publication, pass).unwrap();
    }
    assert_eq!(count(&managed.vm), 2);
    assert_eq!(managed.pool.snapshot().live_leases, 2);
    assert_eq!(
        managed.pool.snapshot().charged_bytes,
        charge(&managed.vm, root) + charge(&managed.vm, child)
    );
    assert_eq!(
        managed.vm.eval("String(changing.offsetWidth)").unwrap(),
        "150"
    );

    let before = managed.vm.layout_pass_observability_for_test().1;
    managed
        .vm
        ._context_host
        .borrow()
        .clear_realtime_layout_working_sets_for_test();
    assert_eq!(managed.pool.snapshot().charged_bytes, 0);
    assert_eq!(managed.pool.snapshot().live_leases, 0);
    assert_eq!(
        tickets(&managed.vm),
        2,
        "cache eviction preserves exact-slot residence"
    );
    assert_eq!(
        managed.vm.eval("String(changing.offsetWidth)").unwrap(),
        "150"
    );
    assert_eq!(managed.vm.layout_pass_observability_for_test().1, before);
}

#[test]
fn failed_managed_publication_never_queues_a_candidate_or_trims_a_peer() {
    for change_inputs in [false, true] {
        let (pool, clock) = budget_pool(BudgetLimits::default());
        let mut peer = managed_fixture_with(pool.clone(), clock.clone(), 1);
        let mut candidate = managed_fixture_with(pool.clone(), clock.clone(), 2);
        let peer_charge = occupy_peer(&mut peer);
        pool.set_pressure_target(peer_charge).unwrap();
        clock.at_millis(11);
        let host = candidate.vm._context_host.clone();
        let screen = viewport(&candidate.vm);
        let builds = candidate.vm.layout_pass_observability_for_test().1;
        let consumer_entered = std::cell::Cell::new(false);
        let result = candidate.vm.with_fresh_layout_pass(
            LayoutPassRequest::new(screen, LayoutFlushReason::Test),
            LayoutPublicationRequest::Screen,
            |_| {
                consumer_entered.set(true);
                if change_inputs {
                    host.borrow().note_realtime_layout_input_change();
                    Ok(())
                } else {
                    Err(LayoutError::source_contract(
                        "test output",
                        "consumer failed",
                    ))
                }
            },
        );
        assert!(
            consumer_entered.get(),
            "the real pass must reach its output consumer"
        );
        assert_eq!(
            candidate.vm.layout_pass_observability_for_test().1,
            builds + 1
        );
        assert!(matches!(result, Err(LayoutError::SourceContract { .. })));
        let after = pool.snapshot();
        assert_eq!(after.charged_bytes, peer_charge);
        assert_eq!(after.live_leases, 1);
        assert_eq!(after.queued_pages, 0);
        assert_eq!(after.trim_requests, 0);
        assert_eq!(count(&candidate.vm), 0);
        assert_eq!(count(&peer.vm), 1);

        // The same real candidate can wait after a successful publication.
        // This distinguishes publish failure from a test that never reached
        // a meaningful Browser-capacity conflict.
        force_screen_pass(&candidate.vm, None);
        let after_success = pool.snapshot();
        assert_eq!(after_success.queued_pages, 1);
        assert_eq!(after_success.charged_bytes, peer_charge);
        assert_eq!(after_success.trim_requests, 1);
        assert_eq!(
            candidate.vm.eval("String(changing.offsetWidth)").unwrap(),
            "100"
        );
    }
}

#[test]
fn managed_pressure_zero_rejects_completed_candidates_and_recovers_on_real_demand() {
    let mut managed = managed_fixture(BudgetLimits::default());
    managed.vm.eval("changing.offsetWidth").unwrap();
    let retained = managed.pool.snapshot().charged_bytes;
    assert!(retained > 0);
    {
        let host = managed.vm._context_host.borrow();
        let root = host.document_handle();
        let screen = host.layout_viewport_for_document(root);
        let mut publication = host
            .prepare_layout_publication(root, screen, LayoutPublicationRequest::Screen)
            .unwrap();
        let pass = host
            .build_layout_pass_for_document_in_environment(
                root,
                LayoutPassRequest::new(screen, LayoutFlushReason::Test),
                None,
                publication.working_sets_for_pass(),
            )
            .unwrap()
            .unwrap();
        managed.pool.set_pressure_target(0).unwrap();
        assert_eq!(managed.pool.snapshot().charged_bytes, retained);
        host.finish_layout_publication(publication, pass).unwrap();
    }
    assert_eq!(count(&managed.vm), 0);
    assert_eq!(managed.pool.snapshot().charged_bytes, 0);
    assert_eq!(managed.pool.snapshot().queued_pages, 0);
    assert_eq!(tickets(&managed.vm), 1);
    assert_eq!(
        managed
            .vm
            .eval("changing.style.width='120px';String(changing.offsetWidth)")
            .unwrap(),
        "120"
    );
    assert_eq!(count(&managed.vm), 0);
    assert_eq!(managed.pool.snapshot().live_leases, 0);
    managed
        .pool
        .set_pressure_target(managed.pool.limits().browser_bytes)
        .unwrap();
    assert_eq!(
        count(&managed.vm),
        0,
        "restoring a target does not warm a cache"
    );
    managed
        .vm
        .eval("changing.style.width='130px';changing.offsetWidth")
        .unwrap();
    assert_eq!(count(&managed.vm), 1);
    assert_eq!(main_stats(&managed.vm).unwrap().reused_box_slots, 0);
    assert!(managed.pool.snapshot().charged_bytes > 0);
}

#[test]
fn managed_tickets_retire_with_documents_and_closed_hosts_not_cache_eviction() {
    let mut managed = managed_fixture(BudgetLimits::default());
    install_child(&mut managed.vm);
    assert_eq!(tickets(&managed.vm), 2);
    managed
        .vm
        .eval("frame.style.display='none';changing.offsetWidth")
        .unwrap();
    assert_eq!(tickets(&managed.vm), 1);
    managed
        .vm
        .eval("frame.style.display='block';changing.offsetWidth")
        .unwrap();
    assert_eq!(tickets(&managed.vm), 2);
    managed.vm.eval("child.document.open()").unwrap();
    assert_eq!(tickets(&managed.vm), 1);
    managed.vm.eval("document.open()").unwrap();
    assert_eq!(tickets(&managed.vm), 0);
    assert_eq!(managed.pool.snapshot().charged_bytes, 0);
    assert_eq!(managed.pool.snapshot().queued_pages, 0);
    managed.vm.eval("document.write('<!doctype html><div id=next style=width:70px>next</div>');document.close();next.offsetWidth").unwrap();
    assert_eq!(tickets(&managed.vm), 1);
    assert_eq!(main_stats(&managed.vm).unwrap().reused_box_slots, 0);
    let host = managed.vm._context_host.clone();
    managed
        .vm
        .close_page_context_resources_for_context_teardown();
    assert_eq!(
        host.borrow()
            .realtime_layout_working_set_ticket_count_for_test(),
        0
    );
    assert_eq!(
        host.borrow().realtime_layout_working_set_count_for_test(),
        0
    );
    assert_eq!(managed.pool.snapshot().charged_bytes, 0);
    assert_eq!(managed.pool.snapshot().live_leases, 0);
    assert_eq!(
        managed.pool.snapshot().open_pages,
        1,
        "old Host teardown cannot retire the stable Page"
    );
    managed._page.close();
    assert_eq!(managed.pool.snapshot().open_pages, 0);
}

#[test]
fn local_candidate_spool_keeps_root_before_earlier_completed_child() {
    let mut managed = managed_fixture(BudgetLimits::default());
    let child = install_child(&mut managed.vm);
    let root = managed.vm._context_host.borrow().document_handle();
    let root_charge = charge(&managed.vm, root);
    let child_charge = charge(&managed.vm, child);
    let limit = root_charge.max(child_charge);
    assert!(root_charge > 0 && child_charge > 0 && limit < root_charge + child_charge);
    force_screen_pass(&managed.vm, Some(limit));
    assert_eq!(count(&managed.vm), 1);
    assert!(
        main_stats(&managed.vm).is_some(),
        "root has the first visit despite completing last"
    );
    assert_eq!(charge(&managed.vm, child), 0);
    assert_eq!(
        managed.pool.snapshot().charged_bytes,
        charge(&managed.vm, root)
    );
    assert_eq!(
        managed.pool.snapshot().queued_pages,
        0,
        "local rejection is not Browser demand"
    );
    assert_eq!(
        managed
            .vm
            .eval("String(child.document.getElementById('target').offsetWidth)")
            .unwrap(),
        "80"
    );
}

#[test]
fn local_candidate_spool_gives_existing_child_demand_priority_over_root() {
    let mut managed = managed_fixture(BudgetLimits::default());
    let child = install_child(&mut managed.vm);
    let root = managed.vm._context_host.borrow().document_handle();
    let limit = charge(&managed.vm, root).max(charge(&managed.vm, child));
    managed
        .vm
        ._context_host
        .borrow()
        .clear_realtime_layout_working_sets_for_test();
    managed.pool.set_pressure_target(limit).unwrap();
    force_screen_pass(&managed.vm, None);
    let waiting = managed.pool.snapshot();
    assert_eq!(count(&managed.vm), 1);
    assert!(main_stats(&managed.vm).is_some());
    assert_eq!(waiting.queued_pages, 1);
    assert_eq!(waiting.pages[0].demand.unwrap().slot.document, child);
    assert_eq!(
        tickets(&managed.vm),
        2,
        "the refused child still owns its demand ticket"
    );

    // A whole child fits, but root+child do not. The existing exact child
    // demand wins the local spool before shared Browser admission is retried.
    force_screen_pass(&managed.vm, Some(limit));
    assert_eq!(count(&managed.vm), 1);
    assert!(main_stats(&managed.vm).is_none());
    assert!(charge(&managed.vm, child) > 0);
    assert_eq!(
        managed.pool.snapshot().charged_bytes,
        charge(&managed.vm, child)
    );
    assert_eq!(managed.pool.snapshot().queued_pages, 0);
    assert_eq!(managed.vm.eval("JSON.stringify([changing.offsetWidth,child.document.getElementById('target').offsetWidth])").unwrap(), "[100,80]");
}

#[test]
fn retired_main_owner_keeps_inflight_payload_charged_until_failed_publication_drops() {
    let mut managed = managed_fixture(BudgetLimits::default());
    managed.vm.eval("changing.offsetWidth").unwrap();
    let retained = managed.pool.snapshot().charged_bytes;
    let (publication, pass) = {
        let host = managed.vm._context_host.borrow();
        let root = host.document_handle();
        let screen = host.layout_viewport_for_document(root);
        let mut publication = host
            .prepare_layout_publication(root, screen, LayoutPublicationRequest::Screen)
            .unwrap();
        let pass = host
            .build_layout_pass_for_document_in_environment(
                root,
                LayoutPassRequest::new(screen, LayoutFlushReason::Test),
                None,
                publication.working_sets_for_pass(),
            )
            .unwrap()
            .unwrap();
        (publication, pass)
    };
    managed.vm.eval("document.open()").unwrap();
    assert_eq!(tickets(&managed.vm), 0);
    assert_eq!(managed.pool.snapshot().charged_bytes, retained);
    assert_eq!(managed.pool.snapshot().live_leases, 1);
    let result = managed
        .vm
        ._context_host
        .borrow()
        .finish_layout_publication(publication, pass);
    assert!(matches!(result, Err(LayoutError::SourceContract { .. })));
    assert_eq!(managed.pool.snapshot().charged_bytes, 0);
    assert_eq!(managed.pool.snapshot().live_leases, 0);
    assert_eq!(managed.pool.snapshot().queued_pages, 0);
    managed.vm.eval("document.write('<!doctype html><div id=next style=width:70px>next</div>');document.close();next.offsetWidth").unwrap();
    assert_eq!(tickets(&managed.vm), 1);
    assert_eq!(count(&managed.vm), 1);
    assert_eq!(main_stats(&managed.vm).unwrap().reused_box_slots, 0);
}

#[test]
fn native_root_reset_cancels_dead_head_before_unrelated_child_ticket_drop() {
    let (pool, clock) = budget_pool(BudgetLimits::default());
    let mut peer = managed_fixture_with(pool.clone(), clock.clone(), 1);
    let mut waiting = managed_fixture_with(pool.clone(), clock.clone(), 2);
    let peer_charge = occupy_peer(&mut peer);
    pool.set_pressure_target(peer_charge).unwrap();
    install_child(&mut waiting.vm);
    let root = waiting.vm._context_host.borrow().document_handle();
    assert_eq!(count(&waiting.vm), 0);
    assert_eq!(tickets(&waiting.vm), 2);
    let before = pool.snapshot();
    assert_eq!(before.queued_pages, 1);
    assert_eq!(before.trim_requests, 0);
    let demand = before.pages.iter().find_map(|page| page.demand).unwrap();
    assert_eq!(demand.slot.document, root);
    assert!(demand.bytes <= peer_charge);

    // The peer becomes eligible for a trim at exactly this boundary, but
    // the demand is already known dead when the native registry resets.
    // Canceling an unrelated child first would advance that stale demand.
    clock.at_millis(11);
    waiting
        .vm
        ._context_host
        .borrow()
        .reset_document_layout_state();
    let after = pool.snapshot();
    assert_eq!(tickets(&waiting.vm), 0);
    assert_eq!(after.queued_pages, 0);
    assert_eq!(after.trim_requests, 0);
    assert!(after.pages.iter().all(|page| page.trim.is_none()));
    assert_eq!(after.charged_bytes, peer_charge);
    assert_eq!(count(&peer.vm), 1);
}

#[test]
fn hidden_head_is_canceled_before_other_unvisited_tickets_at_publication() {
    let (pool, clock) = budget_pool(BudgetLimits::default());
    let mut peer = managed_fixture_with(pool.clone(), clock.clone(), 1);
    let mut waiting = managed_fixture_with(pool.clone(), clock.clone(), 2);
    let peer_charge = occupy_peer(&mut peer);
    let child = install_child(&mut waiting.vm);
    waiting
        .vm
        .eval(
            "for(let i=0;i<16;i++){const n=child.document.createElement('div');\
         n.textContent='waiting child text '+i;child.document.body.append(n)}frame.offsetWidth",
        )
        .unwrap();
    waiting.vm.eval(
        "globalThis.extra=document.createElement('iframe');extra.style.cssText='width:80px;height:40px';\
         document.body.append(extra);const d=extra.contentDocument;d.open();\
         d.write('<!doctype html><div>second child</div>');d.close();extra.offsetWidth",
    ).unwrap();
    assert_eq!(count(&waiting.vm), 3);
    let root_charge = main_stats(&waiting.vm).unwrap().estimated_retained_bytes;
    let child_charge = charge(&waiting.vm, child);
    assert!(
        root_charge < child_charge && child_charge <= peer_charge,
        "self-trimming root {root_charge} must still leave a shortfall for child {child_charge}; peer {peer_charge} can satisfy it"
    );
    // Hidden source-frontier metadata can change the root's retained charge.
    // Reserve measured headroom for that root, but not enough for the child
    // even after the requester's own root has actually been released.
    let root_allowance = root_charge + (child_charge - root_charge) / 2;
    assert!(root_charge < root_allowance && root_allowance < child_charge);
    waiting
        .vm
        ._context_host
        .borrow()
        .clear_realtime_layout_working_sets_for_test();
    pool.set_pressure_target(peer_charge + root_allowance)
        .unwrap();
    force_screen_pass(&waiting.vm, None);
    assert_eq!(count(&waiting.vm), 1);
    assert_eq!(tickets(&waiting.vm), 3);
    let before = pool.snapshot();
    assert_eq!(before.queued_pages, 1);
    assert_eq!(
        before.trim_requests, 1,
        "the requester's own root already needs SelfTrim"
    );
    let requester = before
        .pages
        .iter()
        .find(|page| page.demand.is_some())
        .unwrap();
    let waiting_demand = requester.demand.unwrap();
    assert!(before.pressure_target - before.charged_bytes < waiting_demand.bytes);
    assert!(root_allowance < waiting_demand.bytes);
    assert_eq!(requester.trim.unwrap().cause, TrimCause::SelfTrim);
    assert!(
        before
            .pages
            .iter()
            .filter(|page| page.demand.is_none())
            .all(|page| page.trim.is_none())
    );
    assert_eq!(
        before
            .pages
            .iter()
            .find_map(|page| page.demand)
            .unwrap()
            .slot
            .document,
        child
    );
    waiting
        .vm
        .eval("frame.style.display='none';extra.style.display='none'")
        .unwrap();
    let (publication, pass) = {
        let host = waiting.vm._context_host.borrow();
        let root = host.document_handle();
        let screen = host.layout_viewport_for_document(root);
        let mut publication = host
            .prepare_layout_publication(root, screen, LayoutPublicationRequest::Screen)
            .unwrap();
        let pass = host
            .build_layout_pass_for_document_in_environment(
                root,
                LayoutPassRequest::new(screen, LayoutFlushReason::Test),
                None,
                publication.working_sets_for_pass(),
            )
            .unwrap()
            .unwrap();
        (publication, pass)
    };
    clock.at_millis(11);
    waiting
        .vm
        ._context_host
        .borrow()
        .finish_layout_publication(publication, pass)
        .unwrap();
    let after = pool.snapshot();
    assert_eq!(tickets(&waiting.vm), 1);
    assert_eq!(
        after.queued_pages,
        0,
        "hidden-child publication: peer_charge={peer_charge}, root_charge={root_charge}, \
         root_allowance={root_allowance}, child_charge={child_charge}, root={:?}, old_child={child:?}, \
         retained_root={:?}, before={before:#?}, after={after:#?}",
        waiting.vm._context_host.borrow().document_handle(),
        main_stats(&waiting.vm),
    );
    assert_eq!(
        after.trim_requests, before.trim_requests,
        "a dead child demand must never add a peer Fairness trim"
    );
    assert!(after.pages.iter().all(|page| page.trim.is_none()));
    assert_eq!(count(&peer.vm), 1);
    let retained_root = main_stats(&waiting.vm).unwrap().estimated_retained_bytes;
    assert!(
        retained_root <= root_allowance,
        "published root {retained_root} exceeded measured allowance {root_allowance}"
    );
    assert_eq!(after.charged_bytes, peer_charge + retained_root);
    assert_eq!(
        waiting
            .vm
            .eval("JSON.stringify([changing.offsetWidth,frame.offsetWidth,extra.offsetWidth])")
            .unwrap(),
        "[100,0,0]"
    );
}

#[test]
fn a_late_managed_binding_cannot_publish_an_unbudgeted_inflight_candidate() {
    for enabled in [false, true] {
        let (pool, _clock) = budget_pool(BudgetLimits::default());
        let page = enabled.then(|| {
            pool.register_page(RendererPageToken::new_for_testing(PageId::new_for_testing(
                1,
            )))
            .unwrap()
        });
        let mut vm = fixture();
        vm.eval("changing.offsetWidth").unwrap();
        assert_eq!(count(&vm), 1);
        {
            let host = vm._context_host.borrow();
            let root = host.document_handle();
            let screen = host.layout_viewport_for_document(root);
            let mut publication = host
                .prepare_layout_publication(root, screen, LayoutPublicationRequest::Screen)
                .unwrap();
            let pass = host
                .build_layout_pass_for_document_in_environment(
                    root,
                    LayoutPassRequest::new(screen, LayoutFlushReason::Test),
                    None,
                    publication.working_sets_for_pass(),
                )
                .unwrap()
                .unwrap();
            host.bind_realtime_layout_working_set_budget(
                page.as_ref().map(PageRegistration::handle),
            )
            .unwrap();
            host.finish_layout_publication(publication, pass).unwrap();
        }
        assert_eq!(count(&vm), 0);
        assert_eq!(
            tickets(&vm),
            0,
            "a late unmanaged completion cannot resurrect old tickets"
        );
        assert_eq!(pool.snapshot().charged_bytes, 0);
        assert_eq!(pool.snapshot().live_leases, 0);
        assert_eq!(
            vm.eval("changing.style.width='140px';String(changing.offsetWidth)")
                .unwrap(),
            "140"
        );
        assert_eq!(count(&vm), usize::from(enabled));
        assert_eq!(tickets(&vm), usize::from(enabled));
        assert_eq!(pool.snapshot().live_leases, usize::from(enabled));
        if enabled {
            assert!(pool.snapshot().charged_bytes > 0);
        } else {
            assert_eq!(pool.snapshot().charged_bytes, 0);
        }
    }
}

#[test]
fn exhausted_document_registration_keeps_real_geometry_without_local_retention() {
    let mut managed = managed_fixture(BudgetLimits::default());
    managed.pool.set_next_id_for_test(u64::MAX);
    assert_eq!(
        managed
            .vm
            .eval("changing.style.width='140px';String(changing.offsetWidth)")
            .unwrap(),
        "140"
    );
    assert_eq!(count(&managed.vm), 0);
    assert_eq!(tickets(&managed.vm), 0);
    assert_eq!(managed.pool.snapshot().charged_bytes, 0);
    assert_eq!(managed.pool.snapshot().live_leases, 0);
    assert_eq!(managed.pool.snapshot().queued_pages, 0);
}
