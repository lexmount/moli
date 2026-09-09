use super::*;

#[tokio::test(flavor = "current_thread")]
async fn child_modulepreload_start_rejects_replaced_realm_with_same_document_owner() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const frame = document.createElement('iframe'); \
             frame.id = 'modulepreload-start-realm-replacement'; \
             document.body.appendChild(frame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("modulepreload-start-realm-replacement")
            .expect("realm replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "modulepreload-start-realm-replacement",
        )?;
        let retired_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("first child realm should have an exact module target");
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start()
            .enqueue_local_for_test(
                root_document,
                test_child_modulepreload_start_task_for_target(
                    retired_target,
                    "retired-modulepreload-start-realm",
                ),
            );

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "modulepreload-start-realm-replacement",
        )?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("replacement child realm should have an exact module target");
        assert_eq!(current_target.task_owner(), retired_target.task_owner());
        assert_ne!(current_target.realm_id(), retired_target.realm_id());

        let outcome = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("stale-realm start should consume exactly one bounded turn");
        assert_eq!(
            outcome.action.owner,
            RendererPageModulepreloadStartOwner::new(root_document, retired_target)
        );
        assert_eq!(
            outcome.action.document_effect,
            PageModulepreloadStartDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageModulepreloadStartOwner::new(
                    root_document,
                    current_target,
                )),
            }
        );

        assert!(
            !page_vm
                .page_task_executor_sources_for_test()
                .modulepreload_start()
                .has_ready_task(),
            "a stale typed start must consume only its stable source head"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Document stale modulepreload start realm test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn child_modulepreload_start_rejects_reused_local_owner_from_retired_root_document() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const frame = document.createElement('iframe'); \
             frame.id = 'modulepreload-start-root-replacement'; \
             document.body.appendChild(frame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("modulepreload-start-root-replacement")
            .expect("root replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "modulepreload-start-root-replacement",
        )?;
        let reused_local_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("current PageVm should expose an exact child target");
        let current_root = page_vm.document_lifecycle.identity().document;
        let retired_root = current_root.successor_for_testing();
        page_vm.page_task_executor_sources_for_test().modulepreload_start()
            .enqueue_local_for_test(
                retired_root,
                test_child_modulepreload_start_task_for_target(
                    reused_local_target,
                    "retired-root-modulepreload-start",
                ),
            );

        let outcome = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("retired-root start should consume one stale turn");
        assert_eq!(
            outcome.action.owner,
            RendererPageModulepreloadStartOwner::new(retired_root, reused_local_target)
        );
        assert_eq!(
            outcome.action.document_effect,
            PageModulepreloadStartDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageModulepreloadStartOwner::new(
                    current_root,
                    reused_local_target,
                )),
            },
            "PageVm-local owner counters may collide after replacement, so the root token must remain part of authorization"
        );

        assert!(
            !page_vm.page_task_executor_sources_for_test().modulepreload_start()
                .has_ready_task(),
            "a root-stale typed start must consume only its stable source head"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("retired-root modulepreload start test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn modulepreload_start_source_consumes_one_exact_owner_per_turn_in_fifo_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let first_target = ChildDocumentModuleFetchTarget::new(
            crate::dom::native::NativeNodeId::new(701),
            FrameDocumentTaskOwner::new(
                crate::frame_owner_model::FrameSchedulerLaneId(703),
                crate::frame_owner_model::LocalWindowId(709),
                DocumentId(719),
            ),
            FrameRealmId(727),
        );
        let second_target = ChildDocumentModuleFetchTarget::new(
            crate::dom::native::NativeNodeId::new(733),
            FrameDocumentTaskOwner::new(
                crate::frame_owner_model::FrameSchedulerLaneId(739),
                crate::frame_owner_model::LocalWindowId(743),
                DocumentId(751),
            ),
            FrameRealmId(757),
        );
        let source = page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start();
        source.enqueue_local_for_test(
            root_document,
            test_child_modulepreload_start_task_for_target(first_target, "first-start-turn"),
        );
        source.enqueue_local_for_test(
            root_document,
            test_child_modulepreload_start_task_for_target(second_target, "second-start-turn"),
        );

        let first = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("first start should consume one turn");
        assert_eq!(
            first.action.owner,
            RendererPageModulepreloadStartOwner::new(root_document, first_target)
        );
        assert!(matches!(
            first.action.document_effect,
            PageModulepreloadStartDocumentEffect::DiscardedStaleOwner {
                current_owner: None
            }
        ));

        let second = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("second start should consume a separate turn");
        assert_eq!(
            second.action.owner,
            RendererPageModulepreloadStartOwner::new(root_document, second_target)
        );

        assert!(!source.has_ready_task());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("modulepreload start FIFO test should run");
}
#[tokio::test]
async fn page_vm_child_modulepreload_terminal_event_does_not_delay_complete() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (
            lifecycle_turns,
            events_before_preload_event,
            ready_state_before_preload_event,
            events_after_preload_event,
            ready_state_after_preload_event,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__childModulepreloadTerminalEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModulepreloadTerminalEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childModulepreloadTerminalEvents.push("before");
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childModulepreloadTerminalEvents.push("dcl:" + document.readyState);
      });
    <\/script>
    <link rel="modulepreload" href="/invalid-modulepreload.bin" as="image" onerror="parent.__childModulepreloadTerminalEvents.push('preload-error')">
    <script>parent.__childModulepreloadTerminalEvents.push("after");<\/script>
  `;
  body.appendChild(frame);
})()
"#,
                )?;

                let mut lifecycle_turns = Vec::new();
                while let Some(source) = page_vm
                    .run_next_child_frame_task_source_for_semantic_test()
                    .await
                {
                    lifecycle_turns.push(source);
                }

                let events_before_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadTerminalEvents.join('|')")?;
                let ready_state_before_preload_event = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument?.readyState || 'missing'",
                )?;

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildModulepreloadEventAction,
                            &loader,
                        )
                        .await?,
                    "the deliberately withheld modulepreload error action must remain queued"
                );
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadTerminalEvents.join('|')")?;
                let ready_state_after_preload_event = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument?.readyState || 'missing'",
                )?;

                Ok::<_, anyhow::Error>((
                    lifecycle_turns,
                    events_before_preload_event,
                    ready_state_before_preload_event,
                    events_after_preload_event,
                    ready_state_after_preload_event,
                ))
            })
            .await
            .expect("terminal child modulepreload lifecycle test should run");

        assert!(
            lifecycle_turns
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::DocumentLifecycle)
                .count()
                == 2,
            "DOMContentLoaded and complete must each take a queued turn while the terminal link event remains queued: {lifecycle_turns:?}"
        );
        assert!(
            lifecycle_turns.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "iframe load must remain independently runnable while the terminal link event is withheld: {lifecycle_turns:?}"
        );
        assert_eq!(
            events_before_preload_event,
            "before|after|dcl:interactive|frame-load",
            "child complete and iframe load must not wait for the queued modulepreload error"
        );
        assert_eq!(
            ready_state_before_preload_event, "complete",
            "a terminal modulepreload event must not hold the child Document load gate"
        );
        assert_eq!(
            events_after_preload_event,
            "before|after|dcl:interactive|frame-load|preload-error",
            "the independently queued modulepreload event must still dispatch afterward"
        );
        assert_eq!(
            ready_state_after_preload_event, "complete",
            "post-complete modulepreload dispatch must not regress readyState"
        );
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_modulepreload_fetch_does_not_delay_iframe_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/standalone-preload.js",
            "HTTP/1.1 200 OK",
            "export const preloaded = true;".to_owned(),
            Duration::from_millis(100),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(2)),
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
        let page_resource_queue = page_vm.page_resource_completion_queue();
        let local_executor = page_vm.local_executor.clone();

        let (
            typed_modulepreload_start_ran,
            startup_sources,
            events_before_completion,
            ready_state_before_completion,
            completion,
            events_after_preload_event,
            ready_state_after_preload_event,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut page_resource_queue = page_resource_queue;
                let mut owner_wake_rx = owner_wake_rx;
                let preload_url = format!("{base_url}/standalone-preload.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childStandaloneModulepreloadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childStandaloneModulepreloadEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childStandaloneModulepreloadEvents.push("before");
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childStandaloneModulepreloadEvents.push("dcl:" + document.readyState);
      }});
    <\/script>
    <link rel="modulepreload" href="{preload_url}" onload="parent.__childStandaloneModulepreloadEvents.push('preload-load')" onerror="parent.__childStandaloneModulepreloadEvents.push('preload-error')">
    <script>parent.__childStandaloneModulepreloadEvents.push("after");<\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut typed_modulepreload_start_ran = false;
                let mut startup_sources = Vec::new();
                for _ in 0..10 {
                    if page_resource_queue.has_ready_completion() {
                        break;
                    }
                    if page_vm.page_task_executor_sources_for_test().modulepreload_start()
                        .has_ready_task()
                    {
                        assert!(
                            page_vm
                                .run_exact_selected_page_task_for_test(
                                    PageSelectedTaskTestSelector::ModulepreloadStart,
                                    &loader,
                                )
                                .await?,
                            "typed modulepreload start should enter the production selected dispatcher",
                        );

                        assert!(
                            !typed_modulepreload_start_ran,
                            "one parser-discovered link must publish one typed start"
                        );
                        typed_modulepreload_start_ran = true;
                        continue;
                    }
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    startup_sources.push(source);
                }
                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx
                                .recv()
                                .await
                                .expect("owner-attached modulepreload wake route should remain open");
                        }
                    })
                    .await
                    .expect("typed modulepreload completion should arrive before timeout");
                }
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childStandaloneModulepreloadEvents.join('|')")?;
                let ready_state_before_completion = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument.readyState",
                )?;

                let activity_epoch_before_completion =
                    page_vm.vm().subresource_activity_epoch();
                let completion = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("typed modulepreload completion should consume one Page owner turn");
                assert_eq!(
                    completion.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    completion.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );
                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_completion,
                    "current modulepreload Network output must advance current Document activity"
                );
                assert!(
                    !page_resource_queue.has_ready_completion(),
                    "one modulepreload producer terminal must be consumed exactly once"
                );
                let (network_records, websocket_events, websocket_lifecycle_events) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(websocket_events.is_empty());
                assert!(websocket_lifecycle_events.is_empty());
                assert_eq!(network_records.len(), 1);
                assert!(network_records[0].frame_id().is_some());
                assert_eq!(network_records[0].document_url().as_str(), "about:srcdoc");
                assert_eq!(network_records[0].url().as_str(), preload_url);
                run_expected_child_modulepreload_event_action_for_test(
                    &mut page_vm,
                    &loader,
                    "owner-scheduled child modulepreload event dispatch",
                )
                .await;
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childStandaloneModulepreloadEvents.join('|')")?;
                let ready_state_after_preload_event = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument.readyState",
                )?;

                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "owner-scheduled modulepreload sequence should not leave child frame work"
                );

                Ok::<_, anyhow::Error>((
                    typed_modulepreload_start_ran,
                    startup_sources,
                    events_before_completion,
                    ready_state_before_completion,
                    completion,
                    events_after_preload_event,
                    ready_state_after_preload_event,
                ))
            })
            .await
            .expect("owner-scheduled child modulepreload lifecycle test should run");

        assert!(
            typed_modulepreload_start_ran,
            "parser-discovered modulepreload should start from its typed source: {startup_sources:?}"
        );
        assert!(
            startup_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::DocumentLifecycle)
                .count()
                == 2,
            "DOMContentLoaded and complete should each take a queued turn while modulepreload fetch is pending: {startup_sources:?}"
        );
        assert!(startup_sources.contains(&ChildFrameSemanticTurnKind::HostLoad));
        assert_eq!(
            events_before_completion,
            "before|after|dcl:interactive|frame-load"
        );
        assert_eq!(
            ready_state_before_completion, "complete",
            "fetching modulepreload must not delay document complete or iframe load"
        );
        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_preload_event,
            "before|after|dcl:interactive|frame-load|preload-load",
            "a slow modulepreload link event may dispatch after iframe load"
        );
        assert_eq!(
            ready_state_after_preload_event, "complete",
            "post-complete modulepreload event dispatch must not regress readyState"
        );

        server
            .await
            .expect("standalone child modulepreload server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_modulepreload_fetch_runs_before_joined_module_root() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/preloaded-root.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childModulepreloadJoinEvents.push("module:" + (globalThis === self));
globalThis.__childModulepreloadJoinValue = 551;
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
            startup_sources,
            events_before_completion,
            completion,
            events_after_preload_event,
            events_after_module_terminal,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/preloaded-root.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModulepreloadJoinEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModulepreloadJoinEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModulepreloadJoinEvents.push("before:" + (globalThis === self));<\/script>
    <link id="preload" rel="modulepreload" href="{root_url}" onload="parent.__childModulepreloadJoinEvents.push('preload-load')" onerror="parent.__childModulepreloadJoinEvents.push('preload-error')">
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModulepreloadJoinEvents.push("script-load");
      }});
      document.getElementById("external-module").addEventListener("error", () => {{
        parent.__childModulepreloadJoinEvents.push("script-error");
      }});
      parent.__childModulepreloadJoinEvents.push("after:" + String(globalThis.__childModulepreloadJoinValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let startup_sources =
                    drive_child_modulepreload_startup_until_resource_completion_ready(
                        &mut page_vm,
                        &loader,
                        8,
                    )
                    .await;
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child modulepreload completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child modulepreload completion sender should remain open"
                    );
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;

                run_expected_child_modulepreload_event_action_for_test(
                    &mut page_vm,
                    &loader,
                    "child modulepreload event dispatch",
                )
                .await;
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "same-URL modulepreload terminal fanout",
                )
                .await;
                let events_after_module_terminal = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                let script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "same-URL modulepreload graph-ready execution",
                )
                .await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "same-URL modulepreload iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "same-URL modulepreload sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    startup_sources,
                    events_before_completion,
                    completion,
                    events_after_preload_event,
                    events_after_module_terminal,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child modulepreload joined root test should run");

        assert!(
            startup_sources.contains(&ChildModulepreloadStartupTurn::ChildSemanticTurn(
                ChildFrameSemanticTurnKind::DocumentScriptReady,
            )),
            "child modulepreload startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
        );
        let modulepreload_fetch_position = startup_sources
            .iter()
            .position(|turn| *turn == ChildModulepreloadStartupTurn::TypedModulepreloadStart)
            .expect("parser-discovered modulepreload should start a fetch before completion");
        let parser_root_position = startup_sources
            .iter()
            .position(|turn| {
                *turn
                    == ChildModulepreloadStartupTurn::ChildSemanticTurn(
                        ChildFrameSemanticTurnKind::ParserModuleRootStart,
                    )
            })
            .expect("same-URL parser module root should still run and join the module map entry");
        assert!(
            modulepreload_fetch_position < parser_root_position,
            "parser-discovered modulepreload should start before same-URL parser module root joins it: {startup_sources:?}"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "modulepreload fetch start should not execute module or dispatch link/frame events"
        );
        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_preload_event, "before:true|after:undefined|preload-load",
            "modulepreload link load should dispatch before module script execution"
        );
        assert_eq!(
            events_after_module_terminal, "before:true|after:undefined|preload-load",
            "ModuleScriptTerminal should not execute the joined module inline"
        );
        assert_eq!(
            script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "joined module graph-ready work should execute on DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true|after:undefined|preload-load|module:true|script-load",
            "DocumentScriptReady should execute the joined module without iframe load"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a later HostLoad source"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|preload-load|module:true|script-load|frame-load",
            "HostLoad should dispatch iframe load after joined modulepreload execution"
        );

        server
            .await
            .expect("child modulepreload joined root server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_modulepreload_failure_wakes_joined_root_before_host_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/preloaded-root.js",
            "HTTP/1.1 404 Not Found",
            "modulepreload missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            startup_sources,
            events_before_completion,
            completion,
            events_after_preload_event,
            events_after_module_terminal,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/preloaded-root.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModulepreloadFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModulepreloadFailureEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModulepreloadFailureEvents.push("before:" + (globalThis === self));<\/script>
    <link id="preload" rel="modulepreload" href="{root_url}" onload="parent.__childModulepreloadFailureEvents.push('preload-load')" onerror="parent.__childModulepreloadFailureEvents.push('preload-error')">
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModulepreloadFailureEvents.push("script-load");
      }});
      document.getElementById("external-module").addEventListener("error", () => {{
        parent.__childModulepreloadFailureEvents.push("script-error");
      }});
      parent.__childModulepreloadFailureEvents.push("after:" + String(globalThis.__childModulepreloadFailureValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let startup_sources =
                    drive_child_modulepreload_startup_until_resource_completion_ready(
                        &mut page_vm,
                        &loader,
                        8,
                    )
                    .await;
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child failed modulepreload completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child failed modulepreload completion sender should remain open"
                    );
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;

                run_expected_child_modulepreload_event_action_for_test(
                    &mut page_vm,
                    &loader,
                    "child failed modulepreload event dispatch",
                )
                .await;
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "failed same-URL modulepreload terminal fanout",
                )
                .await;
                let events_after_module_terminal = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                let script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "failed same-URL modulepreload graph-failed dispatch",
                )
                .await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "failed same-URL modulepreload iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "failed same-URL modulepreload sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    startup_sources,
                    events_before_completion,
                    completion,
                    events_after_preload_event,
                    events_after_module_terminal,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child failed modulepreload joined root test should run");

        assert!(
            startup_sources.contains(&ChildModulepreloadStartupTurn::ChildSemanticTurn(
                ChildFrameSemanticTurnKind::DocumentScriptReady,
            )),
            "child failed modulepreload startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
        );
        let modulepreload_fetch_position = startup_sources
            .iter()
            .position(|turn| *turn == ChildModulepreloadStartupTurn::TypedModulepreloadStart)
            .expect("parser-discovered modulepreload should start a fetch before failure");
        let parser_root_position = startup_sources
            .iter()
            .position(|turn| {
                *turn
                    == ChildModulepreloadStartupTurn::ChildSemanticTurn(
                        ChildFrameSemanticTurnKind::ParserModuleRootStart,
                    )
            })
            .expect("same-URL parser module root should still run and join the failed module map entry");
        assert!(
            modulepreload_fetch_position < parser_root_position,
            "parser-discovered modulepreload should start before same-URL parser module root joins it: {startup_sources:?}"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "modulepreload fetch start should not dispatch link/script/frame events before failure"
        );
        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_preload_event, "before:true|after:undefined|preload-error",
            "modulepreload link error should dispatch before joined module script failure"
        );
        assert_eq!(
            events_after_module_terminal, "before:true|after:undefined|preload-error",
            "ModuleScriptTerminal should not dispatch the joined script error inline"
        );
        assert_eq!(
            script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "joined module graph-failed work should dispatch on DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready, "before:true|after:undefined|preload-error|script-error",
            "DocumentScriptReady should dispatch the joined module script error without iframe load"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a later HostLoad source after modulepreload failure"
        );
        assert_eq!(
            final_events, "before:true|after:undefined|preload-error|script-error|frame-load",
            "HostLoad should dispatch iframe load after joined modulepreload failure finalizes"
        );

        server
            .await
            .expect("child failed modulepreload joined root server should finish");
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn modulepreload_fetch_failure_wakes_joined_parser_script_waiter() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/page.html").expect("document URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());

        let root_url = Url::parse("https://example.com/root.mjs").expect("root URL");
        let shared_url = Url::parse("https://cdn.example.com/shared.mjs").expect("shared URL");
        let shared_key = ModuleMapKey::java_script(shared_url.clone());
        let modulepreload = NativeModuleSingleFetchRequest::new(
            shared_url.clone(),
            shared_url.clone(),
            document_url,
            shared_key.clone(),
            ModuleFetchMetadata::default(),
        );
        let start = page_vm
            .vm_mut()
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload)
            .expect("modulepreload registration should succeed");
        let crate::module_runtime::NativeModulepreloadFetchStart::Started(modulepreload) = start
        else {
            panic!("new modulepreload should start a single-module fetch");
        };
        let modulepreload_load_id = page_vm
            .vm_mut()
            .document_runtime
            .suspend_native_modulepreload_fetch(*modulepreload);

        let root_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9301, root_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, root_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, parser_module_work)
            .await
            .expect("module defer page task should watch loading tree");

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            1,
            &root_url,
            r#"
import "https://cdn.example.com/shared.mjs";
export const root = 1;
"#,
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("root completion should run")
                .is_some(),
            "root completion should be consumed"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "parser graph should wait on the in-flight modulepreload entry"
        );

        enqueue_main_modulepreload_fetch_error_for_test(
            &mut page_vm,
            modulepreload_load_id,
            &shared_url,
            "modulepreload fetch failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("modulepreload completion should run")
                .is_some(),
            "modulepreload completion should be consumed"
        );
        run_next_native_module_owner_event_for_test(
            &mut page_vm,
            &loader,
            "joined modulepreload failure",
        )
        .await;
        let shared_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&shared_key)
            .expect("failed modulepreload should remain in module map");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(shared_entry),
            ModuleMapEntryState::Failed,
            "failed modulepreload must become terminal"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "joined parser graph should be woken by the modulepreload failure"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "joined parser graph failure should wait for the document-owned ready dispatch"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "joined modulepreload graph failure",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "joined parser graph failure should leave no pending parser-owned module script"
        );
        let root_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &root_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("parser module script should fail after joined modulepreload wakeup");
        assert!(
            root_failure.contains("modulepreload fetch failed"),
            "joined script should observe the terminal modulepreload failure: {root_failure}"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn parser_module_script_observes_prior_modulepreload_failure() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/page.html").expect("document URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());

        let module_url = Url::parse("https://example.com/module.mjs").expect("module URL");
        let modulepreload = NativeModuleSingleFetchRequest::new(
            module_url.clone(),
            module_url.clone(),
            document_url,
            ModuleMapKey::java_script(module_url.clone()),
            ModuleFetchMetadata::default(),
        );
        let start = page_vm
            .vm_mut()
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload)
            .expect("modulepreload registration should succeed");
        let crate::module_runtime::NativeModulepreloadFetchStart::Started(modulepreload) = start
        else {
            panic!("new modulepreload should start a single-module fetch");
        };
        let modulepreload_load_id = page_vm
            .vm_mut()
            .document_runtime
            .suspend_native_modulepreload_fetch(*modulepreload);

        enqueue_main_modulepreload_fetch_error_for_test(
            &mut page_vm,
            modulepreload_load_id,
            &module_url,
            "modulepreload integrity failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("modulepreload completion should run")
                .is_some(),
            "modulepreload completion should be consumed"
        );

        let module_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9401, module_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, module_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, parser_module_work)
            .await
            .expect("module defer page task should watch failed tree");
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "the parser-deferred owner turn should consume the already-terminal failure"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "sticky failure must not materialize broad ready work"
        );

        let root_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &module_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("parser module script should fail from sticky modulepreload failure");
        assert!(
            root_failure.contains("modulepreload integrity failed"),
            "module script should report sticky modulepreload failure: {root_failure}"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn parser_module_script_joins_inflight_same_url_modulepreload_failure() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/page.html").expect("document URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());

        let module_url = Url::parse("https://example.com/module.mjs").expect("module URL");
        let modulepreload = NativeModuleSingleFetchRequest::new(
            module_url.clone(),
            module_url.clone(),
            document_url,
            ModuleMapKey::java_script(module_url.clone()),
            ModuleFetchMetadata::default(),
        );
        let start = page_vm
            .vm_mut()
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload)
            .expect("modulepreload registration should succeed");
        let crate::module_runtime::NativeModulepreloadFetchStart::Started(modulepreload) = start
        else {
            panic!("new modulepreload should start a single-module fetch");
        };
        let modulepreload_load_id = page_vm
            .vm_mut()
            .document_runtime
            .suspend_native_modulepreload_fetch(*modulepreload);

        let module_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9501, module_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, module_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("module defer page task should watch joined modulepreload");
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "parser root graph should wait on the in-flight same-URL modulepreload entry"
        );

        enqueue_main_modulepreload_fetch_error_for_test(
            &mut page_vm,
            modulepreload_load_id,
            &module_url,
            "modulepreload integrity failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("modulepreload completion should run")
                .is_some(),
            "modulepreload completion should be consumed"
        );
        run_next_native_module_owner_event_for_test(
            &mut page_vm,
            &loader,
            "same-URL joined modulepreload failure",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "same-URL joined parser graph should be woken by modulepreload failure"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "same-URL joined parser graph failure should wait for the document-owned ready dispatch"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "same-URL joined modulepreload failure",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "same-URL joined parser graph failure should leave no pending parser-owned module script"
        );

        let root_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &module_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("parser module script should fail from same-URL modulepreload failure");
        assert!(
            root_failure.contains("modulepreload integrity failed"),
            "module script should report joined modulepreload failure: {root_failure}"
        );
    })
    .await;
}
