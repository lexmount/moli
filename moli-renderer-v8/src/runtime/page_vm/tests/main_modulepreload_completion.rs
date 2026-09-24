use super::*;

use crate::dom::native::NativeNodeId;
use crate::module_runtime::{
    ModulePreloadJobRun, NativeModulepreloadFetchStart, NativeModulepreloadLinkClient,
};
use crate::page_resource_completion::{
    MainModulepreloadFetchCompletion, MainModulepreloadFetchTarget,
    RendererPageResourceCompletionLocalOwner,
};

fn modulepreload_request(
    document_url: &Url,
    request_url: &Url,
) -> (ModuleMapKey, NativeModuleSingleFetchRequest) {
    let key = ModuleMapKey::java_script(request_url.clone());
    let request = NativeModuleSingleFetchRequest::new(
        request_url.clone(),
        request_url.clone(),
        document_url.clone(),
        key.clone(),
        ModuleFetchMetadata::default(),
    );
    (key, request)
}

fn modulepreload_completion(
    target: MainModulepreloadFetchTarget,
    request_url: Url,
    source: std::result::Result<&str, &str>,
) -> MainModulepreloadFetchCompletion {
    let result = source
        .map(|source| {
            ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text(source.to_owned()),
            )
        })
        .map_err(str::to_owned);
    MainModulepreloadFetchCompletion::new(target, result, None, request_url)
}

fn reserve_modulepreload_without_network(
    page_vm: &mut PageVm,
    document_url: &Url,
    request_url: &Url,
) -> (ModuleMapKey, MainModulepreloadFetchTarget) {
    let (key, request) = modulepreload_request(document_url, request_url);
    let start = page_vm
        .vm_mut()
        .document_runtime
        .fetch_single_native_module_for_modulepreload(request)
        .expect("modulepreload registration should succeed");
    let NativeModulepreloadFetchStart::Started(request) = start else {
        panic!("fresh modulepreload key should reserve one fetch");
    };
    let load_id = page_vm
        .vm_mut()
        .document_runtime
        .suspend_native_modulepreload_fetch(*request);
    let target = page_vm
        .vm()
        .current_main_modulepreload_fetch_target(load_id)
        .expect("suspended modulepreload should expose its exact target");
    (key, target)
}

fn enqueue_modulepreload_completion(
    queue: &mut RendererPageNetworkingSource,
    root_document: crate::runtime::RendererDocumentToken,
    completion: MainModulepreloadFetchCompletion,
) {
    queue.enqueue_local_for_test(RendererPageResourceCompletion::main_modulepreload_fetch(
        root_document,
        completion,
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn native_module_owner_event_tail_rearms_one_exact_turn_at_a_time() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/native-owner-event-tail.html").expect("document URL");
        let (mut page_vm, _resource_source, _wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm
            .vm_mut()
            .document_runtime
            .post_modulepreload_link_error_event(NativeNodeId::new(7201));
        page_vm
            .vm_mut()
            .document_runtime
            .post_modulepreload_link_error_event(NativeNodeId::new(7202));

        for turn_index in 0..2 {
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(&format!(
                "globalThis.__nativeOwnerTurnCheckpoint = {turn_index}; Promise.resolve().then(() => __nativeOwnerTurnCheckpoint += 1); 'queued'",
            ))?;
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                            PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent,
                        ))
                    .await?,
                "each posted native owner event should retain one selected turn"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        "String(globalThis.__nativeOwnerTurnCheckpoint)",
                    )?,
                (turn_index + 1).to_string(),
                "each concrete current owner event must finish its own checkpoint"
            );
            assert_eq!(
                page_vm.vm_mut().has_ready_native_module_owner_actions(),
                turn_index == 0,
                "the first turn must retain and rearm exactly one tail event"
            );
        }
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent,
                    ))
                .await?,
            "two posted events must not create a phantom third turn"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("native module owner-event one-turn test should run");
}

fn modulepreload_performance_summary(
    page_vm: &mut PageVm,
    request_url: &Url,
) -> anyhow::Result<String> {
    page_vm.vm_mut().eval(&format!(
        "performance.getEntriesByName({:?}).map(entry => entry.initiatorType).join('|')",
        request_url.as_str()
    ))
}

#[tokio::test(flavor = "current_thread")]
async fn production_main_modulepreload_uses_stable_typed_route_and_applies_module_map() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/typed-main-modulepreload.mjs",
            "HTTP/1.1 200 OK",
            "export const preloaded = true;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).unwrap();
        let request_url = Url::parse(&format!("{base_url}/typed-main-modulepreload.mjs")).unwrap();
        let (key, request) = modulepreload_request(&document_url, &request_url);
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());

        assert_eq!(
            page_vm
                .vm_mut()
                .register_native_modulepreload_for_owner(request)
                .map_err(anyhow::Error::msg)?,
            Some(ModulePreloadJobRun::Scheduled)
        );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        super::child_document_completion::wait_for_page_resource_completion(
            &mut page_vm,
            &mut queue,
            &mut wake_rx,
            "main modulepreload fetch",
        )
        .await;
        let owner = queue
            .next_ready_owner()
            .expect("modulepreload terminal should retain an exact owner");
        assert_eq!(
            owner.root_document(),
            page_vm.document_lifecycle.identity().document
        );
        let RendererPageResourceCompletionLocalOwner::MainModulepreloadFetch(target) =
            owner.local_owner()
        else {
            panic!("production terminal should use the main modulepreload source: {owner:?}");
        };
        assert_eq!(
            page_vm
                .vm()
                .current_main_modulepreload_fetch_target(target.load_id()),
            Some(target),
            "exact-target lookup must be non-consuming before arbitration"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_modulepreload_fetch_target(target.load_id()),
            Some(target),
            "repeated readiness checks must not checkout the in-flight request"
        );

        page_vm
            .vm_mut()
            .eval("performance.clearResourceTimings()")?;
        let (records, _, _) = split_network_output_items(page_vm.vm_mut().take_network_output());
        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("current modulepreload terminal should consume one typed turn");
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

        assert_eq!(
            page_vm
                .vm()
                .current_main_modulepreload_fetch_target(target.load_id()),
            None,
            "application must consume the exact in-flight request"
        );
        let entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&key)
            .expect("successful modulepreload should remain in the Document module map");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(entry),
            ModuleMapEntryState::Fetched
        );
        assert!(
            page_vm.vm().subresource_activity_epoch() > activity_epoch_before,
            "current modulepreload Network output should count as current Document activity"
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "the business terminal must not republish its physical response"
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].document_url(), &document_url);
        assert_eq!(records[0].url(), &request_url);
        assert_eq!(
            records[0].request_initiator_type(),
            SubresourceRequestInitiatorType::Parser
        );
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Script);
        assert_eq!(
            modulepreload_performance_summary(&mut page_vm, &request_url)?,
            "link",
            "modulepreload should retain its producer-attributed Resource Timing initiator"
        );
        assert!(!queue.has_ready_completion());

        server
            .await
            .expect("main modulepreload server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production main modulepreload test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn failed_main_modulepreload_queues_joined_client_fanout_for_a_later_turn() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/failed-main-modulepreload.mjs",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/failure-page.html")).unwrap();
        let request_url = Url::parse(&format!("{base_url}/failed-main-modulepreload.mjs")).unwrap();
        let (key, request) = modulepreload_request(&document_url, &request_url);
        let link_client = NativeModulepreloadLinkClient::new(NativeNodeId::new(7101), key.clone());
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());

        assert_eq!(
            page_vm
                .vm_mut()
                .register_native_modulepreload_link_for_owner(request, link_client)
                .map_err(anyhow::Error::msg)?,
            Some(ModulePreloadJobRun::Scheduled)
        );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        super::child_document_completion::wait_for_page_resource_completion(
            &mut page_vm,
            &mut queue,
            &mut wake_rx,
            "failed main modulepreload fetch",
        )
        .await;
        assert!(
            !page_vm.vm_mut().has_ready_native_module_owner_actions(),
            "the joined link client must not be notified before the resource terminal is applied"
        );
        page_vm
            .vm_mut()
            .eval("performance.clearResourceTimings()")?;
        let (records, _, _) = split_network_output_items(page_vm.vm_mut().take_network_output());

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("failed modulepreload terminal should consume one typed turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        let entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&key)
            .expect("failed modulepreload should remain terminal in the module map");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(entry),
            ModuleMapEntryState::Failed
        );
        assert!(page_vm.vm().subresource_activity_epoch() > activity_epoch_before);
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "the module-map result must not republish its request"
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].document_url(), &document_url);
        assert_eq!(records[0].url(), &request_url);
        assert_eq!(
            records[0].request_initiator_type(),
            SubresourceRequestInitiatorType::Parser
        );
        assert_eq!(
            modulepreload_performance_summary(&mut page_vm, &request_url)?,
            "link",
            "failed fetches still produce a modulepreload Resource Timing entry"
        );
        assert!(page_vm.vm_mut().has_ready_native_module_owner_actions());
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent,
                    )
                )
                .await?,
            "joined-client notification should consume one selected main runtime turn"
        );
        assert!(
            !page_vm.vm_mut().has_ready_native_module_owner_actions(),
            "one terminal notification must not be dispatched twice"
        );

        server
            .await
            .expect("failed main modulepreload server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("main modulepreload failure/fanout test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_preserves_queued_main_modulepreload_realm_cache_without_document_effects() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/old-document-modulepreload.mjs",
            "HTTP/1.1 200 OK",
            "export const oldDocument = true;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/old-document.html")).unwrap();
        let request_url =
            Url::parse(&format!("{base_url}/old-document-modulepreload.mjs")).unwrap();
        let (old_key, request) = modulepreload_request(&document_url, &request_url);
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());
        page_vm
            .vm_mut()
            .register_native_modulepreload_for_owner(request)
            .map_err(anyhow::Error::msg)?;
        let entry_before_replacement = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&old_key)
            .expect("modulepreload should reserve one module-map entry");
        super::child_document_completion::wait_for_page_resource_completion(
            &mut page_vm,
            &mut queue,
            &mut wake_rx,
            "old Document modulepreload fetch",
        )
        .await;
        let owner = queue.next_ready_owner().expect("queued exact owner");
        let RendererPageResourceCompletionLocalOwner::MainModulepreloadFetch(target) =
            owner.local_owner()
        else {
            panic!("old terminal must retain a modulepreload target: {owner:?}");
        };

        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); \
             document.close(); 'replaced'",
        )?;
        let retained_target = page_vm
            .vm()
            .current_main_modulepreload_fetch_target(target.load_id())
            .expect("document.open must retain the ScriptState-owned module-map request");
        assert_ne!(
            retained_target, target,
            "the retained realm request must project through the replacement Document owner"
        );
        page_vm
            .vm_mut()
            .eval("performance.clearResourceTimings()")?;
        let (records, _, _) = split_network_output_items(page_vm.vm_mut().take_network_output());
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale modulepreload terminal should consume one discard turn");
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
            "historical modulepreload Network output must not become replacement activity"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_id(&old_key),
            Some(entry_before_replacement),
            "document.open must preserve the same ScriptState module-map entry"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(entry_before_replacement),
            ModuleMapEntryState::Fetched,
            "the old link's terminal must settle the retained realm cache"
        );
        assert_eq!(
            modulepreload_performance_summary(&mut page_vm, &request_url)?,
            "",
            "historical Network output must not leak into replacement Resource Timing"
        );
        assert!(!page_vm.vm_mut().has_ready_native_module_owner_actions());
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "the business terminal must not republish its physical response"
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].document_url(), &document_url);
        assert_eq!(records[0].url(), &request_url);
        assert_eq!(
            records[0].request_initiator_type(),
            SubresourceRequestInitiatorType::Parser
        );

        server
            .await
            .expect("old Document modulepreload server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Page stale main modulepreload test should run");
}

#[test]
fn real_page_vm_replacement_rejects_naturally_colliding_main_modulepreload_target() {
    run_page_vm_large_stack_async_test(
        "main-modulepreload-real-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![
                (
                    "/old-modulepreload.mjs",
                    "HTTP/1.1 200 OK",
                    "export const oldPreload = true;".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement.html",
                    "HTTP/1.1 200 OK",
                    "<!doctype html><body>replacement</body>".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement-modulepreload.mjs",
                    "HTTP/1.1 200 OK",
                    "export const replacementPreload = true;".to_owned(),
                    Duration::ZERO,
                ),
            ])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let initial_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, queue, wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, initial_url.clone());
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let mut queue = queue;
                    let mut wake_rx = wake_rx;
                    let old_request_url =
                        Url::parse(&format!("{base_url}/old-modulepreload.mjs")).unwrap();
                    let (old_key, old_request) =
                        modulepreload_request(&initial_url, &old_request_url);
                    page_vm
                        .vm_mut()
                        .register_native_modulepreload_for_owner(old_request)
                        .map_err(anyhow::Error::msg)?;
                    super::child_document_completion::wait_for_page_resource_completion(&mut page_vm,
&mut queue,
&mut wake_rx,
"old PageVm modulepreload fetch")
                    .await;
                    let (historical_records, _, _) = split_network_output_items(page_vm.vm_mut().take_network_output());
                    let (_, old_envelope) = queue
                        .pop_front()
                        .expect("old PageVm modulepreload terminal should remain queued");
                    let old_owner = old_envelope.owner();
                    let RendererPageResourceCompletionLocalOwner::MainModulepreloadFetch(
                        old_target,
                    ) = old_owner.local_owner()
                    else {
                        panic!("old terminal should retain a modulepreload target: {old_owner:?}");
                    };
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
                    let replacement_document_url =
                        Url::parse(&format!("{base_url}/replacement.html")).unwrap();
                    let replacement_request_url =
                        Url::parse(&format!("{base_url}/replacement-modulepreload.mjs")).unwrap();
                    let (replacement_key, replacement_request) = modulepreload_request(
                        &replacement_document_url,
                        &replacement_request_url,
                    );
                    page_vm
                        .vm_mut()
                        .register_native_modulepreload_for_owner(replacement_request)
                        .map_err(anyhow::Error::msg)?;
                    let activity_before_response = page_vm.vm().subresource_activity_epoch();
                    super::child_document_completion::wait_for_page_resource_completion(&mut page_vm,
&mut queue,
&mut wake_rx,
"replacement PageVm modulepreload fetch")
                    .await;
                    assert!(page_vm.vm().subresource_activity_epoch() > activity_before_response,
                        "the replacement physical response advances current activity");
                    let (_, replacement_envelope) = queue
                        .pop_front()
                        .expect("replacement modulepreload terminal should remain queued");
                    let replacement_owner = replacement_envelope.owner();
                    let RendererPageResourceCompletionLocalOwner::MainModulepreloadFetch(
                        replacement_target,
                    ) = replacement_owner.local_owner()
                    else {
                        panic!(
                            "replacement terminal should retain a modulepreload target: {replacement_owner:?}"
                        );
                    };
                    assert_eq!(
                        old_target, replacement_target,
                        "fresh PageVm counters should naturally reuse main Document and load ids"
                    );
                    assert_ne!(old_owner.root_document(), replacement_owner.root_document());

                    page_vm.vm_mut().eval("performance.clearResourceTimings()")?;
                    let (current_records, _, _) = split_network_output_items(page_vm.vm_mut().take_network_output());
                    queue.enqueue_local_for_test(old_envelope);
                    queue.enqueue_local_for_test(replacement_envelope);
                    let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
                    let stale = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("old PageVm modulepreload terminal should consume one stale turn");
                    assert_eq!(
                        stale.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(replacement_owner),
                        }
                    );
                    assert_eq!(
                        page_vm.vm().subresource_activity_epoch(),
                        activity_epoch_before
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .native_module_entry_id(&old_key),
                        None
                    );
                    let replacement_entry = page_vm
                        .vm()
                        .document_runtime
                        .native_module_entry_id(&replacement_key)
                        .expect("replacement module map should retain its in-flight entry");
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .native_module_entry_state(replacement_entry),
                        ModuleMapEntryState::Fetching
                    );
                    assert_eq!(
                        modulepreload_performance_summary(&mut page_vm, &old_request_url)?,
                        ""
                    );
                    assert!(page_vm.vm_mut().take_network_output().is_empty(), "a stale module result must not republish the original request");
                    assert_eq!(historical_records.len(), 1);
                    assert_eq!(historical_records[0].document_url(), &initial_url);
                    assert_eq!(historical_records[0].url(), &old_request_url);

                    let current = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("replacement modulepreload terminal should consume the next turn");
                    assert_eq!(
                        current.action.document_effect,
                        PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                    );
                    assert_eq!(page_vm.vm().subresource_activity_epoch(), activity_epoch_before,
                        "module-map application must not duplicate physical response activity");
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .native_module_entry_state(replacement_entry),
                        ModuleMapEntryState::Fetched
                    );
                    assert_eq!(
                        modulepreload_performance_summary(
                            &mut page_vm,
                            &replacement_request_url,
                        )?,
                        "link"
                    );
                    assert_eq!(
                        modulepreload_performance_summary(&mut page_vm, &old_request_url)?,
                        ""
                    );
                    assert!(page_vm.vm_mut().take_network_output().is_empty(), "applying the module map must not duplicate current network records");
                    assert_eq!(current_records.len(), 1);
                    assert_eq!(current_records[0].document_url(), &replacement_document_url);
                    assert_eq!(current_records[0].url(), &replacement_request_url);
                    assert!(!queue.has_ready_completion());
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("real PageVm modulepreload replacement fixture should run");

            server
                .await
                .expect("real PageVm modulepreload replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn main_modulepreload_source_consumes_one_terminal_per_turn_in_fifo_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let document_url = Url::parse("https://example.test/modulepreload-fifo.html").unwrap();
        let first_url = Url::parse("https://example.test/first-modulepreload.mjs").unwrap();
        let second_url = Url::parse("https://example.test/second-modulepreload.mjs").unwrap();
        let (first_key, first_target) =
            reserve_modulepreload_without_network(&mut page_vm, &document_url, &first_url);
        let (second_key, second_target) =
            reserve_modulepreload_without_network(&mut page_vm, &document_url, &second_url);
        let mut queue = RendererPageNetworkingSource::new_for_test();
        enqueue_modulepreload_completion(
            &mut queue,
            root_document,
            modulepreload_completion(first_target, first_url, Ok("export const first = true;")),
        );
        enqueue_modulepreload_completion(
            &mut queue,
            root_document,
            modulepreload_completion(second_target, second_url, Ok("export const second = true;")),
        );

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("first modulepreload terminal should consume one turn");
        assert_eq!(
            first.action.owner,
            RendererPageResourceCompletionOwner::main_modulepreload_fetch(
                root_document,
                first_target,
            )
        );

        assert!(queue.has_ready_completion());
        let first_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&first_key)
            .expect("first entry");
        let second_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&second_key)
            .expect("second entry");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(first_entry),
            ModuleMapEntryState::Fetched
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(second_entry),
            ModuleMapEntryState::Fetching,
            "one resource turn must not drain the next modulepreload terminal"
        );

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("second modulepreload terminal should consume its own turn");
        assert_eq!(
            second.action.owner,
            RendererPageResourceCompletionOwner::main_modulepreload_fetch(
                root_document,
                second_target,
            )
        );

        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(second_entry),
            ModuleMapEntryState::Fetched
        );
        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("main modulepreload FIFO terminal test should run");
}
