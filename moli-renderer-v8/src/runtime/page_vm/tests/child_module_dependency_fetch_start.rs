use super::*;

use anyhow::Context;

use crate::page_task_queue::{
    PageChildModuleDependencyFetchStartTargetEffect, RendererOwnerWakeSource,
    RendererPageChildModuleDependencyFetchStartOwner,
};

/// Produce a dependency-start task through the real child module graph path,
/// but stop before the typed start executor consumes it.
async fn queue_real_child_module_dependency_start(
    page_vm: &mut PageVm,
    resource_source: &mut crate::page_task_queue::RendererPageResourceCompletionTestSource,
    owner_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::page_task_queue::RendererOwnerWake,
    >,
    base_url: &str,
    frame_id: &str,
) -> anyhow::Result<ChildDocumentModuleFetchTarget> {
    page_vm.vm_mut().eval(&format!(
        r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.id = {frame_id:?};
  frame.srcdoc = `<script type="module" src="{base_url}/dependency-root.js"><\/script>`;
  document.body.appendChild(frame);
  return "queued";
}})()
"#,
    ))?;
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(frame_id)
        .expect("dependency-start fixture should retain its child handle");

    let startup_sources =
        drive_child_frame_task_sources_until_resource_completion_ready(page_vm, 8).await;
    anyhow::ensure!(
        startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
        "child module root fetch should start through its existing source: {startup_sources:?}"
    );
    super::child_document_completion::wait_for_page_resource_completion(
        page_vm,
        resource_source,
        owner_wake_rx,
        "child module root completion",
    )
    .await;
    page_vm
        .apply_one_page_resource_terminal_owner_admission_for_test(resource_source)?
        .context("registered child module root should consume one stable typed turn")?;
    anyhow::ensure!(
        page_vm
            .run_child_module_script_terminal_body_for_test()
            .is_some(),
        "the stable module-terminal source should produce the typed dependency-start task"
    );
    let _ = page_vm.vm_mut().take_network_output();
    page_vm
        .vm()
        .current_child_document_module_fetch_target(child_handle)
        .context("real dependency producer should retain an exact child realm")
}

async fn await_dependency_response_server(server: JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("child module dependency request should reach the test server")
        .expect("child module dependency response server should finish");
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_dependency_fetch_start_body_does_not_checkpoint_or_consume_terminal() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/dependency-root.js",
                "HTTP/1.1 200 OK",
                "import './dependency.js';".to_owned(),
                Duration::ZERO,
            ),
            (
                "/dependency.js",
                "HTTP/1.1 200 OK",
                "export const value = 1;".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_real_child_module_dependency_start(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            &base_url,
            "dependency-start-body-child",
        )
        .await?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__childDependencyStartBodyCheckpoint = 0;
Promise.resolve().then(() => __childDependencyStartBodyCheckpoint += 1);
"reaction queued"
"#,
            )?;

        let outcome = page_vm
            .run_child_module_dependency_fetch_start_body_for_test()?
            .expect("one exact child dependency-start body should be ready");
        assert!(matches!(
            outcome.action.target_effect,
            PageChildModuleDependencyFetchStartTargetEffect::AppliedToCurrentOwner {
                outcome
            } if outcome.fetch_was_scheduled()
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__childDependencyStartBodyCheckpoint)",
                )?,
            "0",
            "dependency-start body must not discharge the selected task checkpoint",
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "starting a dependency request must not synchronously consume its later terminal",
        );
        await_dependency_response_server(server).await;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child dependency-start body witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_child_module_dependency_fetch_start_submits_task_end_checkpoint() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/dependency-root.js",
                "HTTP/1.1 200 OK",
                "import './dependency.js';".to_owned(),
                Duration::ZERO,
            ),
            (
                "/dependency.js",
                "HTTP/1.1 200 OK",
                "export const value = 1;".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        queue_real_child_module_dependency_start(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            &base_url,
            "selected-dependency-start-child",
        )
        .await?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__selectedChildDependencyStartCheckpoint = 0;
Promise.resolve().then(() => __selectedChildDependencyStartCheckpoint += 1);
"reaction queued"
"#,
            )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildModuleDependencyFetchStart
                )
                .await?,
            "one exact dependency-start task must enter the production selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__selectedChildDependencyStartCheckpoint)",
                )?,
            "1",
            "the production dispatcher must submit the state-only dependency-start checkpoint",
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "task completion must not synchronously consume the dependency network terminal",
        );
        await_dependency_response_server(server).await;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected child dependency-start completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_child_module_dependency_fetch_start_does_not_checkpoint_current_document() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/dependency-root.js",
            "HTTP/1.1 200 OK",
            "import './dependency-never-started.js';".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        let retired_target = queue_real_child_module_dependency_start(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            &base_url,
            "stale-dependency-start-child",
        )
        .await?;
        await_dependency_response_server(server).await;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildModuleDependencyFetchStart,
            )
            .expect("the old child realm must retain one opaque dependency-start claim");

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(retired_target.child_handle());
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "stale-dependency-start-child",
        )?;
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__staleChildDependencyStartCheckpoint = 0;
Promise.resolve().then(() => __staleChildDependencyStartCheckpoint += 1);
"replacement reaction queued"
"#,
            )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(globalThis.__staleChildDependencyStartCheckpoint)",
                )?,
            "0",
            "a stale dependency-start claim must not checkpoint the replacement realm",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child dependency-start completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn real_dependency_start_captures_realm_and_is_discarded_after_realm_replacement() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/dependency-root.js",
            "HTTP/1.1 200 OK",
            "import './dependency-never-started.js';".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        let retired_target = queue_real_child_module_dependency_start(
            &mut page_vm,
            &mut resource_source,
            &mut owner_wake_rx,
            &base_url,
            "dependency-start-realm",
        )
        .await?;
        server
            .await
            .expect("dependency root response server should finish");
        let start_wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .filter(|wake| {
                wake.source_for_test() == RendererOwnerWakeSource::ChildModuleDependencyFetchStart
            })
            .count();
        assert_eq!(
            start_wakes, 1,
            "the real producer must publish one empty-to-nonempty source wake"
        );

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(retired_target.child_handle());
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "dependency-start-realm")?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(retired_target.child_handle())
            .expect("replacement realm should expose its exact target");
        assert_eq!(retired_target.task_owner(), current_target.task_owner());
        assert_ne!(retired_target.realm_id(), current_target.realm_id());

        let outcome = page_vm
            .run_child_module_dependency_fetch_start_body_for_test()?
            .expect("stale dependency start should consume one discard turn");
        let root_document = page_vm.document_lifecycle.identity().document;
        assert_eq!(
            outcome.action.owner,
            RendererPageChildModuleDependencyFetchStartOwner::new(root_document, retired_target,)
        );
        assert_eq!(
            outcome.action.target_effect,
            PageChildModuleDependencyFetchStartTargetEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageChildModuleDependencyFetchStartOwner::new(
                    root_document,
                    current_target,
                )),
            },
            "the old realm's task must never be rebound to the replacement realm"
        );
        assert!(
            page_vm.vm_mut().take_network_output().is_empty(),
            "discarding a start task must not synthesize a Network terminal"
        );
        assert!(
            page_vm
                .run_child_module_dependency_fetch_start_body_for_test()?
                .is_none(),
            "the stale task must be consumed exactly once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dependency-start realm replacement must retain exact ownership");
}

#[test]
fn page_vm_replacement_rejects_naturally_colliding_dependency_start_owner() {
    run_page_vm_large_stack_async_test(
        "child-module-dependency-start-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![
                (
                    "/dependency-root.js",
                    "HTTP/1.1 200 OK",
                    "import './dependency.js';".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/dependency-root.js",
                    "HTTP/1.1 200 OK",
                    "import './dependency.js';".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/replacement.html",
                    "HTTP/1.1 200 OK",
                    "<!doctype html><body></body>".to_owned(),
                    Duration::ZERO,
                ),
                (
                    "/dependency.js",
                    "HTTP/1.1 200 OK",
                    "export const value = 1;".to_owned(),
                    Duration::ZERO,
                ),
            ])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).expect("page URL");
            let (mut page_vm, mut resource_source, mut owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let retired_target = queue_real_child_module_dependency_start(
                        &mut page_vm,
                        &mut resource_source,
                        &mut owner_wake_rx,
                        &base_url,
                        "dependency-start-collision",
                    )
                    .await?;
                    let retired_root = page_vm.document_lifecycle.identity().document;
                    let initial_start_wakes =
                        std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
                            .filter(|wake| {
                                wake.source_for_test()
                                    == RendererOwnerWakeSource::ChildModuleDependencyFetchStart
                            })
                            .count();
                    assert_eq!(
                        initial_start_wakes, 1,
                        "the first PageVm producer must publish one source readiness edge"
                    );

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
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
                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(retired_root, current_root);

                    let current_target = queue_real_child_module_dependency_start(
                        &mut page_vm,
                        &mut resource_source,
                        &mut owner_wake_rx,
                        &base_url,
                        "dependency-start-collision",
                    )
                    .await?;
                    assert_eq!(
                        retired_target, current_target,
                        "fresh PageVm-local child, Document, and realm counters should naturally collide"
                    );
                    let replacement_start_wakes =
                        std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
                        .filter(|wake| {
                            wake.source_for_test()
                                == RendererOwnerWakeSource::ChildModuleDependencyFetchStart
                        })
                        .count();
                    assert_eq!(
                        replacement_start_wakes, 0,
                        "the replacement producer must append to the still-ready stable source without duplicating its readiness edge"
                    );

                    let stale = page_vm
                        .run_child_module_dependency_fetch_start_body_for_test()?
                        .expect("retired PageVm task should consume one stale turn");
                    assert_eq!(
                        stale.action.owner,
                        RendererPageChildModuleDependencyFetchStartOwner::new(
                            retired_root,
                            retired_target,
                        )
                    );
                    assert_eq!(
                        stale.action.target_effect,
                        PageChildModuleDependencyFetchStartTargetEffect::DiscardedStaleOwner {
                            current_owner: Some(
                                RendererPageChildModuleDependencyFetchStartOwner::new(
                                    current_root,
                                    current_target,
                                ),
                            ),
                        },
                        "the root Document namespace must reject otherwise identical local IDs"
                    );

                    let current = page_vm
                        .run_child_module_dependency_fetch_start_body_for_test()?
                        .expect("replacement PageVm task should survive the stale-head discard");
                    assert_eq!(
                        current.action.owner,
                        RendererPageChildModuleDependencyFetchStartOwner::new(
                            current_root,
                            current_target,
                        )
                    );
                    assert!(matches!(
                        current.action.target_effect,
                        PageChildModuleDependencyFetchStartTargetEffect::AppliedToCurrentOwner {
                            outcome
                        } if outcome.fetch_was_scheduled()
                    ));
                    assert!(
                        page_vm
                            .run_child_module_dependency_fetch_start_body_for_test()?
                            .is_none(),
                        "each accepted dependency start must consume exactly one source head"
                    );
                    server
                        .await
                        .expect("dependency replacement server should finish");
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("PageVm replacement dependency starts should use exact root ownership");
        },
    );
}
