use super::*;
use crate::layout_working_set_budget::BudgetSnapshot;
use crate::runtime::owner_local::RendererLayoutWorkingSetObservation;

mod fairness;

// This is a deadlock guard, not a maintenance latency or frame-rate bound.
const OWNER_PRESSURE_TEST_TIMEOUT: Duration = Duration::from_secs(5);

async fn observation(page: &RendererPageHandle) -> RendererLayoutWorkingSetObservation {
    tokio::time::timeout(
        OWNER_PRESSURE_TEST_TIMEOUT,
        RendererPageTestingHandle::new_for_testing(page).layout_working_set_observation_async(),
    )
    .await
    .expect("the read-only owner observation command did not complete")
    .expect("the acknowledged owner boundary must have a resident live Page")
}

async fn establish_idle_frame_boundary(
    page: &RendererPageHandle,
) -> RendererLayoutWorkingSetObservation {
    let (completed, completion) = oneshot::channel();
    let (reply, _) = tokio::time::timeout(
        OWNER_PRESSURE_TEST_TIMEOUT,
        page.run_async_command(RendererPageCommand::ObserveRealtimeOpportunityForTesting {
            expression: "requestAnimationFrame(()=>{})".to_owned(),
            completed,
        }),
    )
    .await
    .expect("the one-time setup frame command did not complete")
    .unwrap();
    assert!(matches!(reply, RendererPageReply::Unit));
    tokio::time::timeout(OWNER_PRESSURE_TEST_TIMEOUT, completion)
        .await
        .expect("the exact setup rendering opportunity did not reach its layout phase")
        .expect("the setup frame's Document retired before its completion signal");

    // The completion signal is sent at the layout phase. A subsequent owner
    // command ACK, rather than that signal alone, confirms that the bounded
    // frame turn has returned and restored its Page entry.
    let idle = observation(page).await;
    assert!(
        idle.frame_idle,
        "static fixture retained frame demand: {idle:#?}"
    );
    assert!(idle.frame.is_some());
    idle
}

async fn wait_for_actual_owner_refund(
    pool: &LayoutWorkingSetBudget,
    page: &RendererPageHandle,
    initial: &BudgetSnapshot,
) -> BudgetSnapshot {
    let initial_page = initial
        .pages
        .iter()
        .find(|state| {
            state.token.local_host_id() == page.owner_local_host_id()
                && state.token.page_id() == page.renderer_page_id()
        })
        .expect("the initial complete Page token is registered");
    let token = initial_page.token;
    let registration = initial_page.registration;
    let completed = tokio::time::timeout(OWNER_PRESSURE_TEST_TIMEOUT, async {
        // This timer only samples owned metadata on the test side. snapshot()
        // does not advance the ledger, wake the renderer or execute layout.
        let mut poll = tokio::time::interval(Duration::from_millis(25));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            poll.tick().await;
            let snapshot = pool.snapshot();
            let account = snapshot
                .pages
                .iter()
                .find(|state| state.token == token)
                .unwrap_or_else(|| panic!("the Page retired instead of trimming: {snapshot:#?}"));
            assert!(
                account.open,
                "the Page closed instead of trimming: {snapshot:#?}"
            );
            assert_eq!(account.registration, registration);
            assert_eq!(snapshot.pressure_target, 0);
            if account.charged_bytes == 0 && snapshot.live_leases == 0 {
                return snapshot;
            }
        }
    })
    .await;
    match completed {
        Ok(snapshot) => snapshot,
        Err(_) => panic!(
            "real owner maintenance did not release the retained payload; \
             no Page commands, layout getters or screenshots were sent while waiting; \
             initial={initial:#?}; current={:#?}",
            pool.snapshot(),
        ),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_realtime_page_reclaims_working_sets_via_real_owner_maintenance() {
    let pool = budget();
    let runtime = realtime_runtime(&pool);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&runtime, &loader).await;
    assert_eq!(
        json_result(
            &page,
            "const el=document.getElementById('target');return [el.offsetWidth,el.offsetHeight]",
        )
        .await,
        serde_json::json!([120, 20]),
    );
    let before = establish_idle_frame_boundary(&page).await;
    assert_eq!((before.working_sets, before.tickets), (1, 1));
    assert!(before.builds > 0);
    assert!(before.screen.3.is_some());
    assert!(before.inputs.is_some());
    let old_deadline = before
        .maintenance_deadline
        .expect("live Page must retain a scheduled or admitted maintenance opportunity");
    let initial = pool.snapshot();
    assert_eq!(initial.open_pages, 1);
    assert_eq!(initial.live_leases, 1);
    assert!(initial.charged_bytes > 0);

    pool.set_pressure_target(0).unwrap();
    // Maintenance can legally run concurrently with set_pressure_target. Do
    // not require the old charge to remain visible after that call returns.
    let refunded = wait_for_actual_owner_refund(&pool, &page, &initial).await;
    assert_eq!(refunded.open_pages, 1);
    assert_eq!(refunded.charged_bytes, 0);
    assert_eq!(refunded.queued_pages, 0);

    // The refund predicate can become true inside the action. This owner ACK
    // also waits for task settlement and the entry's residence restoration.
    let trimmed = observation(&page).await;
    assert_eq!((trimmed.working_sets, trimmed.tickets), (0, 1));
    assert_eq!(trimmed.vm_creation_id, before.vm_creation_id);
    assert_eq!(trimmed.document, before.document);
    assert_eq!(trimmed.builds, before.builds);
    assert_eq!(trimmed.screen, before.screen);
    assert_eq!(trimmed.inputs, before.inputs);
    assert_eq!(trimmed.frame, before.frame);
    assert!(trimmed.frame_idle);
    assert!(
        trimmed
            .maintenance_deadline
            .expect("maintenance must rearm its stable residence")
            > old_deadline,
        "the normal typed maintenance opportunity was not settled: before={before:#?}; after={trimmed:#?}",
    );

    // Only after proving reclamation do we read geometry again. A valid
    // frozen sample survives cache eviction and can answer without rebuilding.
    assert_eq!(
        json_result(
            &page,
            "const el=document.getElementById('target');return [el.offsetWidth,el.offsetHeight]",
        )
        .await,
        serde_json::json!([120, 20]),
    );
    let read = observation(&page).await;
    assert_eq!((read.working_sets, read.tickets), (0, 1));
    assert_eq!(read.builds, before.builds);
    assert_eq!(read.screen.2, before.screen.2);
    assert_eq!(read.screen.3, before.screen.3);
    assert_eq!(read.inputs, before.inputs);
    assert_eq!(read.frame, before.frame);

    pool.set_pressure_target(pool.limits().browser_bytes)
        .unwrap();
    let recovered = observation(&page).await;
    assert_eq!((recovered.working_sets, recovered.tickets), (0, 1));
    assert_eq!(recovered.builds, before.builds);
    assert_eq!(recovered.frame, before.frame);
    assert_eq!(pool.snapshot().charged_bytes, 0);
    assert_eq!(
        json_result(
            &page,
            "const el=document.getElementById('target');el.style.width='170px';return [el.offsetWidth,el.offsetHeight]",
        )
        .await,
        serde_json::json!([170, 20]),
    );
    let rebuilt = observation(&page).await;
    assert_eq!((rebuilt.working_sets, rebuilt.tickets), (1, 1));
    assert!(rebuilt.builds > before.builds);
    assert_eq!(pool.snapshot().live_leases, 1);
    assert!(account(&pool, &page).charged_bytes > 0);

    page.close_async().await.unwrap();
    let closed = pool.snapshot();
    assert_eq!(closed.open_pages, 0);
    assert_eq!(closed.live_leases, 0);
    assert_eq!(closed.charged_bytes, 0);
}
