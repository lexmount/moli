use super::*;

use crate::page_resource_completion::{
    MainParserModuleGraphFetchCompletion, MainParserModuleGraphFetchTarget,
    RendererPageResourceCompletionLocalOwner,
};

fn parser_module_completion(
    target: MainParserModuleGraphFetchTarget,
    request_url: Url,
    source: std::result::Result<&str, &str>,
    network_result: Option<crate::types::SharedNavigationResponseResult>,
) -> MainParserModuleGraphFetchCompletion {
    let result = source
        .map(|source| {
            ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text(source.to_owned()),
            )
        })
        .map_err(str::to_owned);
    MainParserModuleGraphFetchCompletion::new(target, result, network_result, request_url)
}

fn arbitrary_parser_module_target(
    page_vm: &PageVm,
    parser_position: usize,
    load_id: u64,
) -> MainParserModuleGraphFetchTarget {
    let owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("test PageVm should retain a main Document owner");
    MainParserModuleGraphFetchTarget::new(
        ParserPendingScriptId::from_key(
            MainParserDocumentOwner::new(owner),
            ParserPendingScriptKey::from_parts_for_test(
                parser_position,
                NodeId::new(parser_position + 1000),
            ),
        ),
        load_id,
    )
}

fn install_loading_parser_module(
    page_vm: &mut PageVm,
    module_url: Url,
) -> (MainParserModuleGraphFetchTarget, PostParsePageOwnedWork) {
    let script = prepared_external_module_for_page_vm_test_with_node(page_vm, 9801, module_url);
    let work = install_parser_module_defer_work(page_vm, script);
    let target = page_vm
        .vm()
        .current_main_parser_module_graph_fetch_target(0)
        .expect("parser graph start should register its exact root fetch target");
    (target, work)
}

fn enqueue_parser_module_completion(
    queue: &mut RendererPageNetworkingSource,
    root_document: crate::runtime::RendererDocumentToken,
    completion: MainParserModuleGraphFetchCompletion,
) {
    queue.enqueue_local_for_test(
        RendererPageResourceCompletion::main_parser_module_graph_fetch(root_document, completion),
    );
}

async fn run_ready_native_module_owner_turns(page_vm: &mut PageVm) -> anyhow::Result<()> {
    while page_vm.vm_mut().has_ready_native_module_owner_actions() {
        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test()
            .await?
            .expect("one native module owner event should retain one typed main runtime task");
        assert_eq!(
            outcome.action.kind(),
            PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent
        );
        assert_eq!(
            outcome.action.target_effect(),
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
        );
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn production_parser_module_fetch_uses_stable_typed_route_and_applies_registered_graph() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/typed-main-parser.mjs",
            "HTTP/1.1 200 OK",
            "globalThis.__typedMainParserModule = 1; export const value = 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let module_url =
            Url::parse(&format!("{base_url}/typed-main-parser.mjs")).expect("module URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (target, parser_work) = install_loading_parser_module(&mut page_vm, module_url.clone());
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(parser_work)
            .await?;

        super::child_document_completion::wait_for_page_resource_completion(
            &mut page_vm,
            &mut queue,
            &mut wake_rx,
            "main parser module fetch",
        )
        .await;
        assert_eq!(
            queue.next_ready_owner(),
            Some(
                RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                    page_vm.document_lifecycle.identity().document,
                    target,
                )
            )
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("registered parser module terminal should consume one typed turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert_eq!(
            outcome.action.source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert!(!queue.has_ready_completion());

        run_ready_native_module_owner_turns(&mut page_vm).await?;
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            "typed main parser module completion",
        )
        .await;
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            0,
            "typed main parser module completion",
        )
        .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__typedMainParserModule)")?,
            "1"
        );
        assert!(
            page_vm
                .report
                .runs
                .iter()
                .any(|run| run.url() == &module_url),
            "typed terminal should complete the registered parser graph"
        );

        server
            .await
            .expect("main parser module server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production typed main parser module test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn static_dependency_fetch_reenters_the_same_typed_parser_module_source() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/typed-parser-root.mjs",
                "HTTP/1.1 200 OK",
                r#"import "./typed-parser-dependency.mjs";
(globalThis.__typedParserModuleEvents ??= []).push("root");"#
                    .to_owned(),
                Duration::ZERO,
            ),
            (
                "/typed-parser-dependency.mjs",
                "HTTP/1.1 200 OK",
                r#"(globalThis.__typedParserModuleEvents ??= []).push("dependency");
export const dependency = true;"#
                    .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let root_url =
            Url::parse(&format!("{base_url}/typed-parser-root.mjs")).expect("root module URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let (root_target, parser_work) =
            install_loading_parser_module(&mut page_vm, root_url);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(parser_work)
            .await?;
        super::child_document_completion::wait_for_page_resource_completion(&mut page_vm,
&mut queue,
&mut wake_rx,
"main parser module root fetch")
        .await;

        let root_document = page_vm.document_lifecycle.identity().document;
        assert_eq!(
            queue.next_ready_owner(),
            Some(RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                root_document,
                root_target,
            ))
        );
        let root_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("parser module root should consume one typed terminal turn");
        assert_eq!(
            root_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        run_ready_native_module_owner_turns(&mut page_vm).await?;

        super::child_document_completion::wait_for_page_resource_completion(&mut page_vm,
&mut queue,
&mut wake_rx,
"main parser module dependency fetch")
        .await;
        let dependency_owner = queue
            .next_ready_owner()
            .expect("dependency terminal should retain an exact owner");
        assert_eq!(dependency_owner.root_document(), root_document);
        let RendererPageResourceCompletionLocalOwner::MainParserModuleGraphFetch(
            dependency_target,
        ) = dependency_owner.local_owner()
        else {
            panic!("dependency terminal should remain parser-module owned: {dependency_owner:?}");
        };
        assert_ne!(
            dependency_target.load_id(),
            root_target.load_id(),
            "the dependency must retain its own fetch continuation"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_parser_module_graph_fetch_target(dependency_target.load_id()),
            Some(dependency_target),
            "the dependency target must still resolve through the original PendingScript"
        );

        let dependency_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("parser module dependency should consume a second typed terminal turn");
        assert_eq!(
            dependency_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        run_ready_native_module_owner_turns(&mut page_vm).await?;
        run_ready_parser_deferred_body_for_test(&mut page_vm,
"typed parser module dependency completion")
        .await;
        run_parser_module_completion_turns_for_test(&mut page_vm,
0,
"typed parser module dependency completion")
        .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__typedParserModuleEvents.join('|')")?,
            "dependency|root"
        );
        assert!(!queue.has_ready_completion());

        server
            .await
            .expect("typed parser module dependency server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed parser module dependency test should run");
}

#[test]
fn real_page_vm_replacement_rejects_naturally_colliding_parser_module_target() {
    run_page_vm_large_stack_async_test(
        "main-parser-module-real-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![
                (
                    "/old-parser-module.mjs",
                    "HTTP/1.1 200 OK",
                    "globalThis.__oldParserModuleMustNotRun = true;".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement.html",
                    "HTTP/1.1 200 OK",
                    "<!doctype html><body>replacement</body>".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement-parser-module.mjs",
                    "HTTP/1.1 200 OK",
                    "globalThis.__replacementParserModuleRan = true;".to_owned(),
                    Duration::ZERO,
                ),
            ])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let initial_url =
                Url::parse(&format!("{base_url}/initial.html")).expect("initial page URL");
            let (page_vm, queue, wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, initial_url.clone());
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let mut queue = queue;
                    let mut wake_rx = wake_rx;
                    let old_module_url = Url::parse(&format!(
                        "{base_url}/old-parser-module.mjs"
                    ))
                    .expect("old module URL");
                    let (old_target, old_work) =
                        install_loading_parser_module(&mut page_vm, old_module_url.clone());
                    page_vm
                        .execute_post_parse_page_owned_task_on_named_owner_lane(old_work)
                        .await?;
                    super::child_document_completion::wait_for_page_resource_completion(&mut page_vm,
&mut queue,
&mut wake_rx,
"old PageVm parser module fetch")
                    .await;
                    let (network_records, _, _) = split_network_output_items(page_vm.vm_mut().take_network_output());
                    let (_, old_envelope) = queue
                        .pop_front()
                        .expect("old PageVm parser module terminal should remain queued");
                    let old_root = page_vm.document_lifecycle.identity().document;

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm.vm_mut().eval(&format!(
                        "location.href = {replacement_url:?}; 'queued'"
                    ))?;
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
                    let replacement_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(old_root, replacement_root);

                    let replacement_module_url = Url::parse(&format!(
                        "{base_url}/replacement-parser-module.mjs"
                    ))
                    .expect("replacement module URL");
                    let (replacement_target, replacement_work) = install_loading_parser_module(
                        &mut page_vm,
                        replacement_module_url,
                    );
                    page_vm
                        .execute_post_parse_page_owned_task_on_named_owner_lane(replacement_work)
                        .await?;
                    super::child_document_completion::wait_for_page_resource_completion(&mut page_vm,
&mut queue,
&mut wake_rx,
"replacement PageVm parser module fetch")
                    .await;
                    let (_, replacement_envelope) = queue
                        .pop_front()
                        .expect("replacement PageVm parser module terminal should remain queued");
                    assert_eq!(
                        old_target, replacement_target,
                        "fresh PageVm counters and the identical parser slot should naturally reuse the full local target"
                    );

                    let _ = page_vm.vm_mut().take_network_output();
                    queue.enqueue_local_for_test(old_envelope);
                    queue.enqueue_local_for_test(replacement_envelope);
                    let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
                    let stale = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("old PageVm parser terminal should consume one stale turn");
                    assert_eq!(
                        stale.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(
                                RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                                    replacement_root,
                                    replacement_target,
                                ),
                            ),
                        }
                    );
                    assert_eq!(
                        stale.action.output_effect,
                        PageResourceCompletionOutputEffect::None
                    );
                    assert_eq!(
                        page_vm.vm().subresource_activity_epoch(),
                        activity_epoch_before,
                        "historical old-PageVm Network output must not become replacement activity"
                    );
                    assert!(page_vm.vm_mut().take_network_output().is_empty(), "a stale business callback cannot manufacture Network output");
                    assert_eq!(
                        network_records.len(),
                        1,
                        "the original physical response must have published exactly one Network fact"
                    );
                    assert_eq!(network_records[0].document_url(), &initial_url);
                    assert_eq!(network_records[0].url(), &old_module_url);
                    assert_eq!(
                        network_records[0].resource_type(),
                        SubresourceResourceType::Script
                    );
                    assert_eq!(
                        network_records[0].request_initiator_type(),
                        SubresourceRequestInitiatorType::Parser
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .current_main_parser_module_graph_fetch_target(
                                replacement_target.load_id(),
                            ),
                        Some(replacement_target),
                        "the old root namespace must not settle the replacement PendingScript"
                    );

                    let current = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("replacement parser terminal should retain application authority");
                    assert_eq!(
                        current.action.document_effect,
                        PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                    );
                    run_ready_native_module_owner_turns(&mut page_vm).await?;
                    let interactive = poll_post_parse_document_processing_action_for_test(
                        &mut page_vm,
                    )
                    .expect("replacement navigation should expose its interactive action first");
                    let crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(
                        interactive,
                    ) = interactive
                    else {
                        panic!(
                            "replacement navigation should expose page-owned interactive work"
                        );
                    };
                    assert!(
                        interactive.is_main_document_interactive_task(),
                        "replacement parser marker must follow the exact Document's interactive action, got {interactive:?}"
                    );
                    page_vm
                        .execute_post_parse_page_owned_task_on_named_owner_lane(*interactive)
                        .await?;
                    run_ready_parser_deferred_body_for_test(&mut page_vm,
"replacement PageVm parser module completion")
                    .await;
                    run_parser_module_completion_turns_for_test(&mut page_vm,
0,
"replacement PageVm parser module completion")
                    .await;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("String(globalThis.__replacementParserModuleRan)")?,
                        "true"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("String(globalThis.__oldParserModuleMustNotRun)")?,
                        "undefined"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("real PageVm parser-module replacement fixture should run");

            server
                .await
                .expect("real PageVm parser-module replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_terminal_failure_applies_to_its_registered_pending_script() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.test/parser-module-failure.html").expect("document URL");
        let module_url =
            Url::parse("https://example.test/parser-module-failure.mjs").expect("module URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());
        let (target, parser_work) = install_loading_parser_module(&mut page_vm, module_url.clone());
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(parser_work)
            .await?;
        let root_document = page_vm.document_lifecycle.identity().document;
        let mut queue = RendererPageNetworkingSource::new_for_test();
        enqueue_parser_module_completion(
            &mut queue,
            root_document,
            parser_module_completion(
                target,
                module_url.clone(),
                Err("typed parser module fetch failed"),
                None,
            ),
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("registered parser failure should consume one terminal turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        run_ready_parser_deferred_body_for_test(&mut page_vm, "typed parser module failure").await;
        let failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &module_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("typed parser failure should settle the exact PendingScript");
        assert!(failure.contains("typed parser module fetch failed"));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed main parser failure test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_makes_queued_parser_module_terminal_stale_without_network_effect() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.test/parser-module-old-document.html")
            .expect("document URL");
        let module_url =
            Url::parse("https://example.test/parser-module-old-document.mjs").expect("module URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());
        let (target, _parser_work) =
            install_loading_parser_module(&mut page_vm, module_url.clone());
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm.vm_mut().eval("document.open(); 'replaced'")?;
        assert_eq!(
            page_vm
                .vm()
                .current_main_parser_module_graph_fetch_target(target.load_id()),
            None,
            "document.open must retire the old PendingScript fetch target"
        );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        let mut queue = RendererPageNetworkingSource::new_for_test();
        enqueue_parser_module_completion(
            &mut queue,
            root_document,
            parser_module_completion(
                target,
                module_url.clone(),
                Err("old parser module response"),
                Some(Arc::new(Err(
                    "old parser module transport failed".to_owned()
                ))),
            ),
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale parser module terminal should consume one discard turn");
        assert!(matches!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
        ));
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "historical parser module Network output must not become replacement activity"
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "business completion must not manufacture Network output"
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Page stale parser module test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_source_consumes_one_terminal_per_turn_in_fifo_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let first_target = arbitrary_parser_module_target(&page_vm, 61, 701);
        let second_target = arbitrary_parser_module_target(&page_vm, 62, 702);

        let mut queue = RendererPageNetworkingSource::new_for_test();
        enqueue_parser_module_completion(
            &mut queue,
            root_document,
            parser_module_completion(
                first_target,
                Url::parse("https://example.test/parser-module-first.mjs").unwrap(),
                Err("first stale terminal"),
                None,
            ),
        );
        enqueue_parser_module_completion(
            &mut queue,
            root_document,
            parser_module_completion(
                second_target,
                Url::parse("https://example.test/parser-module-second.mjs").unwrap(),
                Err("second stale terminal"),
                None,
            ),
        );

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("first parser module terminal should consume one turn");
        assert_eq!(
            first.action.owner,
            RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                root_document,
                first_target,
            )
        );

        assert!(queue.has_ready_completion());

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("second parser module terminal should consume its own turn");
        assert_eq!(
            second.action.owner,
            RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                root_document,
                second_target,
            )
        );

        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("parser module FIFO terminal test should run");
}
