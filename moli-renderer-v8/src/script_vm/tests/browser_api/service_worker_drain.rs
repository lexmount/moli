use super::*;

// Some fixtures intentionally poll up to 50 zero-delay timer turns for each
// of several child navigations. Keep the bound well above that authored work
// while still failing a genuinely non-settling scheduler deterministically.
const MAX_SERVICE_WORKER_TEST_SETTLE_TURNS: usize = 4_096;
const MAX_SERVICE_WORKER_TEST_CONSECUTIVE_IDLE_TURNS: usize = 64;

/// Publish one browser-context ServiceWorker internal action through the
/// current root-Document route and execute one FIFO head through the production
/// selected-task dispatcher.
///
/// The closure keeps the concrete typed producer visible at each call site.
/// This helper deliberately does not select by internal variant: all
/// ServiceWorker internal actions share one production task source, so an
/// earlier task must remain ahead of the newly published action. Callers prove
/// that their action ran through its stable domain effect.
async fn publish_and_run_service_worker_internal_test_task<E>(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    context: &str,
    publish: impl FnOnce(&crate::page_task_queue::RendererPageServiceWorkerTaskSender) -> Result<(), E>,
) {
    let sender = page.current_service_worker_task_sender_for_test();
    assert!(
        publish(&sender).is_ok(),
        "{context}: typed ServiceWorker Page route closed"
    );
    assert!(
        page.run_one_service_worker_internal_task_executor_turn(loader)
            .await
            .unwrap_or_else(|error| panic!("{context}: selected task failed: {error}")),
        "{context}: expected one ServiceWorker internal FIFO task"
    );
}

pub(super) async fn run_service_worker_client_navigate_request_task_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    context: &str,
    completion: crate::types::ServiceWorkerClientNavigateRequestCompletion,
) {
    publish_and_run_service_worker_internal_test_task(page, loader, context, |sender| {
        sender.send_service_worker_client_navigate_request(completion)
    })
    .await;
}

pub(super) async fn run_service_worker_client_focus_request_task_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    context: &str,
    completion: crate::types::ServiceWorkerClientFocusRequestCompletion,
) {
    publish_and_run_service_worker_internal_test_task(page, loader, context, |sender| {
        sender.send_service_worker_client_focus_request(completion)
    })
    .await;
}

pub(super) async fn run_service_worker_clients_open_window_request_task_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    context: &str,
    completion: crate::types::ServiceWorkerClientsOpenWindowRequestCompletion,
) {
    publish_and_run_service_worker_internal_test_task(page, loader, context, |sender| {
        sender.send_service_worker_clients_open_window_request(completion)
    })
    .await;
}

pub(super) async fn drain_service_worker_test_until_eval_equals(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    browser_context_runtime: &crate::runtime::RendererBrowserContextRuntime,
    loader: &ResourceRequestClient,
    expression: &str,
    expected: &str,
) {
    // A process may be descheduled for several seconds during a full nextest
    // run without consuming a Page scheduler turn. Bound actual production
    // task opportunities so host contention cannot exhaust the test budget.
    let mut consecutive_idle_turns = 0;
    for completed_turns in 0..=MAX_SERVICE_WORKER_TEST_SETTLE_TURNS {
        let value = page
            .eval(expression)
            .expect("service worker test predicate should evaluate");
        if value == expected {
            return;
        }
        if completed_turns == MAX_SERVICE_WORKER_TEST_SETTLE_TURNS {
            let pending_runtime_script_work = page.has_pending_runtime_script_work();
            panic!(
                "service worker test predicate did not settle: {expression} last={value:?} \
                 completed_turns={completed_turns} \
                 consecutive_idle_turns={consecutive_idle_turns} \
                 pending_runtime_script_work={pending_runtime_script_work}"
            );
        }

        if drain_service_worker_test_turn(page, browser_context_runtime, loader).await {
            consecutive_idle_turns = 0;
        } else {
            consecutive_idle_turns += 1;
            assert!(
                consecutive_idle_turns < MAX_SERVICE_WORKER_TEST_CONSECUTIVE_IDLE_TURNS,
                "service worker test predicate stayed idle: {expression} last={value:?} \
                 completed_turns={} consecutive_idle_turns={consecutive_idle_turns}",
                completed_turns + 1
            );
        }
    }
    unreachable!("bounded ServiceWorker settle loop must return or panic")
}

/// Advance one bounded ServiceWorker fixture turn without maintaining a
/// test-only copy of Page task authorization.
///
/// Browser-context work first publishes into the Page's real typed sources.
/// One ready Page task is then applied through
/// `PageVm::apply_selected_page_scheduler_task`, the same dispatch boundary as
/// the production owner-loop. Returning after one selected task is essential:
/// ServiceWorker lifecycle progress and zero-delay Page timers are different
/// owners and must be allowed to interleave instead of being drained in one
/// test-only batch.
pub(super) async fn drain_service_worker_test_turn(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    browser_context_runtime: &crate::runtime::RendererBrowserContextRuntime,
    loader: &ResourceRequestClient,
) -> bool {
    browser_context_runtime.drain_service_worker_service_lane();

    let applied_page_task = page
        .run_one_oldest_ready_page_task_executor_turn(loader)
        .await
        .expect("typed ServiceWorker fixture Page task should apply");
    if !applied_page_task {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            page.wait_for_task_executor_work_arrival(),
        )
        .await;
        return page
            .run_one_oldest_ready_page_task_executor_turn(loader)
            .await
            .expect("woken ServiceWorker fixture Page task should apply");
    }
    true
}
