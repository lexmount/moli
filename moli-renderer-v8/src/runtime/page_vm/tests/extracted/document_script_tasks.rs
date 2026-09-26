use super::*;

#[tokio::test(flavor = "current_thread")]
async fn stale_child_classic_completion_preserves_network_without_document_activity() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let child_handle = crate::dom::native::NativeNodeId::new(401);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(4),
            crate::frame_owner_model::LocalWindowId(5),
            DocumentId(6),
        );
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_classic_script(
            root_document,
            ChildClassicScriptLoadCompletion {
                owner: stale_owner,
                load_id: 7,
                handle: child_handle,
                script_handle: crate::dom::native::NativeNodeId::new(402),
                result: Ok("globalThis.__staleChildClassicRan = true".to_owned()),
                network_result: Some(Arc::new(Err(
                    "stale child classic request failed".to_owned()
                ))),
                network_attribution: ChildClassicScriptNetworkAttribution {
                    frame_id: Some("stale-child-classic-frame".to_owned()),
                    document_url: Url::parse("https://stale-child.test/classic-document").unwrap(),
                    request_url: Url::parse("https://stale-child.test/classic.js").unwrap(),
                },
            },
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("stale child classic arbitration should succeed")
            .expect("stale child classic completion should consume one bounded turn");
        assert_eq!(
            outcome.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::ChildClassicScript,
                owner: RendererPageResourceCompletionOwner::child_document(
                    root_document,
                    child_handle,
                    stale_owner,
                ),
                document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: None,
                },
                body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
                output_effect: PageResourceCompletionOutputEffect::CaptureRequired,
            }
        );

        assert!(!queue.has_ready_completion());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "a retired child classic request must not become current Document activity"
        );
        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 1);
        let network_record = &network_records[0];
        assert_eq!(network_record.frame_id(), Some("stale-child-classic-frame"));
        assert_eq!(
            network_record.document_url().as_str(),
            "https://stale-child.test/classic-document"
        );
        assert_eq!(
            network_record.url().as_str(),
            "https://stale-child.test/classic.js"
        );
        assert_eq!(
            network_record.resource_type(),
            SubresourceResourceType::Script
        );
        assert_eq!(
            network_record.request_initiator_type(),
            SubresourceRequestInitiatorType::Script
        );
        assert_eq!(
            network_record.outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child classic request failed".to_owned(),
            }
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__staleChildClassicRan)")
                .expect("current main Document should remain observable"),
            "undefined"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn child_classic_completion_queue_runs_exactly_one_terminal_per_owner_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let child_handle = crate::dom::native::NativeNodeId::new(411);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(12),
            crate::frame_owner_model::LocalWindowId(13),
            DocumentId(14),
        );
        let completion = |load_id, script_handle| ChildClassicScriptLoadCompletion {
            owner: stale_owner,
            load_id,
            handle: child_handle,
            script_handle: crate::dom::native::NativeNodeId::new(script_handle),
            result: Ok(format!("globalThis.__staleChildClassic{load_id} = true")),
            network_result: None,
            network_attribution: ChildClassicScriptNetworkAttribution {
                frame_id: Some("stale-child-classic-turn-frame".to_owned()),
                document_url: Url::parse("https://stale-child-turn.test/document").unwrap(),
                request_url: Url::parse(&format!(
                    "https://stale-child-turn.test/script-{load_id}.js"
                ))
                .unwrap(),
            },
        };
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_classic_script(
            root_document,
            completion(1, 412),
        ));
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_classic_script(
            root_document,
            completion(2, 413),
        ));

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("first child classic turn should arbitrate")
            .expect("first child classic terminal should be consumed");
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::None,
            "a stale terminal without Network output must not synthesize an output wake"
        );

        assert!(
            queue.has_ready_completion(),
            "one owner turn must not drain the second typed terminal"
        );

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("second child classic turn should arbitrate")
            .expect("second child classic terminal should be consumed");
        assert_eq!(
            second.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );

        assert!(!queue.has_ready_completion());
        assert!(page_vm.vm_mut().take_network_output().is_empty());
        for load_id in [1, 2] {
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval(&format!("String(globalThis.__staleChildClassic{load_id})"))
                    .expect("current Document should remain observable"),
                "undefined"
            );
        }
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_defer_classic_runs_between_interactive_and_domcontentloaded() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-defer-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childClassicDeferWaitEvents.push("defer:" + (globalThis === self));
parent.__childClassicDeferWaitEvents.push("defer-ready:" + document.readyState);
parent.__childClassicDeferWaitEvents.push("current:" + document.currentScript.id);
globalThis.__childClassicDeferWaitValue = 73;
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            bootstrap_sources,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-defer-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicDeferWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicDeferWaitEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicDeferWaitEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-defer-classic" defer src="{script_url}"><\/script>
    <script>
      document.getElementById("external-defer-classic").addEventListener("load", () => {{
        parent.__childClassicDeferWaitEvents.push("script-load");
      }});
      document.addEventListener("readystatechange", () => {{
        parent.__childClassicDeferWaitEvents.push("ready:" + document.readyState);
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childClassicDeferWaitEvents.push("dcl:" + document.readyState);
      }});
      parent.__childClassicDeferWaitEvents.push(
        "after:" + String(globalThis.__childClassicDeferWaitValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?;
                    if bootstrap_events == "before:true|after:undefined|ready:interactive" {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after:undefined|ready:interactive",
                    "parser EOF should dispatch interactive before the deferred script or load"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child defer classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child defer classic completion sender should remain open"
                    );
                }

                let first =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferWaitEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child defer classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?,
                    "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load",
                    "DocumentScriptReady should run defer after interactive and before DOMContentLoaded or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child defer classic DOMContentLoaded transition",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?,
                    "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load|dcl:interactive",
                    "DOMContentLoaded should run on its own lifecycle turn after defer and before complete/load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child defer classic complete transition",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?,
                    "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load|dcl:interactive|ready:complete",
                    "document complete must be a lifecycle turn after defer and DOMContentLoaded"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child defer classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child defer classic follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_sources,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child defer classic deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            first_events, "before:true|after:undefined|ready:interactive",
            "source completion turn should not inline-run child defer classic script"
        );
        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "child defer classic bootstrap should end with the document-owned interactive turn"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "child defer classic should progress through script, DCL, complete, and later HostLoad turns"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load|dcl:interactive|ready:complete|load",
            "defer must run after interactive and before the later HostLoad DCL/complete/load delivery"
        );

        server
            .await
            .expect("child defer classic wait-path server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_defer_classic_source_failure_releases_parser_order_slot() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/missing-child-defer.js",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (completion, events_after_completion, followup_sources, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/missing-child-defer.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicDeferFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicDeferFailureEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicDeferFailureEvents.push("before");<\/script>
    <script id="failed-defer" defer src="{script_url}"><\/script>
    <script>
      document.getElementById("failed-defer").addEventListener("error", () => {{
        parent.__childClassicDeferFailureEvents.push("script-error");
      }});
      document.addEventListener("readystatechange", () => {{
        parent.__childClassicDeferFailureEvents.push("ready:" + document.readyState);
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childClassicDeferFailureEvents.push("dcl");
      }});
      parent.__childClassicDeferFailureEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                for _ in 0..8 {
                    let Some(_) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    if page_vm
                        .vm_mut()
                        .eval("__childClassicDeferFailureEvents.join('|')")?
                        == "before|after|ready:interactive"
                    {
                        break;
                    }
                }
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferFailureEvents.join('|')")?,
                    "before|after|ready:interactive",
                    "failed defer must keep DCL gated while its source completion is pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("failed child defer completion should arrive");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferFailureEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "failed child defer source terminal",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferFailureEvents.join('|')")?,
                    "before|after|ready:interactive|script-error",
                    "source failure must dispatch error and release defer ordering without resuming the parser"
                );
                for (source, label) in [
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "failed child defer DOMContentLoaded",
                    ),
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "failed child defer complete transition",
                    ),
                    (
                        ChildFrameSemanticTurnKind::HostLoad,
                        "failed child defer iframe load",
                    ),
                ] {
                    followup_sources.push(
                        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(&mut page_vm, source, label)
                            .await,
                    );
                }
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferFailureEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    completion,
                    events_after_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("child defer source-failure order test should run");

        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            events_after_completion, "before|after|ready:interactive",
            "resource completion must not report or finalize the failed script inline"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(
            final_events,
            "before|after|ready:interactive|script-error|dcl|ready:complete|load"
        );
        server
            .await
            .expect("failed child defer source server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_async_classic_handoff_queues_document_script_ready_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-async-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childClassicAsyncWaitEvents.push("async:" + (globalThis === self));
parent.__childClassicAsyncWaitEvents.push("current:" + document.currentScript.id);
globalThis.__childClassicAsyncWaitValue = 41;
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            bootstrap_sources,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-async-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicAsyncWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicAsyncWaitEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicAsyncWaitEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-async-classic" async src="{script_url}"><\/script>
    <script>
      document.getElementById("external-async-classic").addEventListener("load", () => {{
        parent.__childClassicAsyncWaitEvents.push("script-load");
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childClassicAsyncWaitEvents.push("dcl:" + document.readyState);
      }});
      parent.__childClassicAsyncWaitEvents.push(
        "after:" + String(globalThis.__childClassicAsyncWaitValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childClassicAsyncWaitEvents.join('|')")?;
                    if bootstrap_events == "before:true|after:undefined|dcl:interactive" {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after:undefined|dcl:interactive",
                    "async classic must not block the document-owned DOMContentLoaded transition"
                );
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "the document-owned async classic delay must keep HostLoad blocked until source completion is applied"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child async classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child async classic completion sender should remain open"
                    );
                }

                let first =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childClassicAsyncWaitEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child async classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicAsyncWaitEvents.join('|')")?,
                    "before:true|after:undefined|dcl:interactive|async:true|current:external-async-classic|script-load",
                    "DocumentScriptReady should execute child async classic and dispatch its script load without iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child async classic complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child async classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicAsyncWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child async classic follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_sources,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child async classic deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            first_events, "before:true|after:undefined|dcl:interactive",
            "source completion turn should not inline-run child async classic script"
        );
        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "child async bootstrap should dispatch interactive and DCL before the later async completion"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "child async classic source completion should progress through script execution, complete, and later HostLoad turns"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|dcl:interactive|async:true|current:external-async-classic|script-load|load",
            "explicit later wait turns should run queued child async classic work"
        );

        server
            .await
            .expect("child async classic wait-path server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_defer_classic_preserves_document_order_when_second_source_finishes_first() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-defer-classic.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childClassicDeferOrderEvents.push("first:" + (globalThis === self));
parent.__childClassicDeferOrderEvents.push("current:" + document.currentScript.id);
"#
                .to_owned(),
                Duration::from_millis(150),
            ),
            (
                "/second-defer-classic.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childClassicDeferOrderEvents.push("second:" + (globalThis === self));
parent.__childClassicDeferOrderEvents.push("current:" + document.currentScript.id);
"#
                .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            second_completion,
            events_after_second_completion,
            first_completion,
            events_after_first_completion,
            bootstrap_sources,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let first_script_url = format!("{base_url}/first-defer-classic.js");
                let second_script_url = format!("{base_url}/second-defer-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicDeferOrderEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicDeferOrderEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicDeferOrderEvents.push("before:" + (globalThis === self));<\/script>
    <script id="first-defer-classic" defer src="{first_script_url}"><\/script>
    <script id="second-defer-classic" defer src="{second_script_url}"><\/script>
    <script>
      document.getElementById("first-defer-classic").addEventListener("load", () => {{
        parent.__childClassicDeferOrderEvents.push("first-load");
      }});
      document.getElementById("second-defer-classic").addEventListener("load", () => {{
        parent.__childClassicDeferOrderEvents.push("second-load");
      }});
      parent.__childClassicDeferOrderEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childClassicDeferOrderEvents.join('|')")?;
                    if source == ChildFrameSemanticTurnKind::DocumentLifecycle {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after",
                    "child parser should continue past both defer classics without firing load"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("second child defer classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child defer classic completion sender should remain open"
                    );
                }
                let second_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_second_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferOrderEvents.join('|')")?;
                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("first child defer classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child defer classic completion sender should remain open"
                    );
                }
                let first_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_first_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferOrderEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first child defer classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferOrderEvents.join('|')")?,
                    "before:true|after|first:true|current:first-defer-classic|first-load",
                    "first ordered DocumentScriptReady should execute the earlier defer classic without second defer or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second child defer classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferOrderEvents.join('|')")?,
                    "before:true|after|first:true|current:first-defer-classic|first-load|second:true|current:second-defer-classic|second-load",
                    "second ordered DocumentScriptReady should run only after the earlier defer classic completes"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child defer classic DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child defer classic complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "ordered child defer classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferOrderEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child defer ordering sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    second_completion,
                    events_after_second_completion,
                    first_completion,
                    events_after_first_completion,
                    bootstrap_sources,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child defer classic ordering test should run");

        assert!(matches!(
            second_completion.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            second_completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            second_completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "child defer ordering bootstrap should reach interactive before either source completion"
        );
        assert_eq!(
            events_after_second_completion, "before:true|after",
            "faster second defer source completion must not run before earlier defer"
        );
        assert!(matches!(
            first_completion.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            first_completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            first_completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            events_after_first_completion, "before:true|after",
            "first defer source completion turn should still not inline-run script"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "ordered child defer classics should execute through two script-ready turns, DOMContentLoaded, and complete before HostLoad"
        );
        assert_eq!(
            final_events,
            "before:true|after|first:true|current:first-defer-classic|first-load|second:true|current:second-defer-classic|second-load|load",
            "defer classics should execute in document order even when the second source finishes first"
        );

        server
            .await
            .expect("child defer classic ordering server should finish");
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn page_vm_realm_materialization_created_ready_work_enters_document_script_ready_directly() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval_with_child_record_sync(
            r#"
(() => {
  globalThis.__realmMaterializationNestedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <iframe srcdoc="&lt;script&gt;parent.parent.__realmMaterializationNestedEvents.push('nested-script:' + (globalThis === self));&lt;/script&gt;"></iframe>
    <script>parent.__realmMaterializationNestedEvents.push("outer-script:" + (globalThis === self));<\/script>
  `;
  body.appendChild(frame);
})()
"#,
        )?;

        for label in ["outer", "nested"] {
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm,
                ChildFrameSemanticTurnKind::NavigationCommit,
                &format!("{label} child navigation commit"),
            )
            .await;
        }
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "outer child parser work should run on the DocumentScriptReady source"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__realmMaterializationNestedEvents.join('|')")?,
            "outer-script:true",
            "outer DocumentScriptReady turn should not inline-run nested parser work"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "nested child realm must be materialized in its own visible family turn"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "outer parser EOF should become interactive before nested document work"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "ready work produced by child realm materialization should enter DocumentScriptReady directly"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__realmMaterializationNestedEvents.join('|')")?,
            "outer-script:true|nested-script:true",
            "nested parser work should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "outer DOMContentLoaded should remain a later FIFO turn after nested work already admitted by realm materialization"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("realm materialization nested ready-work source test should run");
}
#[tokio::test]
async fn child_dynamic_inline_script_runs_synchronously_without_document_script_ready_work() {
    let (immediate_events, settled_events, next_source) = run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__childDynamicReadySourceEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => __childDynamicReadySourceEvents.push("frame-load");
  body.appendChild(frame);
  const script = document.createElement("script");
  script.textContent = `
    parent.__childDynamicReadySourceEvents.push(
      "dynamic:" + document.currentScript.tagName.toLowerCase()
    );
  `;
  frame.contentDocument.body.appendChild(script);
  __childDynamicReadySourceEvents.push("returned");
})()
"#,
        )?;
        let immediate_events = page_vm
            .vm_mut()
            .eval("__childDynamicReadySourceEvents.join('|')")?;
        run_expected_child_realm_materialization_for_wait(
            &mut page_vm,
            "dynamic child inline-script realm",
        )
        .await;
        let next_source = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await;
        let settled_events = page_vm
            .vm_mut()
            .eval("__childDynamicReadySourceEvents.join('|')")?;
        Ok::<_, anyhow::Error>((immediate_events, settled_events, next_source))
    })
    .await
    .expect("child dynamic inline script source test should run");

    assert_eq!(
        immediate_events, "frame-load|dynamic:script|returned",
        "both initial about:blank load and dynamic inline execution are synchronous"
    );
    assert_eq!(settled_events, immediate_events);
    assert_eq!(
        next_source, None,
        "inline execution must leave no DocumentScriptReady or HostLoad task"
    );
}
#[tokio::test]
async fn child_dynamic_external_classic_script_uses_child_resource_and_ready_owners() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-dynamic-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childDynamicExternalEvents.push("external:" + (globalThis === self));
parent.__childDynamicExternalEvents.push("current:" + document.currentScript.id);
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (events_before_completion, completion_source, ready_source, final_events) =
            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let script_url = format!("{base_url}/child-dynamic-classic.js");
                    page_vm.vm_mut().eval(
                        r#"
(() => {
  globalThis.__childDynamicExternalEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "dynamic-external-frame";
  frame.onload = () => __childDynamicExternalEvents.push("frame-load");
  frame.srcdoc = "<!doctype html><html><head><title>child</title></head><body></body></html>";
  body.appendChild(frame);
})()
"#,
                    )?;
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "dynamic external child navigation commit",
                    )
                    .await;
                    run_expected_child_realm_materialization_for_wait(
                        &mut page_vm,
                        "dynamic external child realm",
                    )
                    .await;
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "dynamic external child initial load",
                    )
                    .await;

                    page_vm.vm_mut().eval(&format!(
                        r#"
(() => {{
  const frame = document.getElementById("dynamic-external-frame");
  const script = frame.contentDocument.createElement("script");
  script.id = "dynamic-external";
  script.onload = () => __childDynamicExternalEvents.push("script-load");
  script.onerror = () => __childDynamicExternalEvents.push("script-error");
  script.src = "{script_url}";
  frame.contentDocument.body.appendChild(script);
}})()
"#,
                    ))?;
                    let events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childDynamicExternalEvents.join('|')")?;
                    if !page_vm
                        .page_resource_completion_queue()
                        .has_ready_completion()
                    {
                        let arrived = tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("child dynamic external completion should arrive before timeout");
                        assert!(
                            arrived,
                            "child dynamic external completion sender should remain open"
                        );
                    }
                    let completion = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    let completion_source = completion.action.source();
                    let ready_source =
                        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                            &mut page_vm,
                            ChildFrameSemanticTurnKind::DocumentScriptReady,
                            "child dynamic external classic execution",
                        )
                        .await;
                    let final_events = page_vm
                        .vm_mut()
                        .eval("__childDynamicExternalEvents.join('|')")?;

                    Ok::<_, anyhow::Error>((
                        events_before_completion,
                        completion_source,
                        ready_source,
                        final_events,
                    ))
                })
                .await
                .expect("child dynamic external script test should run");

        assert_eq!(
            events_before_completion, "frame-load",
            "external script fetch and execution must not run inline with insertion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildClassicScript,
            "the child classic resource owner should publish the network terminal"
        );
        assert_eq!(
            ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "the loaded external classic script should execute on the child ready source"
        );
        assert_eq!(
            final_events, "frame-load|external:true|current:dynamic-external|script-load",
            "the current child realm should execute once and dispatch load on its script element"
        );

        server
            .await
            .expect("child dynamic external script server should finish");
    })
    .await;
}
