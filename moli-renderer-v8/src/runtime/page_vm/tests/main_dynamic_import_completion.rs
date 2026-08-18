use super::*;

use crate::{
    module_runtime::DynamicModuleImportOwner,
    page_resource_completion::{
        MainDynamicImportGraphFetchCompletion, MainDynamicImportGraphFetchTarget,
        MainModuleFetchNetworkAttribution, RendererPageResourceCompletionLocalOwner,
    },
};

fn dynamic_import_completion(
    target: MainDynamicImportGraphFetchTarget,
    document_url: Url,
    request_url: Url,
    source: std::result::Result<&str, &str>,
) -> MainDynamicImportGraphFetchCompletion {
    let result = source
        .map(|source| {
            ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text(source.to_owned()),
            )
        })
        .map_err(str::to_owned);
    MainDynamicImportGraphFetchCompletion::new(
        target,
        result,
        None,
        MainModuleFetchNetworkAttribution::new(document_url, request_url),
    )
}

async fn spawn_controlled_module_response_http_server(
    path: &'static str,
    body: &'static str,
) -> (
    String,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind controlled module response server");
    let addr = listener.local_addr().expect("controlled server address");
    let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
    let (release_response_tx, release_response_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept controlled module request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read controlled module request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("controlled module request path");
        assert_eq!(request_path, path);
        request_seen_tx
            .send(())
            .expect("test must still observe the controlled request");
        release_response_rx
            .await
            .expect("test must release the controlled module response");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write controlled module response");
    });
    (
        format!("http://{addr}"),
        request_seen_rx,
        release_response_tx,
        server,
    )
}

async fn start_main_dynamic_import(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    request_url: &Url,
) -> anyhow::Result<()> {
    page_vm.vm_mut().eval(&format!(
        r#"
globalThis.__typedDynamicImportEvents = [];
import({:?}).then(
  namespace => __typedDynamicImportEvents.push("fulfilled:" + namespace.value),
  error => __typedDynamicImportEvents.push("rejected:" + error.name)
);
"queued"
"#,
        request_url.as_str()
    ))?;

    for _ in 0..16 {
        if !page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::DynamicModuleJob,
                ),
                loader,
            )
            .await?
        {
            page_vm.vm_mut().perform_script_task_checkpoint(None)?;
            continue;
        }
        return Ok(());
    }
    anyhow::bail!("dynamic import did not schedule its graph fetch within sixteen exact turns")
}

async fn wait_for_networking_terminal_wake(
    queue: &mut impl crate::page_resource_completion::RendererPageResourceCompletionTestSource,
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
    label: &str,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let wake = wake_rx
                .recv()
                .await
                .unwrap_or_else(|| panic!("{label} owner wake route closed"));
            if wake.source_for_test()
                == crate::page_task_queue::RendererOwnerWakeSource::NetworkingTask
            {
                assert!(
                    queue.has_ready_completion(),
                    "Networking wake must follow durable terminal publication"
                );
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} did not publish a Networking wake before timeout"));
}

#[tokio::test(flavor = "current_thread")]
async fn dynamic_import_graph_body_leaves_user_reaction_for_selected_completion() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen, release_response, server) =
            spawn_controlled_module_response_http_server(
                "/typed-dynamic-import.mjs",
                r#"__typedDynamicImportEvents.push("module"); export const value = 41;"#,
            )
            .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let request_url =
            Url::parse(&format!("{base_url}/typed-dynamic-import.mjs")).expect("module URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        start_main_dynamic_import(&mut page_vm, &loader, &request_url).await?;
        request_seen
            .await
            .expect("dynamic-import fetch must reach the controlled server");
        while wake_rx.try_recv().is_ok() {}
        assert!(
            !queue.has_ready_completion(),
            "a request without a response must not publish a terminal"
        );
        release_response
            .send(())
            .expect("release typed dynamic-import response");
        wait_for_networking_terminal_wake(&mut queue, &mut wake_rx, "main dynamic import").await;

        let owner = queue
            .next_ready_owner()
            .expect("dynamic-import terminal must retain an exact owner");
        let RendererPageResourceCompletionLocalOwner::MainDynamicImportGraphFetch(target) =
            owner.local_owner()
        else {
            panic!("production import used the wrong typed terminal owner: {owner:?}");
        };
        assert_eq!(
            page_vm
                .vm()
                .current_main_dynamic_import_graph_fetch_target(target.load_id()),
            Some(target)
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("dynamic-import terminal should consume one typed turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__typedDynamicImportEvents.join('|')",
                )?,
            "module",
            "the graph-fetch body must leave the user import reaction for selected-task completion"
        );
        assert_eq!(
            outcome.action.body_activity,
            PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted,
            "module evaluation must report that the resource body entered Page code",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__typedDynamicImportEvents.join('|')",
                )?,
            "module",
            "observing the body without a checkpoint must not settle the user reaction"
        );
        assert!(!queue.has_ready_completion());
        server.await.expect("dynamic-import server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dynamic-import graph body-only checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn production_dynamic_import_uses_exact_typed_networking_source() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen, release_response, server) =
            spawn_controlled_module_response_http_server(
                "/selected-typed-dynamic-import.mjs",
                r#"__typedDynamicImportEvents.push("module"); export const value = 41;"#,
            )
            .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let request_url = Url::parse(&format!("{base_url}/selected-typed-dynamic-import.mjs"))
            .expect("module URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        start_main_dynamic_import(&mut page_vm, &loader, &request_url).await?;
        request_seen
            .await
            .expect("dynamic-import fetch must reach the controlled server");
        release_response
            .send(())
            .expect("release selected dynamic-import response");
        wait_for_networking_terminal_wake(&mut queue, &mut wake_rx, "selected main dynamic import")
            .await;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?,
            "the graph terminal must enter the production selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__typedDynamicImportEvents.join('|')",
                )?,
            "module|fulfilled:41",
            "selected completion must settle the user import reaction at this task boundary",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "resource completion must not drain unrelated runtime residence",
        );
        assert!(!queue.has_ready_completion());

        server.await.expect("dynamic-import server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production typed dynamic-import test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn mismatched_import_owner_cannot_consume_colliding_load_id() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen, release_response, server) =
            spawn_controlled_module_response_http_server(
                "/colliding-dynamic-import.mjs",
                "export const value = 51;",
            )
            .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let request_url =
            Url::parse(&format!("{base_url}/colliding-dynamic-import.mjs")).unwrap();
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());
        start_main_dynamic_import(&mut page_vm, &loader, &request_url).await?;
        request_seen
            .await
            .expect("colliding fetch must reach the controlled server");

        let current_owner = page_vm
            .vm()
            .document_runtime
            .with_native_module_owner(|owner| {
                owner.inflight_native_dynamic_module_import_fetch_owner(0)
            })
            .expect("scheduled import must retain its resolver owner");
        let current_target = MainDynamicImportGraphFetchTarget::new(current_owner, 0);
        let forged_target = MainDynamicImportGraphFetchTarget::new(
            DynamicModuleImportOwner::main_for_test_parts(91, 92, 93),
            current_target.load_id(),
        );
        let root_document = page_vm.document_lifecycle.identity().document;
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::main_dynamic_import_graph_fetch(
                root_document,
                dynamic_import_completion(
                    forged_target,
                    document_url,
                    request_url.clone(),
                    Ok("export const value = -1;"),
                ),
            ),
        );

        let forged = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("forged terminal should consume one discarded turn");
        assert!(matches!(
            forged.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
        ));
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .with_native_module_owner(|owner| {
                    owner.inflight_native_dynamic_module_import_fetch_owner(0)
                }),
            Some(current_owner),
            "mismatched import owner must not retire a current fetch with the same load id"
        );

        while wake_rx.try_recv().is_ok() {}
        assert!(!queue.has_ready_completion());
        release_response
            .send(())
            .expect("release real colliding dynamic-import response");
        wait_for_networking_terminal_wake(
            &mut queue,
            &mut wake_rx,
            "colliding main dynamic import",
        )
        .await;
        assert_eq!(
            queue.next_ready_owner(),
            Some(
                crate::page_resource_completion::RendererPageResourceCompletionOwner::main_dynamic_import_graph_fetch(
                    root_document,
                    current_target,
                )
            )
        );
        let current = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("real terminal should remain consumable after collision rejection");
        assert_eq!(
            current.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        server.await.expect("collision server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dynamic-import owner collision test should run");
}

#[test]
fn real_page_vm_replacement_rejects_naturally_colliding_dynamic_import_target() {
    run_page_vm_large_stack_async_test(
        "main-dynamic-import-real-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![
                (
                    "/old-dynamic-import.mjs",
                    "HTTP/1.1 200 OK",
                    "globalThis.__oldDynamicImportMustNotRun = true; export const value = 71;"
                        .to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement.html",
                    "HTTP/1.1 200 OK",
                    "<!doctype html><body>replacement</body>".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement-dynamic-import.mjs",
                    "HTTP/1.1 200 OK",
                    "__typedDynamicImportEvents.push('replacement-module'); export const value = 72;"
                        .to_owned(),
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
                    let old_request_url =
                        Url::parse(&format!("{base_url}/old-dynamic-import.mjs"))
                            .expect("old dynamic-import URL");
                    start_main_dynamic_import(&mut page_vm, &loader, &old_request_url).await?;
                    super::child_document_completion::wait_for_page_resource_completion(
                        &mut queue,
                        &mut wake_rx,
                        "old PageVm dynamic-import fetch",
                    )
                    .await;
                    let (_, old_envelope) = queue
                        .pop_front()
                        .expect("old PageVm dynamic-import terminal should remain queued");
                    let old_owner = old_envelope.owner();
                    let RendererPageResourceCompletionLocalOwner::MainDynamicImportGraphFetch(
                        old_target,
                    ) = old_owner.local_owner()
                    else {
                        panic!(
                            "old terminal should retain a dynamic-import target: {old_owner:?}"
                        );
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

                    let replacement_request_url =
                        Url::parse(&format!("{base_url}/replacement-dynamic-import.mjs"))
                            .expect("replacement dynamic-import URL");
                    start_main_dynamic_import(&mut page_vm, &loader, &replacement_request_url)
                        .await?;
                    super::child_document_completion::wait_for_page_resource_completion(
                        &mut queue,
                        &mut wake_rx,
                        "replacement PageVm dynamic-import fetch",
                    )
                    .await;
                    let (_, replacement_envelope) = queue
                        .pop_front()
                        .expect("replacement dynamic-import terminal should remain queued");
                    let replacement_owner = replacement_envelope.owner();
                    let RendererPageResourceCompletionLocalOwner::MainDynamicImportGraphFetch(
                        replacement_target,
                    ) = replacement_owner.local_owner()
                    else {
                        panic!(
                            "replacement terminal should retain a dynamic-import target: {replacement_owner:?}"
                        );
                    };
                    assert_eq!(
                        old_target, replacement_target,
                        "fresh PageVm counters should naturally reuse import-owner and load ids"
                    );
                    assert_ne!(old_owner.root_document(), replacement_owner.root_document());

                    let _ = page_vm.vm_mut().take_network_output();
                    queue.enqueue_local_for_test(old_envelope);
                    queue.enqueue_local_for_test(replacement_envelope);
                    let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
                    let stale = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("old PageVm dynamic-import terminal should consume one stale turn");
                    assert_eq!(
                        stale.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(
                                RendererPageResourceCompletionOwner::main_dynamic_import_graph_fetch(
                                    replacement_root,
                                    replacement_target,
                                ),
                            ),
                        }
                    );
                    assert_eq!(
                        stale.action.output_effect,
                        PageResourceCompletionOutputEffect::CaptureRequired
                    );
                    assert_eq!(
                        page_vm.vm().subresource_activity_epoch(),
                        activity_epoch_before,
                        "historical old-PageVm Network output must not become replacement activity"
                    );
                    let (network_records, _, _) =
                        split_network_output_items(page_vm.vm_mut().take_network_output());
                    assert_eq!(network_records.len(), 1);
                    assert_eq!(network_records[0].document_url(), &initial_url);
                    assert_eq!(network_records[0].url(), &old_request_url);
                    assert_eq!(
                        page_vm
                            .vm()
                            .current_main_dynamic_import_graph_fetch_target(
                                replacement_target.load_id(),
                            ),
                        Some(replacement_target),
                        "the old root namespace must not settle the replacement import owner"
                    );

                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(
                                PageSelectedTaskTestSelector::ResourceCompletion,
                                &loader,
                            )
                            .await?,
                        "replacement dynamic-import terminal should enter the production selected dispatcher",
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "__typedDynamicImportEvents.join('|')",
                            )?,
                        "replacement-module|fulfilled:72"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("String(globalThis.__oldDynamicImportMustNotRun)")?,
                        "undefined"
                    );
                    assert!(!queue.has_ready_completion());
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("real PageVm dynamic-import replacement fixture should run");

            server
                .await
                .expect("real PageVm dynamic-import replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_preserves_inflight_dynamic_import_in_the_same_script_state() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen, release_response, server) =
            spawn_controlled_module_response_http_server(
                "/stale-dynamic-import.mjs",
                "globalThis.__oldDynamicImportModuleRan = true; export const value = 61;",
            )
            .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let request_url = Url::parse(&format!("{base_url}/stale-dynamic-import.mjs")).unwrap();
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let original_loader = page_vm
            .vm()
            .current_main_document_resource_loader()
            .expect("initial Document resource authority");
        start_main_dynamic_import(&mut page_vm, &loader, &request_url).await?;
        request_seen
            .await
            .expect("stale fetch must reach the controlled server");
        let target = page_vm
            .vm()
            .current_main_dynamic_import_graph_fetch_target(0)
            .expect("scheduled import must expose its exact target");

        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><p id=replacement>replacement</p>'); document.close(); 'replaced'",
        )?;
        let replacement_loader = page_vm
            .vm()
            .current_main_document_resource_loader()
            .expect("replacement Document resource authority");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .with_native_module_owner(|owner| {
                    owner.inflight_native_dynamic_module_import_fetch_owner(target.load_id())
                }),
            Some(target.import_owner()),
            "document.open() must retain the resolver owned by the live ScriptState"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_dynamic_import_graph_fetch_target(target.load_id()),
            Some(target),
            "replacing Moli's exact Document owner must not make the same-realm import stale"
        );
        assert_eq!(
            original_loader.state(),
            crate::network::context::DocumentResourceLoaderState::Detached
        );
        assert_eq!(
            original_loader.load_diagnostics().active_ordinary_load_count,
            0,
            "the retired authority must no longer own the transferred import load"
        );
        assert_eq!(
            replacement_loader
                .load_diagnostics()
                .active_ordinary_load_count,
            1,
            "the replacement authority must retain the LocalWindow's in-flight import load"
        );
        let _ = page_vm.vm_mut().take_network_output();
        while wake_rx.try_recv().is_ok() {}
        assert!(!queue.has_ready_completion());
        release_response
            .send(())
            .expect("release retained dynamic-import response");
        wait_for_networking_terminal_wake(
            &mut queue,
            &mut wake_rx,
            "document.open retained dynamic import",
        )
        .await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?,
            "the retained import terminal must run through the production Networking source"
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__typedDynamicImportEvents.join('|')"
            )?,
            "fulfilled:61",
            "the original import Promise must settle in its unchanged V8 realm"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__oldDynamicImportModuleRan)")?,
            "true"
        );
        assert_eq!(
            page_vm.vm_mut().eval("document.getElementById('replacement').textContent")?,
            "replacement"
        );
        assert!(!queue.has_ready_completion());
        server
            .await
            .expect("retained dynamic-import server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("document.open dynamic-import retention test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn module_reaction_dynamic_import_body_leaves_user_reaction_for_selected_completion() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen, release_response, server) =
            spawn_controlled_module_response_http_server(
                "/tla-module-reaction.mjs",
                r#"
__typedDynamicImportEvents.push("module-start");
await globalThis.__typedDynamicImportGate;
__typedDynamicImportEvents.push("module-end");
export const value = 43;
"#,
            )
            .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let request_url =
            Url::parse(&format!("{base_url}/tla-module-reaction.mjs")).expect("module URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__typedDynamicImportGate = new Promise(resolve => {
  globalThis.__resolveTypedDynamicImportGate = resolve;
});
"initialized"
"#,
        )?;
        start_main_dynamic_import(&mut page_vm, &loader, &request_url).await?;
        request_seen
            .await
            .expect("TLA dynamic-import fetch must reach the controlled server");
        release_response
            .send(())
            .expect("release TLA dynamic-import response");
        wait_for_networking_terminal_wake(&mut queue, &mut wake_rx, "TLA dynamic import").await;
        let terminal = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("TLA graph terminal should consume one typed turn");
        assert_eq!(
            terminal.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedDynamicImportEvents.join('|')")?,
            "module-start",
            "the dynamic-import Promise must remain pending at top-level await"
        );

        page_vm
            .vm_mut()
            .eval("__resolveTypedDynamicImportGate(); 'resolved'")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedDynamicImportEvents.join('|')")?,
            "module-start|module-end",
            "the evaluation reaction must publish one typed ModuleReaction before settling the user import Promise"
        );

        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        let task = page_vm
            .take_module_reaction_body_task_for_test()
            .expect("TLA fulfillment should retain one exact ModuleReaction task");
        let reaction = page_vm.apply_selected_page_module_reaction_turn(task)?;
        assert_eq!(
            reaction.action.target_effect(),
            crate::page_task_queue::PageModuleReactionTargetEffect::AppliedToCurrentOwner(
                crate::page_task_queue::PageModuleReactionCurrentEffect::DynamicImportPromiseSettled,
            )
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__typedDynamicImportEvents.join('|')",
                )?,
            "module-start|module-end",
            "the ModuleReaction body must leave the user import reaction for selected-task completion"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "the ModuleReaction body must not advance unrelated runtime residence"
        );
        let completion = reaction.action.into_page_task_completion();
        assert!(matches!(
            &completion,
            crate::runtime::PageTaskCompletion::CheckpointOnly
        ));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__typedDynamicImportEvents.join('|')",
                )?,
            "module-start|module-end|fulfilled:43",
            "selected completion must drain the user import reaction exactly once"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "checkpoint-only completion must not advance unrelated runtime residence"
        );

        server
            .await
            .expect("TLA module-reaction server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ModuleReaction body/checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn module_reaction_dynamic_import_uses_production_selected_completion() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen, release_response, server) =
            spawn_controlled_module_response_http_server(
                "/tla-selected-module-reaction.mjs",
                r#"
__typedDynamicImportEvents.push("module-start");
await globalThis.__typedDynamicImportGate;
__typedDynamicImportEvents.push("module-end");
export const value = 44;
"#,
            )
            .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let request_url = Url::parse(&format!("{base_url}/tla-selected-module-reaction.mjs"))
            .expect("module URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__typedDynamicImportGate = new Promise(resolve => {
  globalThis.__resolveTypedDynamicImportGate = resolve;
});
"initialized"
"#,
        )?;
        start_main_dynamic_import(&mut page_vm, &loader, &request_url).await?;
        request_seen
            .await
            .expect("TLA dynamic-import fetch must reach the controlled server");
        release_response
            .send(())
            .expect("release TLA dynamic-import response");
        wait_for_networking_terminal_wake(&mut queue, &mut wake_rx, "TLA dynamic import").await;
        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("TLA graph terminal should consume one typed turn");

        page_vm
            .vm_mut()
            .eval("__resolveTypedDynamicImportGate(); 'resolved'")?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ModuleReaction, &loader)
                .await?,
            "TLA fulfillment must publish one exact ModuleReaction task"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__typedDynamicImportEvents.join('|')",
                )?,
            "module-start|module-end|fulfilled:44",
            "the production selected dispatcher must settle the import Promise and submit its task-end checkpoint"
        );
        assert!(
            page_vm.take_module_reaction_body_task_for_test().is_none(),
            "the exact ModuleReaction task must settle once"
        );

        server
            .await
            .expect("selected ModuleReaction server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production ModuleReaction dispatcher test should run");
}
