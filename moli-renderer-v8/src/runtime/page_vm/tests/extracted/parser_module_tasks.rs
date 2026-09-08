use super::*;

#[tokio::test]
async fn page_vm_moved_child_defer_disposes_in_flight_slot_before_later_module() {
    run_page_vm_async_test(async move {
        let (base_url, release_classic, server) = spawn_moved_child_defer_http_server().await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (events_after_dispose, sources, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/moved-child-defer.js");
                let module_url = format!("{base_url}/later-moved-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__movedChildDeferEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "moved-defer-frame";
  frame.onload = () => globalThis.__movedChildDeferEvents.push("load");
  frame.srcdoc = `
    <script>parent.__movedChildDeferEvents.push("before");<\/script>
    <script id="move-defer" defer src="{script_url}"><\/script>
    <script id="later-module" type="module" src="{module_url}"><\/script>
    <script>
      document.getElementById("move-defer").addEventListener("load", () => {{
        parent.__movedChildDeferEvents.push("classic-load");
      }});
      document.getElementById("later-module").addEventListener("load", () => {{
        parent.__movedChildDeferEvents.push("module-load");
      }});
      document.addEventListener("readystatechange", () => {{
        parent.__movedChildDeferEvents.push("ready:" + document.readyState);
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__movedChildDeferEvents.push("dcl");
      }});
      parent.__movedChildDeferEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                for _ in 0..12 {
                    let Some(_) = page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await
                    else {
                        break;
                    };
                }
                assert_eq!(
                    page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?,
                    "before|after|ready:interactive",
                    "later module must remain retained behind the unresolved classic defer"
                );

                let mut classic_completion = None;
                let mut module_completion = false;
                let mut release_classic = Some(release_classic);
                for _ in 0..4 {
                    if !page_vm
                        .page_resource_completion_queue()
                        .has_ready_completion()
                    {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("moved child defer completion should arrive");
                    }
                    let completion = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    if matches!(
                        completion.action.source(),
                        RendererOwnerResourceActivitySource::ChildClassicScript
                    ) {
                        classic_completion = Some(completion);
                    } else {
                        assert!(
                            matches!(
                                completion.action.source(),
                                RendererOwnerResourceActivitySource::ModuleGraphFetch
                            ),
                            "the only other completion is the later module root"
                        );
                        run_expected_child_module_script_terminal_turn(
                            &mut page_vm,
                            "module terminal retained behind the moved classic defer",
                        )
                        .await;
                        module_completion = true;
                        // Make the later module ready while the classic source
                        // is still pending, without relying on transport timing.
                        release_classic
                            .take()
                            .expect("one module completion")
                            .send(())
                            .expect("classic response should still be gated");
                    }
                    if classic_completion.is_some() && module_completion {
                        break;
                    }
                }
                classic_completion
                    .expect("classic source completion must arrive before the script is moved");
                assert!(
                    module_completion,
                    "later module must be ready before testing defer-slot release"
                );
                page_vm.vm_mut().eval(
                    r#"
(() => {
  const frame = document.getElementById("moved-defer-frame");
  document.body.appendChild(frame.contentDocument.getElementById("move-defer"));
  __movedChildDeferEvents.push("moved");
})()
"#,
                )?;

                let mut sources = Vec::new();
                sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "moved classic defer disposal",
                    )
                    .await,
                );
                let events_after_dispose =
                    page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?;
                assert_eq!(
                    events_after_dispose, "before|after|ready:interactive|moved",
                    "disposed classic defer must not execute or dispatch load"
                );
                sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "module after moved classic defer",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?,
                    "before|after|ready:interactive|moved|module-ran|module-load",
                    "disposing the exact classic slot must release the later module-defer"
                );
                for (source, label) in [
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "moved defer DOMContentLoaded",
                    ),
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "moved defer complete transition",
                    ),
                    (
                        ChildFrameSemanticTurnKind::HostLoad,
                        "moved defer iframe load",
                    ),
                ] {
                    sources.push(
                        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                            &mut page_vm,
                            source,
                            label,
                        )
                        .await,
                    );
                }
                let final_events = page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?;
                assert_eq!(
                    page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await,
                    None
                );
                Ok::<_, anyhow::Error>((events_after_dispose, sources, final_events))
            })
            .await
            .expect("moved child defer cancellation test should run");

        assert_eq!(events_after_dispose, "before|after|ready:interactive|moved");
        assert_eq!(
            sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(
            final_events,
            "before|after|ready:interactive|moved|module-ran|module-load|dcl|ready:complete|load"
        );
        server
            .await
            .expect("moved child defer server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_joined_parser_roots_batch_one_module_script_terminal() {
    run_page_vm_async_test(async move {
        let (base_url, shutdown_server_tx, server) = spawn_shutdown_path_response_http_server(vec![(
            "/child-shared-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childJoinedParserRootEvents.push("module:" + (globalThis === self));
globalThis.__childJoinedParserRootValue = 501;
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            pre_completion_sources,
            events_before_completion,
            resource_ready_before_wait,
            completion_source,
            events_after_module_owner,
            first_ready_source,
            events_after_first_ready,
            second_ready_source,
            events_after_second_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-shared-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childJoinedParserRootEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childJoinedParserRootEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childJoinedParserRootEvents.push("before:" + (globalThis === self));<\/script>
    <script id="joined-module-a" type="module" src="{script_url}"><\/script>
    <script id="joined-module-b" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("joined-module-a").addEventListener("load", () => {{
        parent.__childJoinedParserRootEvents.push("script-a-load");
      }});
      document.getElementById("joined-module-b").addEventListener("load", () => {{
        parent.__childJoinedParserRootEvents.push("script-b-load");
      }});
      parent.__childJoinedParserRootEvents.push(
        "after:" + String(globalThis.__childJoinedParserRootValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut pre_completion_sources = Vec::new();
                let mut events_before_completion = String::new();
                for _ in 0..8 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childJoinedParserRootEvents.join('|')")?;
                    if let Some(source) = source {
                        pre_completion_sources.push(source);
                    }
                    if source.is_none()
                        || page_vm.has_ready_page_websocket_task_for_test()
                        || events_before_completion.contains("frame-load")
                    {
                        break;
                    }
                }
                let resource_ready_before_wait = page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion();

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("joined child module completion should arrive before timeout");
                    assert!(
                        arrived,
                        "joined child module completion sender should remain open"
                    );
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "joined parser-root terminal fanout",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                let first_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_first_ready = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                let second_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_second_ready = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "joined child parser-module iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    first_ready_source,
                    events_after_first_ready,
                    second_ready_source,
                    events_after_second_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm joined child parser roots test should run");

        assert_eq!(
            pre_completion_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                .count(),
            2,
            "each parser root should get a reserve/join root-fetch source turn: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed shared module response should leave a window to prove joined roots wait for one completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "joined child parser roots should not evaluate before the shared fetch completes"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after:undefined",
            "module owner event should only enqueue document-script ready work"
        );
        assert_eq!(
            first_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "first joined parser root should execute on a document-script ready turn"
        );
        assert_eq!(
            events_after_first_ready,
            "before:true|after:undefined|module:true|script-a-load",
            "first ready turn should evaluate the shared module and dispatch the first script load"
        );
        assert_eq!(
            second_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "second joined parser root should execute on a later document-script ready turn"
        );
        assert_eq!(
            events_after_second_ready,
            "before:true|after:undefined|module:true|script-a-load|script-b-load",
            "second ready turn should dispatch the second script load without re-evaluating the module"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source after joined parser roots"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|module:true|script-a-load|script-b-load|frame-load",
            "HostLoad should dispatch iframe load after both joined parser roots complete"
        );

        shutdown_server_tx
            .send(())
            .expect("joined child parser roots server shutdown should send");
        let requested_paths = server
            .await
            .expect("joined child parser roots server should finish");
        assert_eq!(
            requested_paths,
            vec!["/child-shared-module.js"],
            "joined parser roots should reserve twice but share one network module fetch"
        );
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_async_module_allows_dcl_before_source_completion() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-async-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childAsyncModuleEvents.push("module:" + (globalThis === self));
globalThis.__childAsyncModuleValue = 307;
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            pre_completion_sources,
            events_at_dcl,
            resource_ready_at_dcl,
            post_dcl_sources_before_completion,
            completion_source,
            document_script_source,
            events_after_module,
            complete_source,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-async-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childAsyncModuleEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childAsyncModuleEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childAsyncModuleEvents.push("before");
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childAsyncModuleEvents.push("dcl");
      }});
    <\/script>
    <script id="async-module" type="module" async src="{script_url}"><\/script>
    <script>
      document.getElementById("async-module").addEventListener("load", () => {{
        parent.__childAsyncModuleEvents.push("script-load");
      }});
      parent.__childAsyncModuleEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut pre_completion_sources = Vec::new();
                let mut events_at_dcl = String::new();
                for _ in 0..12 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    if let Some(source) = source {
                        pre_completion_sources.push(source);
                    }
                    events_at_dcl = page_vm
                        .vm_mut()
                        .eval("__childAsyncModuleEvents.join('|')")?;
                    if events_at_dcl.contains("dcl") || source.is_none() {
                        break;
                    }
                }
                let resource_ready_at_dcl =
                    page_vm.has_ready_page_websocket_task_for_test();
                let mut post_dcl_sources_before_completion = Vec::new();
                for _ in 0..4 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    post_dcl_sources_before_completion.push(source);
                }

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child async module completion should arrive before timeout");
                    assert!(arrived, "child async module completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child async-module terminal",
                )
                .await;
                let document_script_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_module = page_vm
                    .vm_mut()
                    .eval("__childAsyncModuleEvents.join('|')")?;
                let complete_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childAsyncModuleEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_at_dcl,
                    resource_ready_at_dcl,
                    post_dcl_sources_before_completion,
                    completion_source,
                    document_script_source,
                    events_after_module,
                    complete_source,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child async module lifecycle test should run");

        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle),
            "the child document should reach an explicit lifecycle turn before async module completion: {pre_completion_sources:?}"
        );
        assert!(
            !pre_completion_sources.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "the async module load-delay token must keep HostLoad unavailable before source completion: {pre_completion_sources:?}"
        );
        assert_eq!(events_at_dcl, "before|after|dcl");
        assert!(
            !resource_ready_at_dcl,
            "DCL should run while the delayed async module source is still pending"
        );
        assert!(
            pre_completion_sources
                .contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "the async root fetch must start when the parser encounters the module, before DCL observes parser completion: {pre_completion_sources:?}"
        );
        assert!(
            !post_dcl_sources_before_completion
                .contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "one async parser module must publish exactly one root-start task: before={pre_completion_sources:?}, after={post_dcl_sources_before_completion:?}"
        );
        assert!(
            !post_dcl_sources_before_completion
                .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                && !post_dcl_sources_before_completion.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "the async-module token should block complete and HostLoad without blocking DCL: {post_dcl_sources_before_completion:?}"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            document_script_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady)
        );
        assert_eq!(
            events_after_module,
            "before|after|dcl|module:true|script-load",
            "async module execution should release complete without redispatching DCL"
        );
        assert_eq!(
            complete_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle)
        );
        assert_eq!(host_load_source, Some(ChildFrameSemanticTurnKind::HostLoad));
        assert_eq!(
            final_events,
            "before|after|dcl|module:true|script-load|frame-load"
        );

        server
            .await
            .expect("child async module lifecycle server should finish");
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn parser_inline_module_runs_from_parser_after_parsing_order() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(
            "https://example.com/page.html?case=inline-module#document-fragment",
        )
        .expect("document URL");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            document_url.clone(),
        );
        let script = prepared_inline_module_for_page_vm_test(
            &page_vm,
            9002,
            "globalThis.__inlineParserModuleExecuted = (globalThis.__inlineParserModuleExecuted ?? 0) + 1; globalThis.__inlineParserModuleUrl = import.meta.url; export const value = 1;",
        );

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__inlineParserModuleExecuted)")
                .expect("read module side effect before page task"),
            "undefined",
            "prewarmed inline graph must not evaluate before parser after-parsing release"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should release inline graph");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "inline parser module after-parsing release",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__inlineParserModuleExecuted)")
                .expect("read module side effect after page task"),
            "1",
            "parser after-parsing release should evaluate the prewarmed inline graph once"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__inlineParserModuleUrl")
                .expect("read inline module URL after page task"),
            document_url.as_str(),
            "an inline module must expose its document base URL, not its internal module-map key"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn parser_data_url_module_runs_from_parser_after_parsing_order() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse(
            "data:text/javascript,globalThis.__dataParserModuleExecuted%3D1%3Bexport%20const%20value%3D1%3B",
        )
        .expect("data module URL");
        let script = prepared_external_module_for_page_vm_test(&page_vm, module_url);

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__dataParserModuleExecuted)")
                .expect("read data module side effect before page task"),
            "undefined",
            "prepared data graph must not evaluate before parser after-parsing release"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should release the data module graph");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "data parser module after-parsing release",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__dataParserModuleExecuted)")
                .expect("read data module side effect after page task"),
            "1",
            "parser after-parsing release should evaluate the prepared data graph once"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn parser_pending_module_tree_fetch_completion_waits_for_after_parsing_release() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/module.mjs").expect("module URL");
        let script = prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__pendingTreeExecuted = (globalThis.__pendingTreeExecuted ?? 0) + 1; export const value = 1;",
        );

        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("module graph completion should run")
                .is_some(),
            "module graph completion should be consumed"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeExecuted)")
                .expect("read module side effect before page task"),
            "undefined",
            "completed fetch tree must not evaluate before the parser script task watches it"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should release the pending tree");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "completed parser module after-parsing release",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeExecuted)")
                .expect("read module side effect after page task"),
            "1",
            "parser script task watch should evaluate the completed pending tree once"
        );
        assert!(
            page_vm
                .report
                .runs
                .iter()
                .any(|run| run.url() == &module_url),
            "module script run should be reported after watch: {:?}",
            page_vm.report.runs
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn watched_loading_parser_pending_module_tree_runs_after_fetch_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/late-module.mjs").expect("module URL");
        let script = prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should retain the loading pending tree");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeLateExecuted)")
                .expect("read module side effect before fetch completion"),
            "undefined",
            "loading watch must wait for the fetch tree completion"
        );

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__pendingTreeLateExecuted = (globalThis.__pendingTreeLateExecuted ?? 0) + 1; export const value = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("module graph completion should run")
                .is_some(),
            "module graph completion should be consumed"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "after-parsing terminal must stay in its PendingScript instead of the broad ready queue"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "late parser module graph completion",
        )
        .await;
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "late parser module graph completion",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeLateExecuted)")
                .expect("read module side effect after fetch completion"),
            "1",
            "fetch completion after watch should evaluate the pending tree once"
        );
        assert!(
            page_vm
                .report
                .runs
                .iter()
                .any(|run| run.url() == &module_url),
            "module script run should be reported after late completion: {:?}",
            page_vm.report.runs
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn document_replacement_drops_parser_pending_module_tree_owner() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/replaced-module.mjs")
            .expect("module URL");
        let module_script = prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, module_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("module defer page task should watch loading pending tree");
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "module tree should be waiting for its root fetch before replacement"
        );
        let stale_target = page_vm
            .vm()
            .current_main_parser_module_graph_fetch_target(0)
            .expect("loading parser root must expose its exact terminal target");

        let replacement_script = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9100,
            "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        );
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                classic_defer_work(replacement_script),
            )
            .await
            .expect("replacement script task should run");

        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "document replacement must drop parser-owned module fetch continuations"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "document replacement must drop parser pending module scripts"
        );

        let replacement_document_url = page_vm.vm().document_runtime.document_url().clone();
        page_vm
            .vm()
            .resource_completion_sender_for_test()
            .send_main_parser_module_graph_fetch(MainParserModuleGraphFetchCompletion::new(
                stale_target,
                Ok(ModuleGraphFetchedSource::new(
                    module_url.clone(),
                    false,
                    ModuleSource::text(
                        "globalThis.__oldModuleAfterReplacement = true; export const value = 1;"
                            .to_owned(),
                    ),
                )),
                None,
                MainModuleFetchNetworkAttribution::new(
                    replacement_document_url,
                    module_url.clone(),
                ),
            ))
            .expect("stale exact parser terminal should still enter the stable Page source");
        let outcome = run_next_resource_completion_as_typed_page_turn(&mut page_vm)
            .expect("old module graph completion should consume one typed turn");
        assert!(
            matches!(
                outcome.action.document_effect,
                PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
            ),
            "old exact parser target must be rejected before module-map application: {outcome:?}"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__oldModuleAfterReplacement)")
                .expect("read old module side effect"),
            "undefined",
            "old parser module tree must not execute after document replacement"
        );
        assert!(
            page_vm
                .capture_page_state()
                .expect("page state capture")
                .report
                .lifecycle_errors()
                .is_empty(),
            "typed stale-owner rejection is an expected terminal outcome, not a lifecycle error"
        );
    })
    .await;
}
