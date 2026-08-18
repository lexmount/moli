use super::*;

use crate::{
    page_task_queue::{
        PageServiceWorkerInternalTargetEffect, RendererServiceWorkerInternalTaskKind,
        ServiceWorkerInternalCallbackEffect,
    },
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
    service_worker_runtime::{ServiceWorkerRegistrationSnapshot, ServiceWorkerVersionId},
    types::{
        ServiceWorkerClientFocusRequestCompletion, ServiceWorkerLifecycleClientEvent,
        ServiceWorkerLifecycleNotification, ServiceWorkerReadyCompletion,
        ServiceWorkerUnregisterCompletion,
    },
};

struct PreparedServiceWorkerLifecycleTarget {
    registration: ServiceWorkerRegistrationSnapshot,
    document_owner: crate::native_bridge::WindowDocumentOwner,
    storage_key: String,
}

async fn prepare_service_worker_lifecycle_target(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<PreparedServiceWorkerLifecycleTarget> {
    page_vm.vm_mut().eval(
        r#"
globalThis.__serviceWorkerInternalRegistration = null;
navigator.serviceWorker.ready.then(registration => {
  globalThis.__serviceWorkerInternalRegistration = registration;
});
"ready-observer-installed"
"#,
    )?;
    let (request_id, document_owner) = page_vm
        .vm()
        .service_worker_ready_request_for_test(crate::native_bridge::OwnerDispatchScope::Top)
        .expect("top-level ServiceWorker ready request");
    let scope_url = Url::parse("https://service-worker-internal.test/").unwrap();
    let registration = ServiceWorkerRegistrationSnapshot::active_for_binding_test(
        scope_url.clone(),
        Url::parse("https://service-worker-internal.test/worker.js").unwrap(),
    );
    let root_document = page_vm.document_lifecycle.identity().document;
    page_vm
        .service_worker_task_sender_for_root_for_test(root_document)
        .send_service_worker_ready(ServiceWorkerReadyCompletion {
            request_id,
            document_owner,
            registration: registration.clone(),
        })
        .expect("ready completion should enter the stable internal source");
    anyhow::ensure!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ServiceWorkerInternal,
                loader
            )
            .await?,
        "ready completion should return through the production selected dispatcher"
    );
    anyhow::ensure!(
        page_vm
            .vm_mut()
            .eval("globalThis.__serviceWorkerInternalRegistration?.scope")?
            == scope_url.as_str(),
        "selected ready completion should resolve the registration binding"
    );

    let (watcher_owner, storage_key) = page_vm
        .vm()
        .service_worker_lifecycle_watcher_for_test(
            crate::native_bridge::OwnerDispatchScope::Top,
            &scope_url,
        )
        .expect("ready registration should install one exact lifecycle watcher");
    anyhow::ensure!(
        watcher_owner == document_owner,
        "ready request and lifecycle watcher must retain the same exact document owner"
    );
    Ok(PreparedServiceWorkerLifecycleTarget {
        registration,
        document_owner,
        storage_key,
    })
}

fn send_updatefound(page_vm: &PageVm, target: &PreparedServiceWorkerLifecycleTarget) {
    let root_document = page_vm.document_lifecycle.identity().document;
    page_vm
        .service_worker_task_sender_for_root_for_test(root_document)
        .send_service_worker_lifecycle(ServiceWorkerLifecycleNotification {
            document_owner: target.document_owner,
            storage_key: target.storage_key.clone(),
            registration: target.registration.clone(),
            events: vec![ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("lifecycle notification should enter the stable internal source");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_ready_body_leaves_reaction_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-internal.test/ready-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerReadyBoundary = [];
navigator.serviceWorker.ready.then(registration => {
  __serviceWorkerReadyBoundary.push(`reaction:${registration.scope}`);
});
"observer-ready"
"#,
        )?;
        let (request_id, document_owner) = page_vm
            .vm()
            .service_worker_ready_request_for_test(crate::native_bridge::OwnerDispatchScope::Top)
            .expect("top-level ServiceWorker ready request");
        let registration_scope =
            Url::parse("https://service-worker-internal.test/").expect("registration scope");
        let registration = ServiceWorkerRegistrationSnapshot::active_for_binding_test(
            registration_scope.clone(),
            Url::parse("https://service-worker-internal.test/worker.js").expect("worker URL"),
        );
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_ready(ServiceWorkerReadyCompletion {
                request_id,
                document_owner,
                registration,
            })
            .expect("ready completion should enter the stable internal source");

        let task = page_vm
            .take_service_worker_internal_body_task_for_test()
            .expect("one ServiceWorker ready task should be ready");
        let outcome = page_vm.apply_selected_page_service_worker_internal_turn(task)?;
        assert_eq!(
            outcome.action.task_kind,
            RendererServiceWorkerInternalTaskKind::Ready
        );
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerInternalTargetEffect::PromiseSettledAtCurrentRoot
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "globalThis.__serviceWorkerReadyBoundary.join('|')",
                )?,
            "",
            "the ServiceWorker ready body must leave Promise reactions for selected-task completion"
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("globalThis.__serviceWorkerReadyBoundary.join('|')")?,
            format!("reaction:{}", registration_scope.as_str()),
            "the selected internal task must perform the Promise-settlement checkpoint"
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker ready body/completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_lifecycle_callback_uses_selected_completion_and_runtime_follow_up() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-internal.test/lifecycle-callback").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let target = prepare_service_worker_lifecycle_target(&mut page_vm, &loader).await?;

        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerInternalLifecycleEvents = [];
__serviceWorkerInternalRegistration.addEventListener("updatefound", () => {
  __serviceWorkerInternalLifecycleEvents.push("callback");
  Promise.resolve().then(() => {
    __serviceWorkerInternalLifecycleEvents.push("microtask");
    const script = document.createElement("script");
    script.textContent =
      "__serviceWorkerInternalLifecycleEvents.push('runtime-script')";
    document.body.appendChild(script);
  });
});
"listener-ready"
"#,
        )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        send_updatefound(&page_vm, &target);

        let task = page_vm
            .take_service_worker_internal_body_task_for_test()
            .expect("one lifecycle callback task should be ready");
        let outcome = page_vm.apply_selected_page_service_worker_internal_turn(task)?;
        assert_eq!(
            outcome.action.task_kind,
            RendererServiceWorkerInternalTaskKind::Lifecycle
        );
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerInternalTargetEffect::EventDispatchPassCompletedAtCurrentRoot {
                callback_effect: ServiceWorkerInternalCallbackEffect::CallbackBodyDispatched,
            }
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerInternalLifecycleEvents.join('|')")?,
            "callback",
            "the event body must leave its Promise reaction for selected completion"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the lifecycle body must not consume unrelated runtime residence"
        );

        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CallbackCompletion));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerInternalLifecycleEvents.join('|')")?,
            "callback|microtask|runtime-script"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test()
                == 1,
            "callback-created runtime work must not consume an unrelated pending source"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker lifecycle callback completion witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_lifecycle_without_callback_is_checkpoint_only() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-internal.test/lifecycle-no-callback").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let target = prepare_service_worker_lifecycle_target(&mut page_vm, &loader).await?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        send_updatefound(&page_vm, &target);

        let task = page_vm
            .take_service_worker_internal_body_task_for_test()
            .expect("one callback-free lifecycle task should be ready");
        let outcome = page_vm.apply_selected_page_service_worker_internal_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerInternalTargetEffect::EventDispatchPassCompletedAtCurrentRoot {
                callback_effect: ServiceWorkerInternalCallbackEffect::NoCallbackBodyDispatched,
            }
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a callback-free lifecycle task must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker callback-free lifecycle completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_internal_action_is_checkpoint_only() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-internal.test/internal-action").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let target = page_vm
            .vm()
            .service_worker_internal_window_client_target_for_test(
                crate::native_bridge::OwnerDispatchScope::Top,
            )
            .expect("top-level Window client target");
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm
            .service_worker_task_sender_for_root_for_test(root_document)
            .send_service_worker_client_focus_request(ServiceWorkerClientFocusRequestCompletion {
                target,
                request_id: 41,
                source_version_id: ServiceWorkerVersionId::from_u64_for_test(43),
                source_run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
            })
            .expect("focus request should enter the stable internal source");

        let task = page_vm
            .take_service_worker_internal_body_task_for_test()
            .expect("one internal action should be ready");
        let outcome = page_vm.apply_selected_page_service_worker_internal_turn(task)?;
        assert_eq!(
            outcome.action.task_kind,
            RendererServiceWorkerInternalTaskKind::ClientFocusRequest
        );
        assert_eq!(
            outcome.action.target_effect,
            PageServiceWorkerInternalTargetEffect::InternalActionAppliedAtCurrentRoot
        );
        let completion = outcome.action.into_page_task_completion();
        assert!(matches!(completion, PageTaskCompletion::CheckpointOnly));
        page_vm
            .finish_selected_page_task_completion(completion, &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a state-only internal action must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker internal-action completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_lifecycle_completion_reconciles_listener_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-internal.test/document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let target = prepare_service_worker_lifecycle_target(&mut page_vm, &loader).await?;
        let root_document = page_vm.document_lifecycle.identity().document;
        let retired_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial exact Document owner");

        page_vm.vm_mut().eval(
            r#"
globalThis.__serviceWorkerInternalDocumentOpenEvents = [];
__serviceWorkerInternalRegistration.addEventListener("updatefound", () => {
  __serviceWorkerInternalDocumentOpenEvents.push("callback");
  document.open();
  document.write("<!doctype html><body>replacement</body>");
  document.close();
  Promise.resolve().then(() => {
    __serviceWorkerInternalDocumentOpenEvents.push("microtask");
  });
});
"listener-ready"
"#,
        )?;
        send_updatefound(&page_vm, &target);
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ServiceWorkerInternal,
                    &loader
                )
                .await?,
            "lifecycle callback must return through the production selected dispatcher"
        );

        let current_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.open replacement owner");
        assert_ne!(current_document_owner, retired_document_owner);
        assert_eq!(
            page_vm.document_lifecycle.identity().document,
            root_document,
            "document.open keeps the Page root identity while rotating the ScriptVm Document owner"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__serviceWorkerInternalDocumentOpenEvents.join('|')")?,
            "callback|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker lifecycle document.open completion test should run");
}

#[test]
fn service_worker_internal_task_rejects_a_real_root_document_replacement() {
    run_page_vm_large_stack_async_test(
        "service-worker-internal-real-root-replacement",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let retired_root = page_vm.document_lifecycle.identity().document;
                    page_vm
                        .service_worker_task_sender_for_root_for_test(retired_root)
                        .send_service_worker_unregister(ServiceWorkerUnregisterCompletion {
                            request_id: u64::MAX - 11,
                            document_owner: crate::native_bridge::WindowDocumentOwner::for_test(
                                u64::MAX - 7,
                            ),
                            result: false,
                        })
                        .expect("retired-root internal task should remain resident");

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
                    page_vm
                        .vm_mut()
                        .eval_without_microtask_checkpoint_for_test(
                            r#"
globalThis.__staleServiceWorkerInternalReplacementBoundary = [];
Promise.resolve().then(() => {
  __staleServiceWorkerInternalReplacementBoundary.push("microtask");
});
"queued"
"#,
                        )?;
                    page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ServiceWorkerInternal,
                        )
                        .expect("retired-root internal task should remain a bounded stale turn");
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "__staleServiceWorkerInternalReplacementBoundary.join('|')",
                            )?,
                        "",
                        "retired internal work must not checkpoint the replacement realm"
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .document_runtime
                            .runtime_script_work()
                            .dynamic_scripts.pending_source_load_count_for_test(),
                        1,
                        "retired internal work must not checkpoint or advance replacement runtime residence"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("ServiceWorker internal root replacement proof should run");

            server.await.expect("replacement fixture server");
        },
    );
}
