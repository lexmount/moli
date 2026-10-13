use super::*;
use crate::layout_working_set_budget::{HandoffSnapshot, PageBudgetSnapshot};

#[derive(Debug)]
struct OwnerFairnessClock {
    origin: Instant,
    now: parking_lot::Mutex<Instant>,
}

impl OwnerFairnessClock {
    fn new() -> Self {
        let origin = Instant::now();
        Self {
            origin,
            now: parking_lot::Mutex::new(origin),
        }
    }

    fn at_millis(&self, millis: u64) {
        // Changing this source alone does not advance the ledger or wake an
        // owner. The next existing maintenance opportunity must consume it.
        *self.now.lock() = self.origin + Duration::from_millis(millis);
    }
}

impl BudgetClock for OwnerFairnessClock {
    fn now(&self) -> Instant {
        *self.now.lock()
    }
}

fn fairness_pool() -> (LayoutWorkingSetBudget, Arc<OwnerFairnessClock>) {
    let clock = Arc::new(OwnerFairnessClock::new());
    let pool = LayoutWorkingSetBudget::with_clock(
        BudgetLimits::default(),
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

fn same_account<'a>(
    snapshot: &'a BudgetSnapshot,
    original: &PageBudgetSnapshot,
) -> &'a PageBudgetSnapshot {
    let current = snapshot
        .pages
        .iter()
        .find(|page| page.token == original.token)
        .unwrap_or_else(|| panic!("the exact Page retired during handoff: {snapshot:#?}"));
    assert!(current.open);
    assert_eq!(current.registration, original.registration);
    current
}

async fn wait_for_real_owner_handoff(
    pool: &LayoutWorkingSetBudget,
    initial: &BudgetSnapshot,
    incumbent: &PageBudgetSnapshot,
    waiter: &PageBudgetSnapshot,
) -> BudgetSnapshot {
    let completed = tokio::time::timeout(OWNER_PRESSURE_TEST_TIMEOUT, async {
        let mut poll = tokio::time::interval(Duration::from_millis(25));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            poll.tick().await;
            // No Page command, layout read, manual maintenance helper or
            // ledger-advancing API is called during this wait.
            let snapshot = pool.snapshot();
            assert_eq!(snapshot.open_pages, 2);
            assert_eq!(snapshot.pressure_target, initial.pressure_target);
            let current_incumbent = same_account(&snapshot, incumbent);
            let current_waiter = same_account(&snapshot, waiter);
            assert_eq!(
                current_waiter.charged_bytes, 0,
                "a waiting Page must not be warmed by maintenance"
            );
            assert_eq!(snapshot.charged_bytes, current_incumbent.charged_bytes);
            assert!(snapshot.charged_bytes <= snapshot.pressure_target);
            if current_incumbent.charged_bytes == 0
                && snapshot.live_leases == 0
                && matches!(
                    snapshot.handoff,
                    Some(HandoffSnapshot::ReadyToClaim { head, .. }) if head == waiter.token
                )
            {
                return snapshot;
            }
        }
    })
    .await;
    match completed {
        Ok(snapshot) => snapshot,
        Err(_) => panic!(
            "normal owner maintenance did not release the incumbent and ready the exact waiter; \
             no Page commands were sent while waiting; initial={initial:#?}; current={:#?}",
            pool.snapshot(),
        ),
    }
}

fn assert_same_layout(
    current: &RendererLayoutWorkingSetObservation,
    initial: &RendererLayoutWorkingSetObservation,
) {
    assert_eq!(current.vm_creation_id, initial.vm_creation_id);
    assert_eq!(current.document, initial.document);
    assert_eq!(current.builds, initial.builds);
    assert_eq!(current.screen, initial.screen);
    assert_eq!(current.inputs, initial.inputs);
    assert_eq!(current.frame, initial.frame);
    assert!(current.frame_idle);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_owners_handoff_whole_document_retention_only_on_actual_demand() {
    let (pool, clock) = fairness_pool();
    // Each constructor creates an independent Context/owner. Explicitly
    // injecting one pool must aggregate both despite equal local Page IDs.
    let first_runtime = realtime_runtime(&pool);
    let second_runtime = realtime_runtime(&pool);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut first = page(&first_runtime, &loader).await;
    assert_eq!(
        json_result(
            &first,
            "for(let i=0;i<128;i++){const n=document.createElement('div');n.style.cssText='width:16px;height:1px';document.body.append(n)}const el=document.getElementById('target');return [el.offsetWidth,el.offsetHeight]",
        )
        .await,
        serde_json::json!([120, 20]),
    );
    let first_before = establish_idle_frame_boundary(&first).await;
    assert_eq!((first_before.working_sets, first_before.tickets), (1, 1));
    let first_account = account(&pool, &first);
    let first_charge = first_account.charged_bytes;
    assert!(first_charge > 0);
    assert_eq!(pool.snapshot().charged_bytes, first_charge);

    // The target comes from an actual retained tree, not a fabricated lease
    // or an assumed per-node byte count. The smaller candidate is checked
    // below against its actual queued estimate.
    pool.set_pressure_target(first_charge).unwrap();
    let mut second = page(&second_runtime, &loader).await;
    assert_eq!(first.renderer_page_id(), second.renderer_page_id());
    assert_ne!(first.owner_local_host_id(), second.owner_local_host_id());
    assert_eq!(
        json_result(
            &second,
            "const el=document.getElementById('target');return [el.offsetWidth,el.offsetHeight]",
        )
        .await,
        serde_json::json!([120, 20]),
    );
    let second_before = establish_idle_frame_boundary(&second).await;
    assert_eq!((second_before.working_sets, second_before.tickets), (0, 1));
    let second_account = account(&pool, &second);
    assert_ne!(first_account.registration, second_account.registration);
    assert_eq!(second_account.charged_bytes, 0);
    let demand = second_account
        .demand
        .expect("real geometry must leave an exact-slot demand");
    assert_eq!(demand.slot.document, second_before.document);
    assert!(demand.bytes > 0 && demand.bytes <= first_charge);
    assert!(first_charge.checked_add(demand.bytes).unwrap() > first_charge);

    let queued = pool.snapshot();
    assert_eq!(queued.open_pages, 2);
    assert_eq!(queued.live_leases, 1);
    assert_eq!(queued.charged_bytes, first_charge);
    assert_eq!(queued.queued_pages, 1);
    assert_eq!(queued.trim_requests, 0);
    assert_eq!(queued.fifo, vec![second_account.token]);
    assert_eq!(
        queued.handoff,
        Some(HandoffSnapshot::WaitingRelease {
            head: second_account.token
        })
    );
    assert!(queued.pages.iter().all(|page| page.trim.is_none()));
    assert_eq!(demand.claim_deadline, None);
    let first_deadline = first_before.maintenance_deadline.unwrap();

    // No layout or pool operation after this write drives reclamation. The
    // owners keep their existing real one-second maintenance opportunities.
    // Budget time remains 11ms throughout the wait, so claim/TTL expiration
    // cannot accidentally follow real wall-clock scheduling latency.
    clock.at_millis(11);
    let ready = wait_for_real_owner_handoff(&pool, &queued, &first_account, &second_account).await;
    assert_eq!(ready.charged_bytes, 0);
    assert_eq!(ready.live_leases, 0);
    assert_eq!(ready.queued_pages, 1);
    assert_eq!(ready.trim_requests, 1);
    assert_eq!(ready.ready_bytes, demand.bytes);
    assert_eq!(ready.fifo, vec![second_account.token]);
    assert!(ready.pages.iter().all(|page| page.trim.is_none()));
    assert_eq!(
        ready.handoff,
        Some(HandoffSnapshot::ReadyToClaim {
            head: second_account.token,
            reserved_bytes: demand.bytes,
            deadline: clock.now() + Duration::from_millis(20),
        }),
    );

    // ACKs run after normal owner action/settlement/restore, as in the single
    // Page pressure test. Snapshot polling alone could observe mid-action.
    let first_trimmed = observation(&first).await;
    let second_waiting = observation(&second).await;
    assert_same_layout(&first_trimmed, &first_before);
    assert_same_layout(&second_waiting, &second_before);
    assert_eq!((first_trimmed.working_sets, first_trimmed.tickets), (0, 1));
    assert_eq!(
        (second_waiting.working_sets, second_waiting.tickets),
        (0, 1)
    );
    assert!(first_trimmed.maintenance_deadline.unwrap() > first_deadline);

    // A frozen geometry hit is not a new retention request and cannot warm
    // either Page. This also proves A's eviction did not invalidate its API.
    for page in [&first, &second] {
        assert_eq!(
            json_result(
                page,
                "const el=document.getElementById('target');return [el.offsetWidth,el.offsetHeight]",
            )
            .await,
            serde_json::json!([120, 20]),
        );
    }
    for (page, before) in [(&first, &first_before), (&second, &second_before)] {
        let read = observation(page).await;
        assert_eq!((read.working_sets, read.tickets), (0, 1));
        assert_eq!(read.builds, before.builds);
        assert_eq!(read.screen.2, before.screen.2);
        assert_eq!(read.screen.3, before.screen.3);
        assert_eq!(read.inputs, before.inputs);
        assert_eq!(read.frame, before.frame);
    }
    assert_eq!(pool.snapshot().charged_bytes, 0);
    assert_eq!(pool.snapshot().handoff, ready.handoff);

    assert_eq!(
        json_result(
            &second,
            "const el=document.getElementById('target');el.style.width='170px';return [el.offsetWidth,el.offsetHeight]",
        )
        .await,
        serde_json::json!([170, 20]),
    );
    let claimed = observation(&second).await;
    assert_eq!((claimed.working_sets, claimed.tickets), (1, 1));
    assert!(claimed.builds > second_before.builds);
    let admitted = pool.snapshot();
    assert_eq!(admitted.open_pages, 2);
    assert_eq!(admitted.live_leases, 1);
    assert!(admitted.charged_bytes > 0 && admitted.charged_bytes <= first_charge);
    assert_eq!(admitted.queued_pages, 0);
    assert_eq!(admitted.ready_bytes, 0);
    assert_eq!(admitted.handoff, None);
    assert_eq!(same_account(&admitted, &first_account).charged_bytes, 0);
    assert_eq!(
        same_account(&admitted, &second_account).charged_bytes,
        admitted.charged_bytes
    );
    assert!(same_account(&admitted, &second_account).demand.is_none());

    first.close_async().await.unwrap();
    assert_eq!(pool.snapshot().open_pages, 1);
    second.close_async().await.unwrap();
    let closed = pool.snapshot();
    assert_eq!(closed.open_pages, 0);
    assert_eq!(closed.live_leases, 0);
    assert_eq!(closed.charged_bytes, 0);
    assert_eq!(closed.queued_pages, 0);
    assert!(closed.pages.is_empty());
}
