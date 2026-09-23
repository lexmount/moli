use super::*;

#[tokio::test]
async fn worker_post_message_flows_through_page_client_event_source() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerMessage = null;
                        globalThis.__workerMessageShape = null;
                        globalThis.__workerDone = false;
                        const worker = new Worker("data:text/javascript,postMessage('from-worker')");
                        worker.onmessage = (event) => {
                            globalThis.__workerMessage = event.data;
                            globalThis.__workerMessageShape = [
                                event instanceof MessageEvent,
                                Object.prototype.toString.call(event),
                                event.type
                            ].join("|");
                            globalThis.__workerDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker runtime event should arrive",
                )
                .await?;
                assert_eq!(page_vm.vm_mut().eval("globalThis.__workerMessage")?, "from-worker");
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__workerMessageShape")?,
                    "true|[object MessageEvent]|message"
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker event test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn dedicated_worker_main_scripts_publish_split_target_lifecycle_records() {
    run_page_vm_async_test(async move {
        let external_source = "postMessage('external-ready');";
        let blob_source = "postMessage('blob-ready');";
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/worker.js",
            "HTTP/1.1 200 OK",
            external_source.to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let external_url = Url::parse(&format!("{base_url}/worker.js")).expect("worker URL");
        let mut page_vm = test_page_vm_with_document_url(document_url.clone());
        let output_journal = crate::runtime::RendererTurnOutputJournal::new(
            crate::runtime::RendererOutputStreamIdentity::new_page_for_protocol_test(
                page_vm.page_id,
            ),
        );
        page_vm
            .vm_mut()
            .bind_renderer_output_journal_for_test(output_journal.clone());
        let local_executor = page_vm.local_executor.clone();

        let target_events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__workerMainScriptMessages = [];
  globalThis.__workerMainScriptDone = false;
  const onmessage = event => {{
    __workerMainScriptMessages.push(String(event.data));
    __workerMainScriptDone = __workerMainScriptMessages.length === 2;
  }};
  globalThis.__externalMainScriptWorker = new Worker("/worker.js#runtime-fragment");
  __externalMainScriptWorker.onmessage = onmessage;
  const blobUrl = URL.createObjectURL(new Blob(
    [{blob_source:?}],
    {{ type: "text/javascript" }}
  ));
  globalThis.__blobMainScriptWorker = new Worker(blobUrl);
  __blobMainScriptWorker.onmessage = onmessage;
}})()
"#,
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerMainScriptDone === true)",
                    "external and blob worker scripts should complete",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__workerMainScriptMessages.sort())")?,
                    r#"["blob-ready","external-ready"]"#
                );
                assert!(
                    page_vm.vm_mut().take_network_output().is_empty(),
                    "DedicatedWorker main scripts are not complete Page subresources"
                );
                let publication = output_journal
                    .settle()
                    .expect("DedicatedWorker target events should settle as Page output");
                anyhow::Ok(
                    publication
                        .into_records()
                        .into_iter()
                        .filter_map(|record| match record.into_parts().1 {
                            RendererOutputItem::OwnerAction(
                                RendererOwnerAction::DedicatedWorkerTargetLifecycle(event),
                            ) => Some(event),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .await
            .expect("worker main-script Network test should run on owner lane");

        server
            .await
            .expect("worker main-script Network server should finish");
        let created = target_events
            .iter()
            .filter_map(|event| match event {
                crate::runtime::RendererDedicatedWorkerTargetEvent::Created(info) => Some(info),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(created.len(), 2, "events: {target_events:#?}");
        assert!(
            created
                .iter()
                .all(|info| info.document_url == document_url.as_str())
        );
        let external_created = created
            .iter()
            .copied()
            .find(|info| info.request_url == external_url.as_str())
            .expect("external Worker target creation");
        let blob_created = created
            .iter()
            .copied()
            .find(|info| info.request_url.starts_with("blob:"))
            .expect("blob Worker target creation");

        let loaded = target_events
            .iter()
            .filter_map(|event| match event {
                crate::runtime::RendererDedicatedWorkerTargetEvent::ScriptLoaded {
                    instance_id,
                    script_url,
                    response,
                } => Some((*instance_id, script_url, response.as_ref())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(loaded.len(), 2, "events: {target_events:#?}");
        let (_, external_script_url, external_response) = loaded
            .iter()
            .copied()
            .find(|(instance_id, _, _)| *instance_id == external_created.instance_id)
            .expect("external Worker main-script completion");
        assert_eq!(
            external_script_url,
            &format!("{external_url}#runtime-fragment")
        );
        assert_eq!(external_response.status, 200);
        assert_eq!(external_response.body_text(), external_source);
        assert!(external_response.network_request_headers().is_some());
        assert_eq!(
            external_response.negotiated_http_version,
            Some(moli_fetch::NegotiatedHttpVersion::Http11)
        );

        let (_, blob_script_url, blob_response) = loaded
            .iter()
            .copied()
            .find(|(instance_id, _, _)| *instance_id == blob_created.instance_id)
            .expect("blob Worker main-script completion");
        assert_eq!(blob_script_url, &blob_created.request_url);
        assert_eq!(blob_response.status, 200);
        assert_eq!(blob_response.body_text(), blob_source);
        assert_eq!(blob_response.network_request_headers(), None);
        assert_eq!(blob_response.negotiated_http_version, None);
    })
    .await;
}

#[tokio::test]
async fn worker_message_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://worker-ready-source.test/page.html").expect("document URL");
        let (page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            events_after_worker_message,
            script_ready_source,
            events_after_script_ready,
            lifecycle_and_host_load_sources,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__workerReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  globalThis.__workerReadyWorker = new Worker(
    "data:text/javascript,postMessage('go')"
  );
  __workerReadyWorker.onmessage = (event) => {
    __workerReadyEvents.push("message:" + event.data);
    const frame = document.createElement("iframe");
    frame.onload = () => __workerReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__workerReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  };
})()
"#,
                )?;

                let mut completion_sources = Vec::new();
                let events_after_worker_message = loop {
                    if let Some(claimed) = page_vm.claim_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                    ) {
                        let event_kind = claimed
                            .dedicated_worker_owner_and_event_kind()
                            .map(|(_, event_kind)| event_kind)
                            .expect("DedicatedWorker selector must retain its event kind");
                        page_vm
                            .run_claimed_selected_page_task_for_test(claimed, &loader)
                            .await?;
                        completion_sources.push(RendererOwnerResourceActivitySource::Worker);
                        let events = page_vm.vm_mut().eval("__workerReadyEvents.join('|')")?;
                        if events == "message:go" {
                            assert_eq!(
                                event_kind,
                                crate::page_task_queue::RendererDedicatedWorkerClientEventKind::Message
                            );
                            break events;
                        }
                        assert!(
                            completion_sources.len() < 16,
                            "worker message handler should run after bounded completions; sources: {completion_sources:?}, events: {events}"
                        );
                        continue;
                    }
                    if page_vm.has_ready_page_websocket_task_for_test() {
                        let completion_source = page_vm
                            .run_exact_page_websocket_selected_task_for_test().await?
                            .expect("advertised WebSocket task should remain ready");
                        completion_sources.push(completion_source);
                        continue;
                    }
                    {
                        let wake = tokio::time::timeout(
                            Duration::from_secs(2),
                            owner_wake_rx.recv(),
                        )
                        .await
                        .expect("worker completion should signal its owner before timeout")
                        .expect("worker completion owner-wake route should remain open");
                        assert_eq!(
                            wake.page_id(),
                            PageId::new_for_testing(1),
                            "worker task wake must remain attached to the originating Page"
                        );
                    }
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "worker-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "worker-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready =
                    page_vm.vm_mut().eval("__workerReadyEvents.join('|')")?;
                let mut lifecycle_and_host_load_sources = Vec::new();
                let events_after_host_load = loop {
                    let source = page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await
                        .expect("child lifecycle or HostLoad source should remain ready");
                    lifecycle_and_host_load_sources.push(source);
                    let events = page_vm.vm_mut().eval("__workerReadyEvents.join('|')")?;
                    if events == "message:go|child-script:true|frame-load" {
                        break events;
                    }
                    assert_eq!(
                        source,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "only document-owned lifecycle turns may precede the final HostLoad delivery"
                    );
                    assert!(
                        lifecycle_and_host_load_sources.len() < 8,
                        "worker-created child lifecycle should reach HostLoad in bounded owner turns: {lifecycle_and_host_load_sources:?}"
                    );
                };

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_worker_message,
                    script_ready_source,
                    events_after_script_ready,
                    lifecycle_and_host_load_sources,
                    events_after_host_load,
                ))
            })
            .await
            .expect("worker ready-work source test should run");

        assert!(
            completion_sources.contains(&RendererOwnerResourceActivitySource::Worker),
            "worker handler should be driven by a Worker completion: {completion_sources:?}"
        );
        assert_eq!(
            events_after_worker_message, "message:go",
            "worker message handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "worker-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "message:go|child-script:true",
            "child parser work should run on the later DocumentScriptReady turn"
        );
        assert!(
            lifecycle_and_host_load_sources.len() >= 2,
            "document-owned lifecycle must complete before HostLoad: {lifecycle_and_host_load_sources:?}"
        );
        assert!(
            lifecycle_and_host_load_sources[..lifecycle_and_host_load_sources.len() - 1]
                .iter()
                .all(|source| *source == ChildFrameSemanticTurnKind::DocumentLifecycle),
            "only DocumentLifecycle turns may run between script execution and load delivery: {lifecycle_and_host_load_sources:?}"
        );
        assert_eq!(
            lifecycle_and_host_load_sources.last(),
            Some(&ChildFrameSemanticTurnKind::HostLoad),
            "iframe load must remain a later HostLoad turn after document lifecycle"
        );
        assert_eq!(
            events_after_host_load, "message:go|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );
    })
    .await;
}

#[tokio::test]
async fn shared_worker_error_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/missing-shared-worker.js",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            events_after_shared_worker_error,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__sharedWorkerReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const worker = new SharedWorker("/missing-shared-worker.js", "ready-output-error");
  worker.onerror = (event) => {
    __sharedWorkerReadyEvents.push("error:" + event.type);
    const frame = document.createElement("iframe");
    frame.onload = () => __sharedWorkerReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__sharedWorkerReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  };
  worker.port.start();
})()
"#,
                )?;

                let deadline = Instant::now() + Duration::from_secs(10);
                let events_after_shared_worker_error = loop {
                    // This direct PageVm fixture has no render-owner loop.
                    // Admit the SharedWorker service result explicitly before
                    // selecting its Page task; timer/WebSocket helpers must
                    // not provide this unrelated owner responsibility.
                    page_vm
                        .runtime_hooks
                        .browser_context_runtime
                        .drain_shared_worker_service_lane();
                    let loader = page_vm.main_document_resource_loader();
                    let shared_worker_event_ran = page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::SharedWorkerClientEvent,
                            loader.request_client(),
                        )
                        .await?;
                    if !shared_worker_event_ran
                        && page_vm.has_ready_page_websocket_task_for_test()
                    {
                        let _ = page_vm.run_exact_page_websocket_selected_task_for_test().await?;
                    } else if !shared_worker_event_ran {
                        page_vm
                            .advance_timers_until_deadline_for_test(loader.request_client())
                            .await?;
                        let _ = tokio::time::timeout(
                            Duration::from_millis(100),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    let events = page_vm.vm_mut().eval("__sharedWorkerReadyEvents.join('|')")?;
                    if events == "error:error" {
                        break events;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "SharedWorker error handler should run after bounded owner turns; events: {events}"
                    );
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "SharedWorker-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "SharedWorker-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__sharedWorkerReadyEvents.join('|')")?;
                let host_load_source = Some(
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "SharedWorker-created child iframe load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__sharedWorkerReadyEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    events_after_shared_worker_error,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    events_after_host_load,
                ))
            })
            .await
            .expect("SharedWorker ready-work source test should run");

        assert_eq!(
            events_after_shared_worker_error, "error:error",
            "SharedWorker error handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "SharedWorker-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "error:error|child-script:true",
            "child parser work should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a separate HostLoad turn after SharedWorker error dispatch"
        );
        assert_eq!(
            events_after_host_load, "error:error|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server
            .await
            .expect("SharedWorker ready-work server should finish");
    })
    .await;
}

#[tokio::test]
async fn blob_worker_created_in_data_iframe_inherits_opaque_broadcast_channel_owner_key() {
    run_page_vm_async_test(async move {
        let document_url =
            Url::parse("https://broadcast-channel-opaque-worker.test/page.html")
                .expect("document url");
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__opaqueWorkerBroadcastChannelMessages = [];
                        globalThis.__opaqueWorkerBroadcastChannelDone = false;
                        const topChannel = new BroadcastChannel("opaque-child-worker-owner");
                        topChannel.onmessage = event => {
                            __opaqueWorkerBroadcastChannelMessages.push("top:" + event.data + ":" + event.origin);
                        };
                        addEventListener("message", event => {
                            const value = String(event.data);
                            __opaqueWorkerBroadcastChannelMessages.push(value);
                            if (value === "child-worker:null:null") {
                                __opaqueWorkerBroadcastChannelDone = true;
                            }
                        });

                        const frame = document.createElement("iframe");
                        frame.src = "data:text/html," + encodeURIComponent(`
                            <!doctype html>
                            <script>
                                const channel = new BroadcastChannel("opaque-child-worker-owner");
                                channel.onmessage = event => {
                                    if (event.data === "ping") {
                                        channel.postMessage("pong");
                                    } else {
                                        parent.postMessage("child-worker:" + event.data + ":" + event.origin, "*");
                                    }
                                };
                                const workerSource = \`
                                    const workerChannel = new BroadcastChannel("opaque-child-worker-owner");
                                    workerChannel.postMessage("ping");
                                    workerChannel.onmessage = event => workerChannel.postMessage(event.origin);
                                \`;
                                const workerUrl = URL.createObjectURL(
                                    new Blob([workerSource], { type: "text/javascript" })
                                );
                                const worker = new Worker(workerUrl);
                                worker.onerror = event => parent.postMessage("worker-error:" + event.message, "*");
                            <\/script>
                        `);
                        (document.body || document.documentElement || document).appendChild(frame);
                    })()
                    "#,
                )?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__opaqueWorkerBroadcastChannelDone === true)",
                    "opaque child blob worker BroadcastChannel delivery should complete",
                )
                .await?;

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__opaqueWorkerBroadcastChannelMessages)")?,
                    r#"["child-worker:null:null"]"#
                );
                anyhow::Ok(())
            })
            .await
            .expect("opaque child blob worker BroadcastChannel test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn child_message_handler_external_dedicated_worker_binds_child_client_event_owner() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/worker.js",
            "HTTP/1.1 200 OK",
            "postMessage('worker-loaded');".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__childMessageWorkerCreated = false;
  globalThis.__childMessageWorkerLoaded = false;
  window.addEventListener("message", event => {
    if (event.data && event.data.kind === "child-message-worker-created") {
      __childMessageWorkerCreated = true;
    }
    if (event.data && event.data.kind === "child-message-worker-loaded") {
      __childMessageWorkerLoaded = event.data.value === "worker-loaded";
    }
  });

  const frame = document.createElement("iframe");
  frame.id = "child-message-worker-owner";
  frame.srcdoc = `
    <!doctype html>
    <script>
      onmessage = event => {
        if (event.data !== "start-worker") return;
        const worker = new Worker("/worker.js");
        worker.onmessage = event => {
          parent.postMessage({
            kind: "child-message-worker-loaded",
            value: event.data
          }, "*");
        };
        parent.postMessage({ kind: "child-message-worker-created" }, "*");
      };
    <\/script>
  `;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
                )?;

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "message-target child navigation",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "message-target child script ready",
                )
                .await;
                let child_handle = page_vm
                    .vm()
                    .element_handle_by_id_for_test("child-message-worker-owner")
                    .expect("child worker owner iframe handle");
                let child_owner = page_vm
                    .vm()
                    .current_child_document_task_owner(child_handle)
                    .expect("child worker owner document");

                page_vm.vm_mut().eval(
                    r#"
document
  .getElementById("child-message-worker-owner")
  .contentWindow
  .postMessage("start-worker", "*");
"posted"
"#,
                )?;
                drive_window_message_until(
                    &mut page_vm,
                    "String(globalThis.__childMessageWorkerLoaded === true)",
                    "child message handler should create and load its Worker",
                )
                .await?;

                let workers = page_vm.vm().dedicated_worker_execution_contexts_for_test();
                assert_eq!(workers.len(), 1);
                let worker_id = workers[0].0;
                assert_eq!(
                    workers[0].1,
                    crate::native_bridge::WindowExecutionContextOwner::Frame(
                        child_owner.local_window_id
                    ),
                    "the child message-created Worker must retain the child LocalWindow owner"
                );
                let identity = page_vm
                    .vm()
                    .current_dedicated_worker_client_event_identity(worker_id)
                    .expect("child Worker client-event identity should remain current");
                assert_eq!(
                    identity.owner(),
                    crate::native_bridge::WindowExecutionContextOwner::Frame(
                        child_owner.local_window_id
                    ),
                    "the Worker client-event producer must not bind the top Window identity"
                );
                assert_eq!(
                    identity.dispatch_scope(),
                    crate::native_bridge::OwnerDispatchScope::Child(child_handle)
                );
                anyhow::Ok(())
            })
            .await
            .expect("child message-created Worker owner test should run on owner lane");

        server
            .await
            .expect("child message-created Worker server should finish");
    })
    .await;
}

#[tokio::test]
async fn child_worker_message_without_listener_drops_and_restores_top_owner_scope() {
    run_page_vm_async_test(async move {
        let document_url =
            Url::parse("https://worker-owner-restore.test/page.html").expect("document url");
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__topBroadcastChannelMessages = [];
                        globalThis.__topBroadcastChannelDone = false;
                        globalThis.__childWorkerMessages = [];
                        globalThis.__childWorkerDone = false;
                        window.addEventListener("message", event => {
                            if (event.data && event.data.kind === "child-worker-result") {
                                globalThis.__childWorkerMessages = event.data.messages;
                                globalThis.__childWorkerDone = true;
                            }
                        });
                        const receiver = new BroadcastChannel("owner-restore-after-worker");
                        receiver.onmessage = event => {
                            __topBroadcastChannelMessages.push(event.data + ":" + event.origin);
                            __topBroadcastChannelDone = true;
                        };

                        const frame = document.createElement("iframe");
                        frame.src = "data:text/html," + encodeURIComponent(`
                            <!doctype html>
                            <script>
                                const worker = new Worker("data:text/javascript,onmessage = () => postMessage('after-listener'); postMessage('before-listener')");
                                const messages = [];
                                addEventListener("message", event => {
                                    if (event.data !== "install-late-listener") return;
                                    worker.onmessage = event => {
                                        messages.push(event.data);
                                        parent.postMessage({
                                            kind: "child-worker-result",
                                            messages
                                        }, "*");
                                    };
                                    worker.postMessage("go");
                                });
                            <\/script>
                        `);
                        (document.body || document.documentElement || document).appendChild(frame);
                    })()
                    "#,
                )?;

                drive_until_worker_completion_observed(
                    &mut page_vm,
                    "child worker no-listener owner restore setup",
                )
                .await?;

                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        document.querySelector("iframe").contentWindow.postMessage("install-late-listener", "*");
                        const sender = new BroadcastChannel("owner-restore-after-worker");
                        sender.postMessage("top-still-active");
                    })()
                    "#,
                )?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__topBroadcastChannelDone === true)",
                    "top BroadcastChannel should still use top owner after child worker no-listener completion",
                )
                .await?;

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__topBroadcastChannelMessages)")?,
                    r#"["top-still-active:https://worker-owner-restore.test"]"#
                );
                drive_window_message_until(
                    &mut page_vm,
                    "String(globalThis.__childWorkerDone === true)",
                    "child worker late listener result delivery",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__childWorkerMessages)")?,
                    r#"["after-listener"]"#
                );
                anyhow::Ok(())
            })
            .await
            .expect("child worker no-listener owner restore test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn message_port_handler_worker_uses_lightweight_popup_owner_scope() {
    run_page_vm_async_test(async move {
        // This callback must originate in the popup, independently of its target.
        let popup_source = r#"
onmessage = event => {
  if (event.data !== "setup") {
      return;
  }
  const popupChannel = new BroadcastChannel("message-port-popup-worker-owner");
  popupChannel.onmessage = channelEvent => {
      event.source.postMessage("popup-worker:" + channelEvent.data, event.origin);
  };
  const channel = new MessageChannel();
  channel.port2.onmessage = () => {
      const workerSource = `
          const channel = new BroadcastChannel("message-port-popup-worker-owner");
          channel.postMessage("worker-origin");
          channel.onmessage = event => channel.postMessage(event.origin);
      `;
      const workerUrl = URL.createObjectURL(
          new Blob([workerSource], { type: "text/javascript" })
      );
      const worker = new Worker(workerUrl);
      worker.onerror = error => event.source.postMessage("worker-error:" + error.message, event.origin);
  };
  channel.port1.postMessage("start");
};
opener.postMessage("fixture-ready", "*");
"#;
        let (popup_origin, server) = spawn_path_response_http_server(vec![(
            "/popup.html", "HTTP/1.1 200 OK",
            format!("<!doctype html><script>{popup_source}</script>"), Duration::ZERO,
        )]).await;
        let popup_url = format!("{popup_origin}/popup.html");
        let document_url = Url::parse("https://message-port-worker-popup-owner.test/page.html")
            .expect("document url");
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    &r#"
                    (() => {
                        let resolvePopupReady;
                        const popupReady = new Promise(resolve => { resolvePopupReady = resolve; });
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        const topChannel = new BroadcastChannel("message-port-popup-worker-owner");
                        topChannel.onmessage = event => {
                            __wsEvents.push("top-bc:" + event.data + ":" + event.origin);
                        };
                        addEventListener("message", event => {
                            if (event.data === "fixture-ready") { resolvePopupReady(); return; }
                            const value = String(event.data);
                            __wsEvents.push(value + ":" + event.origin);
                            if (value === "popup-worker:worker-origin") {
                                __wsDone = true;
                            }
                        });

                        const popup = open("__POPUP_CALLBACK_URL__");

                        popupReady.then(() => popup.postMessage("setup", "*"));
                    })()
                    "#.replace("__POPUP_CALLBACK_URL__", &popup_url),
                )?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "popup MessagePort handler Worker should use popup owner scope",
                )
                .await?;

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__wsEvents)")?,
                    format!(r#"["popup-worker:worker-origin:{popup_origin}"]"#)
                );
                server.await.expect("popup fixture should finish");
                anyhow::Ok(())
            })
            .await
            .expect("popup MessagePort Worker owner test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn worker_pending_activity_diagnostics_split_loading_and_running_worker_isolates() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/ready-worker.js",
            "HTTP/1.1 200 OK",
            "postMessage('ready');".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerDiagnosticDone = false;
                        const worker = new Worker("/ready-worker.js");
                        worker.onmessage = () => {
                            globalThis.__workerDiagnosticDone = true;
                        };
                    })()
                    "#,
                )?;

                let loading_snapshot = page_vm.page_diagnostics_snapshot()?;
                assert_eq!(
                    loading_snapshot.diagnostics.dedicated_worker_loading_count,
                    1
                );
                assert_eq!(
                    loading_snapshot
                        .diagnostics
                        .dedicated_worker_running_worker_isolate_count,
                    0
                );

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDiagnosticDone === true)",
                    "worker diagnostics probe should reach the running worker",
                )
                .await?;

                let running_snapshot = page_vm.page_diagnostics_snapshot()?;
                assert_eq!(
                    running_snapshot.diagnostics.dedicated_worker_loading_count,
                    0
                );
                assert_eq!(
                    running_snapshot
                        .diagnostics
                        .dedicated_worker_running_worker_isolate_count,
                    1
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker diagnostics test should run on owner lane");
        server
            .await
            .expect("worker diagnostics server should finish");
    })
    .await;
}

#[tokio::test]
async fn external_dedicated_module_worker_retains_creator_csp_for_static_imports() {
    run_page_vm_async_test(async move {
        let (dependency_base_url, mut dependency_request, dependency_server) =
            spawn_shared_worker_script_capture_http_server("export const value = 'unexpected';")
                .await;
        let dependency_url = format!("{dependency_base_url}/dependency.js");
        let worker_source = format!(
            r#"
            import {dependency_url:?};
            postMessage("unexpected");
        "#
        );
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/worker.js",
            "HTTP/1.1 200 OK",
            worker_source,
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url.clone());
        page_vm.vm_mut().set_main_navigation_policy_container(
            crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                &[(
                    "Content-Security-Policy".to_owned(),
                    b"worker-src 'self'; script-src data:".to_vec(),
                )],
                &document_url,
            ),
        );
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__moduleWorkerCspResult = null;
                        globalThis.__moduleWorkerCspDone = false;
                        const worker = new Worker("/worker.js", { type: "module" });
                        worker.onmessage = event => {
                            globalThis.__moduleWorkerCspResult = "message:" + event.data;
                            globalThis.__moduleWorkerCspDone = true;
                        };
                        worker.onerror = event => {
                            event.preventDefault();
                            globalThis.__moduleWorkerCspResult = JSON.stringify([
                                event.type,
                                Object.getPrototypeOf(event) === Event.prototype,
                                event.bubbles, event.cancelable, event.composed, event.isTrusted,
                                event.defaultPrevented,
                                ['message', 'filename', 'lineno', 'colno', 'error'].some(name => name in event)
                            ]);
                            globalThis.__moduleWorkerCspDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__moduleWorkerCspDone === true)",
                    "creator CSP should settle the external module worker",
                )
                .await?;
                let result = page_vm
                    .vm_mut()
                    .eval("globalThis.__moduleWorkerCspResult")?;
                assert_eq!(
                    result,
                    r#"["error",true,false,false,false,true,false,false]"#,
                    "blocked static import should fire a bootstrap Event before evaluation"
                );
                anyhow::Ok(())
            })
            .await
            .expect("external module worker creator CSP test should run on owner lane");
        server
            .await
            .expect("external module worker CSP server should finish");
        assert!(
            matches!(dependency_request.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)),
            "creator CSP must block the dependency before any HTTP request is sent"
        );
        dependency_server.abort();
    })
    .await;
}

#[tokio::test]
async fn service_worker_register_starts_module_worker_global() {
    run_page_vm_async_test(async move {
        let (base_url, finished_rx, server) =
            spawn_service_worker_execution_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expected_scope = format!("{base_url}/app/");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__serviceWorkerRegisterSettled = "pending";
                    navigator.serviceWorker.register("/sw.js", {
                        scope: "/app/",
                        type: "module"
                    }).then(
                        async registration => {
                            await navigator.serviceWorker.ready;
                            const worker = registration.active ?? registration.waiting ?? registration.installing;
                            globalThis.__serviceWorkerRegisterSettled =
                                registration.scope + "|" + worker.state;
                        },
                        error => {
                            globalThis.__serviceWorkerRegisterSettled = "rejected:" + error.name;
                        }
                    );
                    "#,
                )?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__serviceWorkerRegisterSettled")?,
                    "pending"
                );
                let body = tokio::time::timeout(Duration::from_secs(5), finished_rx)
                    .await
                    .expect("service worker script should POST /finished")
                    .expect("service worker execution capture sender should stay alive");
                assert_eq!(
                    body,
                    format!(
                        "[object ServiceWorkerGlobalScope]|true|{}|function|function",
                        expected_scope
                    )
                );
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                let loader = page_vm.main_document_resource_loader();
                while tokio::time::Instant::now() < deadline {
                    page_vm
                        .runtime_hooks
                        .browser_context_runtime
                        .drain_service_worker_service_lane();
                    while page_vm
                        .run_one_oldest_ready_page_task_on_owner_lane_for_test(
                            loader.request_client(),
                        )
                        .await?
                    {
                        page_vm
                            .runtime_hooks
                            .browser_context_runtime
                            .drain_service_worker_service_lane();
                    }
                    if page_vm.vm_mut().eval("globalThis.__serviceWorkerRegisterSettled")?
                        == format!("{expected_scope}|activated")
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert!(
                    page_vm.vm_mut().eval("globalThis.__serviceWorkerRegisterSettled")?
                        == format!("{expected_scope}|activated"),
                    "service worker should activate after register and lifecycle completions"
                );
                let diagnostics = page_vm
                    .runtime_hooks
                    .browser_context_runtime
                    .moli_memory_diagnostics();
                assert_eq!(diagnostics["serviceWorker"]["runtimeRegistrations"], 1);
                assert_eq!(diagnostics["serviceWorker"]["versions"], 1);
                assert_eq!(diagnostics["serviceWorker"]["startingVersions"], 0);
                assert_eq!(diagnostics["serviceWorker"]["runningVersions"], 1);
                assert_eq!(diagnostics["serviceWorker"]["runningWorkers"], 1);
                anyhow::Ok(())
            })
            .await
            .expect("service worker execution test should run on owner lane");
        server
            .await
            .expect("service worker execution capture server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_intercepts_dedicated_worker_main_script() {
    run_page_vm_async_test(async move {
        let (base_url, worker_request_rx, server) =
            spawn_service_worker_worker_main_script_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expected_worker_url = format!("{base_url}/app/worker.js");
        let expected_service_worker_url = format!("{base_url}/app/sw.js");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerWorkerMainProbe = "pending";
                        (async () => {
                            await navigator.serviceWorker.register("sw.js", { scope: "./" });
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {
                                await new Promise(resolve => {
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        { once: true }
                                    );
                                });
                            }
                            const worker = new Worker("worker.js");
                            worker.onmessage = event => {
                                globalThis.__serviceWorkerWorkerMainProbe = event.data;
                            };
                            worker.onerror = event => {
                                globalThis.__serviceWorkerWorkerMainProbe =
                                    "error:" + event.message;
                            };
                        })().catch(error => {
                            globalThis.__serviceWorkerWorkerMainProbe =
                                "error:" + String(error && error.message);
                        });
                    })()
                    "#,
                )?;
                drive_service_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerWorkerMainProbe !== 'pending')",
                    "service worker should intercept dedicated Worker main script",
                )
                .await?;
                let result: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerWorkerMainProbe)")?,
                )
                .expect("dedicated worker controller result should be JSON");
                assert_eq!(
                    result,
                    serde_json::json!({
                        "main": format!("sw-main:{expected_worker_url}"),
                        "serviceWorkerType": "object",
                        "controllerScriptURL": expected_service_worker_url,
                        "controllerState": "activated",
                        "oncontrollerchangeIsNull": true,
                        "addEventListenerType": "function",
                    })
                );
                anyhow::Ok(())
            })
            .await
            .expect("service worker Worker main script test should run on owner lane");

        assert!(
            worker_request_rx.await.is_err(),
            "dedicated Worker main script should be served by the service worker, not network fallback"
        );
        server
            .await
            .expect("service worker Worker main script server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_claim_dispatches_controllerchange_to_dedicated_worker_client() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_service_worker_worker_controllerchange_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let expected_service_worker_url = format!("{base_url}/app/sw.js");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerWorkerControllerChangeProbe = "pending";
                        globalThis.__serviceWorkerWorkerControllerChangeReady = "pending";
                        const worker = new Worker("worker.js");
                        worker.onmessage = async event => {
                            try {
                                const message = JSON.parse(event.data);
                                if (message.ready) {
                                    globalThis.__serviceWorkerWorkerControllerChangeReady =
                                        event.data;
                                    await navigator.serviceWorker.register("sw.js", {
                                        scope: "./"
                                    });
                                    await navigator.serviceWorker.ready;
                                    return;
                                }
                                globalThis.__serviceWorkerWorkerControllerChangeProbe =
                                    event.data;
                            } catch (error) {
                                globalThis.__serviceWorkerWorkerControllerChangeProbe =
                                    "error:" + String(error && error.message);
                            }
                        };
                        worker.onerror = event => {
                            globalThis.__serviceWorkerWorkerControllerChangeProbe =
                                "error:" + event.message;
                        };
                    })()
                    "#,
                )?;
                drive_service_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerWorkerControllerChangeProbe !== 'pending')",
                    "Service Worker claim should dispatch controllerchange to a worker client",
                )
                .await?;
                let ready: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerWorkerControllerChangeReady)")?,
                )
                .expect("worker ready payload should be JSON");
                assert_eq!(ready["ready"], serde_json::json!(true));
                assert_eq!(ready["initialControllerIsNull"], serde_json::json!(true));

                let result: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerWorkerControllerChangeProbe)")?,
                )
                .expect("worker controllerchange result should be JSON");
                assert_eq!(result["initialControllerIsNull"], serde_json::json!(true));
                assert_eq!(
                    result["controllerScriptURL"],
                    serde_json::json!(expected_service_worker_url)
                );
                assert_eq!(result["controllerState"], serde_json::json!("activated"));
                let events = result["events"]
                    .as_array()
                    .expect("worker controllerchange events should be an array");
                assert!(
                    events
                        .iter()
                        .any(|event| event.as_str() == Some("listener:controllerchange")),
                    "listener should observe worker controllerchange: {events:?}"
                );
                assert!(
                    events
                        .iter()
                        .any(|event| event.as_str() == Some("handler:controllerchange")),
                    "oncontrollerchange should observe worker controllerchange: {events:?}"
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker controllerchange test should run on owner lane");

        server
            .await
            .expect("service worker worker controllerchange server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_intercepted_window_fetch_abort_rejects_and_clears_pending_job() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_service_worker_abort_fetch_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerAbortFetchDone = false;
                        globalThis.__serviceWorkerAbortFetchObserved = "pending";
                        (async () => {
                            await navigator.serviceWorker.register("sw.js", { scope: "./" });
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {
                                await new Promise(resolve => {
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        { once: true }
                                    );
                                });
                            }
                            const controller = new AbortController();
                            const promise = fetch("slow.txt", {
                                signal: controller.signal
                            }).then(
                                () => "fulfilled",
                                error => [
                                    "error",
                                    error && error.name,
                                    error instanceof DOMException,
                                    error && error.message,
                                    controller.signal.aborted
                                ].join(":")
                            );
                            setTimeout(() => controller.abort(), 0);
                            globalThis.__serviceWorkerAbortFetchObserved = await promise;
                            globalThis.__serviceWorkerAbortFetchDone = true;
                        })().catch(error => {
                            globalThis.__serviceWorkerAbortFetchObserved =
                                "outer-error:" + String(error && error.message);
                            globalThis.__serviceWorkerAbortFetchDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_service_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerAbortFetchDone === true)",
                    "service worker intercepted fetch abort should reject",
                )
                .await?;
                let observed = page_vm
                    .vm_mut()
                    .eval("String(globalThis.__serviceWorkerAbortFetchObserved)")?;
                assert_eq!(
                    page_vm.pending_subresource_request_count(),
                    0,
                    "aborted service worker fetch should not leave a pending subresource"
                );
                anyhow::Ok(observed)
            })
            .await
            .expect("service worker abort fetch test should run on owner lane");

        server
            .await
            .expect("service worker abort fetch server should finish");
        assert_eq!(
            observed,
            "error:AbortError:true:The operation was aborted.:true"
        );
    })
    .await;
}

#[tokio::test]
async fn service_worker_intercepts_shared_worker_main_script() {
    run_page_vm_async_test(async move {
        let (base_url, worker_request_rx, server) =
            spawn_service_worker_shared_worker_main_script_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expected_worker_url = format!("{base_url}/app/shared-worker.js");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerSharedWorkerMainProbe = "pending";
                        (async () => {
                            await navigator.serviceWorker.register("sw.js", { scope: "./" });
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {
                                await new Promise(resolve => {
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        { once: true }
                                    );
                                });
                            }
                            const worker = new SharedWorker(
                                "shared-worker.js",
                                "service-worker-main-script"
                            );
                            worker.port.onmessage = event => {
                                globalThis.__serviceWorkerSharedWorkerMainProbe = event.data;
                            };
                            worker.onerror = event => {
                                globalThis.__serviceWorkerSharedWorkerMainProbe =
                                    "error:" + event.message;
                            };
                            worker.port.start();
                        })().catch(error => {
                            globalThis.__serviceWorkerSharedWorkerMainProbe =
                                "error:" + String(error && error.message);
                        });
                    })()
                    "#,
                )?;
                drive_service_worker_and_shared_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerSharedWorkerMainProbe !== 'pending')",
                    "service worker should intercept SharedWorker main script",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerSharedWorkerMainProbe)")?,
                    format!("sw-main:{expected_worker_url}")
                );
                anyhow::Ok(())
            })
            .await
            .expect("service worker SharedWorker main script test should run on owner lane");

        assert!(
            worker_request_rx.await.is_err(),
            "SharedWorker main script should be served by the service worker, not network fallback"
        );
        server
            .await
            .expect("service worker SharedWorker main script server should finish");
    })
    .await;
}
