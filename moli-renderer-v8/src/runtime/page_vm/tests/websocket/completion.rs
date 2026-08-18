use super::*;

/// Claim a raw WebSocket task only for the body/completion boundary witness.
///
/// The caller must execute only `apply_selected_page_websocket_turn()` and
/// must not submit task completion. Every complete WebSocket workflow uses the
/// opaque shared selected-task claim below.
fn take_ready_websocket_body_task_for_test(
    page_vm: &mut PageVm,
) -> Option<crate::page_task_queue::RendererPageWebSocketTask> {
    let sources = page_vm.page_task_executor_sources_for_test();
    let task = sources.take_scheduler_task_for_executor_test(|descriptor| {
        matches!(
            descriptor,
            crate::page_task_queue::RendererPageReadyDescriptor::WebSocket { .. }
        ) && page_vm.page_ready_descriptor_is_eligible(descriptor)
    })?;
    let crate::page_task_queue::RendererPageSchedulerTask::WebSocket(task) = task else {
        panic!("WebSocket descriptor dequeued a different scheduler task")
    };
    Some(task)
}

fn claim_ready_websocket_selected_task(
    page_vm: &mut PageVm,
) -> Option<crate::runtime::page_vm::ClaimedPageSelectedTaskForTest> {
    page_vm.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebSocket)
}

async fn wait_for_websocket_candidate<T>(
    page_vm: &mut PageVm,
    mut take: impl FnMut(&mut PageVm) -> Option<T>,
    context: &str,
) -> anyhow::Result<T> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(task) = take(page_vm) {
            return Ok(task);
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
        anyhow::ensure!(
            Instant::now() < deadline,
            "expected one ready WebSocket event for {context}"
        );
    }
}

async fn drive_websocket_until_open(
    page_vm: &mut PageVm,
    socket_expression: &str,
) -> anyhow::Result<u64> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut socket_id = None;
    loop {
        if page_vm
            .vm_mut()
            .eval(&format!("String({socket_expression}.readyState)"))?
            == "1"
        {
            return socket_id.ok_or_else(|| {
                anyhow::anyhow!("WebSocket reached open without a selected event identity")
            });
        }
        if let Some(claimed) = claim_ready_websocket_selected_task(page_vm) {
            socket_id = Some(
                claimed
                    .websocket_owner()
                    .expect("exact WebSocket claim must expose its owner")
                    .socket_id(),
            );
            let loader = page_vm.request_client.clone();
            page_vm
                .run_claimed_selected_page_task_for_test(claimed, &loader)
                .await?;
            continue;
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
        anyhow::ensure!(
            Instant::now() < deadline,
            "WebSocket should reach the open state"
        );
    }
}

async fn enqueue_triggered_websocket_message(
    page_vm: &mut PageVm,
    url: &str,
    opened_rx: tokio::sync::oneshot::Receiver<()>,
    message_tx: tokio::sync::oneshot::Sender<String>,
) -> anyhow::Result<()> {
    let url_literal = serde_json::to_string(url).expect("serialize WebSocket URL");
    page_vm.vm_mut().eval(&format!(
        r#"
globalThis.__webSocketTaskBoundary = [];
globalThis.__webSocketTaskBoundarySocket = new WebSocket({url_literal});
__webSocketTaskBoundarySocket.onmessage = event => {{
  __webSocketTaskBoundary.push("callback:" + event.data);
  Promise.resolve().then(() => {{
    __webSocketTaskBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__webSocketTaskBoundary.push('runtime-script')";
    document.body.appendChild(script);
  }});
}};
"queued"
"#
    ))?;
    tokio::time::timeout(Duration::from_secs(2), opened_rx)
        .await
        .map_err(|_| anyhow::anyhow!("triggered WebSocket server should accept"))?
        .map_err(|_| anyhow::anyhow!("triggered WebSocket server dropped its open signal"))?;
    let _socket_id = drive_websocket_until_open(page_vm, "__webSocketTaskBoundarySocket").await?;
    message_tx
        .send("payload".to_owned())
        .map_err(|_| anyhow::anyhow!("triggered WebSocket server dropped its message receiver"))?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn websocket_message_body_leaves_reactions_for_selected_completion() {
    run_page_vm_async_test(async move {
        let (url, opened_rx, message_tx, server) =
            spawn_triggered_text_websocket_server().await;
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                enqueue_triggered_websocket_message(
                    &mut page_vm,
                    &url,
                    opened_rx,
                    message_tx,
                )
                .await?;
                let task = wait_for_websocket_candidate(
                    &mut page_vm,
                    take_ready_websocket_body_task_for_test,
                    "message body boundary",
                )
                .await?;
                anyhow::ensure!(
                    matches!(
                        task.event(),
                        moli_websocket::Event::TextMessage { data, .. }
                            if data == "payload"
                    ),
                    "expected the triggered WebSocket text message, got {:?}",
                    task.event()
                );
                let outcome = page_vm.apply_selected_page_websocket_turn(task)?;
                assert_eq!(
                    outcome.action.target_effect,
                    crate::page_task_queue::PageWebSocketTargetEffect::CallbackVisibleWorkAppliedToCurrentDocument
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__webSocketTaskBoundary.join('|')")?,
                    "callback:payload",
                    "the WebSocket body must leave Promise reactions for selected-task completion"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("WebSocket body boundary witness should run");

        server.await.expect("triggered WebSocket server");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn websocket_internal_state_selected_task_does_not_advance_unrelated_runtime_work() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_delayed_passive_close_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize WebSocket URL");
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
globalThis.__webSocketInternalStateSocket = new WebSocket({url_literal});
"queued"
"#
                ))?;
                let socket_id =
                    drive_websocket_until_open(&mut page_vm, "__webSocketInternalStateSocket")
                        .await?;
                page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
                assert!(
                    page_vm
                        .vm()
                        .websocket_sender_for_test()
                        .event_sender()
                        .send(moli_websocket::Event::BufferedAmountConsumed {
                            socket_id,
                            amount: 0,
                        })
                        .await,
                    "internal WebSocket transition should enter the typed source"
                );
                let claimed = wait_for_websocket_candidate(
                    &mut page_vm,
                    claim_ready_websocket_selected_task,
                    "internal-state selected task",
                )
                .await?;
                let loader = page_vm.request_client.clone();
                page_vm
                    .run_claimed_selected_page_task_for_test(claimed, &loader)
                    .await?;
                assert_eq!(
                    page_vm
                        .vm()
                        .document_runtime
                        .runtime_script_work()
                        .dynamic_scripts
                        .pending_source_load_count_for_test(),
                    1,
                    "checkpoint-only WebSocket state must not advance unrelated runtime work"
                );
                page_vm
                    .vm_mut()
                    .eval("__webSocketInternalStateSocket.close(); 'closing'")?;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("WebSocket internal-state completion witness should run");

        server.await.expect("delayed-close WebSocket server");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn websocket_missing_current_target_has_no_completion_authority() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        assert!(
            page_vm
                .vm()
                .websocket_sender_for_test()
                .event_sender()
                .send(moli_websocket::Event::TextMessage {
                    socket_id: u64::MAX - 1,
                    data: "missing-target".to_owned(),
                })
                .await,
            "missing-target WebSocket event should enter the typed source"
        );
        let claimed = wait_for_websocket_candidate(
            &mut page_vm,
            claim_ready_websocket_selected_task,
            "missing-target selected task",
        )
        .await
        .expect("missing-target task should become ready");
        let loader = page_vm.request_client.clone();
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await
            .expect("missing-target task should settle through the selected dispatcher");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "missing target must not enter V8 or advance unrelated runtime residence"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn websocket_stale_root_task_cannot_complete_in_replacement_page_vm() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        let current_document = page_vm.document_lifecycle.identity().document;
        let stale_document = current_document.successor_for_testing();
        let senders = page_vm
            .runtime_hooks
            .standalone_page_task_residence()
            .expect("standalone PageVm should retain its production task routes")
            .runtime_source()
            .bound_task_producer_senders(stale_document)
            .expect("test Page routes should bind an exact stale producer");
        let (js_context_senders, _, _, _, _, _) = senders.into_parts();
        assert!(
            js_context_senders
                .websocket()
                .event_sender()
                .send(moli_websocket::Event::TextMessage {
                    socket_id: u64::MAX - 2,
                    data: "stale-root".to_owned(),
                })
                .await,
            "stale-root WebSocket event should enter the stable Page source"
        );

        let claimed = wait_for_websocket_candidate(
            &mut page_vm,
            claim_ready_websocket_selected_task,
            "stale-root selected task",
        )
        .await
        .expect("stale-root task should remain selectable for retirement");
        assert_eq!(
            claimed
                .websocket_owner()
                .expect("stale WebSocket claim must expose its owner")
                .root_document(),
            stale_document
        );
        let loader = page_vm.request_client.clone();
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await
            .expect("stale-root task should retire through the selected dispatcher");
        assert_eq!(
            page_vm.document_lifecycle.identity().document,
            current_document,
            "retiring a stale WebSocket task must not replace the current Document"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "retired root work must not checkpoint or advance replacement runtime residence"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn websocket_selected_dispatcher_owns_checkpoint_and_runtime_follow_up() {
    run_page_vm_async_test(async move {
        let (url, opened_rx, message_tx, server) = spawn_triggered_text_websocket_server().await;
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                enqueue_triggered_websocket_message(&mut page_vm, &url, opened_rx, message_tx)
                    .await?;
                let claimed = wait_for_websocket_candidate(
                    &mut page_vm,
                    claim_ready_websocket_selected_task,
                    "callback selected task",
                )
                .await?;
                page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
                assert_eq!(
                    page_vm
                        .vm()
                        .document_runtime
                        .runtime_script_work()
                        .dynamic_scripts
                        .pending_source_load_count_for_test(),
                    1,
                    "the setup must leave one unrelated runtime residence pending"
                );
                let loader = page_vm.request_client.clone();
                page_vm
                    .run_claimed_selected_page_task_for_test(claimed, &loader)
                    .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("__webSocketTaskBoundary.join('|')")?,
                    "callback:payload|microtask|runtime-script",
                    "the selected WebSocket dispatcher must own checkpoint and runtime follow-up"
                );
                assert!(
                    page_vm
                        .vm()
                        .document_runtime
                        .runtime_script_work()
                        .dynamic_scripts
                        .pending_source_load_count_for_test()
                        == 1,
                    "selected callback completion must not consume an unrelated pending source"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("WebSocket selected-dispatcher witness should run");

        server.await.expect("triggered WebSocket server");
    })
    .await;
}
