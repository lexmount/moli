use super::*;

#[tokio::test]
async fn page_vm_child_classic_resource_completion_queues_child_frame_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childClassicWaitEvents.push("external:" + (globalThis === self));
parent.__childClassicWaitEvents.push("current:" + document.currentScript.id);
globalThis.__childClassicWaitValue = 91;
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            bootstrap_source,
            blocked_host_load_before_completion,
            blocked_host_load_pump_before_completion,
            events_after_blocked_host_load_before_completion,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicWaitEvents.push("load");
  frame.srcdoc = `
    <script id="external-classic" src="{script_url}"><\/script>
    <script id="after-external-classic">
      parent.__childClassicWaitEvents.push(
        "inline:" + globalThis.__childClassicWaitValue
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "child classic srcdoc navigation commit",
                )
                .await;
                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "child classic exact realm",
                )
                .await;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicWaitEvents.join('|')")?,
                    "",
                    "pending child external classic script should block inline script and load"
                );
                let blocked_host_load_before_completion = page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildHostLoad,
                        &loader,
                    )
                    .await?;
                let blocked_host_load_pump_before_completion =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_blocked_host_load_before_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicWaitEvents.join('|')")?;

                let page_resource_queue = page_vm.page_resource_completion_queue();
                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        page_vm.page_task_queue.wait_for_page_runtime_wake(),
                    )
                    .await
                    .expect("child classic completion should arrive before timeout");
                }
                let (_, queued_completion) = page_resource_queue
                    .pop_front()
                    .expect("child classic completion should remain queued");
                let completion_owner = queued_completion.owner();
                let completion = match queued_completion.into_terminal() {
                    RendererPageResourceTerminal::ChildClassicScript { completion } => completion,
                    other => panic!("expected child classic completion, got {other:?}"),
                };
                let root_document = page_vm.document_lifecycle.identity().document;
                assert!(
                    completion.network_result.is_some(),
                    "loader-backed child classic completion must retain its Network fact"
                );
                let expected_network_frame_id = completion.network_attribution.frame_id.clone();
                let expected_network_document_url =
                    completion.network_attribution.document_url.clone();
                let expected_network_request_url =
                    completion.network_attribution.request_url.clone();
                let _ = page_vm.vm_mut().take_network_output();
                let activity_epoch_before_completion =
                    page_vm.vm().subresource_activity_epoch();
                let first = page_vm
                    .apply_selected_page_resource_completion_turn(
                        RendererPageResourceCompletion::child_classic_script(
                            root_document,
                            completion,
                        ),
                    )?;
                assert_eq!(first.action.owner, completion_owner);
                assert_eq!(
                    first.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    first.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_completion,
                    "a current child classic Network terminal must advance current Document activity"
                );
                let (network_records, websocket_events, websocket_lifecycle_events) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(websocket_events.is_empty());
                assert!(websocket_lifecycle_events.is_empty());
                assert_eq!(network_records.len(), 1);
                let network_record = &network_records[0];
                assert_eq!(
                    network_record.frame_id(),
                    expected_network_frame_id.as_deref(),
                    "consumer must use producer-captured child frame attribution"
                );
                assert_eq!(
                    network_record.document_url(),
                    &expected_network_document_url
                );
                assert_eq!(network_record.url(), &expected_network_request_url);
                assert_eq!(network_record.resource_type(), SubresourceResourceType::Script);
                assert_eq!(
                    network_record.request_initiator_type(),
                    SubresourceRequestInitiatorType::Script
                );
                assert!(matches!(
                    network_record.outcome(),
                    SubresourceNetworkOutcome::Success { .. }
                ));
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childClassicWaitEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child classic external execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicWaitEvents.join('|')")?,
                    "external:true|current:external-classic",
                    "first DocumentScriptReady should execute the external classic script without parser continuation or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child classic parser continuation",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicWaitEvents.join('|')")?,
                    "external:true|current:external-classic|inline:91",
                    "second DocumentScriptReady should run parser continuation without iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child classic parser EOF interactive transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child classic DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child classic complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child classic follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_source,
                    blocked_host_load_before_completion,
                    blocked_host_load_pump_before_completion,
                    events_after_blocked_host_load_before_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child classic deferred completion test should run");

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
            first_events, "",
            "resource completion turn should not inline-run child classic script"
        );
        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::ClassicScriptSourceLoad),
            "child classic bootstrap should start the external source load with one explicit source turn"
        );
        assert!(
            !blocked_host_load_before_completion,
            "HostLoad source should report no progress while parser-blocking classic source is pending"
        );
        assert_eq!(
            blocked_host_load_pump_before_completion, None,
            "blocked lifecycle must not produce a HostLoad task in the stable child-frame family"
        );
        assert_eq!(
            events_after_blocked_host_load_before_completion, "",
            "no HostLoad delivery may exist before classic source completion"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "child classic completion should progress through script, parser continuation, interactive, DOMContentLoaded, complete, and HostLoad turns"
        );
        assert_eq!(
            final_events,
            "external:true|current:external-classic|inline:91|load",
            "explicit later wait turns should run queued child classic work"
        );

        server
            .await
            .expect("child classic wait-path server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_parser_blocking_classic_waits_for_preceding_stylesheet() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![(
            "/parser-blocking.css",
            "HTTP/1.1 200 OK",
            ":root { --child-parser-style-gate: stylesheet-ready; }".to_owned(),
            Duration::from_millis(180),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            navigation_source,
            completion,
            events_after_completion,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childParserStylesheetEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childParserStylesheetEvents.push("load");
  frame.srcdoc = `
    <link rel="stylesheet" href="{base_url}/parser-blocking.css">
    <script>
      parent.__childParserStylesheetEvents.push(
        "script:" + getComputedStyle(document.documentElement)
          .getPropertyValue("--child-parser-style-gate").trim() + ":" + document.readyState
      );
    <\/script>
    <body></body>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let navigation_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "stylesheet-gated child srcdoc navigation commit",
                )
                .await;
                let materialization = page_vm
                    .run_child_realm_materialization_body_for_test()?
                    .expect("child parser stylesheet fixture should queue one typed realm turn");
                assert_eq!(
                    materialization.action.target_effect,
                    crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
                );

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childParserStylesheetEvents.join('|')")?,
                    "",
                    "parser-blocking classic must remain pending while its preparation-time stylesheet snapshot is unresolved"
                );
                assert!(
                    !page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildHostLoad,
                            &loader,
                        )
                        .await?,
                    "HostLoad must not bypass a stylesheet-blocked parser"
                );

                let page_resource_queue = page_vm.page_resource_completion_queue();
                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        page_vm.page_task_queue.wait_for_page_runtime_wake(),
                    )
                    .await
                    .expect("child stylesheet completion should arrive");
                }
                let (_, queued_completion) = page_resource_queue
                    .pop_front()
                    .expect("child stylesheet completion should remain queued");
                let completion_owner = queued_completion.owner();
                let completion = match queued_completion.into_terminal() {
                    RendererPageResourceTerminal::ChildBlockingStylesheet { completion } => {
                        completion
                    }
                    other => panic!("expected child stylesheet completion, got {other:?}"),
                };
                let root_document = page_vm.document_lifecycle.identity().document;
                let completion = page_vm
                    .apply_selected_page_resource_completion_turn(
                        RendererPageResourceCompletion::child_blocking_stylesheet(
                            root_document,
                            completion,
                        ),
                    )?;
                assert_eq!(completion.action.owner, completion_owner);
                let events_after_completion = page_vm
                    .vm_mut()
                    .eval("__childParserStylesheetEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "stylesheet-released child parser-blocking classic",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childParserStylesheetEvents.join('|')")?,
                    "script:stylesheet-ready:loading",
                    "stylesheet source must be installed before the parser-blocking script executes while the document is still loading"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-gated child interactive transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-gated child DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-gated child complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "stylesheet-gated child iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childParserStylesheetEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    navigation_source,
                    completion,
                    events_after_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("child parser stylesheet test should run");

        assert_eq!(navigation_source, ChildFrameSemanticTurnKind::NavigationCommit);
        assert_eq!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert_eq!(
            completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert_eq!(
            events_after_completion, "",
            "stylesheet resource completion must not inline-run child script"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(final_events, "script:stylesheet-ready:loading|load");
        server.await.expect("child parser stylesheet server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_parser_defer_preserves_cross_kind_document_order() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-module.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("first-module");"#.to_owned(),
                Duration::from_millis(220),
            ),
            (
                "/second-classic.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("second-classic");"#.to_owned(),
                Duration::ZERO,
            ),
            (
                "/third-classic.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("third-classic");"#.to_owned(),
                Duration::from_millis(420),
            ),
            (
                "/fourth-module.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("fourth-module");"#.to_owned(),
                Duration::from_millis(20),
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            execution_sources,
            lifecycle_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childMixedDeferEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childMixedDeferEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childMixedDeferEvents.push("before");<\/script>
    <script id="first-module" type="module" src="{base_url}/first-module.js"><\/script>
    <script id="second-classic" defer src="{base_url}/second-classic.js"><\/script>
    <script id="third-classic" defer src="{base_url}/third-classic.js"><\/script>
    <script id="fourth-module" type="module" src="{base_url}/fourth-module.js"><\/script>
    <script>
      for (const id of ["first-module", "second-classic", "third-classic", "fourth-module"]) {{
        document.getElementById(id).addEventListener("load", () => {{
          parent.__childMixedDeferEvents.push(id + "-load");
        }});
      }}
      parent.__childMixedDeferEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                for _ in 0..12 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    if source == ChildFrameSemanticTurnKind::DocumentLifecycle
                        && bootstrap_sources
                            .iter()
                            .filter(|source| {
                                **source == ChildFrameSemanticTurnKind::ParserModuleRootStart
                            })
                            .count()
                            == 2
                    {
                        break;
                    }
                }
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after",
                    "parser EOF must not execute any mixed parser-deferred script"
                );
                assert!(
                    bootstrap_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle),
                    "mixed parser-deferred document should reach interactive"
                );
                assert_eq!(
                    bootstrap_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        .count(),
                    2,
                    "both module roots should start before terminal ordering is tested"
                );

                let mut completion_sources = Vec::new();
                let mut terminal_turn_count = 0;
                for completion_index in 0..3 {
                    if !page_vm.page_resource_completion_queue().has_ready_completion() {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            panic!(
                                "mixed parser-deferred completion {completion_index} should arrive"
                            )
                        });
                    }
                    let completion =
                        run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    completion_sources.push(completion.action.source());
                    if completion.action.source()
                        == RendererOwnerResourceActivitySource::ModuleGraphFetch
                    {
                        run_expected_child_module_script_terminal_turn(
                            &mut page_vm,
                            "mixed child module terminal",
                        )
                        .await;
                        terminal_turn_count += 1;
                    }
                    assert_eq!(
                        page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                        "before|after",
                        "out-of-order completions and retained terminals must not execute a later parser-deferred script"
                    );
                }

                assert_eq!(
                    completion_sources,
                    vec![
                        RendererOwnerResourceActivitySource::ChildClassicScript,
                        RendererOwnerResourceActivitySource::ModuleGraphFetch,
                        RendererOwnerResourceActivitySource::ModuleGraphFetch,
                    ],
                    "network timing should make later classic and module work terminal before the first module"
                );
                assert_eq!(terminal_turn_count, 2);

                let mut execution_sources = Vec::new();
                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first mixed parser-deferred module",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after|first-module|first-module-load",
                    "earlier module must execute before the already-ready later classic"
                );
                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second mixed parser-deferred classic",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after|first-module|first-module-load|second-classic|second-classic-load",
                    "second classic should execute only after the first module finishes"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("third classic completion should arrive");
                }
                let third_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                completion_sources.push(third_completion.action.source());

                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "third mixed parser-deferred classic",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after|first-module|first-module-load|second-classic|second-classic-load|third-classic|third-classic-load",
                    "third classic must execute before the already-terminal fourth module"
                );
                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "fourth mixed parser-deferred module",
                    )
                    .await,
                );

                let lifecycle_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "mixed parser-deferred DOMContentLoaded",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "mixed parser-deferred complete transition",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "mixed parser-deferred iframe load",
                    )
                    .await,
                ];
                let final_events = page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    execution_sources,
                    lifecycle_sources,
                    final_events,
                ))
            })
            .await
            .expect("mixed child parser-deferred ordering test should run");

        assert_eq!(
            completion_sources.last(),
            Some(&RendererOwnerResourceActivitySource::ChildClassicScript)
        );
        assert_eq!(
            execution_sources,
            vec![ChildFrameSemanticTurnKind::DocumentScriptReady; 4]
        );
        assert_eq!(
            lifecycle_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ],
            "DCL, complete, and iframe load must remain later lifecycle turns"
        );
        assert_eq!(
            final_events,
            "before|after|first-module|first-module-load|second-classic|second-classic-load|third-classic|third-classic-load|fourth-module|fourth-module-load|load",
            "mixed parser-deferred scripts must execute in one cross-kind document order"
        );

        server
            .await
            .expect("mixed child parser-deferred ordering server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_document_completion_queues_document_script_ready_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-document.html",
            "HTTP/1.1 200 OK",
            r#"
<!doctype html>
<script>
parent.__childDocumentLoadWaitEvents.push("child-script:" + (globalThis === self));
globalThis.__childDocumentLoadWaitValue = 42;
</script>
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
            realm_pending_after_first,
            host_load_pending_after_completion,
            first_followup_source,
            events_after_first_followup,
            lifecycle_ready_after_first_followup,
            interactive_source,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child-document.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childDocumentLoadWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childDocumentLoadWaitEvents.push("frameload");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;
                let startup_sources =
                    drive_child_frame_task_sources_until_resource_completion_ready(
                        &mut page_vm,
                        8,
                    )
                    .await;
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::NavigationCommit),
                    "external child document startup should begin from an explicit NavigationCommit source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childDocumentLoadWaitEvents.join('|')")?,
                    "",
                    "child document script and frame load should wait for document completion"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child document completion sender should remain open"
                    );
                }

                let first = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childDocumentLoadWaitEvents.join('|')")?;
                let realm_pending_after_first = page_vm
                    .vm()
                    .has_pending_child_frame_realm_materialization();
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child document parser realm prerequisite",
                )?;
                let host_load_pending_after_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

                let first_followup_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child document parser script execution",
                )
                .await;
                let events_after_first_followup = page_vm
                    .vm_mut()
                    .eval("__childDocumentLoadWaitEvents.join('|')")?;
                let lifecycle_ready_after_first_followup = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );

                let interactive_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "child document parser EOF interactive transition",
                )
                .await;
                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "child document iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childDocumentLoadWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child document completion sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    realm_pending_after_first,
                    host_load_pending_after_completion,
                    first_followup_source,
                    events_after_first_followup,
                    lifecycle_ready_after_first_followup,
                    interactive_source,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child document deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ChildDocument
        ));
        assert_eq!(
            first_events, "",
            "child document completion turn should not inline-run child document script or frame load"
        );
        assert!(
            realm_pending_after_first,
            "child document completion must leave a typed realm prerequisite queued"
        );
        assert!(
            !host_load_pending_after_completion,
            "child document completion should not wake HostLoad while initial parser work is pending"
        );
        assert_eq!(
            first_followup_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "first child frame task turn should run document-script ready work"
        );
        assert_eq!(
            events_after_first_followup, "child-script:true",
            "first child frame task turn should run document-script ready work, not iframe load"
        );
        assert!(
            lifecycle_ready_after_first_followup,
            "document-script ready should make the later lifecycle turn runnable"
        );
        assert_eq!(
            interactive_source,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "parser EOF should become interactive before HostLoad"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a later HostLoad source"
        );
        assert_eq!(
            final_events,
            "child-script:true|frameload",
            "explicit later HostLoad turn should dispatch iframe load"
        );

        server
            .await
            .expect("child document wait-path server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_document_ready_work_runs_one_item_per_turn() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-a.html",
                "HTTP/1.1 200 OK",
                r#"
<script>
parent.__multiChildDocumentEvents.push("child-a-script:" + (globalThis === self));
</script>
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/child-b.html",
                "HTTP/1.1 200 OK",
                r#"
<script>
parent.__multiChildDocumentEvents.push("child-b-script:" + (globalThis === self));
</script>
"#
                .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (owner_wake_tx, mut owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
        );
        let runtime_hooks =
            PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                owner_wake,
            );
        let page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader,
            Vec::new(),
            document_url,
            runtime_hooks,
        );
        let mut page_resource_queue = page_vm.page_resource_completion_queue();
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            events_after_completions,
            first_script_ready_source,
            events_after_first_script_ready,
            second_script_ready_source,
            events_after_second_script_ready,
            lifecycle_ready_after_second_script,
            lifecycle_sources,
            first_host_load_source,
            events_after_first_host_load,
            second_host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_a_url = format!("{base_url}/child-a.html");
                let child_b_url = format!("{base_url}/child-b.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__multiChildDocumentEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  for (const [name, url] of [["a", "{child_a_url}"], ["b", "{child_b_url}"]]) {{
    const frame = document.createElement("iframe");
    frame.onload = () => globalThis.__multiChildDocumentEvents.push("frame-" + name + "-load");
    frame.src = url;
    body.appendChild(frame);
  }}
}})()
"#
                ))?;
                let startup_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "first external child navigation start",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "second external child navigation start",
                    )
                    .await,
                ];
                assert_eq!(
                    startup_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::NavigationCommit)
                        .count(),
                    2,
                    "each child document startup should consume one NavigationCommit source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__multiChildDocumentEvents.join('|')")?,
                    "",
                    "child documents should wait for external document completions"
                );

                let mut completion_sources = Vec::new();
                for label in ["first child document", "second child document"] {
                    child_document_completion::wait_for_page_resource_completion(
                        &mut page_resource_queue,
                        &mut owner_wake_rx,
                        label,
                    )
                    .await;
                    let completion = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                        .expect("child document completion should consume one typed Page turn");
                    completion_sources.push(completion.action.source);
                }
                for label in ["first child realm", "second child realm"] {
                    run_expected_pending_child_realm_materialization_turn(
                        &mut page_vm,
                        label,
                    )?;
                }
                let events_after_completions = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;

                let first_script_ready_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_first_script_ready = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;
                let second_script_ready_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_second_script_ready = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;
                let lifecycle_ready_after_second_script = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );
                let lifecycle_sources = vec![
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                ];
                let first_host_load_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_first_host_load = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;

                let second_host_load_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "multi-child document completion sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_completions,
                    first_script_ready_source,
                    events_after_first_script_ready,
                    second_script_ready_source,
                    events_after_second_script_ready,
                    lifecycle_ready_after_second_script,
                    lifecycle_sources,
                    first_host_load_source,
                    events_after_first_host_load,
                    second_host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm multi child document ready-work test should run");

        assert_eq!(
            completion_sources,
            vec![
                RendererOwnerResourceActivitySource::ChildDocument,
                RendererOwnerResourceActivitySource::ChildDocument,
            ]
        );
        assert_eq!(
            events_after_completions, "",
            "document completion turns should not inline-run either child document"
        );
        assert_eq!(
            first_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "first child frame task turn should run only one document-script ready item"
        );
        let first_turn_ran_a = events_after_first_script_ready.contains("child-a-script:true");
        let first_turn_ran_b = events_after_first_script_ready.contains("child-b-script:true");
        assert_ne!(
            first_turn_ran_a, first_turn_ran_b,
            "first document-script ready turn should run exactly one child script; events: {events_after_first_script_ready}"
        );
        assert_eq!(
            second_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "the stable child-frame family must preserve the already-enqueued second script ahead of lifecycle work produced by the first"
        );
        for expected in ["child-a-script:true", "child-b-script:true"] {
            assert!(
                events_after_second_script_ready.contains(expected),
                "second document-script ready turn should run both child scripts across two turns; events: {events_after_second_script_ready}"
            );
        }
        for unexpected in ["frame-a-load", "frame-b-load"] {
            assert!(
                !events_after_second_script_ready.contains(unexpected),
                "second document-script ready turn should still not dispatch iframe load inline; events: {events_after_second_script_ready}"
            );
        }
        assert!(
            lifecycle_ready_after_second_script,
            "both scripts should leave the oldest exact-Document lifecycle action at the family head"
        );
        assert_eq!(
            lifecycle_sources,
            vec![Some(ChildFrameSemanticTurnKind::DocumentLifecycle); 6],
            "interactive, DOMContentLoaded and complete must each consume one lifecycle turn per child"
        );
        assert_eq!(
            first_host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "first iframe load should remain a later HostLoad source"
        );
        let first_host_load_dispatched_a =
            events_after_first_host_load.contains("frame-a-load");
        let first_host_load_dispatched_b =
            events_after_first_host_load.contains("frame-b-load");
        assert_ne!(
            first_host_load_dispatched_a,
            first_host_load_dispatched_b,
            "first HostLoad turn should dispatch exactly one iframe load; events: {events_after_first_host_load}"
        );
        assert_eq!(
            second_host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "second iframe load should remain a separate HostLoad source"
        );
        for expected in [
            "child-a-script:true",
            "child-b-script:true",
            "frame-a-load",
            "frame-b-load",
        ] {
            assert!(
                final_events.contains(expected),
                "explicit script-ready and HostLoad turns should finish both child documents; final events: {final_events}"
            );
        }

        server
            .await
            .expect("multi child document wait-path server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_document_script_ready_leaves_complete_and_load_for_later_sources() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child.html",
            "HTTP/1.1 200 OK",
            r#"
<script>
parent.__childReadyHostLoadEvents.push("child-script:" + (globalThis === self));
</script>
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
            bootstrap_source,
            completion_source,
            host_load_pending_after_completion,
            script_ready_source,
            events_after_script_ready,
            lifecycle_ready_after_script,
            interactive_source,
            host_load_source,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childReadyHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childReadyHostLoadEvents.push("frame-load");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;

                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childReadyHostLoadEvents.join('|')")?,
                    "",
                    "NavigationCommit should start the child document load without dispatching load"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(arrived, "child document completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                let host_load_pending_after_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child script-ready realm prerequisite",
                )?;

                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childReadyHostLoadEvents.join('|')")?;
                let lifecycle_ready_after_script = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );

                let interactive_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child document iframe load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childReadyHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    bootstrap_source,
                    completion_source,
                    host_load_pending_after_completion,
                    script_ready_source,
                    events_after_script_ready,
                    lifecycle_ready_after_script,
                    interactive_source,
                    host_load_source,
                    events_after_host_load,
                ))
            })
            .await
            .expect("page vm child script-ready host-load boundary test should run");

        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "bootstrap child frame turn should commit navigation before lifecycle delivery"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildDocument
        );
        assert!(
            !host_load_pending_after_completion,
            "child document completion should not leave HostLoad pending before document-script ready work runs"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "child document completion follow-up should run document-script ready work"
        );
        assert_eq!(
            events_after_script_ready, "child-script:true",
            "DocumentScriptReady should run the child script but not dispatch iframe load inline"
        );
        assert!(
            lifecycle_ready_after_script,
            "DocumentScriptReady should make the later lifecycle turn runnable"
        );
        assert_eq!(
            interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "parser EOF should dispatch interactive before HostLoad"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "HostLoad should be the source that dispatches iframe load"
        );
        assert_eq!(
            events_after_host_load, "child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server
            .await
            .expect("child document host-load boundary server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_input_event_commits_child_navigation_before_document_script_ready() {
    let page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();

    let input = run_on_page_vm_local_executor(local_executor, async move {
        let mut page_vm = page_vm;
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__inputReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const button = document.createElement("button");
  button.textContent = "create child";
  button.style.cssText = "display:block;width:100px;height:20px";
  button.addEventListener("click", () => {
    __inputReadyEvents.push("click");
    const frame = document.createElement("iframe");
    frame.onload = () => __inputReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__inputReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  });
  body.appendChild(button);
})()
"#,
        )?;

        page_vm.vm_mut().publish_layout_for_test()?;

        let mouse_down_handled = page_vm
            .vm_mut()
            .dispatch_mouse_event_at_point(10.0, 10.0, "mousedown", 0, None, 0.0, 0.0)?
            .handled;
        let mouse_up_handled = page_vm
            .vm_mut()
            .dispatch_mouse_event_at_point(10.0, 10.0, "mouseup", 0, Some(0), 0.0, 0.0)?
            .handled;
        let events_after_click = page_vm.vm_mut().eval("__inputReadyEvents.join('|')")?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "input-created child navigation commit",
        )
        .await;
        run_expected_child_realm_materialization_for_wait(
            &mut page_vm,
            "input-created child realm",
        )
        .await;
        let script_ready_source = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await;
        let events_after_script_ready = page_vm.vm_mut().eval("__inputReadyEvents.join('|')")?;
        let host_load_source = Some(
            run_child_interactive_domcontentloaded_then_host_load_for_wait(
                &mut page_vm,
                "input-created child iframe load",
            )
            .await,
        );
        let events_after_host_load = page_vm.vm_mut().eval("__inputReadyEvents.join('|')")?;

        Ok::<_, anyhow::Error>((
            mouse_down_handled,
            mouse_up_handled,
            events_after_click,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ))
    });
    let (
        mouse_down_handled,
        mouse_up_handled,
        events_after_click,
        script_ready_source,
        events_after_script_ready,
        host_load_source,
        events_after_host_load,
    ) = run_page_vm_async_test(input)
        .await
        .expect("input ready-work source test should run");

    assert!(mouse_down_handled, "mousedown should hit the button");
    assert!(
        mouse_up_handled,
        "mouseup should hit the button and dispatch click"
    );
    assert_eq!(
        events_after_click, "click",
        "input click should create the child frame without running its parser script inline"
    );
    assert_eq!(
        script_ready_source,
        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
        "input-created child parser work should follow its navigation commit"
    );
    assert_eq!(
        events_after_script_ready, "click|child-script:true",
        "child parser work should run on the later DocumentScriptReady turn"
    );
    assert_eq!(
        host_load_source,
        Some(ChildFrameSemanticTurnKind::HostLoad),
        "iframe load should remain a separate HostLoad turn after input dispatch"
    );
    assert_eq!(
        events_after_host_load, "click|child-script:true|frame-load",
        "iframe load should dispatch only on the HostLoad turn"
    );
}
#[tokio::test]
async fn page_vm_message_port_event_commits_child_navigation_before_document_script_ready() {
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let document_url = Url::parse("https://example.com/message-port-child-navigation").unwrap();
    let (page_vm, _resource_source, mut owner_wake_rx) =
        page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
    let local_executor = page_vm.local_executor.clone();
    let selected_task_local_set = tokio::task::LocalSet::new();

    let (
        completion_sources,
        message_port_turns,
        events_after_message,
        script_ready_source,
        events_after_script_ready,
        host_load_source,
        events_after_host_load,
    ) = selected_task_local_set
        .run_until(local_executor.run(async move {
            let mut page_vm = page_vm;
            page_vm.vm_mut().eval(
                r#"
(() => {
  globalThis.__messagePortReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const workerSource = `
    onmessage = (event) => {
      if (event.data !== "connect") {
        return;
      }
      const port = event.ports[0];
      port.postMessage("go");
    };
  `;
  globalThis.__messagePortReadyWorker = new Worker(
    "data:text/javascript," + encodeURIComponent(workerSource)
  );
  globalThis.__messagePortReadyChannel = new MessageChannel();
  const channel = globalThis.__messagePortReadyChannel;
  channel.port1.onmessage = (event) => {
    __messagePortReadyEvents.push("message:" + event.data);
    const frame = document.createElement("iframe");
    frame.onload = () => __messagePortReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__messagePortReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  };
  globalThis.__messagePortReadyWorker.postMessage("connect", [channel.port2]);
})()
"#,
            )?;

            let mut completion_sources = Vec::new();
            let mut message_port_turns = 0;
            let events_after_message = loop {
                if page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                        &loader,
                    )
                    .await?
                {
                    completion_sources.push(RendererOwnerResourceActivitySource::Worker);
                } else if page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MessagePortDelivery, &loader)
                    .await?
                {
                    message_port_turns += 1;
                } else if page_vm.has_ready_page_websocket_task_for_test() {
                    let completion_source = page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .expect("ready Worker completion should remain available");
                    completion_sources.push(completion_source);
                } else {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        async {
                            tokio::select! {
                                wake = owner_wake_rx.recv() => wake.is_some(),
                                arrived = page_vm.wait_for_page_websocket_task_for_test() => arrived,
                            }
                        },
                    )
                    .await
                    .expect("MessagePort/worker completion should arrive before timeout");
                    assert!(
                        arrived,
                        "Worker completion sender should remain open"
                    );
                }
                let events = page_vm
                    .vm_mut()
                    .eval("__messagePortReadyEvents.join('|')")?;
                if events == "message:go" {
                    break events;
                }
                assert!(
                    completion_sources.len() + message_port_turns < 16,
                    "MessagePort handler should run after bounded turns; sources: {completion_sources:?}, message_port_turns={message_port_turns}, events: {events}"
                );
            };
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm,
                ChildFrameSemanticTurnKind::NavigationCommit,
                "MessagePort-created child navigation commit",
            )
            .await;
            run_expected_child_realm_materialization_for_wait(
                &mut page_vm,
                "MessagePort-created child realm",
            )
            .await;
            let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
            let events_after_script_ready = page_vm
                .vm_mut()
                .eval("__messagePortReadyEvents.join('|')")?;
            let host_load_source = Some(
                run_child_interactive_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "MessagePort-created child iframe load",
                )
                .await,
            );
            let events_after_host_load = page_vm
                .vm_mut()
                .eval("__messagePortReadyEvents.join('|')")?;

            Ok::<_, anyhow::Error>((
                completion_sources,
                message_port_turns,
                events_after_message,
                script_ready_source,
                events_after_script_ready,
                host_load_source,
                events_after_host_load,
            ))
        }))
        .await
        .expect("MessagePort ready-work source test should run");

    assert_eq!(
        message_port_turns, 1,
        "one Worker reply should be consumed by one typed MessagePort turn; completion sources: {completion_sources:?}"
    );
    assert_eq!(
        events_after_message, "message:go",
        "MessagePort handler should create the child frame without running its parser script inline"
    );
    assert_eq!(
        script_ready_source,
        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
        "MessagePort-created child parser work should follow its navigation commit"
    );
    assert_eq!(
        events_after_script_ready, "message:go|child-script:true",
        "child parser work should run on the later DocumentScriptReady turn"
    );
    assert_eq!(
        host_load_source,
        Some(ChildFrameSemanticTurnKind::HostLoad),
        "iframe load should remain a separate HostLoad turn after MessagePort dispatch"
    );
    assert_eq!(
        events_after_host_load, "message:go|child-script:true|frame-load",
        "iframe load should dispatch only on the HostLoad turn"
    );
}
#[tokio::test]
async fn page_vm_broadcast_channel_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/broadcast-ready-worker.js",
            "HTTP/1.1 200 OK",
            r#"
const sender = new BroadcastChannel("broadcast-ready-work");
sender.postMessage("go");
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (page_vm, _typed_resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            events_after_broadcast,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let worker_url = format!("{base_url}/broadcast-ready-worker.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__broadcastReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  globalThis.__broadcastReadyReceiver = new BroadcastChannel("broadcast-ready-work");
  __broadcastReadyReceiver.onmessage = (event) => {{
    __broadcastReadyEvents.push("message:" + event.data);
    const frame = document.createElement("iframe");
    frame.onload = () => __broadcastReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__broadcastReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  }};
  globalThis.__broadcastReadyWorker = new Worker("{worker_url}");
}})()
"#
                ))?;

                let mut completion_sources = Vec::new();
                let events_after_broadcast = loop {
                    if page_vm
                        .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::BroadcastChannel),
                            &loader,
                        )
                        .await?
                    {
                        completion_sources
                            .push(RendererOwnerResourceActivitySource::BroadcastChannel);
                        let events = page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;
                        if events == "message:go" {
                            break events;
                        }
                        continue;
                    }
                    if page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                            &loader,
                        )
                        .await?
                    {
                        completion_sources.push(RendererOwnerResourceActivitySource::Worker);
                        continue;
                    }
                    if !page_vm.has_ready_page_websocket_task_for_test() {
                        let websocket_arrival = page_vm.wait_for_page_websocket_task_for_test();
                        tokio::pin!(websocket_arrival);
                        tokio::time::timeout(Duration::from_secs(2), async {
                            tokio::select! {
                                arrived = &mut websocket_arrival => {
                                    assert!(
                                        arrived,
                                        "BroadcastChannel/worker completion sender should remain open"
                                    );
                                }
                                wake = owner_wake_rx.recv() => {
                                    assert!(
                                        wake.is_some(),
                                        "typed BroadcastChannel owner wake sender should remain open"
                                    );
                                }
                            }
                        })
                        .await
                        .expect("BroadcastChannel/worker work should arrive before timeout");
                        continue;
                    }
                    let completion_source = page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .expect("BroadcastChannel/worker completion should be ready");
                    completion_sources.push(completion_source);
                    let events = page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;
                    if events == "message:go" {
                        break events;
                    }
                    assert!(
                        completion_sources.len() < 16,
                        "BroadcastChannel handler should run after bounded completions; sources: {completion_sources:?}, events: {events}"
                    );
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "BroadcastChannel-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "BroadcastChannel-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready =
                    page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;
                let host_load_source = Some(
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "BroadcastChannel-created child iframe load",
                    )
                    .await,
                );
                let events_after_host_load =
                    page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_broadcast,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    events_after_host_load,
                ))
            })
            .await
            .expect("BroadcastChannel ready-work source test should run");

        assert!(
            completion_sources.contains(&RendererOwnerResourceActivitySource::BroadcastChannel),
            "BroadcastChannel handler should be driven by a BroadcastChannel completion: {completion_sources:?}"
        );
        assert_eq!(
            events_after_broadcast, "message:go",
            "BroadcastChannel handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "BroadcastChannel-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "message:go|child-script:true",
            "child parser work should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a separate HostLoad turn after BroadcastChannel dispatch"
        );
        assert_eq!(
            events_after_host_load, "message:go|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server
            .await
            .expect("BroadcastChannel ready-work worker server should finish");
    })
    .await;
}
#[tokio::test]
async fn child_document_write_nested_external_classic_blocks_domcontentloaded() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-written-classic.js",
            "HTTP/1.1 200 OK",
            "parent.__childWrittenEvents.push('external');".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (events_after_outer, source_load_source, events_after_external, final_events) =
            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let script_url = format!("{base_url}/child-written-classic.js");
                    page_vm.vm_mut().eval(&format!(
                        r#"
(() => {{
  globalThis.__childWrittenEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => __childWrittenEvents.push("load");
  frame.srcdoc = `<script>
    parent.__childWrittenEvents.push("outer");
    document.write("<script>parent.__childWrittenEvents.push('nested');<\\/script>");
    document.write("<script src='{script_url}'><\\/script>");
    document.addEventListener("DOMContentLoaded", () => parent.__childWrittenEvents.push("dcl"));
  <\/script>`;
  body.appendChild(frame);
}})()
"#,
                    ))?;
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "nested document.write child navigation commit",
                    )
                    .await;
                    run_expected_child_realm_materialization_for_wait(
                        &mut page_vm,
                        "nested document.write child realm",
                    )
                    .await;
                    assert_eq!(
                        page_vm
                            .run_next_child_frame_task_source_for_semantic_test()
                            .await,
                        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
                        "outer parser script should run before its written external source starts"
                    );
                    let events_after_outer =
                        page_vm.vm_mut().eval("__childWrittenEvents.join('|')")?;
                    let source_load_source = page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await;
                    if !page_vm
                        .page_resource_completion_queue()
                        .has_ready_completion()
                    {
                        let arrived = tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("written external completion should arrive before timeout");
                        assert!(
                            arrived,
                            "written external completion sender should remain open"
                        );
                    }
                    let _ = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "written external classic execution",
                    )
                    .await;
                    let events_after_external =
                        page_vm.vm_mut().eval("__childWrittenEvents.join('|')")?;
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "nested document.write child load",
                    )
                    .await;
                    let final_events = page_vm.vm_mut().eval("__childWrittenEvents.join('|')")?;

                    Ok::<_, anyhow::Error>((
                        events_after_outer,
                        source_load_source,
                        events_after_external,
                        final_events,
                    ))
                })
                .await
                .expect("nested child document.write test should run");

        assert_eq!(events_after_outer, "outer|nested");
        assert_eq!(
            source_load_source,
            Some(ChildFrameSemanticTurnKind::ClassicScriptSourceLoad),
            "the written external script should start before the parser resumes"
        );
        assert_eq!(events_after_external, "outer|nested|external");
        assert_eq!(final_events, "outer|nested|external|dcl|load");

        server
            .await
            .expect("nested child document.write server should finish");
    })
    .await;
}
#[tokio::test]
async fn child_document_close_defers_while_written_external_classic_blocks_parser() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-close-blocker.js",
            "HTTP/1.1 200 OK",
            "parent.__childCloseEvents.push('external:' + Boolean(document.getElementById('after-blocker')));"
                .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            events_after_close,
            source_load_source,
            events_after_external,
            final_events,
            tail_exists,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__childCloseEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "child-close-frame";
  document.body.appendChild(frame);
})()
"#,
                )?;
                materialize_child_realm_through_page_turn_for_test(
                    &mut page_vm,
                    "child-close-frame",
                )?;

                let script_url = format!("{base_url}/child-close-blocker.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  const frame = document.getElementById("child-close-frame");
  frame.onload = () => __childCloseEvents.push("load");
  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.addEventListener("DOMContentLoaded", () => __childCloseEvents.push("dcl"));
  childDocument.write(`<script src="{script_url}"><\/script><main id="after-blocker">tail</main>`);
  childDocument.close();
  __childCloseEvents.push("after-close");
}})()
"#,
                ))?;
                let events_after_close = page_vm.vm_mut().eval("__childCloseEvents.join('|')")?;
                let source_load_source =
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
                        "document.close delayed external classic source load",
                    )
                    .await;
                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child close blocker completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child close blocker completion sender should remain open"
                    );
                }
                let _ = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "document.close delayed external classic execution",
                )
                .await;
                let events_after_external =
                    page_vm.vm_mut().eval("__childCloseEvents.join('|')")?;
                run_child_interactive_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "document.close delayed child load",
                )
                .await;
                let final_events = page_vm.vm_mut().eval("__childCloseEvents.join('|')")?;
                let tail_exists = page_vm.vm_mut().eval(
                    "String(Boolean(document.getElementById('child-close-frame').contentDocument.getElementById('after-blocker')))",
                )?;

                Ok::<_, anyhow::Error>((
                    events_after_close,
                    source_load_source,
                    events_after_external,
                    final_events,
                    tail_exists,
                ))
            })
            .await
            .expect("child document.close delayed EOF test should run");

        assert_eq!(
            events_after_close, "after-close",
            "document.close() should return without executing or bypassing the blocker"
        );
        assert_eq!(
            source_load_source,
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad
        );
        assert_eq!(
            events_after_external, "after-close|external:false",
            "the external parser-blocking script must run before future markup is parsed"
        );
        assert_eq!(final_events, "after-close|external:false|dcl|load");
        assert_eq!(tail_exists, "true");

        server
            .await
            .expect("child close blocker server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_window_load_navigation_uses_navigation_commit_followup() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-load-navigate.html",
            "HTTP/1.1 200 OK",
            r#"
<script>
parent.__childLoadNavigationEvents.push("child-script");
const clickOrder = [];
addEventListener("click", event => {
  clickOrder.push("before");
  onclick = () => {
    clickOrder.push("property");
    return false;
  };
  event.stopPropagation();
});
onclick = () => clickOrder.push("stale");
addEventListener("click", () => clickOrder.push("after"));
const clickDispatchResult = document.dispatchEvent(new Event("click", {
  bubbles: true,
  cancelable: true
}));
parent.__childLoadNavigationEvents.push("click:" + clickOrder.join(",") + ":" + clickDispatchResult);
onerror = function(message, source, line, column, error, extra) {
  parent.__childLoadNavigationEvents.push([
    "child-error",
    arguments.length,
    message,
    source.endsWith("child-load-navigate.html"),
    line > 0,
    column > 0,
    error && error.message,
    extra === undefined
  ].join(":"));
  return true;
};
const marker = new Error("child-error-marker");
const errorDispatchResult = dispatchEvent(new ErrorEvent("error", {
  cancelable: true,
  message: marker.message,
  filename: location.href,
  lineno: 7,
  colno: 9,
  error: marker
}));
parent.__childLoadNavigationEvents.push("error-dispatch:" + errorDispatchResult);
addEventListener("load", () => parent.__childLoadNavigationEvents.push("load-listener-before"));
onload = () => {
  parent.__childLoadNavigationEvents.push("child-load");
  location.href = "data:text/html,<!doctype html><script>parent.postMessage('can navigate', '*')<\/script>";
};
addEventListener("load", () => parent.__childLoadNavigationEvents.push("load-listener-after"));
</script>
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let loader_for_task = loader.clone();
        let local_executor = page_vm.local_executor.clone();

        let (
            bootstrap_source,
            completion_source,
            script_ready_source,
            events_after_script_ready,
            interactive_source,
            host_load_source,
            events_after_host_load,
            pending_after_host_load,
            navigation_commit_source,
            events_after_navigation_commit,
            navigation_script_ready_source,
            events_after_navigation_script_ready,
            message_ready_after_navigation_script,
            message_endpoints_after_navigation_script,
            events_after_window_message,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child-load-navigate.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childLoadNavigationEvents = [];
  addEventListener("message", event => __childLoadNavigationEvents.push(event.data));
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;

                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(arrived, "child document completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child window-load realm prerequisite",
                )?;

                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;

                let interactive_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child window-load navigation initial load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;
                let pending_after_host_load = page_vm
                    .vm()
                    .has_pending_child_navigation_commit_for_test();
                let navigation_commit_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "child window load navigation commit",
                )
                .await;
                let events_after_navigation_commit = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child window-load navigation realm prerequisite",
                )?;
                let navigation_script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child window load navigation script",
                )
                .await;
                let events_after_navigation_script_ready = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;
                let message_endpoints_after_navigation_script =
                    page_vm.vm().pending_window_message_endpoints_for_test();
                let message_ready_after_navigation_script = page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage, &loader_for_task)
                    .await?;
                let events_after_window_message = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    bootstrap_source,
                    completion_source,
                    script_ready_source,
                    events_after_script_ready,
                    interactive_source,
                    host_load_source,
                    events_after_host_load,
                    pending_after_host_load,
                    navigation_commit_source,
                    events_after_navigation_commit,
                    navigation_script_ready_source,
                    events_after_navigation_script_ready,
                    message_ready_after_navigation_script,
                    message_endpoints_after_navigation_script,
                    events_after_window_message,
                ))
            })
            .await
            .expect("child load navigation host-load follow-up test should run");

        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "bootstrap child frame turn should commit navigation before lifecycle delivery"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildDocument
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady)
        );
        assert_eq!(
            events_after_script_ready,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false",
            "DocumentScriptReady should not dispatch child window load inline"
        );
        assert_eq!(
            interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "child document should become interactive before its window load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "child window load should dispatch on a HostLoad turn"
        );
        assert_eq!(
            events_after_host_load,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after",
            "HostLoad should invoke one registration-ordered child window load dispatch without finishing navigation inline"
        );
        assert!(
            pending_after_host_load,
            "child window load navigation should leave follow-up child frame work"
        );
        assert_eq!(
            navigation_commit_source,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "navigation queued by child window load should commit through NavigationCommit"
        );
        assert_eq!(
            events_after_navigation_commit,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after",
            "NavigationCommit should not deliver the child postMessage inline"
        );
        assert_eq!(
            navigation_script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "data-url navigation script should run from DocumentScriptReady"
        );
        assert_eq!(
            events_after_navigation_script_ready,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after",
            "navigation script should queue a parent window-message task without delivering it inline"
        );
        assert!(
            message_ready_after_navigation_script,
            "child navigation script should queue a ready parent window-message task"
        );
        assert_eq!(
            message_endpoints_after_navigation_script.len(),
            1,
            "child navigation script should queue exactly one window-message task: {message_endpoints_after_navigation_script:?}"
        );
        assert!(
            matches!(
                message_endpoints_after_navigation_script.as_slice(),
                [(
                    PendingWindowMessageEndpoint::TopWindow,
                    PendingWindowMessageEndpoint::ChildWindow(_)
                )]
            ),
            "child navigation script should target the top window from the child: {message_endpoints_after_navigation_script:?}"
        );
        assert_eq!(
            events_after_window_message,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after|can navigate",
            "advancing the ready window-message task should deliver the child navigation postMessage"
        );

        server
            .await
            .expect("child load navigation host-load follow-up server should finish");
    })
    .await;
}
