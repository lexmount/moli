use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::*;
use crate::{
    PageId,
    document_runtime::DomHandle,
    frame_owner_model::{DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId},
    native_bridge::{OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget},
    runtime::RendererPageToken,
};

const MIB: usize = 1024 * 1024;

#[derive(Debug)]
struct ManualClock {
    origin: Instant,
    now: Mutex<Instant>,
}

impl ManualClock {
    fn new() -> Self {
        let origin = Instant::now();
        Self {
            origin,
            now: Mutex::new(origin),
        }
    }

    fn at(&self, milliseconds: u64) {
        *self.now.lock() = self.origin + Duration::from_millis(milliseconds);
    }
}

impl BudgetClock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock()
    }
}

struct Fixture {
    budget: LayoutWorkingSetBudget,
    clock: Arc<ManualClock>,
}

impl Fixture {
    fn new(browser_mib: usize, page_mib: usize, document_mib: usize) -> Self {
        let clock = Arc::new(ManualClock::new());
        let budget = LayoutWorkingSetBudget::with_clock(
            BudgetLimits {
                browser_bytes: browser_mib * MIB,
                page_bytes: page_mib * MIB,
                document_bytes: document_mib * MIB,
            },
            BudgetTiming {
                protection: Duration::from_millis(10),
                idle_after: Duration::from_millis(30),
                demand_ttl: Duration::from_millis(100),
                claim_ttl: Duration::from_millis(20),
            },
            clock.clone(),
        )
        .unwrap();
        Self { budget, clock }
    }

    fn page(&self, number: u64) -> PageRegistration {
        self.budget.register_page(token(number)).unwrap()
    }

    fn charged(&self) -> usize {
        self.budget.snapshot().charged_bytes
    }
}

fn token(number: u64) -> RendererPageToken {
    RendererPageToken::new_for_testing(PageId::new_for_testing(number))
}

fn slot(number: u64) -> DocumentSlot {
    DocumentSlot {
        target: WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(1),
                LocalWindowId(number),
                DocumentId(number),
            )),
            OwnerDispatchScope::Top,
        ),
        document: DomHandle::new(number as usize * 2),
        root: DomHandle::new(number as usize * 2 + 1),
    }
}

fn document(page: &PageRegistration, number: u64) -> DocumentTicket {
    page.handle().register_document(slot(number)).unwrap()
}

fn stamp(document: &DocumentBudgetHandle) -> AdmissionStamp {
    match document.begin_pass() {
        PassRetentionPermit::Enabled(stamp) => stamp,
        PassRetentionPermit::Disabled => panic!("expected an eligible document pass"),
    }
}

fn commit(
    document: &DocumentBudgetHandle,
    mib: usize,
    lease: &mut Option<WorkingSetLease>,
) -> Result<(), RetentionDenied> {
    document.try_commit_retained(stamp(document), mib * MIB, lease)
}

fn retain(document: &DocumentBudgetHandle, mib: usize) -> Option<WorkingSetLease> {
    let mut lease = None;
    commit(document, mib, &mut lease).unwrap();
    assert!(lease.is_some());
    lease
}

fn queue(document: &DocumentBudgetHandle, mib: usize) {
    let mut lease = None;
    let denied = commit(document, mib, &mut lease).unwrap_err();
    assert!(lease.is_none());
    document.activate_demand(
        denied
            .demand
            .expect("a feasible candidate must be able to wait"),
    );
}

#[test]
fn whole_document_handoff_waits_for_real_refund_and_allows_both_pages_to_progress() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let mut a_lease = retain(&da.handle(), 7);
    queue(&db.handle(), 7);
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().queued_pages, 1);
    assert_eq!(f.budget.snapshot().ready_bytes, 0);
    assert!(a.handle().maintenance(TrimEpoch::default()).is_none());

    f.clock.at(11);
    let trim = a.handle().maintenance(TrimEpoch::default()).unwrap();
    assert_eq!(
        a.handle().acknowledge_trim(trim),
        TrimStatus::PendingRelease
    );
    assert_eq!(f.charged(), 7 * MIB, "acknowledgement is not a refund");
    assert_eq!(f.budget.snapshot().ready_bytes, 0);
    drop(a_lease.take());
    assert_eq!(f.charged(), 0);
    assert_eq!(f.budget.snapshot().ready_bytes, 7 * MIB);
    let b_lease = retain(&db.handle(), 7);
    assert_eq!(f.budget.snapshot().queued_pages, 0);

    // A still has a valid document and may wait for its next residency turn.
    a.handle().acknowledge_trim(trim);
    queue(&da.handle(), 7);
    f.clock.at(22);
    let b_trim = b.handle().maintenance(TrimEpoch::default()).unwrap();
    assert_eq!(f.charged(), 7 * MIB);
    drop(b_lease);
    b.handle().acknowledge_trim(b_trim);
    a_lease = retain(&da.handle(), 7);
    assert_eq!(f.charged(), 7 * MIB);
    drop(a_lease);
    assert_eq!(f.charged(), 0);
}

#[test]
fn hits_growth_and_child_admission_do_not_restart_the_residency_protection() {
    let f = Fixture::new(20, 20, 16);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let child = document(&a, 3);
    let db = document(&b, 2);
    let mut lease = retain(&da.handle(), 6);
    let initial = f
        .budget
        .snapshot()
        .pages
        .into_iter()
        .find(|page| page.token == token(1))
        .unwrap();
    assert_eq!(initial.resident_since, Some(f.clock.origin));
    f.clock.at(2);
    commit(&da.handle(), 7, &mut lease).unwrap();
    f.clock.at(4);
    let child_lease = retain(&child.handle(), 2);
    f.clock.at(6);
    commit(&da.handle(), 7, &mut lease).unwrap();
    let updated = f
        .budget
        .snapshot()
        .pages
        .into_iter()
        .find(|page| page.token == token(1))
        .unwrap();
    assert_eq!(updated.resident_since, initial.resident_since);
    assert_eq!(updated.protected_until, initial.protected_until);

    f.clock.at(8);
    queue(&db.handle(), 15);
    assert!(a.handle().maintenance(TrimEpoch::default()).is_none());
    f.clock.at(11);
    let trim = a.handle().maintenance(TrimEpoch::default()).unwrap();
    let requests = f.budget.snapshot().trim_requests;
    a.handle().maintenance(TrimEpoch::default());
    assert_eq!(f.budget.snapshot().trim_requests, requests);
    drop((lease, child_lease));
    a.handle().acknowledge_trim(trim);
    assert_eq!(f.budget.snapshot().ready_bytes, 15 * MIB);
}

#[test]
fn a_page_has_one_fifo_ticket_and_another_slot_cannot_replace_or_refresh_it() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let root = document(&b, 2);
    let child = document(&b, 3);
    let _lease = retain(&da.handle(), 7);
    queue(&root.handle(), 7);
    let before = f
        .budget
        .snapshot()
        .pages
        .into_iter()
        .find(|page| page.token == token(2))
        .unwrap()
        .demand
        .unwrap();
    f.clock.at(5);
    queue(&child.handle(), 8);
    let snapshot = f.budget.snapshot();
    assert_eq!(snapshot.fifo, vec![token(2)]);
    let after = snapshot
        .pages
        .into_iter()
        .find(|page| page.token == token(2))
        .unwrap()
        .demand
        .unwrap();
    assert_eq!(after.slot, before.slot);
    assert_eq!(after.bytes, before.bytes);
    assert_eq!(after.last_demand_at, before.last_demand_at);

    queue(&root.handle(), 8);
    let snapshot = f.budget.snapshot();
    assert_eq!(snapshot.fifo, vec![token(2)]);
    let updated = snapshot
        .pages
        .into_iter()
        .find(|page| page.token == token(2))
        .unwrap()
        .demand
        .unwrap();
    assert_eq!(updated.bytes, 8 * MIB);
    assert_eq!(updated.enqueued_at, before.enqueued_at);
    assert!(updated.last_demand_at > before.last_demand_at);
}

#[test]
fn same_slot_resize_charges_only_the_difference_and_failure_keeps_the_old_lease() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let mut lease = retain(&da.handle(), 6);
    let other = retain(&db.handle(), 5);
    commit(&da.handle(), 7, &mut lease).unwrap();
    assert_eq!(f.charged(), 12 * MIB);
    let denied = commit(&da.handle(), 8, &mut lease).unwrap_err();
    assert!(denied.demand.is_some());
    assert_eq!(f.charged(), 12 * MIB);
    assert_eq!(f.budget.snapshot().live_leases, 2);
    commit(&da.handle(), 4, &mut lease).unwrap();
    assert_eq!(f.charged(), 9 * MIB);
    drop(lease);
    assert_eq!(f.charged(), 5 * MIB);
    drop(other);
}

#[test]
fn an_unactivated_rejection_cannot_enqueue_or_evict_another_page() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let _lease = retain(&da.handle(), 7);
    f.clock.at(11);
    let denied = commit(&db.handle(), 7, &mut None).unwrap_err();
    assert!(denied.demand.is_some());
    // The producer may still fail publication. Merely producing a feasible
    // demand description must not perform its explicit activation step.
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    assert_eq!(f.budget.snapshot().trim_requests, 0);
    assert_eq!(f.charged(), 7 * MIB);
    assert!(a.handle().maintenance(TrimEpoch::default()).is_none());
}

#[test]
fn waiting_for_a_whole_document_protects_fragments_of_free_space_until_ready() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let c = f.page(3);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let dc = document(&c, 3);
    let a_lease = retain(&da.handle(), 7);
    queue(&db.handle(), 7);
    let mut c_lease = None;
    let denied = commit(&dc.handle(), 3, &mut c_lease).unwrap_err();
    assert_eq!(denied.reason, RetentionDenial::Queued);
    assert!(c_lease.is_none());
    assert_eq!(f.charged(), 7 * MIB);
    f.clock.at(11);
    a.handle().maintenance(TrimEpoch::default());
    drop(a_lease);
    assert_eq!(f.budget.snapshot().ready_bytes, 7 * MIB);
    // Once the head's whole amount is reserved, the remaining five MiB can
    // be used without postponing the head's next actual layout.
    commit(&dc.handle(), 3, &mut c_lease).unwrap();
    assert_eq!(f.budget.snapshot().ready_bytes, 7 * MIB);
    let b_lease = retain(&db.handle(), 7);
    assert_eq!(f.charged(), 10 * MIB);
    drop((b_lease, c_lease));
}

#[test]
fn another_slot_can_self_trim_without_losing_the_exact_waiting_ticket() {
    let f = Fixture::new(32, 12, 12);
    let page = f.page(1);
    let old = document(&page, 1);
    let next = document(&page, 2);
    let lease = retain(&old.handle(), 6);
    queue(&next.handle(), 7);
    assert_eq!(
        f.budget.snapshot().ready_bytes,
        0,
        "Browser space alone is insufficient"
    );
    f.clock.at(11);
    let trim = page.handle().maintenance(TrimEpoch::default()).unwrap();
    assert_eq!(
        page.handle().acknowledge_trim(trim),
        TrimStatus::PendingRelease
    );
    assert_eq!(f.budget.snapshot().fifo, vec![token(1)]);
    drop(lease);
    page.handle().acknowledge_trim(trim);
    let snapshot = f.budget.snapshot();
    assert_eq!(snapshot.fifo, vec![token(1)]);
    assert_eq!(snapshot.ready_bytes, 7 * MIB);
    assert_eq!(
        snapshot
            .pages
            .into_iter()
            .find(|page| page.token == token(1))
            .unwrap()
            .demand
            .unwrap()
            .slot,
        slot(2)
    );
    let next_lease = retain(&next.handle(), 7);
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    drop(next_lease);
}

#[test]
fn another_page_can_expire_a_busy_ready_head_and_an_old_pass_cannot_jump_the_queue() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let c = f.page(3);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let dc = document(&c, 3);
    let lease = retain(&da.handle(), 7);
    queue(&db.handle(), 7);
    queue(&dc.handle(), 7);
    f.clock.at(11);
    a.handle().maintenance(TrimEpoch::default());
    drop(lease);
    assert_eq!(f.budget.snapshot().ready_bytes, 7 * MIB);
    let busy_head_stamp = stamp(&db.handle());

    // B does no further work. C's existing maintenance opportunity must advance
    // the shared queue; no task running in B is needed to revoke the reservation.
    f.clock.at(32);
    c.handle().maintenance(TrimEpoch::default());
    assert_eq!(f.budget.snapshot().fifo, vec![token(3)]);
    let mut expired_lease = None;
    let denied = db
        .handle()
        .try_commit_retained(busy_head_stamp, 7 * MIB, &mut expired_lease)
        .unwrap_err();
    assert!(expired_lease.is_none());
    db.handle().activate_demand(denied.demand.unwrap());
    assert_eq!(f.budget.snapshot().fifo, vec![token(3), token(2)]);
    let c_lease = retain(&dc.handle(), 7);
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().fifo, vec![token(2)]);
    drop(c_lease);
}

#[test]
fn a_larger_ready_candidate_rechecks_capacity_without_extending_its_claim_deadline() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let c = f.page(3);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let dc = document(&c, 3);
    let a_lease = retain(&da.handle(), 7);
    let c_lease = retain(&dc.handle(), 5);
    queue(&db.handle(), 7);
    f.clock.at(11);
    a.handle().maintenance(TrimEpoch::default());
    drop(a_lease);
    let initial_deadline = f
        .budget
        .snapshot()
        .pages
        .into_iter()
        .find(|page| page.token == token(2))
        .unwrap()
        .demand
        .unwrap()
        .claim_deadline;
    assert!(initial_deadline.is_some());
    assert_eq!(f.budget.snapshot().ready_bytes, 7 * MIB);

    f.clock.at(12);
    queue(&db.handle(), 8);
    let snapshot = f.budget.snapshot();
    assert_eq!(snapshot.ready_bytes, 0);
    assert_eq!(snapshot.charged_bytes, 5 * MIB);
    let demand = snapshot
        .pages
        .into_iter()
        .find(|page| page.token == token(2))
        .unwrap()
        .demand
        .unwrap();
    assert_eq!(demand.bytes, 8 * MIB);
    assert_eq!(demand.claim_deadline, initial_deadline);
    drop(c_lease);
    assert_eq!(f.budget.snapshot().ready_bytes, 8 * MIB);
    assert_eq!(
        f.budget
            .snapshot()
            .pages
            .into_iter()
            .find(|page| page.token == token(2))
            .unwrap()
            .demand
            .unwrap()
            .claim_deadline,
        initial_deadline
    );

    // A smaller current result consumes only its actual estimate, not the old
    // whole-document reservation. Inputs changing do not require a new ticket.
    let b_lease = retain(&db.handle(), 5);
    assert_eq!(f.charged(), 5 * MIB);
    assert_eq!(f.budget.snapshot().ready_bytes, 0);
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    drop(b_lease);
}

#[test]
fn waiting_demand_ttl_does_not_wait_for_a_busy_victim_to_release() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let c = f.page(3);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let dc = document(&c, 3);
    let _lease = retain(&da.handle(), 7);
    queue(&db.handle(), 7);
    f.clock.at(99);
    queue(&dc.handle(), 7);
    assert_eq!(f.budget.snapshot().fifo, vec![token(2), token(3)]);
    f.clock.at(101);
    c.handle().maintenance(TrimEpoch::default());
    assert_eq!(f.budget.snapshot().fifo, vec![token(3)]);
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().ready_bytes, 0);
}

#[test]
fn positive_pressure_reclaims_without_a_waiter_but_does_not_pretend_pending_trim_is_free() {
    let f = Fixture::new(32, 16, 16);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let a_lease = retain(&da.handle(), 7);
    let b_lease = retain(&db.handle(), 7);
    f.budget.set_pressure_target(8 * MIB).unwrap();
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    assert!(a.handle().maintenance(TrimEpoch::default()).is_none());
    f.clock.at(11);
    let trim = a.handle().maintenance(TrimEpoch::default()).unwrap();
    assert_eq!(
        a.handle().acknowledge_trim(trim),
        TrimStatus::PendingRelease
    );
    assert_eq!(f.charged(), 14 * MIB);
    drop(a_lease);
    a.handle().acknowledge_trim(trim);
    assert_eq!(f.charged(), 7 * MIB);
    assert!(b.handle().maintenance(TrimEpoch::default()).is_none());
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    drop(b_lease);
}

#[test]
fn zero_pressure_blocks_inflight_shrink_and_stale_demand_until_new_work_after_recovery() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let mut lease = retain(&da.handle(), 7);
    let old_stamp = stamp(&da.handle());
    let deferred_demand = commit(&db.handle(), 7, &mut None)
        .unwrap_err()
        .demand
        .unwrap();
    f.budget.set_pressure_target(0).unwrap();
    let zero = f.budget.snapshot();
    assert_eq!(zero.ready_bytes, 0);
    assert_eq!(zero.queued_pages, 0);
    assert!(matches!(
        da.handle().begin_pass(),
        PassRetentionPermit::Disabled
    ));
    assert!(matches!(
        db.handle().begin_pass(),
        PassRetentionPermit::Disabled
    ));
    let denied = da
        .handle()
        .try_commit_retained(old_stamp, 6 * MIB, &mut lease)
        .unwrap_err();
    assert!(denied.demand.is_none());
    assert_eq!(
        f.charged(),
        7 * MIB,
        "even a shrink must honor revoked pressure authority"
    );
    db.handle().activate_demand(deferred_demand);
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    f.budget.set_pressure_target(0).unwrap();
    assert_eq!(f.budget.snapshot().pressure_epoch, zero.pressure_epoch);
    assert_eq!(f.budget.snapshot().trim_requests, zero.trim_requests);

    drop(lease.take());
    for page in [&a, &b] {
        if let Some(trim) = page.handle().maintenance(TrimEpoch::default()) {
            page.handle().acknowledge_trim(trim);
        }
    }
    f.budget.set_pressure_target(12 * MIB).unwrap();
    assert_eq!(
        f.charged(),
        0,
        "recovery must not warm caches or grant old candidates"
    );
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    assert!(
        da.handle()
            .try_commit_retained(old_stamp, 6 * MIB, &mut lease)
            .is_err()
    );
    assert!(lease.is_none());
    let fresh = retain(&db.handle(), 7);
    assert_eq!(f.charged(), 7 * MIB);
    drop(fresh);
}

#[test]
fn lowering_pressure_removes_an_impossible_head_and_preserves_a_feasible_follower() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let c = f.page(3);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let dc = document(&c, 3);
    let lease = retain(&da.handle(), 7);
    queue(&db.handle(), 8);
    queue(&dc.handle(), 6);
    f.budget.set_pressure_target(6 * MIB).unwrap();
    assert_eq!(f.budget.snapshot().fifo, vec![token(3)]);
    f.clock.at(11);
    a.handle().maintenance(TrimEpoch::default());
    drop(lease);
    assert_eq!(f.budget.snapshot().ready_bytes, 6 * MIB);
    let retained = retain(&dc.handle(), 6);
    assert_eq!(f.charged(), 6 * MIB);
    drop(retained);
}

#[test]
fn stable_pages_do_not_trim_without_competition_or_pressure() {
    let f = Fixture::new(12, 12, 12);
    let page = f.page(1);
    let doc = document(&page, 1);
    let lease = retain(&doc.handle(), 7);
    f.clock.at(10_000);
    assert!(page.handle().maintenance(TrimEpoch::default()).is_none());
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().trim_requests, 0);
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    drop(lease);
}

struct ChargedPayload {
    budget: LayoutWorkingSetBudget,
    expected_charge: usize,
    dropped: Arc<AtomicBool>,
}

impl Drop for ChargedPayload {
    fn drop(&mut self) {
        // This also takes the ledger lock: callers must not destroy payloads
        // while holding that lock. The adapter must preserve this same order.
        assert_eq!(self.budget.snapshot().charged_bytes, self.expected_charge);
        self.dropped.store(true, Ordering::SeqCst);
    }
}

struct PayloadWithLease {
    payload: Option<ChargedPayload>,
    lease: Option<WorkingSetLease>,
}

impl Drop for PayloadWithLease {
    fn drop(&mut self) {
        drop(self.payload.take());
        drop(self.lease.take());
    }
}

#[test]
fn moving_and_retiring_a_page_never_refunds_a_still_owned_payload() {
    let f = Fixture::new(12, 12, 12);
    let mut page = f.page(1);
    let doc = document(&page, 1);
    let old_handle = doc.handle();
    let old_stamp = stamp(&old_handle);
    let dropped = Arc::new(AtomicBool::new(false));
    let previous = PayloadWithLease {
        payload: Some(ChargedPayload {
            budget: f.budget.clone(),
            expected_charge: 7 * MIB,
            dropped: dropped.clone(),
        }),
        lease: retain(&old_handle, 7),
    };
    let in_pass = previous;
    let mut completed = vec![in_pass];
    page.close();
    page.close();
    assert_eq!(f.budget.snapshot().open_pages, 0);
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().live_leases, 1);
    assert!(!dropped.load(Ordering::SeqCst));
    assert!(matches!(
        old_handle.begin_pass(),
        PassRetentionPermit::Disabled
    ));
    let denied = old_handle
        .try_commit_retained(old_stamp, 7 * MIB, &mut completed[0].lease)
        .unwrap_err();
    assert!(denied.demand.is_none());
    assert_eq!(f.charged(), 7 * MIB);
    drop(completed);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(f.charged(), 0);
    assert_eq!(f.budget.snapshot().live_leases, 0);
}

#[test]
fn document_retirement_cancels_its_ticket_but_not_its_lease_or_other_document() {
    let f = Fixture::new(12, 12, 12);
    let page = f.page(1);
    let mut old_doc = document(&page, 1);
    let old_handle = old_doc.handle();
    let old_stamp = stamp(&old_handle);
    let lease = retain(&old_handle, 7);
    old_doc.cancel();
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().open_pages, 1);
    let replacement = document(&page, 1);
    let mut new_lease = None;
    let denied = replacement
        .handle()
        .try_commit_retained(old_stamp, 5 * MIB, &mut new_lease)
        .unwrap_err();
    assert!(denied.demand.is_none());
    assert!(matches!(
        old_handle.begin_pass(),
        PassRetentionPermit::Disabled
    ));
    drop(lease);
    new_lease = retain(&replacement.handle(), 7);
    drop(old_doc);
    assert_eq!(
        f.charged(),
        7 * MIB,
        "a stale guard must not cancel the replacement ticket"
    );
    assert!(matches!(
        replacement.handle().begin_pass(),
        PassRetentionPermit::Enabled(_)
    ));
    drop(new_lease);
}

#[test]
fn canceled_waiting_slot_releases_fifo_even_when_it_never_had_a_cache() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let mut root = document(&b, 2);
    let next = document(&b, 3);
    let _lease = retain(&da.handle(), 7);
    queue(&root.handle(), 7);
    assert_eq!(b.handle().current_demand_slot(), Some(slot(2)));
    root.cancel();
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    assert_eq!(f.charged(), 7 * MIB);
    queue(&next.handle(), 8);
    assert_eq!(b.handle().current_demand_slot(), Some(slot(3)));
    assert_eq!(f.budget.snapshot().queued_pages, 1);
}

#[test]
fn a_closed_registration_handle_cannot_modify_a_new_registration_for_the_same_page() {
    let f = Fixture::new(12, 12, 12);
    let mut old = f.page(1);
    let old_doc = document(&old, 1);
    let old_handle = old_doc.handle();
    let old_stamp = stamp(&old_handle);
    let old_registration = f.budget.snapshot().pages[0].registration;
    old.close();
    let replacement = f.page(1);
    let new_doc = document(&replacement, 1);
    let mut new_lease = retain(&new_doc.handle(), 7);
    let denied = old_handle
        .try_commit_retained(old_stamp, 6 * MIB, &mut new_lease)
        .unwrap_err();
    assert!(denied.demand.is_none());
    drop(old_doc);
    drop(old);
    assert_eq!(f.charged(), 7 * MIB);
    let page = f
        .budget
        .snapshot()
        .pages
        .into_iter()
        .find(|page| page.open)
        .unwrap();
    assert_ne!(page.registration, old_registration);
    assert_eq!(page.documents, 1);
    assert!(matches!(
        new_doc.handle().begin_pass(),
        PassRetentionPermit::Enabled(_)
    ));
    drop(new_lease);
}

#[test]
fn identical_page_ids_in_different_owners_have_independent_accounts() {
    let f = Fixture::new(12, 12, 12);
    let a_token = RendererPageToken::new_for_testing_with_host(1, PageId::new_for_testing(1));
    let b_token = RendererPageToken::new_for_testing_with_host(2, PageId::new_for_testing(1));
    let a = f.budget.register_page(a_token).unwrap();
    let b = f.budget.register_page(b_token).unwrap();
    let da = document(&a, 1);
    let db = document(&b, 1);
    let a_lease = retain(&da.handle(), 5);
    let b_lease = retain(&db.handle(), 6);
    let snapshot = f.budget.snapshot();
    assert_eq!(snapshot.open_pages, 2);
    assert_eq!(snapshot.charged_bytes, 11 * MIB);
    assert_eq!(
        snapshot
            .pages
            .iter()
            .find(|page| page.token == a_token)
            .unwrap()
            .charged_bytes,
        5 * MIB
    );
    assert_eq!(
        snapshot
            .pages
            .iter()
            .find(|page| page.token == b_token)
            .unwrap()
            .charged_bytes,
        6 * MIB
    );
    drop(a_lease);
    assert_eq!(f.charged(), 6 * MIB);
    drop(b_lease);
}

#[test]
fn concurrent_growth_is_atomic_and_a_failed_attempt_keeps_its_existing_charge() {
    let f = Fixture::new(10, 10, 10);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let a_lease = retain(&da.handle(), 2);
    let b_lease = retain(&db.handle(), 2);
    let barrier = Arc::new(Barrier::new(3));
    let attempts = std::thread::scope(|scope| {
        let a_handle = da.handle();
        let a_barrier = barrier.clone();
        let first = scope.spawn(move || {
            let mut lease = a_lease;
            a_barrier.wait();
            let result = commit(&a_handle, 7, &mut lease);
            (result.is_ok(), lease)
        });
        let b_handle = db.handle();
        let b_barrier = barrier.clone();
        let second = scope.spawn(move || {
            let mut lease = b_lease;
            b_barrier.wait();
            let result = commit(&b_handle, 7, &mut lease);
            (result.is_ok(), lease)
        });
        barrier.wait();
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(
        attempts.0.0, attempts.1.0,
        "exactly one five-MiB growth fits"
    );
    assert_eq!(f.charged(), 9 * MIB);
    assert_eq!(f.budget.snapshot().live_leases, 2);
    drop(attempts);
    assert_eq!(f.charged(), 0);
}

#[test]
fn older_clock_samples_cannot_move_shared_deadlines_backwards() {
    let f = Fixture::new(12, 12, 12);
    let a = f.page(1);
    let b = f.page(2);
    let da = document(&a, 1);
    let db = document(&b, 2);
    let lease = retain(&da.handle(), 7);
    queue(&db.handle(), 7);
    f.clock.at(11);
    a.handle().maintenance(TrimEpoch::default());
    drop(lease);
    let before = f.budget.snapshot();
    assert!(matches!(
        before.handoff,
        Some(HandoffSnapshot::ReadyToClaim { .. })
    ));
    f.clock.at(5); // An earlier sample obtains the shared lock later.
    b.handle().maintenance(TrimEpoch::default());
    let after = f.budget.snapshot();
    assert_eq!(after.last_observed_now, before.last_observed_now);
    assert_eq!(after.handoff, before.handoff);
    f.clock.at(32);
    a.handle().maintenance(TrimEpoch::default());
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    assert_eq!(f.budget.snapshot().ready_bytes, 0);
}

#[test]
fn impossible_absolute_candidates_never_create_waiters_or_trim_requests() {
    for (browser, page_limit, document_limit, requested) in
        [(12, 12, 6, 7), (12, 6, 12, 7), (6, 12, 12, 7)]
    {
        let f = Fixture::new(browser, page_limit, document_limit);
        let page = f.page(1);
        let doc = document(&page, 1);
        let denied = commit(&doc.handle(), requested, &mut None).unwrap_err();
        assert_eq!(denied.reason, RetentionDenial::AbsoluteLimit);
        assert!(denied.demand.is_none());
        assert_eq!(f.budget.snapshot().queued_pages, 0);
        assert_eq!(f.budget.snapshot().trim_requests, 0);
        assert_eq!(f.charged(), 0);
    }
}

#[test]
fn exhausted_ids_fail_closed_without_losing_existing_lease_accounting() {
    let f = Fixture::new(12, 12, 12);
    let page = f.page(1);
    let doc = document(&page, 1);
    let other_doc = document(&page, 2);
    let lease = retain(&doc.handle(), 7);
    f.budget.set_next_id_for_test(u64::MAX);
    assert!(f.budget.register_page(token(2)).is_err());
    assert!(page.handle().register_document(slot(3)).is_err());
    let denied = commit(&other_doc.handle(), 2, &mut None).unwrap_err();
    assert_eq!(denied.reason, RetentionDenial::Exhausted);
    assert!(denied.demand.is_none());
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().live_leases, 1);
    assert_eq!(f.budget.snapshot().queued_pages, 0);
    f.budget.set_pressure_target(0).unwrap();
    let request = page.handle().maintenance(TrimEpoch::default()).unwrap();
    assert_eq!(request.epoch, TrimEpoch(u64::MAX));
    assert_eq!(request.cause, TrimCause::Pressure);
    assert_eq!(f.charged(), 7 * MIB);
    drop(lease);
    assert_eq!(f.charged(), 0);
    assert_eq!(
        page.handle().acknowledge_trim(request),
        TrimStatus::Complete
    );
}

#[test]
fn one_document_cannot_claim_a_second_lease_instead_of_resizing_the_first() {
    let f = Fixture::new(16, 16, 8);
    let page = f.page(1);
    let doc = document(&page, 1);
    let mut lease = retain(&doc.handle(), 7);
    let mut duplicate = None;
    let denied = commit(&doc.handle(), 7, &mut duplicate).unwrap_err();
    assert_eq!(denied.reason, RetentionDenial::InvalidLease);
    assert!(denied.demand.is_none());
    assert!(duplicate.is_none());
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(f.budget.snapshot().live_leases, 1);
    commit(&doc.handle(), 8, &mut lease).unwrap();
    assert_eq!(f.charged(), 8 * MIB);
    assert_eq!(lease.as_ref().unwrap().bytes(), 8 * MIB);
    drop(lease);
    assert_eq!(f.charged(), 0);
}

#[test]
fn a_lease_cannot_be_resized_by_another_slot_or_another_pool() {
    let f = Fixture::new(12, 12, 12);
    let other_pool = Fixture::new(12, 12, 12);
    let page = f.page(1);
    let other_page = other_pool.page(1);
    let first = document(&page, 1);
    let second = document(&page, 2);
    let foreign = document(&other_page, 1);
    let mut lease = retain(&first.handle(), 7);
    let denied = commit(&second.handle(), 3, &mut lease).unwrap_err();
    assert_eq!(denied.reason, RetentionDenial::InvalidLease);
    assert!(denied.demand.is_none());
    let denied = commit(&foreign.handle(), 3, &mut lease).unwrap_err();
    assert_eq!(denied.reason, RetentionDenial::InvalidLease);
    assert!(denied.demand.is_none());
    assert_eq!(f.charged(), 7 * MIB);
    assert_eq!(other_pool.charged(), 0);
    assert_eq!(lease.as_ref().unwrap().bytes(), 7 * MIB);
    drop(lease);
    assert_eq!(f.charged(), 0);
}

#[test]
fn charge_arithmetic_cannot_wrap_or_saturate_into_a_successful_admission() {
    let clock = Arc::new(ManualClock::new());
    let budget = LayoutWorkingSetBudget::with_clock(
        BudgetLimits {
            browser_bytes: usize::MAX,
            page_bytes: usize::MAX,
            document_bytes: usize::MAX,
        },
        BudgetTiming {
            protection: Duration::from_millis(10),
            idle_after: Duration::from_millis(30),
            demand_ttl: Duration::from_millis(100),
            claim_ttl: Duration::from_millis(20),
        },
        clock,
    )
    .unwrap();
    let a = budget.register_page(token(1)).unwrap();
    let b = budget.register_page(token(2)).unwrap();
    let da = document(&a, 1);
    let db = document(&b, 2);
    let mut existing = None;
    da.handle()
        .try_commit_retained(stamp(&da.handle()), usize::MAX - 4, &mut existing)
        .unwrap();
    let mut rejected = None;
    assert!(
        db.handle()
            .try_commit_retained(stamp(&db.handle()), 8, &mut rejected)
            .is_err()
    );
    assert!(rejected.is_none());
    assert_eq!(budget.snapshot().charged_bytes, usize::MAX - 4);
    assert_eq!(budget.snapshot().live_leases, 1);
    drop(existing);
    assert_eq!(budget.snapshot().charged_bytes, 0);
}
