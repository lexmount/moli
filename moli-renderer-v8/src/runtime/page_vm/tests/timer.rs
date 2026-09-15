use super::*;

use super::super::page_timer::PageTimerTurnAction;
use crate::page_task_queue::{
    RendererPageReadyDescriptor, RendererPageSchedulerTask, RendererPageTimerSelection,
};

const ANY_READY_TIMER: RendererPageTimerSelection = RendererPageTimerSelection::AnyReady;

fn due_timer_deadline(page_vm: &PageVm) -> Instant {
    match page_vm
        .due_page_timer_ready_descriptor(ANY_READY_TIMER)
        .expect("a due timer descriptor should be ready")
    {
        RendererPageReadyDescriptor::Timer { deadline, .. } => deadline,
        other => panic!("expected timer descriptor, got {other:?}"),
    }
}

async fn run_timer_through_selected_dispatcher(
    page_vm: &mut PageVm,
    deadline: Instant,
) -> anyhow::Result<()> {
    page_vm
        .apply_selected_page_scheduler_task_on_owner_lane_for_test(
            RendererPageSchedulerTask::Timer {
                deadline,
                selection: ANY_READY_TIMER,
            },
        )
        .await?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn page_timer_turn_consumes_exactly_one_due_timer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__pageTimerTurnOrder = [];
setTimeout(() => {
  __pageTimerTurnOrder.push("first");
  Promise.resolve().then(() => __pageTimerTurnOrder.push("microtask:first"));
}, 0);
setTimeout(() => {
  __pageTimerTurnOrder.push("second");
  Promise.resolve().then(() => __pageTimerTurnOrder.push("microtask:second"));
}, 0);
"queued"
"#,
        )?;

        let first_deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, first_deadline).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__pageTimerTurnOrder)")?,
            r#"["first","microtask:first"]"#,
            "one selected timer task must checkpoint its callback without draining the next timer"
        );

        let second_deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, second_deadline).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__pageTimerTurnOrder)")?,
            r#"["first","microtask:first","second","microtask:second"]"#
        );
        assert!(
            page_vm
                .due_page_timer_ready_descriptor(ANY_READY_TIMER)
                .is_none(),
            "two timer tasks must consume exactly two selected turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed timer one-turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_body_leaves_reactions_for_selected_callback_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__timerBodyBoundary = [];
setTimeout(() => {
  __timerBodyBoundary.push("callback");
  Promise.resolve().then(() => __timerBodyBoundary.push("microtask"));
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        let body = page_vm.apply_selected_page_timer_turn(deadline, ANY_READY_TIMER)?;
        assert_eq!(body.action, PageTimerTurnAction::Consumed { deadline });
        assert_eq!(
            page_vm.vm_mut().eval("__timerBodyBoundary.join('|')")?,
            "callback",
            "the timer heap executor must leave Promise reactions pending"
        );

        page_vm.finish_selected_page_callback_task().await?;
        assert_eq!(
            page_vm.vm_mut().eval("__timerBodyBoundary.join('|')")?,
            "callback|microtask",
            "the selected timer completion must own the single task checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_timer_error_still_completes_its_task_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__timerErrorBoundary = [];
setTimeout(() => {
  __timerErrorBoundary.push("callback");
  Promise.resolve().then(() => __timerErrorBoundary.push("microtask"));
  throw new Error("selected timer error boundary");
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__timerErrorBoundary.join('|')")?,
            "callback|microtask",
            "a throwing callback still consumed a selected timer task whose checkpoint must finish"
        );
        assert!(
            page_vm
                .vm_mut()
                .runtime_observable_lifecycle_errors_for_testing()
                .iter()
                .any(|warning| warning.contains("selected timer error boundary")),
            "the callback failure must remain observable after task completion"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer error completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__timerChildOrder = [];
setTimeout(() => {
  __timerChildOrder.push("callback");
  Promise.resolve().then(() => {
    __timerChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "timer-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm,
deadline).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__timerChildOrder.join('|')")?,
            "callback|microtask"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during timer completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn interval_reaction_can_cancel_the_rescheduled_timer_at_task_end() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__intervalCheckpointOrder = [];
const interval = setInterval(() => {
  __intervalCheckpointOrder.push("callback");
  Promise.resolve().then(() => {
    __intervalCheckpointOrder.push("microtask:clear");
    clearInterval(interval);
  });
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm,
deadline).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__intervalCheckpointOrder.join('|')")?,
            "callback|microtask:clear"
        );
        assert_eq!(
            page_vm.vm().next_timeout_deadline(),
            None,
            "the task-end reaction must cancel the interval body that was rescheduled before checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("interval task-end cancellation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_timer_revalidates_the_heap_head_before_execution() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__revalidatedTimerOrder = [];
setTimeout(() => {
  __revalidatedTimerOrder.push("callback");
  Promise.resolve().then(() => __revalidatedTimerOrder.push("microtask"));
}, 0);
"queued"
"#,
        )?;
        let actual_deadline = page_vm
            .vm()
            .next_timeout_deadline()
            .expect("zero-delay timer must retain a heap deadline");
        let stale_deadline = actual_deadline
            .checked_add(Duration::from_nanos(1))
            .expect("test deadline should be representable");

        let stale = page_vm.apply_selected_page_timer_turn(stale_deadline, ANY_READY_TIMER)?;
        assert_eq!(
            stale.action,
            PageTimerTurnAction::NoLongerRunnable {
                expected_deadline: stale_deadline,
                actual_deadline: Some(actual_deadline),
            }
        );
        assert_eq!(
            page_vm.vm_mut().eval("__revalidatedTimerOrder.join('|')")?,
            "",
            "a stale deadline must not enter V8 or checkpoint another task's reactions"
        );

        run_timer_through_selected_dispatcher(&mut page_vm, actual_deadline).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__revalidatedTimerOrder.join('|')")?,
            "callback|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer descriptor revalidation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_deadline_observation_cannot_consume_a_due_timer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            "globalThis.__waitObserverTimerRan = false; \
             setTimeout(() => { __waitObserverTimerRan = true; }, 0);",
        )?;
        assert!(page_vm.vm().has_ready_timeout());

        let executor = page_vm.local_executor.clone();
        crate::runtime::access::run_named_owner_local_task(
            executor,
            "timer deadline observation test task closed",
            async move {
                let timer_deadline = page_vm.vm().ms_to_next_timeout();
                assert_eq!(timer_deadline, Some(0));
                assert_eq!(
                    page_vm.vm_mut().eval("String(__waitObserverTimerRan)")?,
                    "false",
                    "reading the timer deadline must not steal the scheduler-owned timer"
                );
                assert!(
                    page_vm
                        .due_page_timer_ready_descriptor(ANY_READY_TIMER)
                        .is_some()
                );
                Ok(())
            },
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer deadline observation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_is_not_runnable_before_its_deadline() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "globalThis.__futureTimerRan = false; \
             setTimeout(() => { __futureTimerRan = true; }, 60000);",
        )?;

        assert!(page_vm.vm().next_timeout_deadline().is_some());
        assert!(
            page_vm
                .due_page_timer_ready_descriptor(ANY_READY_TIMER)
                .is_none(),
            "a future heap entry is a deadline, not a runnable Page task"
        );
        assert_eq!(page_vm.vm_mut().eval("String(__futureTimerRan)")?, "false");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("future timer readiness test should run");
}
