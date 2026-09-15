use super::*;

use crate::page_task_queue::{RendererPageReadyDescriptor, RendererPageSchedulerTask};
use crate::runtime::{PageVmNetworkIdleWaitAdvance, PageVmNetworkIdleWaitState};

#[tokio::test(flavor = "current_thread")]
async fn network_idle_wait_observes_deadline_without_checkpointing_the_agent() {
    run_page_vm_async_test(async move {
        let loader_owner =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let loader = loader_owner.handle();
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__networkIdleObserverCheckpoint = 0;
Promise.resolve().then(() => __networkIdleObserverCheckpoint += 1);
"queued"
"#,
            )?;

        let advance = page_vm
            .advance_network_idle_wait_turn(
                PageVmNetworkIdleWaitState::default(),
                Duration::from_secs(1),
            )
            .await?;
        assert!(matches!(
            advance,
            PageVmNetworkIdleWaitAdvance::Waiting { .. }
                | PageVmNetworkIdleWaitAdvance::Progressed { .. }
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(__networkIdleObserverCheckpoint)",
                )?,
            "0",
            "a pure NetworkIdle observation must not execute the Page agent's microtasks",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("NetworkIdle observer authority test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_timer_task_checkpoints_without_a_wait_observer() {
    run_page_vm_async_test(async move {
        let loader_owner =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let loader = loader_owner.handle();
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__timerWithoutObserver = [];
setTimeout(() => {
  __timerWithoutObserver.push("callback");
  Promise.resolve().then(() => __timerWithoutObserver.push("microtask"));
}, 0);
"queued"
"#,
        )?;

        let deadline = match page_vm
            .due_page_timer_ready_descriptor(
                crate::page_task_queue::RendererPageTimerSelection::AnyReady,
            )
            .expect("the zero-delay timer should be scheduler-visible")
        {
            RendererPageReadyDescriptor::Timer { deadline, .. } => deadline,
            other => panic!("expected a timer descriptor, got {other:?}"),
        };
        page_vm
            .apply_selected_page_scheduler_task_on_owner_lane_for_test(
                RendererPageSchedulerTask::Timer {
                    deadline,
                    selection: crate::page_task_queue::RendererPageTimerSelection::AnyReady,
                },
            )
            .await?;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test("__timerWithoutObserver.join('|')",)?,
            "callback|microtask",
            "the selected timer task must finish its own checkpoint without protocol polling",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected timer no-observer liveness test should run");
}
