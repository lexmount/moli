use super::*;

use crate::{
    page_task_queue::{
        PageSharedWorkerClientEventTargetEffect, RendererOwnerWakeSource,
        RendererSharedWorkerClientEventKind,
    },
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
    shared_worker_runtime::{
        SharedWorkerClientEndpointDisposition, SharedWorkerClientError, SharedWorkerClientEvent,
        SharedWorkerRuntimeOwnerWake, shared_worker_owner_wake_channel,
    },
    worker::WorkerParentErrorEventKind,
};

fn shared_worker_test_error(
    endpoint_disposition: SharedWorkerClientEndpointDisposition,
) -> SharedWorkerClientEvent {
    SharedWorkerClientEvent::Error(SharedWorkerClientError::new(
        "test SharedWorker error".to_owned(),
        "https://shared-worker-turn.test/worker.js".to_owned(),
        3,
        5,
        WorkerParentErrorEventKind::ErrorEvent,
        endpoint_disposition,
    ))
}

async fn wait_for_shared_worker_client_event(
    page_vm: &mut PageVm,
    shared_worker_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake>,
    page_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::page_task_queue::RendererOwnerWake,
    >,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                wake = shared_worker_wake_rx.recv() => {
                    let wake = wake.ok_or_else(|| {
                        anyhow::anyhow!("SharedWorker service wake route closed before client event")
                    })?;
                    if matches!(wake, SharedWorkerRuntimeOwnerWake::ServiceLane) {
                        page_vm
                            .runtime_hooks
                            .browser_context_runtime
                            .drain_shared_worker_service_lane();
                    }
                }
                wake = page_wake_rx.recv() => {
                    let wake = wake.ok_or_else(|| {
                        anyhow::anyhow!("Page owner wake route closed before SharedWorker client event")
                    })?;
                    if wake.source_for_test() == RendererOwnerWakeSource::SharedWorkerClientEvent {
                        return Ok(());
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for typed SharedWorker client event"))?
}

pub(super) fn install_shared_worker_service_wake(
    page_vm: &PageVm,
) -> tokio::sync::mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake> {
    let (wake_tx, wake_rx) = shared_worker_owner_wake_channel();
    page_vm
        .runtime_hooks
        .browser_context_runtime
        .add_shared_worker_owner_wake_sender(wake_tx);
    wake_rx
}

#[tokio::test(flavor = "current_thread")]
async fn shared_worker_error_body_cleans_up_callbacks_before_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://shared-worker-turn.test/current").unwrap();
        let (mut page_vm, _resource_source, mut page_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let mut shared_worker_wake_rx = install_shared_worker_service_wake(&page_vm);

        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__typedSharedWorkerEvents = [];
  const brokenSource = "function( { broken syntax";
  globalThis.__typedSharedWorker = new SharedWorker(
    "data:text/javascript," + encodeURIComponent(brokenSource),
    "typed-current-owner"
  );
  __typedSharedWorker.onerror = event => {
    __typedSharedWorkerEvents.push("error:" + event.type);
    Promise.resolve().then(() => {
      __typedSharedWorkerEvents.push("microtask");
      const script = document.createElement("script");
      script.textContent =
        "__typedSharedWorkerEvents.push('runtime-script')";
      document.body.appendChild(script);
    });
  };
  __typedSharedWorker.port.start();
})()
"#,
        )?;
        wait_for_shared_worker_client_event(
            &mut page_vm,
            &mut shared_worker_wake_rx,
            &mut page_wake_rx,
        )
        .await?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let current_root = page_vm.document_lifecycle.identity().document;
        let task = page_vm
            .take_shared_worker_client_event_body_task_for_test()
            .expect("real SharedWorker error should produce one typed Page task");
        let outcome = page_vm.apply_selected_page_shared_worker_client_event_turn(task)?;
        assert_eq!(outcome.action.owner.root_document(), current_root);
        assert_eq!(
            outcome.action.event_kind,
            RendererSharedWorkerClientEventKind::Error
        );
        assert_eq!(
            outcome.action.target_effect,
            PageSharedWorkerClientEventTargetEffect::ErrorCallbackDispatchedToCurrentOwner {
                endpoint_disposition: SharedWorkerClientEndpointDisposition::Retire,
            }
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CallbackCompletion));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedSharedWorkerEvents.join('|')")?,
            "error:error|microtask|runtime-script",
            "listener cleanup must drain reactions before the selected task completes"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the SharedWorker error body must not consume unrelated runtime residence"
        );
        assert_eq!(
            page_vm.vm().shared_worker_client_count_for_test(),
            0,
            "terminal application should forget exactly the authorized wrapper"
        );
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedSharedWorkerEvents.join('|')")?,
            "error:error|microtask|runtime-script",
            "selected task completion must not repeat callback reactions or their inline scripts"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SharedWorker current-owner event should run through its typed executor");
}

#[tokio::test(flavor = "current_thread")]
async fn shared_worker_nonterminal_error_runs_through_selected_completion_and_retains_endpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://shared-worker-turn.test/nonterminal-error").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__sharedWorkerNonterminalEvents = [];
globalThis.__sharedWorkerNonterminal = new SharedWorker(
  "data:text/javascript,onconnect = () => {}",
  "typed-nonterminal-error"
);
__sharedWorkerNonterminal.onerror = event => {
  __sharedWorkerNonterminalEvents.push("error:" + event.type);
  Promise.resolve().then(() =>
    __sharedWorkerNonterminalEvents.push("microtask")
  );
};
__sharedWorkerNonterminal.port.start();
"queued"
"#,
        )?;
        let (_client_id, producer) = page_vm
            .vm()
            .only_shared_worker_client_event_producer_for_test()?;
        producer
            .send(shared_worker_test_error(
                SharedWorkerClientEndpointDisposition::Retain,
            ))
            .expect("the exact nonterminal error should enter its typed source");

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::SharedWorkerClientEvent,
                    &loader
                )
                .await?,
            "one SharedWorker event should run through the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__sharedWorkerNonterminalEvents.join('|')")?,
            "error:error|microtask"
        );
        assert_eq!(
            page_vm.vm().shared_worker_client_count_for_test(),
            1,
            "a nonterminal error must retain the exact SharedWorker endpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SharedWorker nonterminal error completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn shared_worker_error_without_listener_is_checkpoint_only() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://shared-worker-turn.test/no-listener").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__sharedWorkerWithoutListener = new SharedWorker(
  "data:text/javascript,onconnect = () => {}",
  "typed-no-listener"
);
__sharedWorkerWithoutListener.port.start();
"queued"
"#,
        )?;
        let (client_id, producer) = page_vm
            .vm()
            .only_shared_worker_client_event_producer_for_test()?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        page_vm
            .runtime_hooks
            .browser_context_runtime
            .remove_shared_worker_client(client_id);
        producer
            .send(shared_worker_test_error(
                SharedWorkerClientEndpointDisposition::Retire,
            ))
            .expect("the exact terminal error should enter its typed source");

        let task = page_vm
            .take_shared_worker_client_event_body_task_for_test()
            .expect("one exact SharedWorker error should be ready");
        let body = page_vm.apply_selected_page_shared_worker_client_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageSharedWorkerClientEventTargetEffect::CurrentOwnerErrorHadNoCallback {
                endpoint_disposition: SharedWorkerClientEndpointDisposition::Retire,
            }
        );
        let completion = body.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "listener-free error completion must not consume unrelated runtime work"
        );
        assert_eq!(
            page_vm.vm().shared_worker_client_count_for_test(),
            0,
            "the terminal error must still retire its exact endpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SharedWorker no-listener completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn shared_worker_closed_is_checkpoint_only_endpoint_transition() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://shared-worker-turn.test/closed").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__closedSharedWorker = new SharedWorker(
  "data:text/javascript,onconnect = () => {}",
  "typed-closed"
);
__closedSharedWorker.port.start();
"queued"
"#,
        )?;
        let (client_id, producer) = page_vm
            .vm()
            .only_shared_worker_client_event_producer_for_test()?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        page_vm
            .runtime_hooks
            .browser_context_runtime
            .remove_shared_worker_client(client_id);
        producer
            .send(SharedWorkerClientEvent::Closed)
            .expect("the exact close should enter its typed source");

        let task = page_vm
            .take_shared_worker_client_event_body_task_for_test()
            .expect("one exact SharedWorker close should be ready");
        let body = page_vm.apply_selected_page_shared_worker_client_event_turn(task)?;
        assert_eq!(
            body.action.event_kind,
            RendererSharedWorkerClientEventKind::Closed
        );
        assert_eq!(
            body.action.target_effect,
            PageSharedWorkerClientEventTargetEffect::EndpointClosedByCurrentOwner
        );
        let completion = body.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "pure endpoint cleanup must not consume unrelated runtime work"
        );
        assert_eq!(
            page_vm.vm().shared_worker_client_count_for_test(),
            0,
            "close must retire exactly the authorized endpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SharedWorker close completion test should run");
}

#[test]
fn shared_worker_error_rejects_a_real_page_vm_replacement() {
    run_page_vm_large_stack_async_test(
        "shared-worker-client-event-page-vm-replacement",
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
            let (page_vm, _resource_source, mut page_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let mut shared_worker_wake_rx = install_shared_worker_service_wake(&page_vm);
                    page_vm.vm_mut().eval(
                        r#"
(() => {
  globalThis.__retiredSharedWorkerHandlerRan = false;
  const brokenSource = "function( { broken syntax";
  globalThis.__retiredSharedWorker = new SharedWorker(
    "data:text/javascript," + encodeURIComponent(brokenSource),
    "retired-page-vm"
  );
  __retiredSharedWorker.onerror = () => {
    globalThis.__retiredSharedWorkerHandlerRan = true;
  };
  __retiredSharedWorker.port.start();
})()
"#,
                    )?;
                    wait_for_shared_worker_client_event(
                        &mut page_vm,
                        &mut shared_worker_wake_rx,
                        &mut page_wake_rx,
                    )
                    .await?;
                    let retired_root = page_vm.document_lifecycle.identity().document;

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
                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(retired_root, current_root);
                    page_vm
                        .vm_mut()
                        .eval_without_microtask_checkpoint_for_test(
                            r#"
globalThis.__staleSharedWorkerReplacementBoundary = [];
Promise.resolve().then(() => {
  __staleSharedWorkerReplacementBoundary.push("microtask");
});
"queued"
"#,
                        )?;
                    page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::SharedWorkerClientEvent,
                        )
                        .expect("retired SharedWorker event should consume one stale discard turn");
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;
                    assert_eq!(
                        page_vm.vm().shared_worker_client_count_for_test(),
                        0,
                        "replacement must not retain the retired PageVm wrapper"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "__staleSharedWorkerReplacementBoundary.join('|')",
                            )?,
                        "",
                        "the stale event must not checkpoint the replacement realm"
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .runtime_script_work()
                            .dynamic_scripts
                            .pending_source_load_count_for_test(),
                        1,
                        "the stale event must not advance replacement runtime work"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("SharedWorker PageVm replacement should reject the retired event");
            server
                .await
                .expect("SharedWorker replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn shared_worker_error_rejects_a_replaced_child_realm() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://shared-worker-turn.test/child-realm").unwrap();
        let (mut page_vm, _resource_source, mut page_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let mut shared_worker_wake_rx = install_shared_worker_service_wake(&page_vm);

        page_vm.vm_mut().eval(
            "const frame = document.createElement('iframe'); \
             frame.id = 'shared-worker-realm-replacement'; \
             document.body.appendChild(frame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("shared-worker-realm-replacement")
            .expect("realm replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "shared-worker-realm-replacement",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__retiredChildSharedWorkerHandlerRan = false;
  const frame = document.getElementById("shared-worker-realm-replacement");
  const ChildSharedWorker = frame.contentWindow.SharedWorker;
  const brokenSource = "function( { broken syntax";
  globalThis.__retiredChildSharedWorker = new ChildSharedWorker(
    "data:text/javascript," + encodeURIComponent(brokenSource),
    "retired-child-realm"
  );
  __retiredChildSharedWorker.onerror = () => {
    globalThis.__retiredChildSharedWorkerHandlerRan = true;
  };
  __retiredChildSharedWorker.port.start();
})()
"#,
        )?;
        wait_for_shared_worker_client_event(
            &mut page_vm,
            &mut shared_worker_wake_rx,
            &mut page_wake_rx,
        )
        .await?;
        assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 1);

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "shared-worker-realm-replacement",
        )?;
        assert_eq!(
            page_vm.vm().shared_worker_client_count_for_test(),
            0,
            "realm retirement must actively disconnect its SharedWorker wrapper"
        );
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let stale = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::SharedWorkerClientEvent,
            )
            .expect("retired child-realm event should consume one stale discard turn");
        page_vm
            .run_claimed_selected_page_task_for_test(stale, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__retiredChildSharedWorkerHandlerRan)")?,
            "false"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the retired child event must not advance current runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("SharedWorker child-realm replacement should discard the retired event");
}
