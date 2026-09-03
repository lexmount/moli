use super::*;

use crate::page_task_queue::{
    PageDedicatedWorkerClientEventTargetEffect, RendererDedicatedWorkerClientEvent,
    RendererDedicatedWorkerClientEventKind, RendererDedicatedWorkerMessageEvent,
};
use crate::runtime::PageTaskCompletion;
use crate::worker::{
    WorkerErrorPhase, WorkerErrorSource, WorkerParentErrorEventKind, WorkerRuntimeEvent,
};

fn current_single_child_document_owner(
    page_vm: &PageVm,
    context: &str,
) -> crate::frame_owner_model::FrameDocumentTaskOwner {
    page_vm
        .vm()
        .only_child_document_owner_for_dedicated_worker_test(context)
        .unwrap_or_else(|error| panic!("{error:#}"))
}

#[tokio::test(flavor = "current_thread")]
async fn dedicated_worker_message_body_leaves_reactions_and_runtime_scripts_for_selected_completion()
 {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/dedicated-worker-task-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__dedicatedWorkerTaskBoundary = [];
globalThis.__dedicatedWorkerTaskBoundaryWorker =
  new Worker("data:text/javascript,onmessage = () => {}");
__dedicatedWorkerTaskBoundaryWorker.onmessage = () => {
  __dedicatedWorkerTaskBoundary.push("callback");
  Promise.resolve().then(() => {
    __dedicatedWorkerTaskBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent =
      "__dedicatedWorkerTaskBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
};
"queued"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let (_worker_id, producer) = page_vm
            .vm()
            .only_dedicated_worker_client_event_producer_for_test()?;
        producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                RendererDedicatedWorkerMessageEvent::Message(
                    page_vm
                        .vm_mut()
                        .dedicated_worker_message_payload_for_test("payload")?,
                ),
            ))
            .expect("the exact Worker message should enter its typed source");

        let task = page_vm
            .take_dedicated_worker_client_event_body_task_for_test()
            .expect("one exact Worker message should be ready");
        let body = page_vm.apply_selected_page_dedicated_worker_client_event_turn(task)?;
        assert_eq!(
            body.action.event_kind,
            RendererDedicatedWorkerClientEventKind::Message
        );
        assert_eq!(
            body.action.target_effect,
            PageDedicatedWorkerClientEventTargetEffect::CallbackDispatchedToCurrentOwner
        );
        let completion = body.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CallbackCompletion));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__dedicatedWorkerTaskBoundary.join('|')")?,
            "callback",
            "the DedicatedWorker message body must leave listener reactions pending"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the Worker message body must not consume unrelated runtime residence"
        );

        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__dedicatedWorkerTaskBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "selected completion must own the checkpoint and runtime follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DedicatedWorker message body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn dedicated_worker_state_transition_is_checkpoint_only() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/dedicated-worker-state-transition").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (_worker_id, producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        producer
            .send(RendererDedicatedWorkerClientEvent::ClientSourceDrained)
            .expect("the exact Worker terminal should enter its typed source");

        let task = page_vm
            .take_dedicated_worker_client_event_body_task_for_test()
            .expect("one exact Worker terminal should be ready");
        let body = page_vm.apply_selected_page_dedicated_worker_client_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageDedicatedWorkerClientEventTargetEffect::StateAppliedToCurrentOwner
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
            "a non-callback Worker transition must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DedicatedWorker state transition completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn dedicated_worker_message_without_listener_is_checkpoint_only() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/dedicated-worker-no-listener").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (_worker_id, producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;
        let payload = page_vm
            .vm_mut()
            .dedicated_worker_message_payload_for_test("payload")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                RendererDedicatedWorkerMessageEvent::Message(payload),
            ))
            .expect("the exact Worker message should enter its typed source");

        let task = page_vm
            .take_dedicated_worker_client_event_body_task_for_test()
            .expect("one exact Worker message should be ready");
        let body = page_vm.apply_selected_page_dedicated_worker_client_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageDedicatedWorkerClientEventTargetEffect::CurrentOwnerHadNoCallback
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
            "a listener-free Worker task must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DedicatedWorker no-listener completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn dedicated_worker_error_propagation_distinguishes_bootstrap_from_runtime() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/dedicated-worker-bootstrap-error").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__dedicatedWorkerErrorPhases = [];
addEventListener("error", event => {
  __dedicatedWorkerErrorPhases.push(
    `window:${event.constructor.name}:${event.message}`
  );
  event.preventDefault();
});
globalThis.__dedicatedWorkerErrorPhaseWorker =
  new Worker("data:text/javascript,onmessage = () => {}");
__dedicatedWorkerErrorPhaseWorker.onerror = event => {
  __dedicatedWorkerErrorPhases.push(
    `worker:${event.constructor.name}:${event.message}:${event.cancelable}`
  );
};
"ready"
"#,
        )?;
        let (_worker_id, producer) = page_vm
            .vm()
            .only_dedicated_worker_client_event_producer_for_test()?;

        producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                RendererDedicatedWorkerMessageEvent::Error {
                    message: "bootstrap syntax".to_owned(),
                    filename: "https://example.com/bootstrap-worker.js".to_owned(),
                    lineno: 1,
                    colno: 1,
                    event_kind: WorkerParentErrorEventKind::Event,
                    phase: WorkerErrorPhase::Bootstrap,
                    source: WorkerErrorSource::InitialScriptEvaluation,
                },
            ))
            .expect("bootstrap Worker error should enter its typed source");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                    &loader,
                )
                .await?,
            "bootstrap Worker error should consume one selected Page task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__dedicatedWorkerErrorPhases.join('|')")?,
            "worker:Event:bootstrap syntax:true",
            "initial script errors must stop after the Worker error event"
        );

        producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                RendererDedicatedWorkerMessageEvent::Error {
                    message: "runtime boom".to_owned(),
                    filename: "https://example.com/runtime-worker.js".to_owned(),
                    lineno: 2,
                    colno: 3,
                    event_kind: WorkerParentErrorEventKind::ErrorEvent,
                    phase: WorkerErrorPhase::Runtime,
                    source: WorkerErrorSource::Runtime,
                },
            ))
            .expect("runtime Worker error should enter its typed source");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                    &loader,
                )
                .await?,
            "runtime Worker error should consume one selected Page task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__dedicatedWorkerErrorPhases.join('|')")?,
            "worker:Event:bootstrap syntax:true|worker:ErrorEvent:runtime boom:true|window:ErrorEvent:runtime boom",
            "uncanceled runtime errors must retain their owning Window propagation"
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DedicatedWorker bootstrap/runtime error propagation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn dedicated_worker_relay_terminal_waits_for_both_selected_source_fifos() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://worker-terminal-fence.test/selected-sources").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (worker_id, producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;
        let host_sender = page_vm.vm().worker_host_bridge_sender_for_test();
        let message_payload = page_vm
            .vm_mut()
            .dedicated_worker_message_payload_for_test("queued-before-client-terminal")?;

        producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                RendererDedicatedWorkerMessageEvent::Message(message_payload),
            ))
            .expect("client message should enter the production typed source");
        producer
            .send(RendererDedicatedWorkerClientEvent::ClientSourceDrained)
            .expect("client terminal should remain behind the prior message");
        host_sender
            .send(WorkerRuntimeEvent::HostBridgeDrained { worker_id })
            .expect("host terminal should enter the production Networking source");

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WorkerHostBridge,
                    &loader,
                )
                .await?,
            "the host terminal must run through the production selected dispatcher"
        );
        assert!(
            page_vm
                .vm()
                .current_dedicated_worker_client_event_identity(worker_id)
                .is_some(),
            "one drained source must retain the Worker while older client records remain"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                    &loader,
                )
                .await?,
            "the earlier client message must consume its selected Page task"
        );
        assert!(
            page_vm
                .vm()
                .current_dedicated_worker_client_event_identity(worker_id)
                .is_some(),
            "the Worker must remain live until the client terminal reaches its FIFO head"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                    &loader,
                )
                .await?,
            "the client terminal must consume the next selected Page task"
        );
        assert!(
            page_vm
                .vm()
                .current_dedicated_worker_client_event_identity(worker_id)
                .is_none(),
            "both selected source terminals must retire the Worker"
        );

        let (second_worker_id, second_producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;
        second_producer
            .send(RendererDedicatedWorkerClientEvent::ClientSourceDrained)
            .expect("second client terminal should enter its typed source");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                    &loader,
                )
                .await?,
            "the second client terminal must use the same selected dispatcher"
        );
        assert!(
            page_vm
                .vm()
                .current_dedicated_worker_client_event_identity(second_worker_id)
                .is_some(),
            "client completion must not retire a Worker with pending host records"
        );

        host_sender
            .send(WorkerRuntimeEvent::HostBridgeDrained {
                worker_id: second_worker_id,
            })
            .expect("second host terminal should enter the Networking source");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WorkerHostBridge,
                    &loader,
                )
                .await?,
            "the second host terminal must use the production selected dispatcher"
        );
        assert!(
            page_vm
                .vm()
                .current_dedicated_worker_client_event_identity(second_worker_id)
                .is_none(),
            "the terminal fence must work in either selected-source order"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DedicatedWorker relay-terminal selected-source test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_navigation_discards_retired_dedicated_worker_event_without_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://child-owner-worker.test/selected-dispatcher").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  globalThis.__ownerBoundWorkerFrame = frame;
  (document.body || document.documentElement || document).appendChild(frame);
  void frame.contentWindow;
})();
"frame-ready"
"#,
        )?;
        let initial_owner =
            current_single_child_document_owner(&page_vm, "initial-empty child Worker Document");

        page_vm
            .vm_mut()
            .eval("__ownerBoundWorkerFrame.srcdoc = '<p>committed</p>'; 'queued'")?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "first child Worker Document",
        )
        .await;
        let committed_owner =
            current_single_child_document_owner(&page_vm, "committed child Worker Document");
        assert_eq!(
            committed_owner.local_window_id, initial_owner.local_window_id,
            "the first secure commit must reuse the initial-empty LocalWindow"
        );
        assert_ne!(committed_owner.document_id, initial_owner.document_id);

        let child_context_id = page_vm
            .vm_mut()
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child Worker realm should exist");
        page_vm.vm_mut().eval_in_child_default_context(
            child_context_id,
            r#"
globalThis.__ownerBoundWorker =
  new Worker("data:text/javascript,onmessage = () => {}");
"created"
"#,
        )?;
        let workers = page_vm.vm().dedicated_worker_execution_contexts_for_test();
        assert_eq!(workers.len(), 1);
        assert_eq!(
            workers[0].1,
            crate::native_bridge::WindowExecutionContextOwner::Frame(
                committed_owner.local_window_id
            ),
            "the child Worker must retain its preparation-time LocalWindow"
        );
        let retired_worker_id = workers[0].0;
        let retired_producer = page_vm
            .vm()
            .dedicated_worker_client_event_producer_for_test(retired_worker_id)
            .expect("child Worker must retain its exact client-event producer");

        page_vm
            .vm_mut()
            .eval("__ownerBoundWorkerFrame.srcdoc = '<p>replacement</p>'; 'queued'")?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "replacement child Worker Document",
        )
        .await;
        assert!(
            page_vm
                .vm()
                .dedicated_worker_execution_contexts_for_test()
                .is_empty(),
            "child navigation must actively retire the old LocalWindow Worker"
        );

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__retiredChildWorkerCheckpoint = [];
Promise.resolve().then(() => {
  __retiredChildWorkerCheckpoint.push("microtask");
});
"queued"
"#,
            )?;
        retired_producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                // Deliberately not a decodable wire payload: a stale task must
                // be rejected by exact owner authorization before any realm or
                // structured-clone operation can observe it.
                RendererDedicatedWorkerMessageEvent::Message(
                    crate::structured_clone::V8StructuredClonePayload::default(),
                ),
            ))
            .expect("the retired Worker route should remain valid until Page retirement");
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
            )
            .expect("the late Worker event should consume one selected discard turn");
        let (claimed_owner, claimed_kind) = claimed
            .dedicated_worker_owner_and_event_kind()
            .expect("the exact selector must retain the retired Worker identity");
        assert_eq!(claimed_owner.worker_id(), retired_worker_id);
        assert_eq!(
            claimed_kind,
            RendererDedicatedWorkerClientEventKind::Message
        );
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__retiredChildWorkerCheckpoint.join('|')"
                )?,
            "",
            "a stale child Worker event must not checkpoint the current Page realm"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child Worker retirement selected-dispatcher test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn successful_script_loaded_worker_event_transitions_through_selected_dispatcher() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/worker-script-loaded-selected-dispatcher").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (worker_id, producer) = page_vm
            .vm_mut()
            .register_loading_dedicated_worker_client_event_producer_for_test()?;
        let loading = page_vm.page_diagnostics_snapshot()?;
        assert_eq!(loading.diagnostics.dedicated_worker_loading_count, 1);
        assert_eq!(
            loading
                .diagnostics
                .dedicated_worker_running_worker_isolate_count,
            0
        );

        producer
            .send(RendererDedicatedWorkerClientEvent::ScriptLoaded {
                script_url: "https://example.com/worker.js".to_owned(),
                script_source: crate::worker::WorkerScriptSource::text(
                    "self.onmessage = () => {};".to_owned(),
                ),
                network_response: Box::new(
                    crate::protocol_types::NavigationResponse::from_text_body(
                        Url::parse("https://example.com/worker.js").unwrap(),
                        200,
                        vec![(
                            "Content-Type".to_owned(),
                            "application/javascript".to_owned(),
                        )],
                        "self.onmessage = () => {};".to_owned(),
                    ),
                ),
                script_kind: crate::worker::WorkerScriptKind::Classic,
                secure_context: true,
                response_referrer_policy: None,
                network_partition_key: None,
                policy_context: Default::default(),
                content_security_policies: Vec::new(),
                content_security_report_only_policies: Vec::new(),
                content_security_reporting_endpoints:
                    crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(
                    ),
            })
            .expect("successful ScriptLoaded event should enter the typed Page source");
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
            )
            .expect("ScriptLoaded should produce one selected Page task");
        let (claimed_owner, claimed_kind) = claimed
            .dedicated_worker_owner_and_event_kind()
            .expect("the exact selector must retain the ScriptLoaded identity");
        assert_eq!(claimed_owner.worker_id(), worker_id);
        assert_eq!(
            claimed_kind,
            RendererDedicatedWorkerClientEventKind::ScriptLoaded
        );
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;

        let running = page_vm.page_diagnostics_snapshot()?;
        assert_eq!(running.diagnostics.dedicated_worker_loading_count, 0);
        assert_eq!(
            running
                .diagnostics
                .dedicated_worker_running_worker_isolate_count,
            1,
            "ScriptLoaded must become a running Worker before later client events can be delivered"
        );
        page_vm.vm_mut().forget_dedicated_worker_for_test(worker_id);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ScriptLoaded selected-dispatcher transition test should run");
}

#[test]
fn dedicated_worker_client_event_rejects_a_real_page_vm_replacement_identity_collision() {
    run_page_vm_large_stack_async_test(
        "dedicated-worker-client-event-real-page-vm-replacement-collision",
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
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let (retired_worker_id, retired_producer) =
                        page_vm
                            .vm_mut()
                            .register_loading_dedicated_worker_client_event_producer_for_test()?;
                    let retired_owner = retired_producer.owner();
                    assert_eq!(
                        retired_owner.root_document(),
                        page_vm.document_lifecycle.identity().document
                    );
                    retired_producer
                        .send(RendererDedicatedWorkerClientEvent::ScriptLoadFailed {
                            script_url: "https://example.test/retired-worker.js".to_owned(),
                            error_message: "retired worker load failed".to_owned(),
                            network_response: None,
                        })
                        .expect("retired Worker event should enter the stable Page source");

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

                    let (current_worker_id, current_producer) =
                        page_vm
                            .vm_mut()
                            .register_loading_dedicated_worker_client_event_producer_for_test()?;
                    let current_owner = current_producer.owner();
                    assert_eq!(
                        retired_worker_id, current_worker_id,
                        "fresh PageVm counters should naturally reuse the first Worker id"
                    );
                    assert_eq!(
                        retired_owner.execution_context(),
                        current_owner.execution_context(),
                        "fresh PageVm counters should naturally reuse the top Window/realm identity"
                    );
                    assert_ne!(
                        retired_owner.root_document(),
                        current_owner.root_document(),
                        "the stable Page queue must namespace identical local owners by root Document"
                    );
                    assert_eq!(
                        current_owner.root_document(),
                        page_vm.document_lifecycle.identity().document
                    );
                    current_producer
                        .send(RendererDedicatedWorkerClientEvent::ScriptLoadFailed {
                            script_url: "https://example.test/current-worker.js".to_owned(),
                            error_message: "current worker load failed".to_owned(),
                            network_response: None,
                        })
                        .expect("replacement Worker event should enter the same stable Page source");

                    page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                        r#"
globalThis.__dedicatedWorkerReplacementCheckpoint = [];
Promise.resolve().then(() => {
  __dedicatedWorkerReplacementCheckpoint.push("microtask");
});
"queued"
"#,
                    )?;
                    let stale_claim = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                        )
                        .expect("retired Worker event should consume one stale discard turn");
                    let (stale_owner, stale_kind) = stale_claim
                        .dedicated_worker_owner_and_event_kind()
                        .expect("exact selector must retain the stale Worker owner and kind");
                    assert_eq!(stale_owner, retired_owner);
                    assert_eq!(
                        stale_kind,
                        RendererDedicatedWorkerClientEventKind::ScriptLoadFailed
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale_claim, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm()
                            .current_dedicated_worker_client_event_identity(current_worker_id),
                        Some(current_owner.execution_context()),
                        "discarding the old event must not retire the replacement Worker"
                    );
                    assert_eq!(
                        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                            "__dedicatedWorkerReplacementCheckpoint.join('|')"
                        )?,
                        "",
                        "a stale Worker task must not checkpoint the replacement realm"
                    );

                    let current_claim = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                        )
                        .expect("replacement Worker event should consume the following turn");
                    let (claimed_current_owner, current_kind) = current_claim
                        .dedicated_worker_owner_and_event_kind()
                        .expect("exact selector must retain the current Worker owner and kind");
                    assert_eq!(claimed_current_owner, current_owner);
                    assert_eq!(
                        current_kind,
                        RendererDedicatedWorkerClientEventKind::ScriptLoadFailed
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(current_claim, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm()
                            .current_dedicated_worker_client_event_identity(current_worker_id),
                        None,
                        "the current load-failure event should retire exactly the replacement Worker"
                    );
                    assert_eq!(
                        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                            "__dedicatedWorkerReplacementCheckpoint.join('|')"
                        )?,
                        "microtask",
                        "the current Worker callback must complete through the selected-task checkpoint"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("DedicatedWorker replacement should run through the typed task executor");
            server
                .await
                .expect("DedicatedWorker PageVm replacement server should finish");
        },
    );
}
