use super::*;

#[tokio::test(flavor = "current_thread")]
async fn stale_child_module_completions_preserve_network_and_run_one_terminal_per_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm.vm_mut().eval(
            "const staleModuleFrame = document.createElement('iframe'); \
             staleModuleFrame.id = 'stale-module-frame'; \
             document.body.appendChild(staleModuleFrame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("stale-module-frame")
            .expect("same-Page stale module fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "stale-module-frame")?;
        let current_child_owner = page_vm
            .vm()
            .current_child_document_task_owner(child_handle)
            .expect("same-Page stale module fixture should install a child owner");
        let current_module_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("same-Page stale module fixture should install a child realm");
        assert_eq!(current_module_target.task_owner(), current_child_owner);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(
                current_child_owner.scheduler_lane_id.0 + 1,
            ),
            current_child_owner.local_window_id,
            current_child_owner.document_id,
        );
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_parser_module_root_fetch(
                root_document,
                test_child_parser_module_root_completion(
                    child_handle,
                    stale_owner,
                    47,
                    "stale-child-module-root",
                    Some("stale child module root failed"),
                ),
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_module_dependency_fetch(
                root_document,
                test_child_module_dependency_completion(
                    child_handle,
                    stale_owner,
                    53,
                    "stale-child-module-dependency",
                    Some("stale child module dependency failed"),
                ),
            ),
        );
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_modulepreload_fetch(
            root_document,
            test_child_modulepreload_completion_for_target(
                ChildDocumentModuleFetchTarget::new(child_handle, stale_owner, FrameRealmId(109)),
                57,
                "stale-child-modulepreload",
                Some("stale child modulepreload failed"),
            ),
        ));
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_modulepreload_fetch(
            root_document,
            test_child_modulepreload_completion_for_target(
                ChildDocumentModuleFetchTarget::new(child_handle, stale_owner, FrameRealmId(109)),
                59,
                "stale-child-module-no-network",
                None,
            ),
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        let expected_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            ChildDocumentModuleFetchTarget::new(child_handle, stale_owner, FrameRealmId(109)),
        );
        let expected_current_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            current_module_target,
        );

        let root = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale root module terminal should consume one owner turn");
        assert_eq!(root.action.owner, expected_owner);
        assert_eq!(
            root.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            root.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(queue.has_ready_completion());

        let dependency = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale dependency module terminal should consume its own owner turn");
        assert_eq!(dependency.action.owner, expected_owner);
        assert_eq!(
            dependency.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            dependency.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(queue.has_ready_completion());

        let modulepreload = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale modulepreload terminal should consume its own owner turn");
        assert_eq!(modulepreload.action.owner, expected_owner);
        assert_eq!(
            modulepreload.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            modulepreload.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(queue.has_ready_completion());

        let no_network = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale no-Network module terminal should consume its own owner turn");
        assert_eq!(no_network.action.owner, expected_owner);
        assert_eq!(
            no_network.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            no_network.action.output_effect,
            PageResourceCompletionOutputEffect::None,
            "a stale module terminal without a Network fact must not synthesize output"
        );

        assert!(!queue.has_ready_completion());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "retired child module Network facts must not become current Document activity"
        );

        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 3);
        assert_eq!(
            network_records
                .iter()
                .map(|record| record.frame_id())
                .collect::<Vec<_>>(),
            vec![
                Some("stale-child-module-root-frame"),
                Some("stale-child-module-dependency-frame"),
                Some("stale-child-modulepreload-frame"),
            ]
        );
        assert!(network_records.iter().all(|record| {
            record.resource_type() == SubresourceResourceType::Script
                && record.request_initiator_type() == SubresourceRequestInitiatorType::Parser
        }));
        assert_eq!(
            network_records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child module root failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[1].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child module dependency failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[2].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child modulepreload failed".to_owned(),
            }
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child module completion test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn child_module_completion_rejects_replaced_realm_with_same_document_owner() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const realmReplacementFrame = document.createElement('iframe'); \
             realmReplacementFrame.id = 'realm-replacement-frame'; \
             document.body.appendChild(realmReplacementFrame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("realm-replacement-frame")
            .expect("realm replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "realm-replacement-frame",
        )?;
        let retired_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("first child realm should have an exact module target");
        let root_completion = test_child_parser_module_root_completion_for_target(
            retired_target,
            61,
            "stale-child-module-root-realm",
            Some("retired child realm root fetch failed"),
        );
        let dependency_completion = test_child_module_dependency_completion_for_target(
            retired_target,
            67,
            "stale-child-module-dependency-realm",
            Some("retired child realm dependency fetch failed"),
        );
        let modulepreload_completion = test_child_modulepreload_completion_for_target(
            retired_target,
            71,
            "stale-child-modulepreload-realm",
            Some("retired child realm modulepreload failed"),
        );

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "realm-replacement-frame",
        )?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("replacement child realm should have an exact module target");
        assert_eq!(current_target.child_handle(), retired_target.child_handle());
        assert_eq!(current_target.task_owner(), retired_target.task_owner());
        assert_ne!(
            current_target.realm_id(),
            retired_target.realm_id(),
            "realm replacement must preserve the Document owner while changing execution identity"
        );

        let root_document = page_vm.document_lifecycle.identity().document;
        let expected_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            retired_target,
        );
        let expected_current_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            current_target,
        );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_parser_module_root_fetch(
                root_document,
                root_completion,
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_module_dependency_fetch(
                root_document,
                dependency_completion,
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_modulepreload_fetch(
                root_document,
                modulepreload_completion,
            ),
        );

        let root_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired-realm terminal should consume exactly one owner turn");
        assert_eq!(root_outcome.action.owner, expected_owner);
        assert_eq!(
            root_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            root_outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        let dependency_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired-realm dependency must consume a second owner turn");
        assert_eq!(dependency_outcome.action.owner, expected_owner);
        assert_eq!(
            dependency_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            dependency_outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        let modulepreload_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired-realm modulepreload must consume a third owner turn");
        assert_eq!(modulepreload_outcome.action.owner, expected_owner);
        assert_eq!(
            modulepreload_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            modulepreload_outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "historical Network output from a retired realm must not become current Document activity"
        );

        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 3);
        assert_eq!(
            network_records[0].frame_id(),
            Some("stale-child-module-root-realm-frame")
        );
        assert_eq!(
            network_records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "retired child realm root fetch failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[1].frame_id(),
            Some("stale-child-module-dependency-realm-frame")
        );
        assert_eq!(
            network_records[1].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "retired child realm dependency fetch failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[2].frame_id(),
            Some("stale-child-modulepreload-realm-frame")
        );
        assert_eq!(
            network_records[2].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "retired child realm modulepreload failed".to_owned(),
            }
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Document stale child realm test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn production_child_module_route_reaches_exact_realm_page_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const productionModuleFrame = document.createElement('iframe'); \
             productionModuleFrame.id = 'production-module-frame'; \
             document.body.appendChild(productionModuleFrame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("production-module-frame")
            .expect("production route fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "production-module-frame",
        )?;
        let target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("production route fixture should expose an exact module target");
        let root_document = page_vm.document_lifecycle.identity().document;

        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token = crate::runtime::RendererPageToken::new_for_testing(root_document.page_id);
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(wake_tx, token);
        let mut page_queue = RendererPageNetworkingSource::new_owner_attached(
            crate::page_task_queue::PageRuntimeWakeSignal::default(),
            owner_wake.clone(),
        );
        let sender =
            crate::page_task_queue::RendererResourceCompletionSender::for_page_resource_test(
                page_queue.sender(),
                root_document,
            );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        sender
            .send_child_parser_module_root_fetch(
                test_child_parser_module_root_completion_for_target(
                    target,
                    67,
                    "production-child-module-route",
                    Some("production routed module fetch failed"),
                ),
            )
            .expect("production sender should enqueue the typed child module terminal");

        assert_eq!(
            wake_rx
                .try_recv()
                .expect("accepted production terminal should publish one Page wake")
                .page_id(),
            root_document.page_id
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "one accepted terminal must not publish a duplicate wake"
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_queue)?
            .expect("production-routed terminal should execute through the typed Page turn");
        assert_eq!(
            outcome.action.owner,
            RendererPageResourceCompletionOwner::child_module_fetch(root_document, target)
        );
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(!page_queue.has_ready_completion());
        assert!(
            page_vm.vm().subresource_activity_epoch() > activity_epoch_before,
            "current-target Network output must be attributed to current Document activity"
        );

        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 1);
        assert_eq!(
            network_records[0].frame_id(),
            Some("production-child-module-route-frame")
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production child module stable-route test should run");
}
#[tokio::test]
async fn page_vm_child_module_defer_preserves_document_order_when_second_graph_finishes_first() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-module-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDeferOrderEvents.push("first-module:" + (globalThis === self));
globalThis.__childModuleDeferOrderFirst = 1;
"#
                .to_owned(),
                Duration::from_millis(350),
            ),
            (
                "/second-module-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDeferOrderEvents.push("second-module:" + (globalThis === self));
globalThis.__childModuleDeferOrderSecond = 2;
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
            events_after_second_module_owner,
            first_completion,
            events_after_first_completion,
            bootstrap_sources,
            events_after_first_module_owner,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let first_script_url = format!("{base_url}/first-module-defer.js");
                let second_script_url = format!("{base_url}/second-module-defer.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDeferOrderEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDeferOrderEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childModuleDeferOrderEvents.push("before:" + (globalThis === self));<\/script>
    <script id="first-module-defer" type="module" src="{first_script_url}"><\/script>
    <script id="second-module-defer" type="module" src="{second_script_url}"><\/script>
    <script>
      document.getElementById("first-module-defer").addEventListener("load", () => {{
        parent.__childModuleDeferOrderEvents.push("first-load");
      }});
      document.getElementById("second-module-defer").addEventListener("load", () => {{
        parent.__childModuleDeferOrderEvents.push("second-load");
      }});
      parent.__childModuleDeferOrderEvents.push(
        "after:" + String(globalThis.__childModuleDeferOrderFirst) + ":" +
          String(globalThis.__childModuleDeferOrderSecond)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..10 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childModuleDeferOrderEvents.join('|')")?;
                    if bootstrap_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        .count()
                        == 2
                        && bootstrap_sources
                            .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                        && bootstrap_events == "before:true|after:undefined:undefined"
                    {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after:undefined:undefined",
                    "child parser should continue past both parser module-defer scripts without evaluating them"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("second child module-defer completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let second_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_second_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "later child module-defer terminal",
                )
                .await;
                let events_after_second_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("first child module-defer completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let first_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_first_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "earlier child module-defer terminal",
                )
                .await;
                let events_after_first_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first child module-defer execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferOrderEvents.join('|')")?,
                    "before:true|after:undefined:undefined|first-module:true|first-load",
                    "first ordered DocumentScriptReady should execute the earlier module-defer without second module or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second child module-defer execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferOrderEvents.join('|')")?,
                    "before:true|after:undefined:undefined|first-module:true|first-load|second-module:true|second-load",
                    "second ordered DocumentScriptReady should run only after the earlier module-defer completes"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "ordered child module-defer iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child module-defer ordering sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    second_completion,
                    events_after_second_completion,
                    events_after_second_module_owner,
                    first_completion,
                    events_after_first_completion,
                    bootstrap_sources,
                    events_after_first_module_owner,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module-defer ordering test should run");

        assert!(matches!(
            second_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            bootstrap_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                .count(),
            2,
            "child module-defer ordering bootstrap should start both parser-module roots explicitly: {bootstrap_sources:?}"
        );
        assert_eq!(
            events_after_second_completion, "before:true|after:undefined:undefined",
            "faster second module graph completion must not evaluate before earlier parser module"
        );
        assert_eq!(
            events_after_second_module_owner, "before:true|after:undefined:undefined",
            "later terminal source must only retain the terminal behind the earlier parser module"
        );
        assert!(matches!(
            first_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_first_completion, "before:true|after:undefined:undefined",
            "first module graph completion turn should still not inline-run script"
        );
        assert_eq!(
            events_after_first_module_owner, "before:true|after:undefined:undefined",
            "ModuleScriptTerminal should only enqueue ordered document-script work"
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
            "ordered child module-defer scripts should execute through two script-ready turns, DOMContentLoaded, and complete before HostLoad"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined:undefined|first-module:true|first-load|second-module:true|second-load|load",
            "module-defer scripts should execute in document order even when the second graph finishes first"
        );

        server
            .await
            .expect("child module-defer ordering server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_module_defer_retains_later_graph_failure_behind_earlier_graph() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-module-defer-success.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDeferFailureOrderEvents.push("first-module:" + (globalThis === self));
globalThis.__childModuleDeferFailureOrderFirst = 1;
"#
                .to_owned(),
                Duration::from_millis(350),
            ),
            (
                "/second-module-defer-failure.js",
                "HTTP/1.1 200 OK",
                "import {".to_owned(),
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
            events_after_second_module_owner,
            source_after_second_module_owner,
            events_after_blocked_second_terminal,
            first_completion,
            events_after_first_completion,
            bootstrap_sources,
            events_after_first_module_owner,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let first_script_url = format!("{base_url}/first-module-defer-success.js");
                let second_script_url = format!("{base_url}/second-module-defer-failure.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDeferFailureOrderEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDeferFailureOrderEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childModuleDeferFailureOrderEvents.push("before:" + (globalThis === self));<\/script>
    <script id="first-module-defer-success" type="module" src="{first_script_url}"><\/script>
    <script id="second-module-defer-failure" type="module" src="{second_script_url}"><\/script>
    <script>
      addEventListener("error", event => {{
        parent.__childModuleDeferFailureOrderEvents.push("window-error:" + (
          event instanceof ErrorEvent && event.error instanceof SyntaxError &&
          event.target === window && event.isTrusted
        ));
        event.preventDefault();
      }});
      document.getElementById("first-module-defer-success").addEventListener("load", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("first-load");
      }});
      document.getElementById("first-module-defer-success").addEventListener("error", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("first-error");
      }});
      document.getElementById("second-module-defer-failure").addEventListener("load", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("second-load");
      }});
      document.getElementById("second-module-defer-failure").addEventListener("error", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("second-error");
      }});
      parent.__childModuleDeferFailureOrderEvents.push(
        "after:" + String(globalThis.__childModuleDeferFailureOrderFirst)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..10 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                    if bootstrap_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        .count()
                        == 2
                        && bootstrap_sources
                            .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                        && bootstrap_events == "before:true|after:undefined"
                    {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events, "before:true|after:undefined",
                    "child parser should continue past success/failure module-defer siblings without evaluating them"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("second child module-defer failure should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let second_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_second_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "later failing child module-defer terminal",
                )
                .await;
                let events_after_second_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                let source_after_second_module_owner =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_blocked_second_terminal = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("first child module-defer success should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let first_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_first_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "earlier successful child module-defer terminal",
                )
                .await;
                let events_after_first_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first successful child module-defer execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferFailureOrderEvents.join('|')")?,
                    "before:true|after:undefined|first-module:true|first-load",
                    "first ordered DocumentScriptReady should execute the earlier successful module before later error dispatch"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second failing child module-defer dispatch",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferFailureOrderEvents.join('|')")?,
                    "before:true|after:undefined|first-module:true|first-load|window-error:true|second-load",
                    "later parse failure should report to its Window and fire external script load only after the earlier module-defer completes"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer graph-failure DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer graph-failure complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "ordered child module-defer graph-failure iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    second_completion,
                    events_after_second_completion,
                    events_after_second_module_owner,
                    source_after_second_module_owner,
                    events_after_blocked_second_terminal,
                    first_completion,
                    events_after_first_completion,
                    bootstrap_sources,
                    events_after_first_module_owner,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module-defer graph-failure ordering test should run");

        assert!(matches!(
            second_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            bootstrap_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                .count(),
            2,
            "child module-defer failure ordering bootstrap should start both roots explicitly: {bootstrap_sources:?}"
        );
        assert_eq!(
            events_after_second_completion, "before:true|after:undefined",
            "faster second graph failure completion must not report an exception before the earlier parser module"
        );
        assert_eq!(
            events_after_second_module_owner, "before:true|after:undefined",
            "later graph failure terminal must be retained behind the earlier parser module"
        );
        assert_eq!(
            source_after_second_module_owner, None,
            "retained later graph failure should not wake DocumentScriptReady before the earlier graph finishes"
        );
        assert_eq!(
            events_after_blocked_second_terminal, "before:true|after:undefined",
            "blocked later graph failure should not report an exception or dispatch script or iframe load"
        );
        assert!(matches!(
            first_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_first_completion, "before:true|after:undefined",
            "first graph completion turn should not inline-run script"
        );
        assert_eq!(
            events_after_first_module_owner, "before:true|after:undefined",
            "ModuleScriptTerminal should only enqueue ordered document-script work"
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
            "ordered child module-defer success/failure should finish through ready turns, DOMContentLoaded, and complete before HostLoad"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|first-module:true|first-load|window-error:true|second-load|load",
            "later graph failure should preserve parser module document order and keep iframe load on HostLoad"
        );

        server
            .await
            .expect("child module-defer graph-failure ordering server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_parser_module_root_completion_queues_module_script_terminal_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-parser-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childParserModuleWaitEvents.push("module:" + (globalThis === self));
globalThis.__childParserModuleWaitValue = 188;
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(1)),
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
            first,
            first_events,
            bootstrap_sources,
            events_after_module_owner,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut page_resource_queue = page_resource_queue;
                let mut owner_wake_rx = owner_wake_rx;
                let script_url = format!("{base_url}/child-parser-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childParserModuleWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childParserModuleWaitEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childParserModuleWaitEvents.push(
        "before:" + (globalThis === self) + ":" + document.currentScript.isConnected
      );
    <\/script>
    <script id="external-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childParserModuleWaitEvents.push("script-load");
      }});
      parent.__childParserModuleWaitEvents.push(
        "after:" + String(globalThis.__childParserModuleWaitValue)
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
                        .eval("__childParserModuleWaitEvents.join('|')")?;
                    if bootstrap_sources
                        .contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        && bootstrap_sources
                            .iter()
                            .filter(|source| {
                                **source == ChildFrameSemanticTurnKind::DocumentScriptReady
                            })
                            .count()
                            == 2
                        && bootstrap_sources
                            .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                        && bootstrap_events == "before:true:true|after:undefined"
                    {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true:true|after:undefined",
                    "parser should continue past child module-defer script while root fetch is pending; sources={bootstrap_sources:?}"
                );

                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx
                                .recv()
                                .await
                                .expect("owner wake route should remain open");
                        }
                    })
                    .await
                    .expect("child parser module completion should arrive before timeout");
                }

                let _ = page_vm.vm_mut().take_network_output();
                let activity_epoch_before_completion =
                    page_vm.vm().subresource_activity_epoch();
                let first = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("production child module completion should consume one typed turn");
                assert!(
                    !page_resource_queue.has_ready_completion(),
                    "the production root terminal must be consumed exactly once"
                );
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
                    "a current child module Network terminal must advance current Document activity"
                );
                let (network_records, websocket_events, websocket_lifecycle_events) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(websocket_events.is_empty());
                assert!(websocket_lifecycle_events.is_empty());
                assert_eq!(network_records.len(), 1);
                let network_record = &network_records[0];
                assert!(
                    network_record.frame_id().is_some(),
                    "producer must retain the child frame attribution"
                );
                assert_eq!(network_record.document_url().as_str(), "about:srcdoc");
                assert_eq!(network_record.url().as_str(), script_url);
                assert_eq!(
                    network_record.request_initiator_type(),
                    SubresourceRequestInitiatorType::Parser
                );
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child parser-module root terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child parser-module iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_sources,
                    events_after_module_owner,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child parser module deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            first_events, "before:true:true|after:undefined",
            "resource completion turn should not inline-run child parser module graph"
        );
        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::ParserModuleRootStart,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "the typed root fetch-start must preserve parser discovery FIFO, then the parser should run the following inline script and reach interactive"
        );
        assert_eq!(
            events_after_module_owner, "before:true:true|after:undefined",
            "ModuleScriptTerminal should not execute the module-defer script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "module-defer graph-ready work should execute through DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true:true|after:undefined|module:true|script-load",
            "DocumentScriptReady should run the module-defer script and dispatch its script load without iframe load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source after module-defer execution"
        );
        assert_eq!(
            final_events,
            "before:true:true|after:undefined|module:true|script-load|frame-load",
            "HostLoad should dispatch iframe load only after module-defer execution completes"
        );

        server
            .await
            .expect("child parser module wait-path server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_module_script_blocks_host_load_until_evaluation_finishes() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-load-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childModuleHostLoadEvents.push("module:" + (globalThis === self));
globalThis.__childModuleHostLoadValue = 203;
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
            initial_host_load_before_root_fetch,
            navigation_commit_source,
            first_document_script_ready_before_root_fetch,
            host_load_attempt_before_root_fetch,
            events_after_pre_root_host_load_attempt,
            pre_completion_sources,
            events_before_completion,
            resource_ready_before_wait,
            completion_source,
            events_after_module_owner,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-load-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="load-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("load-module").addEventListener("load", () => {{
        parent.__childModuleHostLoadEvents.push("script-load");
      }});
      parent.__childModuleHostLoadEvents.push(
        "after:" + String(globalThis.__childModuleHostLoadValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let initial_host_load_before_root_fetch = page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildHostLoad,
                        &loader,
                    )
                    .await?;
                let navigation_commit_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "child parser-module exact realm",
                )
                .await;
                let first_document_script_ready_before_root_fetch =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await
                        == Some(ChildFrameSemanticTurnKind::DocumentScriptReady);
                let host_load_attempt_before_root_fetch = page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildHostLoad,
                        &loader,
                    )
                    .await?;
                let events_after_pre_root_host_load_attempt = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;

                let mut pre_completion_sources = Vec::new();
                let mut events_before_completion = String::new();
                for _ in 0..8 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childModuleHostLoadEvents.join('|')")?;
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
                    .expect("child module completion should arrive before timeout");
                    assert!(arrived, "child module completion sender should remain open");
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child module HostLoad-gate terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;

                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child module iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    initial_host_load_before_root_fetch,
                    navigation_commit_source,
                    first_document_script_ready_before_root_fetch,
                    host_load_attempt_before_root_fetch,
                    events_after_pre_root_host_load_attempt,
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module HostLoad gate test should run");

        assert!(
            !initial_host_load_before_root_fetch,
            "HostLoad source should not progress while initial document-script ready work is pending"
        );
        assert_eq!(
            navigation_commit_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "child navigation must install the active Document before its parser-script turns"
        );
        assert!(
            first_document_script_ready_before_root_fetch,
            "the initial child DocumentScriptReady turn should enqueue parser-module root fetch work"
        );
        assert!(
            !host_load_attempt_before_root_fetch,
            "HostLoad source should make no progress while parser-module root fetch work is still queued"
        );
        assert!(
            !events_after_pre_root_host_load_attempt.contains("frame-load"),
            "queued parser-module root fetch work must block iframe load before source priority runs it"
        );
        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "pre-completion turns should start the module root fetch from its typed source: {pre_completion_sources:?}"
        );
        assert!(
            !pre_completion_sources.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "blocked bootstrap HostLoad should not re-ready itself before module source completion: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed module response should leave a window to prove HostLoad is gated before completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "pending module script must block iframe load before resource completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after:undefined",
            "ModuleScriptTerminal should not execute the module or dispatch iframe load inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "module graph-ready work should execute through DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true|after:undefined|module:true|script-load",
            "module evaluation should dispatch script load but not iframe load inline"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|module:true|script-load|frame-load",
            "HostLoad should dispatch iframe load only after module evaluation finishes"
        );

        server
            .await
            .expect("child module HostLoad gate server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_module_tla_releases_lifecycle_after_evaluation_starts() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-tla-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childModuleTlaHostLoadEvents.push("module-start:" + (globalThis === self));
await new Promise(resolve => {
  parent.__resolveChildModuleTlaHostLoad = resolve;
  parent.__childModuleTlaHostLoadEvents.push("module-pending");
});
globalThis.__childModuleTlaHostLoadValue = 409;
parent.__childModuleTlaHostLoadEvents.push("module-after");
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
            graph_ready_source,
            events_after_pending_evaluation,
            host_load_source_while_tla_pending,
            events_after_host_load,
            parent_completion_recheck_source,
            events_after_module_reaction,
            evaluation_completion_source,
            final_events,
        ) = local_executor
            // Keep this large, stateful scenario off nextest's comparatively
            // small test-thread stack; pinning does not add a scheduler turn.
            .run(Box::pin(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-tla-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleTlaHostLoadEvents = [];
  delete globalThis.__resolveChildModuleTlaHostLoad;
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleTlaHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleTlaHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="tla-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("tla-module").addEventListener("load", () => {{
        parent.__childModuleTlaHostLoadEvents.push("script-load");
      }});
      document.getElementById("tla-module").addEventListener("error", () => {{
        parent.__childModuleTlaHostLoadEvents.push("script-error");
      }});
      parent.__childModuleTlaHostLoadEvents.push(
        "after:" + String(globalThis.__childModuleTlaHostLoadValue)
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
                        .eval("__childModuleTlaHostLoadEvents.join('|')")?;
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
                    .expect("child TLA module completion should arrive before timeout");
                    assert!(arrived, "child TLA module completion sender should remain open");
                }

                let loader = page_vm.main_document_resource_loader();
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child TLA module terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;
                let graph_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_pending_evaluation = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;

                let host_load_source_while_tla_pending = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child module TLA iframe load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;
                let parent_completion_recheck = page_vm
                    .run_page_main_document_runtime_body_for_test(loader.request_client())
                    .await?
                    .expect("child HostLoad must publish one typed parent completion recheck");

                page_vm
                    .vm_mut()
                    .eval("__resolveChildModuleTlaHostLoad(); 'ok'")?;
                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ModuleReaction,
                            loader.request_client(),
                        )
                        .await?,
                    "resolved child TLA should enqueue one typed module reaction"
                );
                let events_after_module_reaction = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;

                let evaluation_completion_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    graph_ready_source,
                    events_after_pending_evaluation,
                    host_load_source_while_tla_pending,
                    events_after_host_load,
                    parent_completion_recheck.action,
                    events_after_module_reaction,
                    evaluation_completion_source,
                    final_events,
                ))
            }))
            .await
            .expect("page vm child TLA module HostLoad gate test should run");

        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "pre-completion turns should start the module root fetch from its typed source: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed TLA module response should leave a window to prove HostLoad is gated before completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "pending TLA module script must block iframe load before resource completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after:undefined",
            "ModuleScriptTerminal should not start TLA evaluation or dispatch iframe load inline"
        );
        assert_eq!(
            graph_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "module graph-ready work should start TLA evaluation through DocumentScriptReady"
        );
        assert_eq!(
            events_after_pending_evaluation,
            "before:true|after:undefined|module-start:true|module-pending|script-load",
            "starting TLA should complete the static module script and dispatch its load event"
        );
        assert_eq!(
            host_load_source_while_tla_pending,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "the evaluation promise must not keep the child lifecycle blocked"
        );
        assert_eq!(
            events_after_host_load,
            "before:true|after:undefined|module-start:true|module-pending|script-load|frame-load",
            "iframe load should run after the static module execution turn while TLA remains pending"
        );
        assert_eq!(
            parent_completion_recheck_source.kind(),
            crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork,
            "child HostLoad completion should publish one concrete parent completion recheck task"
        );
        assert_eq!(
            parent_completion_recheck_source.target_effect(),
            crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner,
            "the exact parent completion recheck must apply through the main-Document arbiter"
        );
        assert_eq!(
            events_after_module_reaction,
            "before:true|after:undefined|module-start:true|module-pending|script-load|frame-load|module-after",
            "module reaction should finish evaluation after document load without redispatching lifecycle events"
        );
        assert_eq!(
            evaluation_completion_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "fulfilled TLA continuation should re-enter DocumentScriptReady"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|module-start:true|module-pending|script-load|frame-load|module-after",
            "TLA completion must not dispatch duplicate script or iframe load events"
        );

        server
            .await
            .expect("child TLA module HostLoad gate server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_dynamic_import_does_not_block_host_load() {
    run_page_vm_async_test(async move {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum PostLoadProgressSource {
            TypedMainDocumentRuntime {
                kind: crate::page_task_queue::PageMainDocumentRuntimeActionKind,
                effect: crate::page_task_queue::PageMainDocumentRuntimeTargetEffect,
            },
            TypedDynamicImportOwnerAction,
        }

        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-dynamic-root.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDynamicImportHostLoadEvents.push("module-start:" + (globalThis === self));
import("./child-dynamic-leaf.js").then(
  () => parent.__childDynamicImportHostLoadEvents.push(
    "dynamic-fulfilled:" + String(globalThis.__childDynamicImportLeafValue)
  ),
  () => parent.__childDynamicImportHostLoadEvents.push("dynamic-rejected")
);
parent.__childDynamicImportHostLoadEvents.push("module-after");
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/child-dynamic-leaf.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDynamicImportHostLoadEvents.push("dynamic-module:" + (globalThis === self));
globalThis.__childDynamicImportLeafValue = 701;
"#
                .to_owned(),
                Duration::from_millis(500),
            ),
        ])
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
            startup_sources,
            events_before_completion,
            completion_source,
            events_after_module_terminal,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
            post_load_progress_sources,
            events_after_dynamic_fetch_scheduled,
            dynamic_fetch_completion_source,
            host_load_pending_after_dynamic_fetch_completion,
            dynamic_owner_action_applied,
            events_after_dynamic_owner_action_body,
            events_after_dynamic_owner_action,
            host_load_pending_after_dynamic_owner_action,
        ) = local_executor
            // Keep this large, stateful scenario off nextest's comparatively
            // small test-thread stack; pinning does not add a scheduler turn.
            .run(Box::pin(async move {
                let mut page_vm = page_vm;
                let mut page_resource_queue = page_resource_queue;
                let mut owner_wake_rx = owner_wake_rx;
                let root_url = format!("{base_url}/child-dynamic-root.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childDynamicImportHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childDynamicImportHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <base href="{base_url}/">
    <script>parent.__childDynamicImportHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="dynamic-root" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("dynamic-root").addEventListener("load", () => {{
        parent.__childDynamicImportHostLoadEvents.push("script-load");
      }});
      document.getElementById("dynamic-root").addEventListener("error", () => {{
        parent.__childDynamicImportHostLoadEvents.push("script-error");
      }});
      parent.__childDynamicImportHostLoadEvents.push(
        "after:" + String(globalThis.__childDynamicImportLeafValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut startup_sources = Vec::new();
                for _ in 0..8 {
                    if page_resource_queue.has_ready_completion() {
                        break;
                    }
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    startup_sources.push(source);
                }
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx
                                .recv()
                                .await
                                .expect("owner-attached dynamic-import route should remain open");
                        }
                    })
                    .await
                    .expect("child dynamic root completion should arrive before timeout");
                }

                let completion = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("typed child root completion should consume one Page turn");
                let completion_source = completion.action.source();
                let (root_network_records, root_websocket_events, root_websocket_lifecycle) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(root_websocket_events.is_empty());
                assert!(root_websocket_lifecycle.is_empty());
                assert_eq!(root_network_records.len(), 1);
                assert_eq!(
                    root_network_records[0].request_initiator_type(),
                    SubresourceRequestInitiatorType::Parser
                );

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child dynamic import root terminal fanout",
                )
                .await;
                let events_after_module_terminal = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                let script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child dynamic import root module execution",
                )
                .await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "child dynamic import iframe load before dynamic fetch",
                )
                .await;
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                let mut post_load_progress_sources = Vec::new();
                let mut events_after_dynamic_fetch_scheduled = String::new();
                for _ in 0..8 {
                    if page_resource_queue.has_ready_completion() {
                        break;
                    }
                    if let Some(outcome) = page_vm
                        .run_page_main_document_runtime_body_for_test(&loader)
                        .await?
                    {
                        post_load_progress_sources.push(
                            PostLoadProgressSource::TypedMainDocumentRuntime {
                                kind: outcome.action.kind(),
                                effect: outcome.action.target_effect(),
                            },
                        );
                        events_after_dynamic_fetch_scheduled = page_vm
                            .vm_mut()
                            .eval("__childDynamicImportHostLoadEvents.join('|')")?;
                        continue;
                    }
                    if page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
                        .has_ready_task()
                    {
                        let action = page_vm
                            .run_page_dynamic_import_owner_action_body_for_test()
                            .expect("typed dynamic-import owner action should win its Page turn");
                        let crate::page_task_queue::PageDynamicImportOwnerActionDocumentEffect::AppliedToCurrentOwner {
                            outcome,
                        } = action.action.document_effect
                        else {
                            panic!("current dynamic-import owner action must not be stale: {action:?}");
                        };
                        assert!(
                            outcome.waiting_fetch_was_scheduled(),
                            "the first typed owner action should schedule the dynamic fetch"
                        );
                        page_vm
                            .finish_selected_page_task_completion(
                                action.action.into_page_task_completion(),
                                &loader,
                            )
                            .await?;
                        post_load_progress_sources
                            .push(PostLoadProgressSource::TypedDynamicImportOwnerAction);
                        events_after_dynamic_fetch_scheduled = page_vm
                            .vm_mut()
                            .eval("__childDynamicImportHostLoadEvents.join('|')")?;
                        continue;
                    }
                    break;
                }

                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx.recv().await.expect(
                                "owner-attached dynamic-import completion route should remain open",
                            );
                        }
                    })
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "child dynamic import completion should arrive before timeout: progress={post_load_progress_sources:?} events={events_after_dynamic_fetch_scheduled}"
                        )
                    });
                }
                let activity_epoch_before_dynamic_completion =
                    page_vm.vm().subresource_activity_epoch();
                let dynamic_fetch_completion = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("child dynamic import fetch completion should be ready");
                assert_eq!(
                    dynamic_fetch_completion.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    dynamic_fetch_completion.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );
                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_dynamic_completion,
                    "a current dynamic-import Network fact must advance current Document activity"
                );
                assert!(
                    !page_resource_queue.has_ready_completion(),
                    "one producer terminal must be consumed exactly once"
                );
                let (dynamic_network_records, dynamic_websocket_events, dynamic_websocket_lifecycle) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(dynamic_websocket_events.is_empty());
                assert!(dynamic_websocket_lifecycle.is_empty());
                assert_eq!(dynamic_network_records.len(), 1);
                assert_eq!(
                    dynamic_network_records[0].url().as_str(),
                    format!("{base_url}/child-dynamic-leaf.js")
                );
                assert_eq!(
                    dynamic_network_records[0].request_initiator_type(),
                    SubresourceRequestInitiatorType::Script
                );
                let dynamic_fetch_completion_source = dynamic_fetch_completion.action.source();
                let host_load_pending_after_dynamic_fetch_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

                let dynamic_owner_action = page_vm
                    .run_page_dynamic_import_owner_action_body_for_test()
                    .expect("dynamic-import ready action should consume one typed Page turn");
                let crate::page_task_queue::PageDynamicImportOwnerActionDocumentEffect::AppliedToCurrentOwner {
                    outcome: dynamic_owner_action_outcome,
                } = dynamic_owner_action.action.document_effect
                else {
                    panic!("current dynamic-import ready action must not be stale: {dynamic_owner_action:?}");
                };
                let dynamic_owner_action_applied =
                    dynamic_owner_action_outcome.evaluation_import_was_resolved();
                let events_after_dynamic_owner_action_body = page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        "__childDynamicImportHostLoadEvents.join('|')",
                    )?;
                page_vm
                    .finish_selected_page_task_completion(
                        dynamic_owner_action.action.into_page_task_completion(),
                        &loader,
                    )
                    .await?;
                let events_after_dynamic_owner_action = page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        "__childDynamicImportHostLoadEvents.join('|')",
                    )?;
                let host_load_pending_after_dynamic_owner_action = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

                Ok::<_, anyhow::Error>((
                    startup_sources,
                    events_before_completion,
                    completion_source,
                    events_after_module_terminal,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    events_after_host_load,
                    post_load_progress_sources,
                    events_after_dynamic_fetch_scheduled,
                    dynamic_fetch_completion_source,
                    host_load_pending_after_dynamic_fetch_completion,
                    dynamic_owner_action_applied,
                    events_after_dynamic_owner_action_body,
                    events_after_dynamic_owner_action,
                    host_load_pending_after_dynamic_owner_action,
                ))
            }))
            .await
            .expect("page vm child dynamic import HostLoad proof should run");

        assert!(
            startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "child dynamic import startup should start the root module fetch from a typed source: {startup_sources:?}"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "pending root module should not run dynamic import or dispatch iframe load before completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_terminal, "before:true|after:undefined",
            "ModuleScriptTerminal should not execute the root module inline"
        );
        assert_eq!(
            script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "root module graph-ready work should execute through DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true|after:undefined|module-start:true|module-after|script-load",
            "DocumentScriptReady should execute the root module and dispatch script load without iframe load or dynamic import completion"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a HostLoad turn after the root module script queues dynamic import"
        );
        assert_eq!(
            events_after_host_load,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load",
            "HostLoad should dispatch iframe load before dynamic import fetch scheduling or completion"
        );
        assert!(
            post_load_progress_sources.contains(
                &PostLoadProgressSource::TypedMainDocumentRuntime {
                    kind: crate::page_task_queue::PageMainDocumentRuntimeActionKind::DynamicModuleJob,
                    effect: crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner,
                },
            ),
            "dynamic import should advance through its concrete exact-main-Document task: {post_load_progress_sources:?}"
        );
        let dynamic_module_job = PostLoadProgressSource::TypedMainDocumentRuntime {
            kind: crate::page_task_queue::PageMainDocumentRuntimeActionKind::DynamicModuleJob,
            effect: crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner,
        };
        let materialize_position = post_load_progress_sources
            .iter()
            .position(|source| *source == dynamic_module_job)
            .expect("dynamic import should expose its concrete graph-advance turn");
        let owner_action_position = post_load_progress_sources
            .iter()
            .position(|source| *source == PostLoadProgressSource::TypedDynamicImportOwnerAction)
            .expect("dynamic import graph advance should publish an exact child owner action");
        assert!(
            owner_action_position > materialize_position,
            "the exact child owner action must follow its graph advance without imposing a cross-source adjacency rule: {post_load_progress_sources:?}"
        );
        assert_eq!(
            events_after_dynamic_fetch_scheduled,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load",
            "dynamic import runtime work should schedule fetch without executing the dynamic module after iframe load"
        );
        assert_eq!(
            dynamic_fetch_completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert!(
            !host_load_pending_after_dynamic_fetch_completion,
            "dynamic import fetch completion should not queue HostLoad after the iframe has already loaded"
        );
        assert!(
            dynamic_owner_action_applied,
            "dynamic import ready action should resolve from the typed Page source"
        );
        assert_eq!(
            events_after_dynamic_owner_action_body,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load|dynamic-module:true",
            "DynamicImportOwnerAction body must leave the user import reaction for selected-task completion"
        );
        assert_eq!(
            events_after_dynamic_owner_action,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load|dynamic-module:true|dynamic-fulfilled:701",
            "selected-task completion should fulfill the child dynamic import after the body returns"
        );
        assert!(
            !host_load_pending_after_dynamic_owner_action,
            "dynamic import ready action should not requeue HostLoad"
        );

        server
            .await
            .expect("child dynamic import HostLoad proof server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_dependency_completion_queues_module_script_terminal_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-root-module.js",
                "HTTP/1.1 200 OK",
                r#"
import { depValue } from "./child-dependency.js";
parent.__childModuleDependencyWaitEvents.push("root:" + depValue);
globalThis.__childModuleDependencyWaitValue = depValue;
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/child-dependency.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDependencyWaitEvents.push("dep:" + (globalThis === self));
export const depValue = "dep-value";
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
            dependency,
            events_after_dependency_completion,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/child-root-module.js");
                let dependency_url = format!("{base_url}/child-dependency.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDependencyWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDependencyWaitEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleDependencyWaitEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModuleDependencyWaitEvents.push("script-load");
      }});
      parent.__childModuleDependencyWaitEvents.push(
        "after:" + String(globalThis.__childModuleDependencyWaitValue)
      );
    <\/script>
  `;
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
                    startup_sources.contains(&ChildFrameSemanticTurnKind::DocumentScriptReady),
                    "child module dependency startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
                );
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
                    "child module dependency startup should start the root fetch from its typed source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyWaitEvents.join('|')")?,
                    "before:true|after:undefined",
                    "parser should continue past child module script while dependency fetches are pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child parser module root completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child parser module root completion sender should remain open"
                    );
                }

                let root = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    root.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    root.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    root.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child dependency root terminal",
                )
                .await;
                let (root_network_records, _, _) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert_eq!(root_network_records.len(), 1);
                assert_eq!(root_network_records[0].url().as_str(), root_url);
                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildModuleDependencyFetchStart,
                            &loader,
                        )
                        .await?,
                    "module terminal should queue one selected dependency-start task",
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child module dependency completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child module dependency completion sender should remain open"
                    );
                }

                let activity_epoch_before_dependency =
                    page_vm.vm().subresource_activity_epoch();
                let dependency =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    dependency.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    dependency.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_dependency,
                    "a current child dependency Network terminal must advance current Document activity"
                );
                let (dependency_network_records, _, _) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert_eq!(dependency_network_records.len(), 1);
                let dependency_network_record = &dependency_network_records[0];
                assert_eq!(dependency_network_record.url().as_str(), dependency_url);
                assert!(dependency_network_record.frame_id().is_some());
                assert_eq!(
                    dependency_network_record.request_initiator_type(),
                    SubresourceRequestInitiatorType::Parser
                );
                let events_after_dependency_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyWaitEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child module dependency terminal fanout",
                )
                .await;
                let mut followup_sources = Vec::new();
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyWaitEvents.join('|')")?,
                    "before:true|after:undefined",
                    "dependency terminal fanout should not execute the module graph inline"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child module dependency graph-ready execution",
                )
                .await,
                );
                let events_after_graph_ready = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyWaitEvents.join('|')")?;
                assert_eq!(
                    events_after_graph_ready,
                    "before:true|after:undefined|dep:true|root:dep-value|script-load",
                    "dependency graph-ready execution should dispatch script load without iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child module dependency iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child module dependency follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    dependency,
                    events_after_dependency_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module dependency deferred completion test should run");

        assert!(matches!(
            dependency.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_dependency_completion, "before:true|after:undefined",
            "dependency completion turn should not inline-run child module graph"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "graph-ready, DOMContentLoaded, complete, and HostLoad must remain separate child turns after typed terminal fanout"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|dep:true|root:dep-value|script-load|frame-load",
            "explicit later turns should run queued child module graph work before iframe load"
        );

        server
            .await
            .expect("child module dependency wait-path server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_child_module_dependency_failure_queues_graph_failed_before_host_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-root-module.js",
                "HTTP/1.1 200 OK",
                r#"
import { depValue } from "./missing-child-dependency.js";
parent.__childModuleDependencyFailureEvents.push("root:" + depValue);
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/missing-child-dependency.js",
                "HTTP/1.1 404 Not Found",
                "missing dependency".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            dependency,
            events_after_dependency_completion,
            followup_sources,
            events_after_graph_failure,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/child-root-module.js");
                let dependency_url = format!("{base_url}/missing-child-dependency.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDependencyFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDependencyFailureEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleDependencyFailureEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModuleDependencyFailureEvents.push("script-load");
      }});
      document.getElementById("external-module").addEventListener("error", () => {{
        parent.__childModuleDependencyFailureEvents.push("script-error");
      }});
      parent.__childModuleDependencyFailureEvents.push("after:" + String(globalThis.__childModuleDependencyFailureValue));
    <\/script>
  `;
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
                    startup_sources.contains(&ChildFrameSemanticTurnKind::DocumentScriptReady),
                    "child module dependency failure startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
                );
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
                    "child module dependency failure startup should start the root fetch from its typed source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyFailureEvents.join('|')")?,
                    "before:true|after:undefined",
                    "parser should continue past child module script while dependency fetch is pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child parser module root completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child parser module root completion sender should remain open"
                    );
                }

                run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child dependency-failure root terminal",
                )
                .await;
                let _ = page_vm.vm_mut().take_network_output();
                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildModuleDependencyFetchStart,
                            &loader,
                        )
                        .await?,
                    "module terminal should queue one selected dependency-start task",
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect(
                        "child module dependency failure completion should arrive before timeout",
                    );
                    assert!(
                        arrived,
                        "child module dependency failure completion sender should remain open"
                    );
                }

                let dependency =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    dependency.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    dependency.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                let (dependency_network_records, _, _) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert_eq!(dependency_network_records.len(), 1);
                assert_eq!(
                    dependency_network_records[0].url().as_str(),
                    dependency_url
                );
                assert!(matches!(
                    dependency_network_records[0].outcome(),
                    SubresourceNetworkOutcome::Success { status: 404, .. }
                ));
                let events_after_dependency_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyFailureEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child module dependency failure terminal fanout",
                )
                .await;
                let mut followup_sources = Vec::new();
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyFailureEvents.join('|')")?,
                    "before:true|after:undefined",
                    "dependency failure terminal fanout should not dispatch script error or iframe load inline"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child module dependency graph-failed dispatch",
                    )
                    .await,
                );
                let events_after_graph_failure = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyFailureEvents.join('|')")?;
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency failure DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency failure complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child module dependency failure iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyFailureEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child module dependency failure sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    dependency,
                    events_after_dependency_completion,
                    followup_sources,
                    events_after_graph_failure,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module dependency failure test should run");

        assert!(matches!(
            dependency.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_dependency_completion, "before:true|after:undefined",
            "dependency failure completion turn should not inline-dispatch script error"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "graph-failed dispatch, DOMContentLoaded, complete, and HostLoad must remain separate child turns after typed terminal fanout"
        );
        assert_eq!(
            events_after_graph_failure, "before:true|after:undefined|script-error",
            "graph-failed work should dispatch script error without iframe load"
        );
        assert_eq!(
            final_events, "before:true|after:undefined|script-error|frame-load",
            "HostLoad should dispatch iframe load only after dependency failure finalizes"
        );

        server
            .await
            .expect("child module dependency failure server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_parse_failure_blocks_host_load_until_exception_is_reported() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-bad-module.js",
            "HTTP/1.1 200 OK",
            "import {".to_owned(),
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
            graph_failure_source,
            events_after_graph_failure,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-bad-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleFailureHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleFailureHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleFailureHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="bad-module" type="module" src="{script_url}"><\/script>
    <script>
      addEventListener("error", event => {{
        parent.__childModuleFailureHostLoadEvents.push("window-error:" + (
          event instanceof ErrorEvent && event.error instanceof SyntaxError &&
          event.target === window && event.isTrusted
        ));
        event.preventDefault();
      }});
      document.getElementById("bad-module").addEventListener("load", () => {{
        parent.__childModuleFailureHostLoadEvents.push("script-load");
      }});
      document.getElementById("bad-module").addEventListener("error", () => {{
        parent.__childModuleFailureHostLoadEvents.push("script-error");
      }});
      parent.__childModuleFailureHostLoadEvents.push("after");
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
                        .eval("__childModuleFailureHostLoadEvents.join('|')")?;
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
                    .expect("child bad module completion should arrive before timeout");
                    assert!(arrived, "child bad module completion sender should remain open");
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child graph-failure module terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleFailureHostLoadEvents.join('|')")?;

                let graph_failure_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_graph_failure = page_vm
                    .vm_mut()
                    .eval("__childModuleFailureHostLoadEvents.join('|')")?;

                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child module graph-failure iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleFailureHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    graph_failure_source,
                    events_after_graph_failure,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child bad module HostLoad gate test should run");

        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "pre-completion turns should start the module root fetch from its typed source: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed bad module response should leave a window to prove HostLoad is gated before completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after",
            "pending bad module script must block iframe load before resource completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after",
            "module owner event should not report an exception or dispatch script or iframe load inline"
        );
        assert_eq!(
            graph_failure_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "graph failure should dispatch through DocumentScriptReady"
        );
        assert_eq!(
            events_after_graph_failure, "before:true|after|window-error:true|script-load",
            "parse failure should report to its Window and fire external script load without iframe load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source after graph failure"
        );
        assert_eq!(
            final_events, "before:true|after|window-error:true|script-load|frame-load",
            "HostLoad should dispatch iframe load only after graph failure finalizes"
        );

        server
            .await
            .expect("child bad module HostLoad gate server should finish");
    })
    .await;
}
