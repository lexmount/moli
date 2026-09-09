use super::*;

use crate::page_resource_completion::{
    MainModuleFetchNetworkAttribution, MainRuntimeModuleGraphFetchCompletion,
    MainRuntimeModuleGraphFetchTarget, RendererPageResourceCompletionLocalOwner,
};

fn runtime_module_completion(
    target: MainRuntimeModuleGraphFetchTarget,
    document_url: Url,
    request_url: Url,
    source: std::result::Result<&str, &str>,
    network_result: Option<crate::types::SharedNavigationResponseResult>,
) -> MainRuntimeModuleGraphFetchCompletion {
    let result = source
        .map(|source| {
            ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text(source.to_owned()),
            )
        })
        .map_err(str::to_owned);
    MainRuntimeModuleGraphFetchCompletion::new(
        target,
        result,
        network_result,
        MainModuleFetchNetworkAttribution::new(document_url, request_url),
    )
}

fn enqueue_runtime_module_completion(
    queue: &mut RendererPageNetworkingSource,
    root_document: crate::runtime::RendererDocumentToken,
    completion: MainRuntimeModuleGraphFetchCompletion,
) {
    queue.enqueue_local_for_test(
        RendererPageResourceCompletion::main_runtime_module_graph_fetch(root_document, completion),
    );
}

async fn start_inline_runtime_module_graph(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    script_handle: &str,
    source: &str,
) -> anyhow::Result<()> {
    page_vm
        .vm_mut()
        .document_runtime
        .note_dom_content_loaded_dispatched();
    page_vm.vm_mut().eval(&format!(
        r#"(() => {{
  const script = document.createElement("script");
  script.type = "module";
  script.id = {script_handle:?};
  script.textContent = {source:?};
  script.onload = () => (globalThis.__runtimeModuleEvents ??= []).push("load");
  script.onerror = () => (globalThis.__runtimeModuleEvents ??= []).push("error");
  document.body.appendChild(script);
  return "installed";
}})()"#,
    ))?;

    for _ in 0..16 {
        if !run_one_runtime_module_followup_turn(page_vm, loader).await? {
            assert!(
                page_vm.has_pending_runtime_owned_module_graph(),
                "runtime module owner became idle without installing a graph wait"
            );
            return Ok(());
        }
    }
    panic!("runtime module graph did not block on its fetch within sixteen exact owner turns");
}

async fn run_one_runtime_module_followup_turn(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<bool> {
    if page_vm
        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ModuleReaction, loader)
        .await?
    {
        return Ok(true);
    }
    if page_vm
        .run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::MainDocumentRuntime(
                PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation,
            ),
            loader,
        )
        .await?
    {
        return Ok(true);
    }
    if page_vm
        .run_page_main_document_runtime_body_for_test(loader)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(false)
}

async fn run_runtime_owned_module_body_without_selected_completion(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<crate::page_task_queue::PageMainDocumentRuntimeTurnOutcome> {
    for _ in 0..16 {
        if page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ModuleReaction,
                loader,
            )
            .await?
        {
            continue;
        }
        let Some(outcome) = page_vm
            .run_page_main_document_runtime_body_for_test(loader)
            .await?
        else {
            anyhow::bail!("runtime module continuation stalled before its exact body task");
        };
        if outcome.action.kind()
            == PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation
        {
            return Ok(outcome);
        }
    }
    anyhow::bail!("runtime module continuation did not reach its exact body within sixteen turns")
}

async fn run_until_selected_runtime_owned_module_continuation(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<()> {
    for _ in 0..16 {
        if page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ModuleReaction,
                loader,
            )
            .await?
        {
            continue;
        }
        if page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation,
                ),
                loader,
            )
            .await?
        {
            return Ok(());
        }
        if page_vm
            .run_page_main_document_runtime_body_for_test(loader)
            .await?
            .is_some()
        {
            continue;
        }
        anyhow::bail!("runtime module continuation stalled before selected dispatch");
    }
    anyhow::bail!("runtime module continuation was not selected within sixteen turns")
}

async fn prepare_runtime_module_style_boundary(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    queue: &mut crate::page_task_queue::RendererPageResourceCompletionTestSource,
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
    dependency_url: &Url,
    script_handle: &str,
    color: &str,
    applied_global: &str,
) -> anyhow::Result<()> {
    let selector = format!(".runtime-module-active {{ color: {color}; }}");
    assert_eq!(
        page_vm.vm_mut().eval(&format!(
            r#"
(() => {{
  const style = document.createElement("style");
  style.textContent = {selector:?};
  document.head.appendChild(style);
  globalThis.__runtimeModuleStyleTarget = document.createElement("div");
  document.body.appendChild(globalThis.__runtimeModuleStyleTarget);
  return getComputedStyle(globalThis.__runtimeModuleStyleTarget).color;
}})()
"#
        ))?,
        "rgb(0, 0, 0)",
        "the witness must cache a selector-dependent style before module execution"
    );
    assert!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::StyleElementEvent,
                loader,
            )
            .await?,
        "the connected <style> terminal must leave the shared Networking FIFO before the module fetch can become its head"
    );
    let source = format!(
        "import {:?}; globalThis.__runtimeModuleStyleTarget.className = 'runtime-module-active'; globalThis[{applied_global:?}] = true;",
        dependency_url.as_str()
    );
    start_inline_runtime_module_graph(page_vm, loader, script_handle, &source).await?;
    super::child_document_completion::wait_for_page_resource_completion(
        queue,
        wake_rx,
        "runtime module style-boundary dependency",
    )
    .await;
    let terminal = page_vm
        .apply_one_page_resource_terminal_owner_admission_for_test(queue)?
        .expect("dependency fetch must publish one typed terminal");
    assert_eq!(
        terminal.action.document_effect,
        PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_owned_module_body_leaves_style_turn_exit_to_selected_completion() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/body-boundary-dependency.mjs",
            "HTTP/1.1 200 OK",
            "export const dependency = true;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/body-boundary.html")).unwrap();
        let dependency_url =
            Url::parse(&format!("{base_url}/body-boundary-dependency.mjs")).unwrap();
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        prepare_runtime_module_style_boundary(
            &mut page_vm,
            &loader,
            &mut queue,
            &mut wake_rx,
            &dependency_url,
            "runtime-module-body-boundary",
            "rgb(1, 2, 3)",
            "__runtimeModuleBodyApplied",
        )
        .await?;

        let outcome =
            run_runtime_owned_module_body_without_selected_completion(&mut page_vm, &loader)
                .await?;
        assert_eq!(
            outcome.action.target_effect(),
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__runtimeModuleBodyApplied)",
                )?,
            "true"
        );
        assert!(
            page_vm
                .vm()
                .pending_style_invalidation_work_item_count_for_current_document_for_test()
                > 0,
            "the module body may record style work, but must not submit the selected task's turn-exit drain"
        );

        server
            .await
            .expect("runtime module body-boundary server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime module body-boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_runtime_owned_module_continuation_submits_its_style_turn_exit() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/selected-boundary-dependency.mjs",
            "HTTP/1.1 200 OK",
            "export const dependency = true;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/selected-boundary.html")).unwrap();
        let dependency_url =
            Url::parse(&format!("{base_url}/selected-boundary-dependency.mjs")).unwrap();
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        prepare_runtime_module_style_boundary(
            &mut page_vm,
            &loader,
            &mut queue,
            &mut wake_rx,
            &dependency_url,
            "runtime-module-selected-boundary",
            "rgb(4, 5, 6)",
            "__selectedRuntimeModuleApplied",
        )
        .await?;

        run_until_selected_runtime_owned_module_continuation(&mut page_vm, &loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedRuntimeModuleApplied)",
                )?,
            "true"
        );
        assert_eq!(
            page_vm
                .vm()
                .pending_style_invalidation_work_item_count_for_current_document_for_test(),
            0,
            "the production selected dispatcher must submit the module task's turn-exit style drain"
        );

        server
            .await
            .expect("runtime module selected-boundary server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime module selected-boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn spent_runtime_owned_module_ticket_does_not_manufacture_a_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://runtime-module.test/spent-ticket").expect("document URL");
        let (mut page_vm, _queue, _wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        assert!(
            page_vm.vm_mut().enqueue_runtime_owned_module_continuation(),
            "test setup must publish one exact continuation ticket"
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__spentRuntimeModuleBoundary = [];
Promise.resolve().then(() => __spentRuntimeModuleBoundary.push("microtask"));
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainDocumentRuntime(
                        PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation,
                    ),
                    &loader,
                )
                .await?,
            "the exact spent ticket must still consume one selected turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__spentRuntimeModuleBoundary.join('|')",
                )?,
            "",
            "a current ticket with no matching ready continuation must not manufacture a checkpoint"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "discarding a spent module ticket must not advance unrelated runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("spent runtime module ticket test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_runtime_owned_module_ticket_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://runtime-module.test/stale-ticket").expect("document URL");
        let (mut page_vm, _queue, _wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner");
        assert!(
            page_vm.vm_mut().enqueue_runtime_owned_module_continuation(),
            "test setup must publish one exact old-Document ticket"
        );
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        assert_ne!(
            page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("replacement main Document owner"),
            retired_owner
        );
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleRuntimeModuleBoundary = [];
Promise.resolve().then(() => __staleRuntimeModuleBoundary.push("microtask"));
"queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation,
                ),
                    &loader,
                )
                .await?,
            "the old exact ticket must consume one stale-discard turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__staleRuntimeModuleBoundary.join('|')",
                )?,
            "",
            "an old-Document ticket must not enter replacement V8 for a checkpoint"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "stale retirement must not advance replacement runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale runtime module ticket test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn production_runtime_module_dependencies_reenter_the_exact_typed_source_one_turn_each() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/runtime-dependency.mjs",
                "HTTP/1.1 200 OK",
                r#"import "./runtime-leaf.mjs";
(globalThis.__typedRuntimeModuleEvents ??= []).push("dependency");
export const dependency = true;"#
                    .to_owned(),
                Duration::ZERO,
            ),
            (
                "/runtime-leaf.mjs",
                "HTTP/1.1 200 OK",
                r#"(globalThis.__typedRuntimeModuleEvents ??= []).push("leaf");
export const leaf = true;"#
                    .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let dependency_url =
            Url::parse(&format!("{base_url}/runtime-dependency.mjs")).expect("dependency URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let root_source = format!(
            "import {:?}; (globalThis.__typedRuntimeModuleEvents ??= []).push('root');",
            dependency_url.as_str()
        );
        start_inline_runtime_module_graph(
            &mut page_vm,
            &loader,
            "typed-runtime-module",
            &root_source,
        )
        .await?;

        super::child_document_completion::wait_for_page_resource_completion(
            &mut queue,
            &mut wake_rx,
            "main runtime module dependency fetch",
        )
        .await;
        let first_owner = queue
            .next_ready_owner()
            .expect("first runtime terminal must retain an exact owner");
        let RendererPageResourceCompletionLocalOwner::MainRuntimeModuleGraphFetch(first_target) =
            first_owner.local_owner()
        else {
            panic!("first runtime dependency must use the typed runtime source: {first_owner:?}");
        };
        assert_eq!(
            page_vm
                .vm()
                .current_main_runtime_module_graph_fetch_target(first_target.load_id()),
            Some(first_target)
        );

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("first runtime dependency must consume one typed terminal turn");
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__typedRuntimeModuleEvents)")?,
            "undefined",
            "a resource terminal must not execute the module body inline"
        );

        super::child_document_completion::wait_for_page_resource_completion(
            &mut queue,
            &mut wake_rx,
            "main runtime module leaf fetch",
        )
        .await;
        let second_owner = queue
            .next_ready_owner()
            .expect("leaf terminal must retain an exact owner");
        let RendererPageResourceCompletionLocalOwner::MainRuntimeModuleGraphFetch(second_target) =
            second_owner.local_owner()
        else {
            panic!("leaf must reenter the typed runtime source: {second_owner:?}");
        };
        assert_eq!(second_owner.root_document(), first_owner.root_document());
        assert_ne!(
            second_target.load_id(),
            first_target.load_id(),
            "each graph fetch must retain its own load continuation"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_runtime_module_graph_fetch_target(second_target.load_id()),
            Some(second_target)
        );

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("leaf must consume a second typed terminal turn");
        assert_eq!(
            second.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__typedRuntimeModuleEvents)")?,
            "undefined",
            "the terminal turn must stop before V8 module evaluation"
        );

        for _ in 0..16 {
            if page_vm
                .vm_mut()
                .eval("String(globalThis.__typedRuntimeModuleEvents?.join('|'))")?
                == "leaf|dependency|root"
            {
                break;
            }
            assert!(
                run_one_runtime_module_followup_turn(&mut page_vm, &loader).await?,
                "runtime module follow-up stalled before evaluation"
            );
        }
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__typedRuntimeModuleEvents.join('|')")?,
            "leaf|dependency|root"
        );
        assert!(!queue.has_ready_completion());

        server
            .await
            .expect("typed runtime module dependency server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production typed runtime module test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_blob_module_dependency_resolves_through_the_local_url_owner() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://blob-module.test/page.html").expect("document URL");
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        let leaf_url = page_vm.vm_mut().eval(
            r#"URL.createObjectURL(new Blob([
  "(globalThis.__runtimeModuleEvents ??= []).push('leaf'); export const leaf = true;"
], { type: "text/javascript" }))"#,
        )?;
        let root_source =
            format!("import {leaf_url:?}; (globalThis.__runtimeModuleEvents ??= []).push('root');");
        start_inline_runtime_module_graph(
            &mut page_vm,
            &loader,
            "blob-runtime-module",
            &root_source,
        )
        .await?;

        super::child_document_completion::wait_for_page_resource_completion(
            &mut queue,
            &mut wake_rx,
            "blob runtime module dependency fetch",
        )
        .await;
        let leaf = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("blob dependency must consume one typed terminal turn");
        assert_eq!(
            leaf.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );

        for _ in 0..16 {
            if page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeModuleEvents?.join('|'))")?
                == "leaf|root"
            {
                break;
            }
            assert!(
                run_one_runtime_module_followup_turn(&mut page_vm, &loader).await?,
                "blob module graph stalled before module evaluation"
            );
        }
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__runtimeModuleEvents.join('|')")?,
            "leaf|root"
        );
        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("blob module graph fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_module_failure_body_cleans_up_callbacks_but_leaves_lifecycle_prime_to_task_end() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/runtime-failure.mjs",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/failure-page.html")).unwrap();
        let failure_url = Url::parse(&format!("{base_url}/runtime-failure.mjs")).unwrap();
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm
            .vm_mut()
            .eval("globalThis.__runtimeModuleEvents = []")?;
        let source = format!("import {:?};", failure_url.as_str());
        start_inline_runtime_module_graph(&mut page_vm, &loader, "runtime-module-failure", &source)
            .await?;
        page_vm.vm_mut().eval(
            r#"
document.getElementById("runtime-module-failure").onerror = () => {
  __runtimeModuleEvents.push("error");
  Promise.resolve().then(() => __runtimeModuleEvents.push("error-microtask"));
};
"installed"
"#,
        )?;
        super::child_document_completion::wait_for_page_resource_completion(
            &mut queue,
            &mut wake_rx,
            "main runtime module failed fetch",
        )
        .await;

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("runtime module failure must consume one typed terminal turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.post_checkpoint_effect,
            PageResourceCompletionPostCheckpointEffect::PrimeMainDocumentLifecycle {
                owner: page_vm
                    .vm()
                    .current_main_document_task_owner()
                    .expect("runtime module Document owner"),
            },
            "the body must return the exact Document whose final load-delay lease was released"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__runtimeModuleEvents.join('|')",
                )?,
            "error|error-microtask",
            "the graph-terminal body must clean up its error listener before returning lifecycle-unblock authority"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(
                    page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("runtime module Document owner"),
                ),
            Some(false),
            "the same terminal action must consume the exact Window-load lease"
        );
        assert!(
            !page_vm
                .page_task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_window_load_task),
            "the body must return lifecycle-unblock authority for selected task completion"
        );

        server
            .await
            .expect("runtime module failure server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime module failure body-boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_runtime_module_failure_checkpoints_and_consumes_exact_load_gate() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/selected-runtime-failure.mjs",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/selected-failure-page.html")).unwrap();
        let failure_url = Url::parse(&format!("{base_url}/selected-runtime-failure.mjs")).unwrap();
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm
            .vm_mut()
            .eval("globalThis.__selectedRuntimeModuleEvents = []")?;
        let source = format!("import {:?};", failure_url.as_str());
        start_inline_runtime_module_graph(
            &mut page_vm,
            &loader,
            "selected-runtime-module-failure",
            &source,
        )
        .await?;
        page_vm.vm_mut().eval(
            r#"
document.getElementById("selected-runtime-module-failure").onerror = () => {
  __selectedRuntimeModuleEvents.push("error");
  Promise.resolve().then(() => __selectedRuntimeModuleEvents.push("error-microtask"));
};
"installed"
"#,
        )?;
        super::child_document_completion::wait_for_page_resource_completion(
            &mut queue,
            &mut wake_rx,
            "selected main runtime module failed fetch",
        )
        .await;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?,
            "the graph failure must enter the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__selectedRuntimeModuleEvents.join('|')",
                )?,
            "error|error-microtask",
            "the script-error reaction must complete before lifecycle is made runnable"
        );
        let document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("runtime module Document owner");
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(document_owner),
            Some(false),
            "the selected terminal must consume the exact load-delay lease"
        );

        server
            .await
            .expect("selected runtime module failure server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected runtime module failure task-end test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_discards_queued_runtime_module_terminal_without_current_activity() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/old-runtime-dependency.mjs",
            "HTTP/1.1 200 OK",
            "export const oldValue = true;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/old-document.html")).unwrap();
        let request_url = Url::parse(&format!("{base_url}/old-runtime-dependency.mjs")).unwrap();
        let (mut page_vm, mut queue, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url.clone());
        let source = format!(
            "import {:?}; globalThis.__oldRuntimeModuleMustNotRun = true;",
            request_url.as_str()
        );
        start_inline_runtime_module_graph(&mut page_vm, &loader, "old-runtime-module", &source)
            .await?;
        super::child_document_completion::wait_for_page_resource_completion(
            &mut queue,
            &mut wake_rx,
            "old Document runtime module fetch",
        )
        .await;
        let owner = queue
            .next_ready_owner()
            .expect("queued exact runtime owner");
        let RendererPageResourceCompletionLocalOwner::MainRuntimeModuleGraphFetch(target) =
            owner.local_owner()
        else {
            panic!("queued terminal must be runtime-module owned: {owner:?}");
        };

        page_vm.vm_mut().eval("document.open(); 'replaced'")?;
        assert_eq!(
            page_vm
                .vm()
                .current_main_runtime_module_graph_fetch_target(target.load_id()),
            None,
            "document.open must retire the old dynamic-script fetch target"
        );
        let _ = page_vm.vm_mut().take_network_output();
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale runtime terminal must consume one discard turn");
        assert!(matches!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
        ));
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "historical runtime module Network output must not become replacement activity"
        );
        let (network_records, _, _) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert_eq!(network_records.len(), 1);
        assert_eq!(network_records[0].document_url(), &document_url);
        assert_eq!(network_records[0].url(), &request_url);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__oldRuntimeModuleMustNotRun)")?,
            "undefined"
        );

        server
            .await
            .expect("old runtime module server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Page stale runtime module test should run");
}

#[test]
fn real_page_vm_replacement_rejects_naturally_colliding_runtime_module_target() {
    run_page_vm_large_stack_async_test(
        "main-runtime-module-real-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![
                (
                    "/old-runtime-dependency.mjs",
                    "HTTP/1.1 200 OK",
                    "export const oldValue = true;".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement.html",
                    "HTTP/1.1 200 OK",
                    "<!doctype html><body>replacement</body>".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement-runtime-dependency.mjs",
                    "HTTP/1.1 200 OK",
                    "export const replacementValue = true;".to_owned(),
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
                    let old_dependency_url = Url::parse(&format!(
                        "{base_url}/old-runtime-dependency.mjs"
                    ))
                    .expect("old dependency URL");
                    let old_source = format!(
                        "import {:?}; globalThis.__oldRuntimeModuleMustNotRun = true;",
                        old_dependency_url.as_str()
                    );
                    start_inline_runtime_module_graph(
                        &mut page_vm,
                        &loader,
                        "old-runtime-module",
                        &old_source,
                    )
                    .await?;
                    super::child_document_completion::wait_for_page_resource_completion(
                        &mut queue,
                        &mut wake_rx,
                        "old PageVm runtime module fetch",
                    )
                    .await;
                    let (_, old_envelope) = queue
                        .pop_front()
                        .expect("old PageVm runtime module terminal should remain queued");
                    let old_owner = old_envelope.owner();
                    let RendererPageResourceCompletionLocalOwner::MainRuntimeModuleGraphFetch(
                        old_target,
                    ) = old_owner.local_owner()
                    else {
                        panic!("old terminal should retain a runtime module target: {old_owner:?}");
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

                    let replacement_dependency_url = Url::parse(&format!(
                        "{base_url}/replacement-runtime-dependency.mjs"
                    ))
                    .expect("replacement dependency URL");
                    let replacement_source = format!(
                        "import {:?}; globalThis.__replacementRuntimeModuleRan = true;",
                        replacement_dependency_url.as_str()
                    );
                    start_inline_runtime_module_graph(
                        &mut page_vm,
                        &loader,
                        "replacement-runtime-module",
                        &replacement_source,
                    )
                    .await?;
                    super::child_document_completion::wait_for_page_resource_completion(
                        &mut queue,
                        &mut wake_rx,
                        "replacement PageVm runtime module fetch",
                    )
                    .await;
                    let (_, replacement_envelope) = queue
                        .pop_front()
                        .expect("replacement runtime module terminal should remain queued");
                    let replacement_owner = replacement_envelope.owner();
                    let RendererPageResourceCompletionLocalOwner::MainRuntimeModuleGraphFetch(
                        replacement_target,
                    ) = replacement_owner.local_owner()
                    else {
                        panic!(
                            "replacement terminal should retain a runtime module target: {replacement_owner:?}"
                        );
                    };
                    assert_eq!(
                        old_target, replacement_target,
                        "fresh PageVm counters should naturally reuse Document, dynamic-owner, and load ids"
                    );

                    let _ = page_vm.vm_mut().take_network_output();
                    queue.enqueue_local_for_test(old_envelope);
                    queue.enqueue_local_for_test(replacement_envelope);
                    let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
                    let stale = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("old PageVm runtime terminal should consume one stale turn");
                    assert_eq!(
                        stale.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(
                                RendererPageResourceCompletionOwner::main_runtime_module_graph_fetch(
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
                    assert_eq!(network_records[0].url(), &old_dependency_url);
                    assert_eq!(
                        page_vm
                            .vm()
                            .current_main_runtime_module_graph_fetch_target(
                                replacement_target.load_id(),
                            ),
                        Some(replacement_target),
                        "the old root namespace must not settle the replacement dynamic owner"
                    );

                    let current = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("replacement runtime terminal should retain application authority");
                    assert_eq!(
                        current.action.document_effect,
                        PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                    );
                    for _ in 0..16 {
                        if page_vm
                            .vm_mut()
                            .eval("String(globalThis.__replacementRuntimeModuleRan)")?
                            == "true"
                        {
                            break;
                        }
                        assert!(
                            run_one_runtime_module_followup_turn(&mut page_vm, &loader).await?,
                            "replacement runtime module stalled before evaluation"
                        );
                    }
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("String(globalThis.__replacementRuntimeModuleRan)")?,
                        "true"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("String(globalThis.__oldRuntimeModuleMustNotRun)")?,
                        "undefined"
                    );
                    assert!(!queue.has_ready_completion());
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("real PageVm runtime-module replacement fixture should run");

            server
                .await
                .expect("real PageVm runtime-module replacement server should finish");
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_module_source_consumes_one_terminal_per_turn_in_fifo_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main Document owner");
        let first_target = MainRuntimeModuleGraphFetchTarget::new(
            document_owner,
            DynamicScriptOwnerId::from_u64(61),
            701,
        );
        let second_target = MainRuntimeModuleGraphFetchTarget::new(
            document_owner,
            DynamicScriptOwnerId::from_u64(62),
            702,
        );
        let document_url = Url::parse("https://example.test/runtime-module-fifo.html").unwrap();
        let mut queue = RendererPageNetworkingSource::new_for_test();
        for (target, suffix) in [(first_target, "first"), (second_target, "second")] {
            enqueue_runtime_module_completion(
                &mut queue,
                root_document,
                runtime_module_completion(
                    target,
                    document_url.clone(),
                    Url::parse(&format!("https://example.test/runtime-module-{suffix}.mjs"))
                        .unwrap(),
                    Err("stale terminal"),
                    None,
                ),
            );
        }

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("first runtime terminal should consume one turn");
        assert_eq!(
            first.action.owner,
            RendererPageResourceCompletionOwner::main_runtime_module_graph_fetch(
                root_document,
                first_target,
            )
        );

        assert!(queue.has_ready_completion());

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("second runtime terminal should consume its own turn");
        assert_eq!(
            second.action.owner,
            RendererPageResourceCompletionOwner::main_runtime_module_graph_fetch(
                root_document,
                second_target,
            )
        );

        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime module FIFO terminal test should run");
}
