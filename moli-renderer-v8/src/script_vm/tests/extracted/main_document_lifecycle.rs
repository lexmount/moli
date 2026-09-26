use super::*;

#[test]
fn main_document_owner_transition_retires_script_vm_local_state_once() {
    let mut vm = new_storage_test_vm("https://main-owner-transition.test/");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    vm.pressed_mouse_buttons = 1;

    vm.eval("document.open(); 'replaced'")
        .expect("document.open should commit the replacement owner transaction");

    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(current_owner, retired_owner);
    assert_eq!(
        vm.pressed_mouse_buttons, 0,
        "runtime turn exit must retire ScriptVm-local input state from the owner transition"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .take_pending_main_document_owner_transitions()
            .is_empty(),
        "runtime turn exit must claim the replacement transaction exactly once"
    );

    vm.pressed_mouse_buttons = 1;
    vm.refresh_script_vm_local_document_state();
    assert_eq!(
        vm.pressed_mouse_buttons, 1,
        "refresh without a new owner transition must not replay document retirement"
    );
}
#[test]
fn main_document_open_preserves_already_recorded_runtime_binding_calls() {
    let mut vm = new_storage_test_vm("https://main-runtime-binding-owner.test/");
    vm.install_runtime_binding("ownerBoundRuntimeBinding", None, None)
        .expect("main Runtime binding should install");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial main Runtime binding owner");

    vm.eval(r#"ownerBoundRuntimeBinding("retired-document")"#)
        .expect("initial Runtime binding call");
    vm.page_diagnostics_snapshot()
        .expect("the initial call should enter the Page activity residence");

    vm.eval(
        r#"
        document.open();
        ownerBoundRuntimeBinding("replacement-document");
        document.close();
        "done"
        "#,
    )
    .expect("same-realm document.open Runtime binding sequence should evaluate");

    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main Runtime binding owner");
    assert_ne!(retired_owner.document_id, current_owner.document_id);
    assert_eq!(retired_owner.local_window_id, current_owner.local_window_id);
    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.payload.as_str())
            .collect::<Vec<_>>(),
        ["retired-document", "replacement-document"],
        "document.open() must not erase a binding invocation which was already accepted and published as Page activity"
    );
    assert_eq!(calls[0].source, calls[1].source);
}
#[test]
fn main_document_open_rebinds_preserved_isolated_runtime_binding_context() {
    let mut vm = new_storage_test_vm("https://isolated-runtime-binding-owner.test/");
    let isolated_context_id = vm
        .create_isolated_world("runtime-binding-owner", false)
        .expect("main isolated Runtime binding world should be created");
    vm.install_runtime_binding(
        "isolatedOwnerBoundRuntimeBinding",
        None,
        Some(isolated_context_id),
    )
    .expect("isolated Runtime binding should install");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial isolated Runtime binding document owner");

    vm.exec_in_execution_context(
        isolated_context_id,
        r#"
        isolatedOwnerBoundRuntimeBinding("retired-document");
        document.open();
        isolatedOwnerBoundRuntimeBinding("replacement-document");
        document.close();
        "#,
    )
    .expect("preserved isolated Runtime binding context should execute across document.open");

    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement isolated Runtime binding document owner");
    assert_ne!(retired_owner.document_id, current_owner.document_id);
    assert_eq!(retired_owner.local_window_id, current_owner.local_window_id);
    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.payload.as_str())
            .collect::<Vec<_>>(),
        ["retired-document", "replacement-document"]
    );
    assert!(
        calls
            .iter()
            .all(|call| call.execution_context_id == isolated_context_id)
    );
    assert_eq!(calls[0].source, calls[1].source);
}
#[tokio::test(flavor = "current_thread")]
async fn main_document_open_preserves_local_window_owned_webcrypto_task_body_authority() {
    let mut vm = new_storage_test_vm("https://main-owner-webcrypto.test/");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let execution_context_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
        before_document_owner.local_window_id,
    );
    let completion = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let resolver =
                v8::PromiseResolver::new(scope).expect("WebCrypto test resolver should exist");
            let promise = resolver.get_promise(scope);
            let context = scope.get_current_context();
            assert_eq!(
                context.global(scope).set(
                    scope,
                    crate::util::v8str(scope, "__localWindowWebCryptoPromise").into(),
                    promise.into(),
                ),
                Some(true)
            );
            let completion = unsafe { &mut *host_ptr }
                .register_pending_webcrypto_task(scope, resolver)
                .expect("WebCrypto task should capture the current execution context");
            Ok(completion)
        })
        .expect("WebCrypto test task should register");
    vm.eval(
        r#"
        globalThis.__localWindowWebCryptoResult = "pending";
        globalThis.__localWindowWebCryptoPromise.then(value => {
          globalThis.__localWindowWebCryptoResult = String(value);
        });
        "attached"
        "#,
    )
    .expect("WebCrypto test reaction should attach");
    let pending_before_open = vm
        ._context_host
        .borrow()
        .pending_webcrypto_execution_contexts_for_test();
    assert_eq!(pending_before_open.len(), 1);
    assert_eq!(pending_before_open[0].0, execution_context_owner);

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the Window execution context"
    );
    assert_ne!(
        after_document_owner.document_id, before_document_owner.document_id,
        "document.open must still rotate the Document owner"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_webcrypto_execution_contexts_for_test(),
        pending_before_open,
        "document.open must neither retire nor rebind LocalWindow-owned WebCrypto work"
    );

    completion
        .send(Ok(crate::context_bootstrap::WebCryptoTaskResult::Bool(
            true,
        )))
        .expect("preserved WebCrypto completion should enter its production source");
    assert!(
        vm.run_webcrypto_task_body_for_authorization_test()
            .expect("preserved WebCrypto task body should settle")
    );
    assert_eq!(
        vm.eval("globalThis.__localWindowWebCryptoResult")
            .expect("WebCrypto reaction state should remain readable after document.open"),
        "pending",
        "the low-level body authority must leave Promise reactions to the selected-task checkpoint"
    );
    assert_eq!(vm._context_host.borrow().pending_webcrypto_task_count(), 0);
}
#[tokio::test(flavor = "current_thread")]
async fn main_document_open_preserves_local_window_owned_dedicated_worker() {
    let mut vm = new_storage_test_vm("https://main-owner-worker.test/");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let expected_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
        before_document_owner.local_window_id,
    );
    let expected_realm = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::native_bridge::current_runtime_observable_context_token(scope)
                .ok_or_else(|| anyhow::anyhow!("main realm should expose an observable token"))
        })
        .expect("main realm token should be readable");

    vm.eval(
        r#"
        globalThis.__localWindowWorker = new Worker(
          "data:text/javascript,onmessage = () => {}"
        );
        "created"
        "#,
    )
    .expect("main DedicatedWorker should register");
    let workers_before_open = vm
        ._context_host
        .borrow()
        .worker_execution_contexts_for_test();
    assert_eq!(workers_before_open.len(), 1);
    assert_eq!(workers_before_open[0].1, expected_owner);
    assert_eq!(workers_before_open[0].2, expected_realm);

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the Window execution context"
    );
    assert_ne!(
        after_document_owner.document_id,
        before_document_owner.document_id
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test(),
        workers_before_open,
        "document.open must neither retire nor rebind a LocalWindow-owned Worker"
    );

    vm._context_host
        .borrow_mut()
        .forget_worker(workers_before_open[0].0);
}
#[test]
fn main_document_open_preserves_active_xhr_and_its_wrapper() {
    let mut vm = new_storage_test_vm("https://main-owner-xhr.test/");
    vm.eval("globalThis.__preservedXhrWrapper = new XMLHttpRequest(); 'created'")
        .expect("main XHR wrapper should capture its creation execution context");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (internal_id, owner, realm_token) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            Ok(register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            ))
        })
        .expect("main XHR should register");
    assert_eq!(
        owner,
        crate::native_bridge::WindowExecutionContextOwner::Frame(
            before_document_owner.local_window_id
        )
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test(),
        vec![(internal_id, owner, realm_token)]
    );

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the XHR-owning LocalWindow"
    );
    assert_ne!(
        after_document_owner.document_id, before_document_owner.document_id,
        "document.open must still rotate Document identity"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test(),
        vec![(internal_id, owner, realm_token)],
        "document.open must preserve an active XHR owned by the same LocalWindow"
    );
    assert!(!cancel_handle.is_cancelled());
    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(internal_id),
        "the preserved XHR must remain in the live request registry"
    );
    assert!(cancel_handle.is_cancelled());

    vm.eval(
        r#"
        __preservedXhrWrapper.open("GET", "data:text/plain,preserved");
        __preservedXhrWrapper.send();
        "queued"
        "#,
    )
    .expect("the same-LocalWindow XHR wrapper should remain usable after document.open");
    vm.eval("0")
        .expect("the preserved XHR data URL completion should run");
    assert_eq!(
        vm.eval("__preservedXhrWrapper.responseText")
            .expect("preserved XHR response should be readable"),
        "preserved"
    );
}
#[test]
fn main_document_open_preserves_ordinary_and_keepalive_fetches() {
    let mut vm = new_storage_test_vm("https://main-owner-fetch.test/");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let (ordinary, keepalive) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            Ok((
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    false,
                    PendingWindowFetchTestStage::Pending,
                ),
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    true,
                    PendingWindowFetchTestStage::Pending,
                ),
            ))
        })
        .expect("main Fetches should register");
    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the Fetch-owning LocalWindow"
    );
    assert_ne!(
        after_document_owner.document_id, before_document_owner.document_id,
        "document.open must still rotate Document identity"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![
            (ordinary.0, false, Some(ordinary.1), Some(ordinary.2)),
            (keepalive.0, false, Some(keepalive.1), Some(keepalive.2)),
        ],
        "document.open must preserve both Fetches and their JS delivery endpoints"
    );
    assert!(!ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());

    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(ordinary.0),
        "the preserved ordinary Fetch must remain explicitly cancellable"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(keepalive.0)
    );
    assert!(ordinary.3.is_cancelled());
    assert!(keepalive.3.is_cancelled());
}
#[test]
fn main_document_open_fetch_redirect_uses_source_document_csp_report_context() {
    let mut vm = new_storage_test_vm("https://main-fetch-csp-owner.test/source-document");
    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    let request_url = Url::parse("https://main-fetch-csp-owner.test/accepted").unwrap();
    let final_url = Url::parse("https://redirected-fetch-csp-owner.test/final").unwrap();
    let report_url = Url::parse("https://main-fetch-csp-owner.test/report").unwrap();
    let policy = crate::document_runtime::DocumentPolicyContainer {
        response_content_security_report_only_policies: vec![format!(
            "connect-src 'none'; report-uri {report_url}"
        )],
        ..Default::default()
    };
    let registered = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            Ok(register_pending_window_fetch_with_connect_policy_for_test(
                scope,
                unsafe { &mut *host_ptr },
                false,
                policy,
                request_url.clone(),
            ))
        })
        .expect("main Fetch should capture its source Document CSP context");
    let source_document_owner = registered.4.owner();

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        globalThis.__replacementFetchCspEvents = 0;
        document.addEventListener("securitypolicyviolation", () => {
          globalThis.__replacementFetchCspEvents += 1;
        });
        "replaced"
        "#,
    )
    .expect("document.open should preserve the Fetch execution context");
    let replacement_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main owner");
    assert_eq!(
        Some(replacement_owner.local_window_id),
        match source_document_owner {
            crate::native_bridge::WindowDocumentOwner::Frame(owner) => {
                Some(owner.local_window_id)
            }
            crate::native_bridge::WindowDocumentOwner::LightweightPopup(_) => None,
        }
    );
    assert_ne!(
        source_document_owner,
        crate::native_bridge::WindowDocumentOwner::Frame(replacement_owner)
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: registered.0,
        request_url: request_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: true,
        response_filter: None,
        network_error_text: None,
        result: Ok(redirected_fetch_response(&request_url, final_url)).into(),
    })
    .expect("source-owned Fetch redirect should complete in the preserved LocalWindow");

    assert_eq!(
        vm.eval("String(globalThis.__replacementFetchCspEvents)")
            .expect("replacement CSP event count"),
        "0",
        "an old Document's redirect violation must not dispatch into its replacement"
    );
    let reports = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].1, registered.4);
    assert!(!reports[0].2, "CSP report transport must not retain V8");
    assert!(vm.take_network_output().into_items().any(|item| matches!(
        item,
        crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
            if record.resource_type() == crate::types::SubresourceResourceType::Fetch
                && matches!(record.outcome(), crate::types::SubresourceNetworkOutcome::Success { .. })
    )));
}
#[test]
fn main_document_open_preserves_accepted_beacon_without_rebind() {
    let mut vm = new_storage_test_vm("https://main-owner-beacon.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    assert_eq!(
        vm.eval(
            r#"String(navigator.sendBeacon(
              "https://beacon-execution-context.test/main",
              "payload"
            ))"#,
        )
        .expect("main Beacon should be accepted"),
        "true"
    );
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(!accepted[0].2, "accepted Beacon must not retain V8");

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted,
        "document.open must not retire or rebind accepted LocalWindow Beacon work"
    );

    let request_url = Url::parse("https://beacon-execution-context.test/main").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: accepted[0].0,
        request_url: request_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: Vec::new().into(),
        request_body: Some("payload".to_owned()),
        response_status_text: Some("No Content".to_owned()),
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            request_url,
            204,
            Vec::new(),
            String::new(),
        ))
        .into(),
    })
    .expect("accepted main Beacon should complete without entering V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test()
            .is_empty()
    );
}
#[test]
fn main_document_open_preserves_accepted_csp_report_but_rejects_stale_owner_reuse() {
    let document_url = Url::parse("https://main-owner-csp-report.test/page").unwrap();
    let report_url = Url::parse("https://main-owner-csp-report.test/report").unwrap();
    let violation = test_window_csp_report_violation(&document_url, &report_url);
    let mut vm = new_storage_test_vm(document_url.as_str());
    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("initial main CSP report owner");
    vm.queue_content_security_policy_violation_event_best_effort(&violation);
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert_eq!(
        accepted[0].1.owner(),
        crate::native_bridge::WindowDocumentOwner::Frame(before_owner)
    );
    assert!(!accepted[0].2);

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should rotate the report Document owner");
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main CSP report owner");
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted
    );

    let fields =
        crate::content_security_policy::ContentSecurityPolicyViolationEventFields::from(&violation);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        crate::network_host::send_content_security_policy_reports_for_window(
            scope,
            unsafe { &mut *host_ptr },
            before_owner,
            None,
            &fields,
            &violation.report_uri_endpoints,
            &violation.report_to_endpoints,
        );
        Ok(())
    })
    .expect("stale main report attempt should be consumed");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted,
        "old main Document owner must not enqueue a second report after document.open"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: accepted[0].0,
        request_url: report_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: Vec::new().into(),
        request_body: Some("report".to_owned()),
        response_status_text: Some("No Content".to_owned()),
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            report_url,
            204,
            Vec::new(),
            String::new(),
        ))
        .into(),
    })
    .expect("accepted main CSP report should complete without entering V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test()
            .is_empty()
    );
}
#[tokio::test(flavor = "current_thread")]
async fn main_document_replacement_rebinds_service_worker_lifecycle_watcher() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://service-worker-main-rebind.test/page.html",
        &loader,
    );
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    vm.eval(
        r#"
        globalThis.__mainServiceWorkerRegistration = null;
        navigator.serviceWorker.register(
          "https://service-worker-main-rebind.test/worker.js"
        ).then(registration => {
          globalThis.__mainServiceWorkerRegistration = registration;
        });
        "register-pending"
        "#,
    )
    .expect("main service worker register should schedule");
    let (request_id, register_owner) = vm
        ._context_host
        .borrow()
        .pending_service_worker_register_owners_for_test()
        .into_iter()
        .next()
        .expect("pending main service worker register");
    assert_eq!(register_owner.document_owner(), Some(retired_owner));
    let registration_scope = url::Url::parse("https://service-worker-main-rebind.test/").unwrap();
    let registration_snapshot =
        crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot::active_for_binding_test(
            registration_scope,
            url::Url::parse("https://service-worker-main-rebind.test/worker.js").unwrap(),
        );
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_register(crate::types::ServiceWorkerRegisterCompletion {
            request_id,
            document_owner: register_owner.window_document_owner(),
            result: Ok(registration_snapshot.clone()),
        })
        .expect("main service worker registration should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Register(request_id),
        "main service worker registration",
    )
    .await;
    vm.eval(
        r#"
        globalThis.__mainServiceWorkerLifecycle = "pending";
        globalThis.__mainServiceWorkerRegistration.addEventListener(
          "updatefound",
          () => { globalThis.__mainServiceWorkerLifecycle = "updatefound"; }
        );
        "listener-ready"
        "#,
    )
    .expect("main lifecycle listener should install");

    vm.eval("document.open(); 'replaced'")
        .expect("main document replacement should commit");
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(current_owner, retired_owner);
    let (rebound_owner, _, storage_key) = vm
        ._context_host
        .borrow()
        .service_worker_registration_watchers_for_test()
        .into_iter()
        .find(|(owner, _, _)| owner.document_owner() == Some(current_owner))
        .expect("same-Window registration watcher must rebind to the replacement document");

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: register_owner.window_document_owner(),
            storage_key: storage_key.clone(),
            registration: registration_snapshot.clone(),
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("retired-generation lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "retired-generation lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__mainServiceWorkerLifecycle")
            .expect("main lifecycle state should evaluate"),
        "pending",
        "the old transport generation must not dispatch through the rebound watcher"
    );

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: rebound_owner.window_document_owner(),
            storage_key,
            registration: registration_snapshot,
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("rebound lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "rebound lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__mainServiceWorkerLifecycle")
            .expect("main rebound lifecycle state should evaluate"),
        "updatefound"
    );
}
#[test]
fn main_connected_style_event_delays_complete_until_event_task_settles() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://main-style-lifecycle.test/",
        concat!(
            "<!doctype html><html><head>",
            "<style id='sheet'>body { color: black; }</style>",
            "</head><body></body></html>",
        ),
    );
    vm.replace_document_resource_runtime(&loader);
    vm.exec(
        r#"
        globalThis.__mainStyleLifecycleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __mainStyleLifecycleEvents.push(`style:${document.readyState}`);
        });
        window.addEventListener("load", () => {
          __mainStyleLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("main style lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");

    vm.queue_initial_connected_style_loads_for_current_owner();
    let ready = vm
        .take_next_connected_style_event_body_for_test()
        .expect("inline stylesheet should queue one ready element event");
    let ready = ready.into_ready();
    let binding = ready
        .load_event_binding()
        .expect("main style event should retain its exact lifecycle binding");
    assert_eq!(
        binding.element(),
        vm.document_runtime
            .get_element_by_id("sheet")
            .expect("stylesheet link")
    );
    assert_eq!(binding.owner(), owner);

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the exact style event binding must delay complete after DOMContentLoaded"
    );

    assert!(vm.dispatch_connected_style_load(ready));
    assert_eq!(
        vm.eval("__mainStyleLifecycleEvents.join('|')")
            .expect("style event trace"),
        "style:interactive",
        "the element event must run before window load"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "dispatching the event must not release its load delay early"
    );

    assert_eq!(
        vm.settle_connected_style_load(Some(binding)),
        crate::page_task_queue::PageConnectedStyleLoadDelayEffect::ReleasedExactBinding
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "settling the event task must release the exact lifecycle delay"
    );
}
#[tokio::test]
async fn main_static_image_event_delays_complete_and_runs_before_window_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-image-lifecycle.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<img id='hero' src='data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'>",
            "</body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainImageLifecycleEvents = [];
        document.getElementById("hero").addEventListener("load", () => {
          __mainImageLifecycleEvents.push(`image:${document.readyState}`);
        });
        window.addEventListener("load", () => {
          __mainImageLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("main image lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let image = vm
        .document_runtime
        .get_element_by_id("hero")
        .expect("static image");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should register static images");
    let pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("static image should own one pending event");
    assert!(matches!(
        pending.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.owner() == owner
                && binding.element() == image
                && binding.load_delay_token().is_some()
    ));

    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the image event task must delay complete without a pending-image scan"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("the image event owner turn should run")
    );
    assert_eq!(
        vm.eval("__mainImageLifecycleEvents.join('|')")
            .expect("image event trace"),
        "image:interactive",
        "the image event must be a separate turn before Window load"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "the image event callback must settle its exact lifecycle delay"
    );
    assert!(
        vm.dispatch_main_document_window_load_lifecycle(owner)
            .expect("main Window load lifecycle should apply")
            .is_none()
    );
    assert_eq!(
        vm.eval("__mainImageLifecycleEvents.join('|')")
            .expect("complete image lifecycle trace"),
        "image:interactive|window:complete",
        "DCL, the image DOM task, and Window load must remain separate ordered turns"
    );
}
#[tokio::test]
async fn main_document_replacement_retires_pending_image_request_sequence() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-image-replacement.test/",
        "<!doctype html><html><head></head><body></body></html>",
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__staleMainImageEventCount = 0;
        const image = document.createElement("img");
        image.id = "stale-image";
        image.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
        image.addEventListener("load", () => {
          globalThis.__staleMainImageEventCount += 1;
        });
        document.body.appendChild(image);
        "#,
        None,
    )
    .expect("stale main image should queue");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("retired main document owner");
    let image = vm
        .document_runtime
        .get_element_by_id("stale-image")
        .expect("stale image handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some()
    );

    vm.exec(
        "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        None,
    )
    .expect("main document replacement should complete");
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(retired_owner, current_owner);
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("stale image task should be consumed without dispatch")
    );
    assert_eq!(
        vm.eval("String(globalThis.__staleMainImageEventCount)")
            .expect("stale image event count"),
        "0"
    );
    assert_eq!(
        vm.current_main_document_task_owner(),
        Some(current_owner),
        "stale image event must not replace or mutate the new owner"
    );
}
#[tokio::test]
async fn main_image_network_terminal_queues_later_load_event_and_retires_delay() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(200).await;
    let document_url = image_url.replace("/image.png", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__networkImageEvents = [];
  const image = document.createElement("img");
  image.id = "network-image";
  image.addEventListener("load", () => __networkImageEvents.push("load:" + image.complete));
  image.addEventListener("error", () => __networkImageEvents.push("error:" + image.complete));
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("network image setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("network-image")
        .expect("network image handle");
    let pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("network image sequence");
    assert!(pending.network_request_id().is_some());
    assert!(matches!(
        pending.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.element() == image && binding.load_delay_token().is_some()
    ));

    let request = request_rx.await.expect("image request should arrive");
    let request_lower = request.to_ascii_lowercase();
    assert!(request.starts_with("GET /image.png HTTP/1.1"));
    assert!(request_lower.contains("sec-fetch-dest: image"));
    assert!(request_lower.contains("sec-fetch-mode: no-cors"));
    assert!(request_lower.contains("accept: image/avif,image/webp"));
    assert_eq!(
        vm.eval("JSON.stringify({events: __networkImageEvents, complete: document.getElementById('network-image').complete})")
            .expect("pre-terminal image state"),
        r#"{"events":[],"complete":false}"#
    );

    release_tx.send(()).expect("release image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "main image network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__networkImageEvents.join('|')")
            .expect("image completion trace"),
        "",
        "resource completion may only enqueue the image event follow-up"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some()
    );
    wait_for_image_load_event_executor_test_task(&mut vm, "network image decode completion").await;

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("network image load event turn")
    );
    assert_eq!(
        vm.eval("__networkImageEvents.join('|')")
            .expect("image load trace"),
        "load:true"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );
    server.await.expect("image server should finish");
}
#[tokio::test]
async fn main_image_http_failure_dispatches_error_only_on_later_event_turn() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(404).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__failedImageEvents = [];
  const image = document.createElement("img");
  image.id = "failed-image";
  image.onload = () => __failedImageEvents.push("load");
  image.onerror = () => __failedImageEvents.push("error:" + image.complete);
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("failed image setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("failed-image")
        .expect("failed image handle");
    request_rx
        .await
        .expect("failed image request should arrive");
    release_tx.send(()).expect("release failed image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "failed image network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__failedImageEvents.join('|')")
            .expect("failed image completion trace"),
        ""
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some()
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("failed image error event turn")
    );
    assert_eq!(
        vm.eval("__failedImageEvents.join('|')")
            .expect("failed image event trace"),
        "error:true"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );
    server.await.expect("failed image server should finish");
}
#[tokio::test]
async fn main_image_source_restart_cancels_exact_request_and_drops_stale_terminal() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(200).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__restartedImageEvents = [];
  const image = document.createElement("img");
  image.id = "restarted-image";
  image.onload = () => __restartedImageEvents.push("load:" + image.currentSrc);
  image.onerror = () => __restartedImageEvents.push("error");
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("network image restart setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("restarted-image")
        .expect("restarted image handle");
    let first = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("first image sequence");
    let first_request_id = first
        .network_request_id()
        .expect("first image sequence should bind its request");
    request_rx.await.expect("first image request should arrive");

    vm.eval(
        "document.getElementById('restarted-image').src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';",
    )
    .expect("replacement image source should evaluate");
    let second = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("replacement image sequence");
    assert_ne!(first.id(), second.id());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "source restart must cancel the exact pending image request"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: first_request_id,
        request_url: Url::parse(&image_url).expect("image request URL"),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale cancelled image completion".to_owned()).into(),
    })
    .expect("stale cancelled image completion should be harmless");
    wait_for_image_load_event_executor_test_task(&mut vm, "replacement image decode completion")
        .await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("replacement image event turn")
    );
    assert!(
        vm.eval("__restartedImageEvents.join('|')")
            .expect("replacement image trace")
            .starts_with("load:data:image/gif")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );

    release_tx.send(()).expect("release cancelled image server");
    server.await.expect("cancelled image server should finish");
}
#[tokio::test]
async fn main_static_media_load_delays_complete_until_loadeddata_owner_turn() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-lifecycle.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<video id='clip' src='data:video/webm;base64,AA=='>",
            "</video></body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainMediaLifecycleEvents = [];
        const clip = document.getElementById("clip");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => {
            __mainMediaLifecycleEvents.push(`${type}:${document.readyState}`);
          });
        }
        window.addEventListener("load", () => {
          __mainMediaLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("main media lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("static media element");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should register static media");
    let pending = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("static media should own one pending sequence");
    assert!(matches!(
        pending.owner(),
        crate::native_bridge::PendingMediaLoadOwner::Main {
            owner: binding_owner,
            load_delay: Some(binding),
        } if binding_owner == owner
            && binding.owner() == owner
            && binding.element() == media
            && binding.load_delay_token().is_some()
    ));

    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "media resource selection must delay complete without blocking DOMContentLoaded"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "loadstart owner turn").await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "loadedmetadata owner turn").await;
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "metadata must not release the media delay"
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "loadeddata owner turn").await;
    assert_eq!(
        vm.eval("__mainMediaLifecycleEvents.join('|')")
            .expect("main media event trace"),
        "loadstart:interactive|loadedmetadata:interactive|loadeddata:interactive"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "loadeddata dispatch must settle the exact media delay"
    );
    assert!(matches!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .expect("canplay continuation should remain queued")
            .owner(),
        crate::native_bridge::PendingMediaLoadOwner::Main {
            owner: binding_owner,
            load_delay: None,
        } if binding_owner == owner
    ));

    run_next_page_media_element_event_for_test(&mut vm, &loader, "canplay owner turn").await;
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "the terminal media event must retire the sequence"
    );
}
#[tokio::test]
async fn main_media_source_mutation_replaces_sequence_without_stale_settlement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-source-restart.test/",
        "<!doctype html><html><head></head><body><video id='clip'></video></body></html>",
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainMediaRestartEvents = [];
        const clip = document.getElementById("clip");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __mainMediaRestartEvents.push(type));
        }
        clip.setAttribute("src", "data:video/webm;base64,AA==");
        "#,
        None,
    )
    .expect("first media source should queue");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("media element");
    let first = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("first source sequence");

    vm.exec(
        "document.getElementById('clip').setAttribute('src', 'data:video/webm;base64,AQ==');",
        None,
    )
    .expect("second media source should replace the sequence");
    let second = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("second source sequence");
    assert_ne!(first.id(), second.id());

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false)
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale first-source task should be consumed",
    )
    .await;
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .expect("new sequence must survive the stale callback")
            .id(),
        second.id()
    );
    assert_eq!(
        vm.eval("__mainMediaRestartEvents.join('|')")
            .expect("media restart event trace"),
        ""
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the stale callback must not settle the replacement sequence"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "new loadstart owner turn").await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "new loadedmetadata owner turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "new loadeddata owner turn").await;
    assert_eq!(
        vm.eval("__mainMediaRestartEvents.join('|')")
            .expect("new media event trace"),
        "loadstart|loadedmetadata|loadeddata"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true)
    );

    vm.exec(
        "document.getElementById('clip').removeAttribute('src');",
        None,
    )
    .expect("removing media source should cancel the canplay continuation");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none()
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "cancelled canplay task should be consumed",
    )
    .await;
    assert_eq!(
        vm.eval("__mainMediaRestartEvents.join('|')")
            .expect("cancelled media event trace"),
        "loadstart|loadedmetadata|loadeddata"
    );
}
#[tokio::test]
async fn main_media_network_terminal_drives_readiness_on_later_event_turns() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(200).await;
    let document_url = media_url.replace("/media", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Video, true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__networkMediaEvents = [];
  const media = document.createElement("video");
  media.id = "network-media";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
    media.addEventListener(type, () => __networkMediaEvents.push(type));
  }}
  media.src = {media_url:?};
  (document.body || document.documentElement || document).appendChild(media);
}})()
"#
    ))
    .expect("network media setup should evaluate");

    let request = request_rx.await.expect("media request should arrive");
    assert!(request.starts_with("GET /media HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: video")
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-mode: no-cors")
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media loadstart turn")
        .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __networkMediaEvents, ready: document.getElementById('network-media').readyState, network: document.getElementById('network-media').networkState})"
        )
        .expect("pre-terminal media state should evaluate"),
        r#"{"events":["loadstart"],"ready":0,"network":2}"#,
        "loadstart must not synthesize media readiness before the network terminal"
    );
    assert!(
        !vm.apply_one_page_resource_terminal_owner_admission()
            .expect("pre-terminal media resource source should be readable")
    );

    release_tx.send(()).expect("release media response");
    wait_for_one_page_resource_completion_executor_test_turn(&mut vm, "media network completion")
        .await;
    assert_eq!(
        vm.eval("__networkMediaEvents.join('|')")
            .expect("media events after network completion should evaluate"),
        "loadstart",
        "resource completion may only enqueue the media event follow-up"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media metadata turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media data turn").await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media canplay turn")
        .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __networkMediaEvents, ready: document.getElementById('network-media').readyState, network: document.getElementById('network-media').networkState})"
        )
        .expect("terminal media state should evaluate"),
        r#"{"events":["loadstart","loadedmetadata","loadeddata","canplay"],"ready":4,"network":1}"#
    );
    server.await.expect("media server should finish");
}
#[tokio::test]
async fn main_media_http_failure_dispatches_error_later_and_retires_delay() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(404).await;
    let document_url = media_url.replace("/media", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Audio, true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__failedMediaEvents = [];
  const media = document.createElement("audio");
  media.id = "failed-media";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
    media.addEventListener(type, () => __failedMediaEvents.push(type));
  }}
  media.src = {media_url:?};
  (document.body || document.documentElement || document).appendChild(media);
}})()
"#
    ))
    .expect("failed media setup should evaluate");
    let media = vm
        .document_runtime
        .get_element_by_id("failed-media")
        .expect("failed media handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some(),
        "network media must own a lifecycle delay before completion"
    );
    let request = request_rx
        .await
        .expect("failed media request should arrive");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: audio")
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "failed media loadstart turn")
        .await;
    release_tx.send(()).expect("release failed media response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "failed media network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__failedMediaEvents.join('|')")
            .expect("failed media completion trace should evaluate"),
        "loadstart",
        "network failure must not dispatch the element error inline"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "failed media error turn").await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __failedMediaEvents, ready: document.getElementById('failed-media').readyState, network: document.getElementById('failed-media').networkState})"
        )
        .expect("failed media terminal state should evaluate"),
        r#"{"events":["loadstart","error"],"ready":0,"network":3}"#
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "the error event turn must settle and retire the exact lifecycle sequence"
    );
    server.await.expect("failed media server should finish");
}
#[tokio::test]
async fn main_media_source_restart_cancels_exact_network_request_and_stale_terminal() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(200).await;
    let document_url = media_url.replace("/media", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Video, true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__restartedNetworkMediaEvents = [];
  const media = document.createElement("video");
  media.id = "restarted-network-media";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
    media.addEventListener(type, () => __restartedNetworkMediaEvents.push(type));
  }}
  media.src = {media_url:?};
  (document.body || document.documentElement || document).appendChild(media);
}})()
"#
    ))
    .expect("network media restart setup should evaluate");
    let media = vm
        .document_runtime
        .get_element_by_id("restarted-network-media")
        .expect("restarted network media handle");
    let first = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("first network media sequence");
    let first_request_id = first
        .network_request_id()
        .expect("first media sequence should bind its exact network request");
    request_rx.await.expect("first media request should arrive");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1
    );

    vm.eval(
        "document.getElementById('restarted-network-media').src = 'data:video/webm;base64,AQ==';",
    )
    .expect("replacement media source should evaluate");
    let second = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("replacement media sequence");
    assert_ne!(first.id(), second.id());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "source restart must cancel and remove the exact pending media request"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: first_request_id,
        request_url: Url::parse(&media_url).expect("media request URL"),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale cancelled media completion".to_owned()).into(),
    })
    .expect("stale cancelled media completion should be harmless");

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale first media loadstart task",
    )
    .await;
    assert_eq!(
        vm.eval("__restartedNetworkMediaEvents.join('|')")
            .expect("stale media callback trace should evaluate"),
        ""
    );
    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("replacement media {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("__restartedNetworkMediaEvents.join('|')")
            .expect("replacement media trace should evaluate"),
        "loadstart|loadedmetadata|loadeddata|canplay"
    );
    release_tx.send(()).expect("release cancelled media server");
    server.await.expect("cancelled media server should finish");
}
#[tokio::test]
async fn main_media_source_children_reselect_through_parent_sequence() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-source-child.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<video id='clip'><source id='source' src='data:video/webm;base64,AA=='></video>",
            "</body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__sourceChildMediaEvents = [];
        const clip = document.getElementById("clip");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __sourceChildMediaEvents.push(type));
        }
        "#,
        None,
    )
    .expect("source child media listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should select the source child");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("source child media handle");
    let first = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("source child should create a parent media sequence");

    vm.exec(
        "document.getElementById('source').src = 'data:video/webm;base64,AQ==';",
        None,
    )
    .expect("source child mutation should restart parent selection");
    let second = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("source child mutation should replace the parent sequence");
    assert_ne!(first.id(), second.id());
    run_next_page_media_element_event_for_test(&mut vm, &loader, "stale source child media task")
        .await;
    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("source child media {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("__sourceChildMediaEvents.join('|')")
            .expect("source child media trace should evaluate"),
        "loadstart|loadedmetadata|loadeddata|canplay"
    );

    vm.exec(
        r#"
        const source = document.getElementById("source");
        source.remove();
        const replacement = document.createElement("source");
        replacement.src = "data:video/webm;base64,Ag==";
        document.getElementById("clip").appendChild(replacement);
        "#,
        None,
    )
    .expect("source child removal and insertion should reselect");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some(),
        "inserting a replacement source child must create a new parent sequence"
    );
}
#[tokio::test]
async fn main_static_text_track_starts_at_interactive_before_window_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-track-lifecycle.test/",
        concat!(
            "<!doctype html><html><head></head><body><video>",
            "<track id='captions' default ",
            "src='data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Ahello'>",
            "</video></body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainTrackLifecycleEvents = [];
        const captions = document.getElementById("captions");
        captions.addEventListener("load", () => {
          __mainTrackLifecycleEvents.push(`track:${document.readyState}`);
        });
        void captions.track;
        "#,
        None,
    )
    .expect("main text-track lifecycle listener should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should seed static text tracks");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("default mode DOM-manipulation turn"),
        "static default track should queue one typed mode-selection task"
    );
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("duplicate default-mode probe"),
        "parser insertion and interactive discovery must coalesce the same exact track"
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("track load-start networking turn")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("track terminal networking turn")
    );
    assert_eq!(
        vm.eval("__mainTrackLifecycleEvents.join('|')")
            .expect("main track event trace"),
        "track:interactive",
        "static tracks must start at interactive rather than inside Window load"
    );
}
#[tokio::test]
async fn main_text_track_network_terminal_gates_canplay_without_delaying_complete() {
    let (track_url, request_rx, release_tx, server) =
        spawn_gated_text_track_resource_server(200).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(
        crate::types::SubresourceResourceType::TextTrack,
        true,
    );
    let document_url = track_url.replace("/captions.vtt", "/page.html");
    let markup = format!(
        concat!(
            "<!doctype html><html><body>",
            "<video id='clip' src='data:video/webm;base64,AA=='>",
            "<track id='captions' default src='{track_url}'>",
            "</video></body></html>"
        ),
        track_url = track_url,
    );
    let mut vm = new_parsed_page_task_executor_test_vm(&document_url, &markup, &loader);
    vm.exec(
        r#"
        globalThis.__trackMediaEvents = [];
        const clip = document.getElementById("clip");
        const captions = document.getElementById("captions");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __trackMediaEvents.push(type));
        }
        captions.addEventListener("load", () => {
          __trackMediaEvents.push(`track-load:${captions.readyState}:${captions.track.cues.length}`);
        });
        captions.addEventListener("error", () => __trackMediaEvents.push("track-error"));
        void captions.track;
        "#,
        None,
    )
    .expect("text-track/media listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("media handle");
    let track = vm
        .document_runtime
        .get_element_by_id("captions")
        .expect("track handle");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should start media and text-track owner sequences");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    let media_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("media sequence")
        .id();
    let track_sequence = vm
        ._context_host
        .borrow()
        .pending_text_track_load_sequence(track)
        .expect("text-track sequence");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(1),
        "resource selection must snapshot the enabled text track"
    );
    assert!(track_sequence.network_request_id().is_none());

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track stable-state networking turn")
    );
    let request = request_rx.await.expect("text-track request should arrive");
    let request = request.to_ascii_lowercase();
    assert!(request.starts_with("get /captions.vtt http/1.1"));
    assert!(request.contains("accept: text/vtt,*/*;q=0.1"));
    assert!(request.contains("sec-fetch-dest: track"));
    assert!(request.contains("sec-fetch-mode: same-origin"));
    let text_track_request_count = vm
        .take_network_output()
        .into_items()
        .filter(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                    if request.resource_type() == crate::types::SubresourceResourceType::TextTrack
            )
        })
        .count();
    assert_eq!(
        text_track_request_count, 1,
        "the element sequence must be the sole text-track request producer"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadstart owner turn")
        .await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("already-applied default track mode turn"),
        "interactive selection should leave one coalesced default-mode task"
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadedmetadata owner turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadeddata owner turn")
        .await;
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("pre-track-terminal event trace"),
        "loadstart|loadedmetadata|loadeddata",
        "canplay must wait for the selection-time text track"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "loadeddata still releases the media load delay while the track remains pending"
    );

    release_tx.send(()).expect("release text-track response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "text-track network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("resource completion event trace"),
        "loadstart|loadedmetadata|loadeddata",
        "resource completion may only queue the later track event"
    );

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track load-event networking turn")
    );
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("track event trace"),
        "loadstart|loadedmetadata|loadeddata|track-load:2:1"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(0)
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "media canplay follow-up owner turn",
    )
    .await;
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("canplay event trace"),
        "loadstart|loadedmetadata|loadeddata|track-load:2:1|canplay"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none()
    );
    server.await.expect("text-track server should finish");
}
#[tokio::test]
async fn main_document_replacement_retires_pending_media_and_text_track_sequences() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-replacement.test/",
        "<!doctype html><html><head></head><body></body></html>",
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__staleMainMediaEventCount = 0;
        globalThis.__staleMainTextTrackEventCount = 0;
        const media = document.createElement("video");
        media.id = "stale-media";
        media.addEventListener("loadstart", () => {
          globalThis.__staleMainMediaEventCount += 1;
        });
        media.src = "data:video/webm;base64,AA==";
        const track = document.createElement("track");
        track.id = "stale-track";
        track.src = "data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Astale";
        for (const type of ["load", "error"]) {
          track.addEventListener(type, () => {
            globalThis.__staleMainTextTrackEventCount += 1;
          });
        }
        media.appendChild(track);
        document.body.appendChild(media);
        track.track.mode = "hidden";
        "#,
        None,
    )
    .expect("stale main media should queue");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("retired main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("stale-media")
        .expect("stale media handle");
    let track = vm
        .document_runtime
        .get_element_by_id("stale-track")
        .expect("stale text-track handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_some()
    );

    vm.exec(
        "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        None,
    )
    .expect("main document replacement should complete");
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(retired_owner, current_owner);
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_none()
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale media event should be consumed without dispatch",
    )
    .await;
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("stale text-track networking task should settle")
    );
    assert_eq!(
        vm.eval("String(globalThis.__staleMainMediaEventCount)")
            .expect("stale media event count"),
        "0"
    );
    assert_eq!(
        vm.eval("String(globalThis.__staleMainTextTrackEventCount)")
            .expect("stale text-track event count"),
        "0"
    );
    assert_eq!(vm.current_main_document_task_owner(), Some(current_owner));
}
