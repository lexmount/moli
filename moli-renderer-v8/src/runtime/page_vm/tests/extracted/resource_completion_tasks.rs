use super::*;

#[tokio::test(flavor = "current_thread")]
async fn page_resource_completion_rejects_stale_document_before_application() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .document_runtime
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete);
        park_current_document_websocket_for_test(
            &mut page_vm,
            moli_websocket::Event::TextMessage {
                socket_id: 72,
                data: "blocked".to_owned(),
            },
        )
        .await;
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("test page should install a main Document owner");
        let root_document = page_vm.document_lifecycle.identity().document;
        let stale_owner = FrameDocumentTaskOwner::new(
            current_owner.scheduler_lane_id,
            current_owner.local_window_id,
            DocumentId(current_owner.document_id.0 + 1),
        );
        let later_stale_owner = FrameDocumentTaskOwner::new(
            current_owner.scheduler_lane_id,
            current_owner.local_window_id,
            DocumentId(current_owner.document_id.0 + 2),
        );
        let completion_for = |owner, parser_position, node_id, network_error: Option<&str>| {
            MainParserDeferredClassicSourceLoadCompletion::new(
                ParserPendingScriptId::from_key(
                    MainParserDocumentOwner::new(owner),
                    ParserPendingScriptKey::from_parts_for_test(
                        parser_position,
                        NodeId::new(node_id),
                    ),
                ),
                PreparedScriptSourceLoadOutcome {
                    source_result: Ok("globalThis.__staleDeferRan = true".to_owned()),
                    source_bytes: None,
                    network_result: network_error.map(|error| Arc::new(Err(error.to_owned()))),
                    muted_errors: false,
                },
            )
        };
        let network_attribution_for = |parser_position| {
            MainParserDeferredClassicSourceNetworkAttribution::new(
                Url::parse("https://stale-defer.test/document").unwrap(),
                Url::parse(&format!(
                    "https://stale-defer.test/script-{parser_position}.js"
                ))
                .unwrap(),
            )
        };
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::main_parser_deferred_classic_source(
                root_document,
                completion_for(stale_owner, 1, 9, Some("stale defer request failed")),
                network_attribution_for(1),
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::main_parser_deferred_classic_source(
                root_document,
                completion_for(later_stale_owner, 2, 10, None),
                network_attribution_for(2),
            ),
        );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("stale completion arbitration should succeed")
            .expect("typed stale completion must remain runnable beside blocked WebSocket work");
        assert_eq!(
            outcome.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::MainParserDeferredClassicSource,
                owner: RendererPageResourceCompletionOwner::main_document(
                    root_document,
                    stale_owner,
                ),
                document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: Some(RendererPageResourceCompletionOwner::main_document(
                        root_document,
                        current_owner,
                    )),
                },
                body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
                output_effect: PageResourceCompletionOutputEffect::CaptureRequired,
            }
        );

        assert!(queue.has_ready_completion());

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("second stale completion arbitration should succeed")
            .expect("second stale completion should consume its own turn");
        assert_eq!(
            second.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::MainParserDeferredClassicSource,
                owner: RendererPageResourceCompletionOwner::main_document(
                    root_document,
                    later_stale_owner,
                ),
                document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: Some(RendererPageResourceCompletionOwner::main_document(
                        root_document,
                        current_owner,
                    )),
                },
                body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
                output_effect: PageResourceCompletionOutputEffect::None,
            }
        );

        assert!(!queue.has_ready_completion());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "stale defer Network output must not become current Document activity"
        );
        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 1);
        assert_eq!(
            network_records[0].document_url().as_str(),
            "https://stale-defer.test/document"
        );
        assert_eq!(
            network_records[0].url().as_str(),
            "https://stale-defer.test/script-1.js"
        );
        assert_eq!(
            network_records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale defer request failed".to_owned(),
            }
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__staleDeferRan)")
                .expect("replacement Document should remain observable"),
            "undefined",
            "stale completion must not execute or rediscover the current Document"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn stale_child_unusable_stylesheet_preserves_physical_output_without_document_effect() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let child_handle = crate::dom::native::NativeNodeId::new(404);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(7),
            crate::frame_owner_model::LocalWindowId(8),
            DocumentId(9),
        );
        let request_url = Url::parse("https://stale-child.test/style.css").unwrap();
        let physical_response = crate::protocol_types::NavigationResponse::from_text_body(
            request_url.clone(),
            200,
            vec![("Content-Type".to_owned(), b"text/html".to_vec())],
            "<html>not a stylesheet</html>".to_owned(),
        );
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_blocking_stylesheet(
            root_document,
            ChildBlockingStylesheetLoadCompletion {
                child_handle,
                owner: stale_owner,
                signature: crate::DocumentBlockingStylesheetSignature::ParserCreatedStyleImport {
                    urls: Vec::new(),
                },
                network_results: vec![ChildBlockingStylesheetNetworkResult {
                    frame_id: Some("stale-child-frame".to_owned()),
                    document_url: Url::parse("https://stale-child.test/document").unwrap(),
                    request_url,
                    initiator_type: SubresourceRequestInitiatorType::Parser,
                    terminal:
                        crate::stylesheet_blocking::StylesheetFetchTerminal::unusable_response(
                            physical_response,
                            false,
                            "stylesheet MIME validation failed",
                        ),
                }],
            },
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("stale child completion arbitration should succeed")
            .expect("stale child completion should consume one bounded turn");
        assert_eq!(
            outcome.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::ChildBlockingStylesheet,
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
            "a retired Document's Network fact must not become activity of the current Document"
        );
        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(
            network_records.len(),
            1,
            "stale child Document must not suppress or duplicate the completed Network fact"
        );
        let network_record = &network_records[0];
        assert_eq!(network_record.frame_id(), Some("stale-child-frame"));
        assert_eq!(
            network_record.document_url().as_str(),
            "https://stale-child.test/document"
        );
        assert_eq!(
            network_record.url().as_str(),
            "https://stale-child.test/style.css"
        );
        assert_eq!(
            network_record.resource_type(),
            SubresourceResourceType::Stylesheet
        );
        assert_eq!(
            network_record.request_initiator_type(),
            SubresourceRequestInitiatorType::Parser
        );
        assert!(matches!(
            network_record.outcome(),
            SubresourceNetworkOutcome::Success { status: 200, .. }
        ));
    })
    .await;
}
#[test]
fn parser_eof_releases_preloads_only_for_the_current_document_transition() {
    fn ready_store(url: Url) -> crate::runtime::DocumentScriptPreloadStore {
        let request = crate::runtime::BufferedScriptPreloadRequest {
            document_referrer_policy: None,
            url: url.clone(),
            initiator_url: url,
            kind_hint: ScriptKind::Classic,
            mode_hint: ScriptMode::Normal,
            resource_type_hint: moli_fetch::RequestResourceType::ParserBlockingScript,
            fetch_metadata: ScriptFetchMetadata::default(),
        };
        let mut store = crate::runtime::DocumentScriptPreloadStore::default();
        store.insert(
            request.cache_key(),
            crate::runtime::script_preloads::BufferedScriptPreloadEntry {
                request,
                load: SharedScriptSourceLoad::ready_ok("window.ready = true;"),
            },
        );
        store
    }

    let mut page_vm = test_page_vm();
    let owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("test page should have a main document owner");
    let live_store =
        ready_store(Url::parse("https://example.test/preloaded.js").expect("preload URL"));
    let live_store_observer = live_store.clone();
    page_vm
        .vm_mut()
        .document_runtime
        .bind_main_document_script_preload_store(live_store);

    assert!(!live_store_observer.is_empty());
    assert!(
        page_vm
            .vm_mut()
            .finish_current_main_document_parsing(owner)
            .is_some()
    );
    assert!(live_store_observer.is_empty());

    let stale_store =
        ready_store(Url::parse("https://example.test/stale.js").expect("preload URL"));
    let stale_store_observer = stale_store.clone();
    page_vm
        .vm_mut()
        .document_runtime
        .bind_main_document_script_preload_store(stale_store);

    assert!(
        page_vm
            .vm_mut()
            .finish_current_main_document_parsing(owner)
            .is_none()
    );
    assert!(
        !stale_store_observer.is_empty(),
        "a stale parser completion must not clear the current preload store"
    );
}
#[tokio::test]
async fn page_vm_child_mixed_parser_defer_waits_for_preceding_stylesheet() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/mixed-defer.css",
                "HTTP/1.1 200 OK",
                ":root { --child-defer-style-gate: stylesheet-ready; }".to_owned(),
                Duration::from_millis(600),
            ),
            (
                "/stylesheet-classic-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDeferStylesheetEvents.push(
  "classic:" + getComputedStyle(document.documentElement)
    .getPropertyValue("--child-defer-style-gate").trim()
);
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/stylesheet-module-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDeferStylesheetEvents.push(
  "module:" + getComputedStyle(document.documentElement)
    .getPropertyValue("--child-defer-style-gate").trim()
);
"#
                .to_owned(),
                Duration::from_millis(20),
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            mut script_completion_sources,
            stylesheet_completion,
            execution_sources,
            lifecycle_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childDeferStylesheetEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childDeferStylesheetEvents.push("load");
  frame.srcdoc = `
    <link rel="stylesheet" href="{base_url}/mixed-defer.css">
    <script defer src="{base_url}/stylesheet-classic-defer.js"><\/script>
    <script type="module" src="{base_url}/stylesheet-module-defer.js"><\/script>
    <body></body>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    if bootstrap_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        && bootstrap_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                    {
                        break;
                    }
                }
                assert!(
                    bootstrap_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        && bootstrap_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle),
                    "child parser must start the module root fetch and reach interactive while classic defer fetches directly: {bootstrap_sources:?}"
                );

                let mut script_completion_sources = Vec::new();
                while script_completion_sources.len() != 2 {
                    if !page_vm.page_resource_completion_queue().has_ready_completion() {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("child deferred script completion should arrive");
                    }
                    let completion =
                        run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    assert_ne!(
                        completion.action.source(),
                        RendererOwnerResourceActivitySource::ChildBlockingStylesheet,
                        "delayed stylesheet must remain pending while both script sources become terminal"
                    );
                    script_completion_sources.push(completion.action.source());
                    if completion.action.source()
                        == RendererOwnerResourceActivitySource::ModuleGraphFetch
                    {
                        run_expected_child_module_script_terminal_turn(
                            &mut page_vm,
                            "stylesheet-blocked child module terminal",
                        )
                        .await;
                    }
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__childDeferStylesheetEvents.join('|')")?,
                        "",
                        "terminal script sources must remain retained behind the stylesheet snapshot"
                    );
                }
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "no script or HostLoad turn should be runnable before stylesheet completion"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("delayed child stylesheet completion should arrive");
                }
                let stylesheet_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childDeferStylesheetEvents.join('|')")?,
                    "",
                    "stylesheet completion must not inline-execute retained deferred scripts"
                );

                let execution_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "stylesheet-released child classic defer",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "stylesheet-released child module defer",
                    )
                    .await,
                ];
                let lifecycle_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-released child DOMContentLoaded",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-released child complete transition",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "stylesheet-released child iframe load",
                    )
                    .await,
                ];
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childDeferStylesheetEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    script_completion_sources,
                    stylesheet_completion,
                    execution_sources,
                    lifecycle_sources,
                    final_events,
                ))
            })
            .await
            .expect("mixed child defer stylesheet test should run");

        script_completion_sources.sort_by_key(|source| match source {
            RendererOwnerResourceActivitySource::ChildClassicScript => 0,
            RendererOwnerResourceActivitySource::ModuleGraphFetch => 1,
            _ => 2,
        });
        assert_eq!(
            script_completion_sources,
            vec![
                RendererOwnerResourceActivitySource::ChildClassicScript,
                RendererOwnerResourceActivitySource::ModuleGraphFetch,
            ]
        );
        assert_eq!(
            stylesheet_completion.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert_eq!(
            execution_sources,
            vec![ChildFrameSemanticTurnKind::DocumentScriptReady; 2]
        );
        assert_eq!(
            lifecycle_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(
            final_events,
            "classic:stylesheet-ready|module:stylesheet-ready|load",
            "mixed parser-deferred scripts must preserve document order and observe the installed stylesheet"
        );
        server
            .await
            .expect("mixed child defer stylesheet server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_failed_child_stylesheet_installs_empty_sheet_before_host_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![(
            "/load-only.css",
            "HTTP/1.1 404 Not Found",
            ".failed-body { color: rgb(11, 12, 13); }".to_owned(),
            Duration::from_millis(220),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (completion, child_stylesheet_surface, lifecycle_sources, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childStylesheetHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childStylesheetHostLoadEvents.push("load");
  frame.srcdoc = `<link rel="stylesheet" href="{base_url}/load-only.css"><body class="failed-body"></body>`;
  body.appendChild(frame);
}})()
"#
                ))?;

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "stylesheet-only child navigation commit",
                )
                .await;
                let lifecycle_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-only child interactive transition",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-only child DOMContentLoaded transition",
                    )
                    .await,
                ];
                assert!(
                    !page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildHostLoad,
                            &loader,
                        )
                        .await?,
                    "HostLoad must remain blocked after DCL while a child stylesheet is pending"
                );
                assert_eq!(
                    page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await,
                    None,
                    "blocked stylesheet lifecycle must not produce HostLoad progress"
                );

                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("failed stylesheet-only completion should arrive");
                }
                let completion = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let child_context_id = page_vm
                    .live_child_default_runtime_realm_inventory()
                    .into_iter()
                    .map(|realm| realm.context_id)
                    .next()
                    .expect("stylesheet child should have one materialized default realm");
                let child_stylesheet_surface = page_vm.vm_mut().eval_in_child_default_context(
                    child_context_id,
                    r#"
(() => {
  const link = document.querySelector("link");
  return JSON.stringify({
    sheet: link.sheet !== null,
    owner: link.sheet && link.sheet.ownerNode === link,
    count: document.styleSheets.length,
    color: getComputedStyle(document.body).color,
  });
})()
"#,
                )?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childStylesheetHostLoadEvents.join('|')")?,
                    "",
                    "stylesheet completion must not inline-dispatch iframe load"
                );
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "stylesheet-terminal child complete transition",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::HostLoad,
                    "stylesheet-terminal child iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childStylesheetHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    completion,
                    child_stylesheet_surface,
                    lifecycle_sources,
                    final_events,
                ))
            })
            .await
            .expect("stylesheet-only child HostLoad test should run");

        assert_eq!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert_eq!(
            child_stylesheet_surface,
            r#"{"sheet":true,"owner":true,"count":1,"color":"rgb(0, 0, 0)"}"#,
            "a failed child stylesheet must expose the same empty sheet surface as Chromium and the main document"
        );
        assert_eq!(
            lifecycle_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
            ]
        );
        assert_eq!(final_events, "load");
        server
            .await
            .expect("stylesheet-only child HostLoad server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_stylesheet_fonts_block_complete_until_the_exact_terminal() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/blocking-resources.css",
                "HTTP/1.1 200 OK",
                r#"
body { background-image: url('/css-image.png'); }
@font-face { font-family: Demo; src: url('/demo.woff2') format('woff2'); }
"#
                .to_owned(),
                Duration::from_millis(80),
            ),
            (
                "/demo.woff2",
                "HTTP/1.1 200 OK",
                "font-body".to_owned(),
                Duration::from_millis(420),
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::IMAGE
                | crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (stylesheet_outcome, lifecycle_ready_after_font, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childCssResourceEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childCssResourceEvents.push("load");
  frame.srcdoc = `<link rel="stylesheet" href="{base_url}/blocking-resources.css"><body></body>`;
  body.appendChild(frame);
}})()
"#
                ))?;

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "CSS-resource child navigation commit",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "CSS-resource child interactive transition",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "CSS-resource child DOMContentLoaded transition",
                )
                .await;

                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("stylesheet completion should arrive");
                }
                let stylesheet_outcome =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert!(
                    !page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildHostLoad,
                            &loader,
                        )
                        .await?,
                    "HostLoad must remain blocked while stylesheet subresources are pending"
                );

                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("stylesheet font terminal should arrive");
                }
                let outcome = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    outcome.action.source(),
                    RendererOwnerResourceActivitySource::AsyncSubresource,
                    "the stylesheet font terminal must use Networking"
                );
                let lifecycle_ready_after_font = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childCssResourceEvents.join('|')")?,
                    "",
                    "the font terminal must not inline-dispatch iframe load"
                );

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "CSS-resource child complete transition",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::HostLoad,
                    "CSS-resource child iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childCssResourceEvents.join('|')")?;
                assert_eq!(
                    page_vm.vm().css_image_resource_observability_for_test(),
                    (0, 0, 0, 0, Vec::<String>::new()),
                    "raw stylesheet text must not eagerly admit CSS image slots before paint layout"
                );
                Ok::<_, anyhow::Error>((
                    stylesheet_outcome,
                    lifecycle_ready_after_font,
                    final_events,
                ))
            })
            .await
            .expect("stylesheet subresource lifecycle test should run");

        assert_eq!(
            stylesheet_outcome.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert!(
            lifecycle_ready_after_font,
            "the exact stylesheet font token must make complete runnable"
        );
        assert_eq!(final_events, "load");
        server
            .await
            .expect("stylesheet subresource server should finish");
    })
    .await;
}
