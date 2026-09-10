use super::*;

use crate::{
    page_task_queue::{
        PageWorkerHostBridgeCurrentEffect, PageWorkerHostBridgeTargetEffect,
        RendererOwnerWakeSource,
    },
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
    worker::{WorkerRuntimeEvent, WorkerToParentMessage},
};

#[tokio::test(flavor = "current_thread")]
async fn worker_host_bridge_context_body_leaves_microtask_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://worker-host-bridge.test/checkpoint-boundary").unwrap();
        let (mut page_vm, _resource_source, _wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__workerHostBridgeCheckpointBoundary = [];
Promise.resolve().then(() => {
  __workerHostBridgeCheckpointBoundary.push("microtask");
});
"queued"
"#,
            )?;

        page_vm
            .vm()
            .worker_host_bridge_sender_for_test()
            .send(WorkerRuntimeEvent::SharedWorkerMessage {
                instance_id: moli_shared_worker::SharedWorkerInstanceId::from_u64(1),
                message: Box::new(WorkerToParentMessage::PendingSubresourceFetchCanceled {
                    fetch_id: 7,
                }),
            })
            .expect("SharedWorker host record should enter the stable Networking source");
        let task = page_vm
            .take_worker_host_bridge_body_task_for_test()
            .expect("one exact Worker host-bridge task should be ready");
        let outcome = page_vm.apply_selected_page_worker_host_bridge_turn(task)?;
        assert_eq!(
            outcome.action.target_effect(),
            PageWorkerHostBridgeTargetEffect::AppliedToCurrentOwner(
                PageWorkerHostBridgeCurrentEffect::StateAppliedInPageContext,
            )
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__workerHostBridgeCheckpointBoundary.join('|')",
                )?,
            "",
            "the Worker host bridge body must leave the task checkpoint to the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "the body must not consume unrelated runtime residence"
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__workerHostBridgeCheckpointBoundary.join('|')",
                )?,
            "microtask",
            "the selected completion must submit the task-end checkpoint exactly once"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "CheckpointOnly must not borrow callback runtime-follow-up authority"
        );

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
Promise.resolve().then(() => {
  __workerHostBridgeCheckpointBoundary.push("selected-dispatcher");
});
"queued"
"#,
            )?;
        page_vm
            .vm()
            .worker_host_bridge_sender_for_test()
            .send(WorkerRuntimeEvent::SharedWorkerMessage {
                instance_id: moli_shared_worker::SharedWorkerInstanceId::from_u64(1),
                message: Box::new(WorkerToParentMessage::PendingSubresourceFetchCanceled {
                    fetch_id: 8,
                }),
            })
            .expect("a second host record should enter the same stable source");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WorkerHostBridge, &loader)
                .await?,
            "the exact WorkerHostBridge variant must return through the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__workerHostBridgeCheckpointBoundary.join('|')",
                )?,
            "microtask|selected-dispatcher"
        );
        assert!(
            page_vm.take_worker_host_bridge_body_task_for_test().is_none(),
            "the exact test driver must consume one WorkerHostBridge task, not scan unrelated Networking work"
        );

        let (worker_id, _client_producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
Promise.resolve().then(() => {
  __workerHostBridgeCheckpointBoundary.push("dedicated-context");
});
"queued"
"#,
            )?;
        page_vm
            .vm()
            .worker_host_bridge_sender_for_test()
            .send(WorkerRuntimeEvent::Message {
                worker_id,
                message: Box::new(WorkerToParentMessage::PendingSubresourceFetchCanceled {
                    fetch_id: 9,
                }),
            })
            .expect("a DedicatedWorker host record should enter the stable source");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WorkerHostBridge, &loader)
                .await?,
            "the DedicatedWorker host record must use the same production completion boundary"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__workerHostBridgeCheckpointBoundary.join('|')",
                )?,
            "microtask|selected-dispatcher|dedicated-context",
            "both SharedWorker and DedicatedWorker context-entering records owe one selected-task checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Worker host bridge body/checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn worker_host_bridge_applies_current_target_and_discards_unknown_target() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://worker-host-bridge.test/current").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (worker_id, _client_producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;
        let root_document = page_vm.document_lifecycle.identity().document;
        let sender = page_vm.vm().worker_host_bridge_sender_for_test();
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__workerHostBridgeContextFreeBoundary = [];
Promise.resolve().then(() => {
  __workerHostBridgeContextFreeBoundary.push("microtask");
});
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        sender
            .send(WorkerRuntimeEvent::HostBridgeDrained { worker_id })
            .expect("current Worker terminal should enter the stable networking source");
        let wake = wake_rx
            .recv()
            .await
            .expect("Worker networking admission should wake the Page owner");
        assert_eq!(
            wake.source_for_test(),
            RendererOwnerWakeSource::NetworkingTask
        );

        let _ = page_vm.page_diagnostics_snapshot()?;

        let current_task = page_vm
            .take_worker_host_bridge_body_task_for_test()
            .expect("current Worker bridge task should remain queued");
        let current = page_vm.apply_selected_page_worker_host_bridge_turn(current_task)?;
        let current_action = current.action;
        assert_eq!(current_action.owner().root_document(), root_document);
        assert_eq!(
            current_action.target_effect(),
            PageWorkerHostBridgeTargetEffect::AppliedToCurrentOwner(
                PageWorkerHostBridgeCurrentEffect::StateAppliedWithoutPageContext,
            ),
            "PageDiagnosticsSnapshot must observe without consuming the queued Worker task"
        );
        let current_completion = current_action.into_page_task_completion();
        assert!(matches!(
            current_completion,
            PageTaskCompletion::CheckpointOnly
        ));
        page_vm
            .finish_selected_page_task_completion(current_completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__workerHostBridgeContextFreeBoundary.join('|')",
                )?,
            "microtask",
            "a current HostBridgeDrained record remains a selected Page task even though its body does not enter V8"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "a context-free control record must not consume unrelated runtime residence"
        );

        let missing_worker_id = crate::types::DedicatedWorkerId::new(worker_id.as_u64() + 1);
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
Promise.resolve().then(() => {
  __workerHostBridgeContextFreeBoundary.push("stale-target-microtask");
});
"queued"
"#,
            )?;
        sender
            .send(WorkerRuntimeEvent::HostBridgeDrained {
                worker_id: missing_worker_id,
            })
            .expect("unknown current-root target should still enter one discard turn");
        let stale_target_task = page_vm
            .take_worker_host_bridge_body_task_for_test()
            .expect("unknown Worker target should consume one typed discard turn");
        let stale_target =
            page_vm.apply_selected_page_worker_host_bridge_turn(stale_target_task)?;
        let stale_target_action = stale_target.action;
        assert_eq!(
            stale_target_action.target_effect(),
            PageWorkerHostBridgeTargetEffect::IgnoredStaleTarget
        );
        let stale_target_completion = stale_target_action.into_page_task_completion();
        assert!(matches!(
            stale_target_completion,
            PageTaskCompletion::NoCompletion
        ));
        page_vm
            .finish_selected_page_task_completion(stale_target_completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__workerHostBridgeContextFreeBoundary.join('|')",
                )?,
            "microtask",
            "a stale target is not a current Page task and must not checkpoint the replacement/current agent"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Worker host bridge current/stale-target turns should complete");
}

#[tokio::test(flavor = "current_thread")]
async fn worker_host_bridge_rejects_client_facing_dedicated_worker_records() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://worker-host-bridge.test/reject-client-record").unwrap();
        let (mut page_vm, _resource_source, _wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (worker_id, _client_producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;

        page_vm
            .vm()
            .worker_host_bridge_sender_for_test()
            .send(WorkerRuntimeEvent::Message {
                worker_id,
                message: Box::new(WorkerToParentMessage::Post(
                    crate::structured_clone::V8StructuredClonePayload::default(),
                )),
            })
            .expect("the malformed internal route should remain observable by the executor");
        let task = page_vm
            .take_worker_host_bridge_body_task_for_test()
            .expect("the malformed host record should retain exact task identity");
        let error = page_vm
            .apply_selected_page_worker_host_bridge_turn(task)
            .expect_err("client-facing records must never silently enter the host/control source");
        assert!(
            error
                .to_string()
                .contains("client-facing DedicatedWorker event entered the host-bridge source"),
            "the release-build invariant error should identify the crossed source boundary: {error:#}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DedicatedWorker source-boundary rejection test should run");
}

#[test]
fn worker_host_bridge_rejects_old_root_before_reused_worker_id() {
    run_page_vm_large_stack_async_test(
        "worker-host-bridge-old-root-reused-worker-id",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let (retired_worker_id, _retired_client_producer) = page_vm
                        .vm_mut()
                        .register_loading_dedicated_worker_client_event_producer_for_test()?;
                    let retired_root = page_vm.document_lifecycle.identity().document;
                    let retired_sender = page_vm.vm().worker_host_bridge_sender_for_test();

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
                    let mut pending_document_lifecycle_turn = None;
                    let navigation = page_vm
                        .follow_pending_location_navigation_one_turn_async(
                            &mut pending_document_lifecycle_turn,
                            PageVmInitStage::Load,
                        )
                        .await?;
                    assert!(matches!(
                        navigation,
                        crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                            | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                                ..
                            }
                    ));

                    let (current_worker_id, _current_client_producer) = page_vm
                        .vm_mut()
                        .register_loading_dedicated_worker_client_event_producer_for_test()?;
                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_eq!(retired_worker_id, current_worker_id);
                    assert_ne!(retired_root, current_root);
                    page_vm
                        .vm_mut()
                        .eval_without_microtask_checkpoint_for_test(
                            r#"
globalThis.__staleWorkerHostBridgeBoundary = [];
Promise.resolve().then(() => {
  __staleWorkerHostBridgeBoundary.push("microtask");
});
"queued"
"#,
                        )?;
                    page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

                    retired_sender
                        .send(WorkerRuntimeEvent::HostBridgeDrained {
                            worker_id: retired_worker_id,
                        })
                        .expect("late old-root terminal should enter the stable Page source");
                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::WorkerHostBridge,
                        )
                        .expect("late old-root task should consume one discard turn");
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "globalThis.__staleWorkerHostBridgeBoundary.join('|')",
                            )?,
                        "",
                        "an old-root bridge task must not enter replacement V8"
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .runtime_script_work()
                            .dynamic_scripts.pending_source_load_count_for_test(),
                        1,
                        "an old-root bridge task must not advance replacement runtime work"
                    );
                    assert!(
                        page_vm
                            .vm()
                            .current_dedicated_worker_client_event_identity(current_worker_id)
                            .is_some(),
                        "discarding the old root must not retire the reused current Worker id"
                    );

                    let current_sender = page_vm.vm().worker_host_bridge_sender_for_test();
                    current_sender
                        .send(WorkerRuntimeEvent::HostBridgeDrained {
                            worker_id: current_worker_id,
                        })
                        .expect("current-root terminal should remain routable");
                    let current = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::WorkerHostBridge,
                        )
                        .expect("the current-root task should follow the stale source head");
                    page_vm
                        .run_claimed_selected_page_task_for_test(current, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "globalThis.__staleWorkerHostBridgeBoundary.join('|')",
                            )?,
                        "microtask",
                        "the current context-free host terminal must still receive ordinary selected-task completion"
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .runtime_script_work()
                            .dynamic_scripts.pending_source_load_count_for_test(),
                        1,
                        "CheckpointOnly must not turn the current host terminal into a generic runtime drain"
                    );
                    assert!(
                        page_vm.take_worker_host_bridge_body_task_for_test().is_none(),
                        "both exact Networking tasks must settle once"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("old-root Worker bridge task should be rejected exactly");
            server.await.expect("replacement server should finish");
        },
    );
}
