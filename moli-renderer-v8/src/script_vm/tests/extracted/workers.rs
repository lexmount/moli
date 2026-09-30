use super::*;

#[test]
fn isolated_realm_destruction_retires_dedicated_worker_without_retiring_local_window() {
    let mut vm = new_storage_test_vm("https://isolated-worker-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("worker-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let owner = host
                .current_runtime_window_execution_context_binding(scope)
                .expect("isolated Worker should capture its relevant realm");
            let outside_settings_load = host
                .register_dedicated_worker_outside_settings_load(owner.dispatch_scope())
                .expect("isolated Worker should capture its Document script-load authority");
            let creator_storage_key = host
                .active_storage_context(scope, None)
                .storage_key()
                .clone();
            let top_level_site = creator_storage_key.top_level_site().to_owned();
            host.register_loading_worker(
                scope,
                v8::Object::new(scope),
                top_level_site,
                creator_storage_key,
                String::new(),
                moli_fetch::RequestCredentialsMode::SameOrigin,
                None,
                outside_settings_load,
                owner,
            );
            Ok(())
        },
    )
    .expect("isolated Worker should register");
    assert_eq!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .len(),
        1
    );

    vm.destroy_isolated_world_context(isolated_context_id);

    assert!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .is_empty(),
        "destroying the Worker relevant realm must terminate the Worker"
    );
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn popup_replacement_retires_local_window_owned_dedicated_worker() {
    let mut vm = new_storage_test_vm("https://popup-owner-worker.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundWorkerPopup = open(
              "about:blank",
              "dedicated-worker-owner-popup"
            );
            __ownerBoundWorkerPopup.location.href = "about:blank?committed";
            String(globalThis.__ownerBoundWorkerPopup !== null)
            "#,
        )
        .expect("popup Worker owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup Worker owner id");
    let initial_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("initial popup LocalWindow owner");

    vm.with_popup_context_scope_and_checkpoint_for_test(popup_id, |scope, host_ptr| {
        let host = unsafe { &mut *host_ptr };
        let owner = host
            .current_runtime_window_execution_context_binding(scope)
            .expect("popup Worker should capture its LocalWindow");
        let outside_settings_load = host
            .register_dedicated_worker_outside_settings_load(owner.dispatch_scope())
            .expect("popup Worker should capture its Document script-load authority");
        let creator_storage_key = host
            .active_storage_context(scope, None)
            .storage_key()
            .clone();
        let top_level_site = creator_storage_key.top_level_site().to_owned();
        host.register_loading_worker(
            scope,
            v8::Object::new(scope),
            top_level_site,
            creator_storage_key,
            String::new(),
            moli_fetch::RequestCredentialsMode::SameOrigin,
            None,
            outside_settings_load,
            owner,
        );
        Ok(())
    })
    .expect("popup Worker should register");
    assert_eq!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .into_iter()
            .map(|(_, owner, _)| owner)
            .collect::<Vec<_>>(),
        vec![
            crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id: initial_local_window_id,
            }
        ]
    );

    vm.eval(
        r#"
        open("about:blank", "dedicated-worker-owner-popup");
        "replacement-committed"
        "#,
    )
    .expect("named popup replacement should commit");

    assert!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .is_empty(),
        "popup replacement must actively terminate old-LocalWindow Workers"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn service_worker_window_requests_bind_and_retire_exact_document_owners() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://service-worker-owner.test/page.html",
        &loader,
    );
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .is_empty(),
        "lazy Navigator bootstrap must not create a service worker ready request"
    );
    vm.eval("void navigator.serviceWorker.ready")
        .expect("top-level service worker ready promise should materialize");
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .into_iter()
            .any(|(_, owner)| owner.document_owner() == Some(main_owner)),
        "the top-level ready promise must bind the initial main document owner"
    );

    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          (document.body || document.documentElement || document).appendChild(frame);
          globalThis.__serviceWorkerOwnerFrame = frame;
        })();
        "frame-ready"
        "#,
    )
    .expect("service worker owner frame should schedule");
    assert_initial_about_blank_child_completed_through_page_for_test(
        &mut vm,
        &loader,
        "service worker container setup",
    )
    .await;
    vm.eval(
        "void globalThis.__serviceWorkerOwnerFrame.contentWindow.navigator.serviceWorker.ready",
    )
    .expect("child service worker ready promise should materialize");

    let child_owner = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("child browsing context");
        host.current_child_document_task_owner(child_handle)
            .expect("child document owner")
    };
    let ready_owners = vm
        ._context_host
        .borrow()
        .pending_service_worker_ready_owners_for_test();
    assert!(
        ready_owners
            .iter()
            .any(|(_, owner)| owner.document_owner() == Some(main_owner)),
        "materializing child ready must not overwrite the main ready resolver"
    );
    assert!(
        ready_owners
            .iter()
            .any(|(_, owner)| owner.document_owner() == Some(child_owner)),
        "child ready must bind the child document owner"
    );
    let (child_ready_request_id, child_ready_owner) = ready_owners
        .iter()
        .copied()
        .find(|(_, owner)| owner.document_owner() == Some(child_owner))
        .expect("child ready request");
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerReadyScope = "pending";
          child.navigator.serviceWorker.ready.then(registration => {
            child.__serviceWorkerOwnerReadyScope = registration.scope;
          });
        })()
        "#,
    )
    .expect("child ready observer should install");
    let ready_scope = url::Url::parse("https://service-worker-owner.test/").unwrap();
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_ready(crate::types::ServiceWorkerReadyCompletion {
            request_id: child_ready_request_id,
            document_owner: child_ready_owner.window_document_owner(),
            registration:
                crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot::active_for_binding_test(
                ready_scope.clone(),
                url::Url::parse("https://service-worker-owner.test/ready-worker.js").unwrap(),
            ),
        })
        .expect("owner-bound child ready completion should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Ready(child_ready_request_id),
        "owner-bound child ready completion",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerReadyScope",
        )
        .expect("child ready result should evaluate"),
        ready_scope.as_str()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .into_iter()
            .any(|(_, owner)| owner.document_owner() == Some(main_owner)),
        "settling child ready must leave the main ready request pending"
    );

    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerRegisterResult = "pending";
          child.navigator.serviceWorker.register(
            "https://service-worker-owner.test/worker.js"
          ).then(
            () => { child.__serviceWorkerOwnerRegisterResult = "resolved"; },
            error => {
              child.__serviceWorkerOwnerRegisterResult =
                error.name + ":" + error.message;
            }
          );
        })();
        "register-pending"
        "#,
    )
    .expect("child service worker register should schedule");
    let (request_id, register_owner) = vm
        ._context_host
        .borrow()
        .pending_service_worker_register_owners_for_test()
        .into_iter()
        .next()
        .expect("pending child service worker register");
    assert_eq!(register_owner.document_owner(), Some(child_owner));
    assert!(register_owner.dispatch_scope().child_window().is_some());

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_register(crate::types::ServiceWorkerRegisterCompletion {
            request_id,
            document_owner: register_owner.window_document_owner(),
            result: Err(
                crate::service_worker_runtime::ServiceWorkerRegistrationError::type_error(
                    "forced owner completion",
                ),
            ),
        })
        .expect("owner-bound child register failure should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Register(request_id),
        "owner-bound child register failure",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__serviceWorkerOwnerFrame.contentWindow
              .__serviceWorkerOwnerRegisterResult
            "#,
        )
        .expect("child register result should evaluate"),
        "TypeError:forced owner completion"
    );
    assert_eq!(
        vm.eval("typeof globalThis.__serviceWorkerOwnerRegisterResult")
            .expect("main register result should evaluate"),
        "undefined",
        "child completion must not settle through the top-level container realm"
    );

    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerRegistrationScope = "pending";
          child.navigator.serviceWorker.register(
            "https://service-worker-owner.test/bound-worker.js"
          ).then(registration => {
            child.__serviceWorkerOwnerRegistration = registration;
            child.__serviceWorkerOwnerRegistrationScope = registration.scope;
          });
        })();
        "binding-register-pending"
        "#,
    )
    .expect("child registration binding should schedule");
    let (binding_request_id, binding_owner) = vm
        ._context_host
        .borrow()
        .pending_service_worker_register_owners_for_test()
        .into_iter()
        .next()
        .expect("pending child registration binding");
    assert_eq!(binding_owner.document_owner(), Some(child_owner));
    let registration_scope = url::Url::parse("https://service-worker-owner.test/").unwrap();
    let registration_snapshot =
        crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot::active_for_binding_test(
            registration_scope.clone(),
            url::Url::parse("https://service-worker-owner.test/bound-worker.js").unwrap(),
        );
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_register(crate::types::ServiceWorkerRegisterCompletion {
            request_id: binding_request_id,
            document_owner: binding_owner.window_document_owner(),
            result: Ok(registration_snapshot.clone()),
        })
        .expect("owner-bound child registration should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Register(binding_request_id),
        "owner-bound child registration",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerRegistrationScope",
        )
        .expect("child registration scope should evaluate"),
        registration_scope.as_str()
    );
    let (_, _, lifecycle_storage_key) = vm
        ._context_host
        .borrow()
        .service_worker_registration_watchers_for_test()
        .into_iter()
        .find(|(owner, _, _)| owner.document_owner() == Some(child_owner))
        .expect("registration lifecycle watcher must retain the child document owner");
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerLifecycle = "pending";
          child.__serviceWorkerOwnerRegistration.addEventListener(
            "updatefound",
            () => { child.__serviceWorkerOwnerLifecycle = "child:updatefound"; }
          );
        })()
        "#,
    )
    .expect("child lifecycle listener should install");
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: binding_owner.window_document_owner(),
            storage_key: "wrong-partition".to_owned(),
            registration: registration_snapshot.clone(),
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("wrong-partition lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "wrong-partition lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerLifecycle",
        )
        .expect("child lifecycle result should evaluate"),
        "pending",
        "lifecycle notification from another storage partition must not fan out"
    );
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: binding_owner.window_document_owner(),
            storage_key: lifecycle_storage_key,
            registration: registration_snapshot,
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("owner-bound lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "owner-bound lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerLifecycle",
        )
        .expect("child lifecycle result should evaluate"),
        "child:updatefound",
        "lifecycle notification must dispatch through the child owner scope"
    );
    assert_eq!(
        vm.eval("typeof globalThis.__serviceWorkerOwnerLifecycle")
            .expect("main lifecycle result should evaluate"),
        "undefined"
    );

    vm.eval(
        r#"
        globalThis.__serviceWorkerOwnerFrame.contentWindow.navigator.serviceWorker
          .register("https://service-worker-owner.test/stale-worker.js");
        globalThis.__serviceWorkerOwnerFrame.srcdoc = "<p>replacement child</p>";
        "stale-register-pending"
        "#,
    )
    .expect("old child register and replacement should schedule");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .len(),
        1
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child replacement must commit before stale wrapper reuse is tested",
    )
    .await;
    let replacement_child_owner = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("replacement child browsing context");
        host.current_child_document_task_owner(child_handle)
            .expect("replacement child document owner")
    };
    assert_ne!(replacement_child_owner, child_owner);
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .is_empty(),
        "child navigation must retire pending register work from the old document"
    );
    assert!(
        vm._context_host
            .borrow()
            .service_worker_registration_watchers_for_test()
            .into_iter()
            .all(|(owner, _, _)| owner.document_owner() != Some(child_owner)),
        "child navigation must retire old registration lifecycle watchers"
    );

    vm.eval(
        r#"
        globalThis.__serviceWorkerOwnerFrame.contentWindow.navigator.serviceWorker
          .register("https://service-worker-owner.test/current-worker.js");
        "current-register-pending"
        "#,
    )
    .expect("replacement child register should schedule");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .into_iter()
            .map(|(_, owner)| owner.document_owner())
            .collect::<Vec<_>>(),
        vec![Some(replacement_child_owner)]
    );

    vm.eval("document.open(); 'replaced'")
        .expect("main replacement should retire main and descendant service worker owners");
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .is_empty(),
        "detaching the child document must retire its pending register resolver"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .into_iter()
            .all(|(_, owner)| {
                owner.document_owner() != Some(main_owner)
                    && owner.document_owner() != Some(child_owner)
                    && owner.document_owner() != Some(replacement_child_owner)
            }),
        "replacement must retire old main and child ready resolvers"
    );
    assert!(
        vm._context_host
            .borrow()
            .service_worker_registration_watchers_for_test()
            .into_iter()
            .all(|(owner, _, _)| {
                owner.document_owner() != Some(main_owner)
                    && owner.document_owner() != Some(child_owner)
            }),
        "replacement must retire lifecycle watchers bound to old documents"
    );
}
#[tokio::test]
async fn script_vm_drop_unregisters_child_service_worker_window_clients() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://child-client-teardown.test/page.html",
            &loader,
        );

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  return "created";
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");
    while vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("child frame Page task should apply")
    {}
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    vm._context_host
        .borrow_mut()
        .register_or_update_service_worker_child_client(child_handle)
        .expect("child frame should register a service worker window client");
    let active_diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(
        active_diagnostics.live_client_count, 2,
        "top-level page and child frame should both be live window clients before teardown"
    );

    drop(vm);
    let after_drop_diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(
        after_drop_diagnostics.live_client_count, 0,
        "dropping ScriptVm should unregister top-level and child window clients"
    );
}
