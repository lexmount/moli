use super::*;

#[tokio::test]
async fn child_navigation_retires_runtime_binding_context_and_stale_function() {
    let mut vm = new_storage_test_vm("https://child-runtime-binding-owner.test/");
    vm.set_stored_runtime_bindings(&[crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "childOwnerBoundRuntimeBinding".to_owned(),
        execution_context_name: None,
    }]);
    vm.eval(
        r#"
        (() => {
          const root = document.documentElement ||
            document.appendChild(document.createElement("html"));
          const body = document.body || root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          globalThis.__runtimeBindingOwnerFrame = frame;
          body.appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("child Runtime binding frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Runtime binding initial-empty setup",
    )
    .await;
    let initial_owner = current_single_child_document_owner_for_test(
        &vm,
        "child Runtime binding initial-empty document",
    );
    vm.eval("__runtimeBindingOwnerFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child Runtime binding document should queue");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "first child Runtime binding document",
    )
    .await;
    let committed_owner = current_single_child_document_owner_for_test(
        &vm,
        "committed child Runtime binding document",
    );
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let initial_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("initial child Runtime binding context should exist");
    vm.eval_in_child_default_context(
        initial_context_id,
        r#"childOwnerBoundRuntimeBinding("retired-inner-realm")"#,
    )
    .expect("old child realm Runtime binding call should queue");

    vm.eval(
        r#"
        globalThis.__retiredChildRuntimeBinding =
          __runtimeBindingOwnerFrame.contentWindow.childOwnerBoundRuntimeBinding;
        __retiredChildRuntimeBinding("retired-document");
        __runtimeBindingOwnerFrame.srcdoc = "<p>replacement</p>";
        "queued"
        "#,
    )
    .expect("old child Runtime binding call and replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let retired_calls = vm.take_runtime_binding_calls();
    assert_eq!(
        retired_calls
            .iter()
            .map(|call| call.payload.as_str())
            .collect::<Vec<_>>(),
        ["retired-inner-realm", "retired-document"],
        "calls accepted before child navigation are historical observations and must survive realm retirement"
    );
    assert!(
        retired_calls
            .iter()
            .all(|call| call.execution_context_id == initial_context_id)
    );

    vm.eval(
        r#"
        __retiredChildRuntimeBinding("stale-function");
        void __runtimeBindingOwnerFrame.contentWindow;
        "requested-replacement-realm"
        "#,
    )
    .expect("stale binding call should fail closed while requesting the replacement realm");
    assert!(
        vm.take_runtime_binding_calls().is_empty(),
        "a function captured from the retired child realm must not target the replacement owner"
    );
    let replacement_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "replacement child Runtime binding context",
    );
    assert_ne!(replacement_context_id, initial_context_id);

    vm.eval(
        r#"
        __runtimeBindingOwnerFrame.contentWindow.childOwnerBoundRuntimeBinding(
          "replacement-document"
        );
        "called"
        "#,
    )
    .expect("replacement child Runtime binding should remain callable");
    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "childOwnerBoundRuntimeBinding");
    assert_eq!(calls[0].payload, "replacement-document");
    assert_eq!(calls[0].execution_context_id, replacement_context_id);
}
#[tokio::test]
async fn inherited_opaque_srcdoc_reuses_initial_empty_child_local_window() {
    let mut vm = new_storage_test_vm("data:text/html,opaque-parent");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "inherited-opaque-frame";
  body.appendChild(frame);
  void frame.contentWindow;
})()
"#,
    )
    .expect("inherited opaque initial child should evaluate");
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "inherited opaque initial child owner");

    vm.eval(
        r#"
document.getElementById("inherited-opaque-frame").srcdoc =
  "<!doctype html><body><p id='opaque-marker'>committed opaque child</p></body>";
"navigating"
"#,
    )
    .expect("inherited opaque srcdoc navigation should evaluate");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "inherited opaque srcdoc commit")
        .await;

    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "inherited opaque committed child owner");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "an inherited opaque origin keeps its nonce identity and securely reuses the initial LocalWindow"
    );
    assert_ne!(
        committed_owner.document_id, initial_owner.document_id,
        "secure LocalWindow reuse must still install a new Document owner"
    );
    assert_eq!(
        vm.eval(
            "document.getElementById('inherited-opaque-frame').contentWindow.document.getElementById('opaque-marker').textContent"
        )
        .expect("opaque creator should access its inherited child origin"),
        "committed opaque child"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn child_navigation_retires_local_window_owned_xhr() {
    let mut vm = new_storage_test_vm("https://child-owner-xhr.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundXhrFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child XHR frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child XHR registration setup",
    )
    .await;
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "child XHR initial-empty document");
    vm.eval("__ownerBoundXhrFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child XHR document should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "first child XHR document").await;
    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "committed child XHR document");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child XHR realm should exist");
    vm.eval_in_child_default_context(
        child_context_id,
        "parent.__retiredChildXhrWrapper = new XMLHttpRequest(); 'captured'",
    )
    .expect("parent should retain the child-created XHR wrapper for stale-owner proof");
    let child_context_ptr = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child XHR realm record");
        &realm.context as *const _
    };
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (internal_id, owner, realm_token) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
            Ok(register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            ))
        })
        .expect("child XHR should register");
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
    assert_eq!(
        owner,
        crate::native_bridge::WindowExecutionContextOwner::Frame(child_owner.local_window_id)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test(),
        vec![(internal_id, owner, realm_token)]
    );

    vm.eval("__ownerBoundXhrFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "NavigationCommit must retire the old child execution context"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty()
    );
    assert!(
        cancel_handle.is_cancelled(),
        "child navigation must abort the old LocalWindow XHR transport"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id,
        request_url: Url::parse("https://xhr-execution-context.test/pending").unwrap(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale retired XHR completion".to_owned()).into(),
    })
    .expect("late completion for retired XHR should be harmless");
    let stale_open = vm
        .eval(
            r#"
        (() => {
          try {
            __retiredChildXhrWrapper.open(
              "GET",
              "https://xhr-execution-context.test/after-navigation"
            );
            return "no-error";
          } catch (error) {
            return [error.name, error.code, error instanceof DOMException].join("|");
          }
        })()
        "#,
        )
        .expect("calling open on a retained old-child XHR wrapper should fail closed");
    assert_eq!(stale_open, "InvalidStateError|11|true");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty(),
        "a retained old-child wrapper must not bind a new request to the replacement LocalWindow"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn child_navigation_aborts_fetch_and_detaches_keepalive() {
    let mut vm = new_storage_test_vm("https://child-owner-fetch.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundFetchFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child Fetch frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Fetch registration setup",
    )
    .await;
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "child Fetch initial-empty document");
    vm.eval("__ownerBoundFetchFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child Fetch document should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "first child Fetch document").await;
    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "committed child Fetch document");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child Fetch realm should exist");
    vm.eval_in_child_default_context(
        child_context_id,
        r#"
        parent.__retiredChildFetch = (...args) => fetch(...args);
        parent.__retiredChildPromiseConstructor = Promise;
        parent.__retiredChildTypeErrorConstructor = TypeError;
        "captured"
        "#,
    )
    .expect("parent should retain an old-child Fetch closure");
    let child_context_ptr = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child Fetch realm record");
        &realm.context as *const _
    };
    let (ordinary, keepalive) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
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
        .expect("child Fetches should register");

    vm.eval("__ownerBoundFetchFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "NavigationCommit must retire the old child execution context"
    );
    assert!(ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))]
    );

    vm.eval(
        r#"
        globalThis.__retiredChildFetchResult = "pending";
        const stalePromise =
          __retiredChildFetch("https://fetch-execution-context.test/stale");
        globalThis.__retiredChildFetchPromiseRealm =
          Object.getPrototypeOf(stalePromise) ===
            __retiredChildPromiseConstructor.prototype;
        stalePromise.then(
          () => { __retiredChildFetchResult = "resolved"; },
          error => {
            __retiredChildFetchResult = JSON.stringify([
              error instanceof __retiredChildTypeErrorConstructor,
              error.message
            ]);
          }
        );
        "queued"
        "#,
    )
    .expect("retained old-child Fetch closure should fail closed");
    vm.eval("0").expect("stale Fetch rejection microtask");
    assert_eq!(
        vm.eval("String(__retiredChildFetchPromiseRealm)")
            .expect("stale Fetch Promise realm"),
        "true",
        "binding-time shutdown must reject in the retained function's old child realm"
    );
    assert_eq!(
        vm.eval("__retiredChildFetchResult")
            .expect("stale Fetch result"),
        r#"[true,"Failed to execute 'fetch' on 'Window': The global scope is shutting down."]"#
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))],
        "old child realm must not bind a new request to the replacement LocalWindow"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: ordinary.0,
        request_url: Url::parse("https://fetch-execution-context.test/pending").unwrap(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale retired Fetch completion".to_owned()).into(),
    })
    .expect("late ordinary Fetch completion should be harmless");
    let _ = vm.take_network_output();
    let final_url = Url::parse("https://fetch-execution-context.test/pending").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: keepalive.0,
        request_url: final_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            final_url,
            200,
            vec![("content-type".to_owned(), b"text/plain".to_vec())],
            "keepalive completed".to_owned(),
        ))
        .into(),
    })
    .expect("detached keepalive completion should remain observable without V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test()
            .is_empty()
    );
    let records = vm
        .take_network_output()
        .into_items()
        .filter(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url().as_str()
                        == "https://fetch-execution-context.test/pending"
            )
        })
        .count();
    assert_eq!(
        records, 1,
        "detached keepalive must preserve network observation without settling the old Promise"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn child_navigation_keeps_accepted_beacon_network_only_and_rejects_stale_sender() {
    let mut vm = new_storage_test_vm("https://child-owner-beacon.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundBeaconFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child Beacon frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Beacon registration setup",
    )
    .await;
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "child Beacon initial-empty document");
    vm.eval("__ownerBoundBeaconFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child Beacon document should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "first child Beacon document")
        .await;
    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "committed child Beacon document");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child Beacon realm should exist");
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            r#"
            parent.__retiredChildBeacon = (...args) => navigator.sendBeacon(...args);
            String(navigator.sendBeacon(
              "https://beacon-execution-context.test/accepted",
              "payload"
            ))
            "#,
        )
        .expect("child Beacon should be accepted"),
        "true"
    );

    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    let (internal_id, accepted_identity, retained_v8_context) = accepted[0];
    assert!(
        !retained_v8_context,
        "accepted Beacon must not retain its source V8 context"
    );
    let child_handle = match accepted_identity.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => handle,
        other => panic!("child Beacon used unexpected dispatch scope: {other:?}"),
    };
    assert_eq!(
        accepted_identity.owner(),
        crate::native_bridge::WindowExecutionContextOwner::Frame(
            vm._context_host
                .borrow()
                .current_child_document_task_owner(child_handle)
                .expect("child Beacon owner should be current before navigation")
                .local_window_id,
        )
    );

    vm.eval("__ownerBoundBeaconFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "NavigationCommit must retire the old child execution context"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted,
        "an accepted Beacon must survive LocalWindow destruction without rebinding"
    );
    assert_ne!(
        vm._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
            .expect("replacement child owner")
            .local_window_id,
        match accepted_identity.owner() {
            crate::native_bridge::WindowExecutionContextOwner::Frame(local_window_id) => {
                local_window_id
            }
            other => panic!("child Beacon used unexpected owner: {other:?}"),
        }
    );
    assert_eq!(
        vm.eval(
            r#"String(__retiredChildBeacon(
              "https://beacon-execution-context.test/stale",
              "stale"
            ))"#,
        )
        .expect("retained old-child Beacon sender should fail closed"),
        "false"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted,
        "old child realm must not bind a new Beacon to the replacement LocalWindow"
    );

    let request_url = Url::parse("https://beacon-execution-context.test/accepted").unwrap();
    let body_source_id = 60_000 + internal_id;
    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        skip_fetch_security_validation: false,
        response_filter: None,
        internal_id,
        request_url: request_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: Vec::new().into(),
        request_body: Some("payload".to_owned()),
        body_source_id,
        network_request_headers: None,
        head: moli_fetch::ResponseHead {
            status_text: None,
            final_url: request_url,
            status: 204,
            headers: Vec::new(),
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
    })
    .expect("accepted Beacon should start streaming without its retired V8 context");
    vm.append_streaming_async_subresource_fetch_chunk(
        body_source_id,
        b"unobservable response body".to_vec(),
    );
    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("accepted Beacon should finish streaming without its retired V8 context");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test()
            .is_empty(),
        "Beacon terminal must release its network-only host state"
    );
    assert_eq!(
        vm.take_network_output()
            .into_items()
            .filter(|item| matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url().as_str()
                        == "https://beacon-execution-context.test/accepted"
            ))
            .count(),
        1,
        "accepted Beacon must remain network-observable after source LocalWindow destruction"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn service_worker_controller_change_targets_exact_child_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://service-worker-client-owner.test/page.html",
        &loader,
    );
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          (document.body || document.documentElement || document).appendChild(frame);
          globalThis.__serviceWorkerClientOwnerFrame = frame;
        })();
        "frame-ready"
        "#,
    )
    .expect("service worker client owner frame should schedule");
    assert_initial_about_blank_child_completed_through_page_for_test(
        &mut vm,
        &loader,
        "service worker client owner setup",
    )
    .await;

    let child_handle = {
        let host = vm._context_host.borrow();
        host.child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("child browsing context")
    };
    let child_client_id = vm
        ._context_host
        .borrow_mut()
        .register_or_update_service_worker_child_client(child_handle)
        .expect("child service worker client should register");
    let child_owner = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("current child owner snapshot");
        assert_eq!(
            snapshot.settings.service_worker_client_id,
            Some(child_client_id)
        );
        host.current_child_document_task_owner(child_handle)
            .expect("current child document owner")
    };
    let child_target = crate::types::ServiceWorkerWindowClientTarget {
        client_id: child_client_id,
        document_owner: crate::native_bridge::WindowDocumentOwner::Frame(child_owner),
    };
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerClientOwnerFrame.contentWindow;
          globalThis.__serviceWorkerMainControllerChangeCount = 0;
          child.__serviceWorkerChildControllerChangeCount = 0;
          navigator.serviceWorker.oncontrollerchange = () => {
            globalThis.__serviceWorkerMainControllerChangeCount++;
          };
          child.navigator.serviceWorker.oncontrollerchange = () => {
            child.__serviceWorkerChildControllerChangeCount++;
          };
        })();
        "listeners-ready"
        "#,
    )
    .expect("service worker client owner listeners should install");

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_controller_change(
            crate::types::ServiceWorkerControllerChangeCompletion {
                target: child_target,
            },
        )
        .expect("child controllerchange should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(&mut vm, &loader, "child controllerchange")
        .await;
    assert_eq!(
        vm.eval(
            "String(globalThis.__serviceWorkerClientOwnerFrame.contentWindow.__serviceWorkerChildControllerChangeCount)",
        )
        .expect("child controllerchange count should evaluate"),
        "1"
    );
    assert_eq!(
        vm.eval("String(globalThis.__serviceWorkerMainControllerChangeCount)")
            .expect("main controllerchange count should evaluate"),
        "0"
    );

    vm.eval(
        r#"
        globalThis.__serviceWorkerClientOwnerFrame.srcdoc = "<p>replacement</p>";
        "replacement-pending"
        "#,
    )
    .expect("child replacement should schedule");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child replacement must commit before stale client completion",
    )
    .await;
    let (replacement_owner, replacement_client_id) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("replacement child owner snapshot");
        (
            host.current_child_document_task_owner(child_handle)
                .expect("replacement child document owner"),
            snapshot
                .settings
                .service_worker_client_id
                .expect("replacement child service worker client"),
        )
    };
    assert_ne!(replacement_owner, child_owner);
    assert_eq!(
        replacement_client_id, child_client_id,
        "srcdoc replacement currently reuses the browser client id, so document epoch must carry currentness"
    );
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerClientOwnerFrame.contentWindow;
          child.__serviceWorkerReplacementControllerChangeCount = 0;
          child.navigator.serviceWorker.oncontrollerchange = () => {
            child.__serviceWorkerReplacementControllerChangeCount++;
          };
        })();
        "replacement-listener-ready"
        "#,
    )
    .expect("replacement child controllerchange listener should install");

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_controller_change(
            crate::types::ServiceWorkerControllerChangeCompletion {
                target: child_target,
            },
        )
        .expect("stale child controllerchange should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "stale child controllerchange",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(globalThis.__serviceWorkerClientOwnerFrame.contentWindow.__serviceWorkerReplacementControllerChangeCount)",
        )
        .expect("replacement controllerchange count should evaluate"),
        "0",
        "retired child client target must not dispatch into the replacement document"
    );

    let replacement_target = crate::types::ServiceWorkerWindowClientTarget {
        client_id: replacement_client_id,
        document_owner: crate::native_bridge::WindowDocumentOwner::Frame(replacement_owner),
    };
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_controller_change(
            crate::types::ServiceWorkerControllerChangeCompletion {
                target: replacement_target,
            },
        )
        .expect("current child controllerchange should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "current child controllerchange",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(globalThis.__serviceWorkerClientOwnerFrame.contentWindow.__serviceWorkerReplacementControllerChangeCount)",
        )
        .expect("current replacement controllerchange count should evaluate"),
        "1"
    );
}
#[tokio::test]
async fn child_default_context_inventory_keeps_initial_empty_lazy_until_commit() {
    let mut vm = new_storage_test_vm("https://child-default-lazy.test/");
    let initial_bridge_ref_count = vm._context_host.borrow().bridge_ref_count_for_test();
    let initial_native_contexts = vm
        .renderer_document_isolate_heap_usage()
        .expect("initial heap usage should be available")
        .number_of_native_contexts;

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "lazy-child-frame";
  body.appendChild(frame);
  return "created";
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");
    let child_handle = vm
        .document_runtime
        .get_element_by_id("lazy-child-frame")
        .expect("lazy child frame handle");

    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "plain child frame lifecycle",
    )
    .await;
    assert!(
        vm.child_frame_realm_store.len() == 0,
        "plain child frame lifecycle should not eagerly create CDP child default contexts"
    );
    assert!(
        !vm._context_host
            .borrow()
            .child_browsing_context_has_cached_snapshot_for_test(child_handle),
        "frame initialization must not represent the internal initial-empty Document as a navigation snapshot"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count,
        "plain child frame lifecycle should not retain a child default bridge ref"
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("post-child-lifecycle heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts,
        "plain child frame lifecycle should not create a V8 native context"
    );

    let realms = vm.live_child_default_runtime_realm_inventory();
    assert!(
        realms.is_empty(),
        "protocol inventory must not materialize an internal initial-empty child context"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count,
        "initial-empty inventory must not retain a child default bridge ref"
    );

    vm.eval(
        r#"
document.getElementById("lazy-child-frame").srcdoc =
  "<!doctype html><title>committed child</title>";
"queued"
"#,
    )
    .expect("committed child navigation should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "committed lazy child document")
        .await;

    let realms = vm.live_child_default_runtime_realm_inventory();
    assert_eq!(
        realms.len(),
        1,
        "inventory may materialize the committed child default context"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count + 1,
        "the committed child default context should retain one bridge ref"
    );
}
#[test]
fn same_origin_child_window_uses_shared_public_surface_shape() {
    let mut vm = new_storage_test_vm("https://child-window-shared-surface.test/");

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><title>child</title>";
  (document.body || document.documentElement || document).appendChild(iframe);

  const main = window;
  const child = iframe.contentWindow;
  const descriptorShape = descriptor => {
    if (descriptor === undefined) {
      return null;
    }
    if ("value" in descriptor) {
      const callable = typeof descriptor.value === "function";
      return {
        kind: "data",
        type: typeof descriptor.value,
        writable: descriptor.writable,
        enumerable: descriptor.enumerable,
        configurable: descriptor.configurable,
        name: callable ? descriptor.value.name : null,
        length: callable ? descriptor.value.length : null
      };
    }
    return {
      kind: "accessor",
      getterName: descriptor.get?.name ?? null,
      getterLength: descriptor.get?.length ?? null,
      setterName: descriptor.set?.name ?? null,
      setterLength: descriptor.set?.length ?? null,
      enumerable: descriptor.enumerable,
      configurable: descriptor.configurable
    };
  };
  const isDynamicOrInternal = name =>
    /^(0|[1-9]\d*)$/.test(name) ||
    name.startsWith("__moli") ||
    name.startsWith("__lm");
  const names = new Set([
    ...Object.getOwnPropertyNames(main),
    ...Object.getOwnPropertyNames(child)
  ]);
  const differences = [];
  for (const name of names) {
    if (isDynamicOrInternal(name)) {
      continue;
    }
    const mainShape = descriptorShape(
      Object.getOwnPropertyDescriptor(main, name)
    );
    const childShape = descriptorShape(
      Object.getOwnPropertyDescriptor(child, name)
    );
    if (JSON.stringify(mainShape) !== JSON.stringify(childShape)) {
      differences.push(name);
    }
  }
  differences.sort();
  return JSON.stringify(differences);
})()
"#,
        )
        .expect("same-origin Window descriptor comparison should evaluate"),
        "[]"
    );
}
#[test]
fn child_window_indexed_access_materializes_only_the_requested_descendant_realm() {
    let mut vm = new_storage_test_vm("https://child-window-indexed-lazy.test/");
    let initial_native_contexts = vm
        .renderer_document_isolate_heap_usage()
        .expect("initial heap usage should be available")
        .number_of_native_contexts;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const outer = document.createElement("iframe");
  outer.id = "outer";
  (document.body || document.documentElement || document).appendChild(outer);

  const nested = outer.contentDocument.createElement("iframe");
  nested.id = "nested";
  (outer.contentDocument.body || outer.contentDocument.documentElement || outer.contentDocument)
    .appendChild(nested);
  globalThis.__indexedLazyOuter = outer;
  globalThis.__indexedLazyNested = nested;
  return "created";
})()
"#,
        )
        .expect("nested initial-empty frame setup should evaluate"),
        "created"
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("outer child heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts + 1,
        "accessing the outer contentDocument should materialize only its LocalWindow context"
    );

    assert_eq!(
        vm.eval("String(__indexedLazyOuter.contentWindow.length)")
            .expect("outer child frame count should evaluate"),
        "1"
    );
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const child = __indexedLazyOuter.contentWindow;
  const length = Object.getOwnPropertyDescriptor(child, "length");
  const credentialless = Object.getOwnPropertyDescriptor(child, "credentialless");
  const navigator = Object.getOwnPropertyDescriptor(child, "navigator");
  const shape = descriptor => ({
    getter: `${descriptor.get.name}:${descriptor.get.length}`,
    setter: typeof descriptor.set,
    enumerable: descriptor.enumerable,
    configurable: descriptor.configurable
  });
  return JSON.stringify({
    length: shape(length),
    credentialless: shape(credentialless),
    navigator: shape(navigator),
    borrowedLength: length.get.call(child),
    borrowedCredentialless: credentialless.get.call(child),
    borrowedNavigatorIsChild:
      navigator.get.call(child) === child.navigator,
    fakeNavigatorReceiver: (() => {
      try {
        navigator.get.call({});
        return "accepted";
      } catch (error) {
        return error.name;
      }
    })()
  });
})()
"#,
        )
        .expect("child Window accessor descriptors should evaluate"),
        r#"{"length":{"getter":"get length:0","setter":"function","enumerable":true,"configurable":true},"credentialless":{"getter":"get credentialless:0","setter":"undefined","enumerable":true,"configurable":true},"navigator":{"getter":"get navigator:0","setter":"undefined","enumerable":true,"configurable":true},"borrowedLength":1,"borrowedCredentialless":false,"borrowedNavigatorIsChild":true,"fakeNavigatorReceiver":"TypeError"}"#
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("indexed child count heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts + 1,
        "enumerating indexed children must not eagerly materialize descendant realms"
    );

    assert_eq!(
        vm.eval(
            "String(__indexedLazyOuter.contentWindow[0] === __indexedLazyNested.contentWindow)",
        )
        .expect("indexed nested WindowProxy access should evaluate"),
        "true"
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("indexed nested child heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts + 2,
        "the indexed getter should materialize only the descendant Window that was requested"
    );
}
#[tokio::test]
async fn child_document_open_nested_frame_uses_inherited_frame_src_policy() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-document-open-frame-csp.test/",
        &loader,
    );
    vm.document_runtime
        .set_response_content_security_policies(&["frame-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__childFrameCspEvents = [];
  addEventListener("message", event => {
    __childFrameCspEvents.push(String(event.data));
  });
  const frame = document.createElement("iframe");
  frame.id = "csp-owner";
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.write(
    '<script>addEventListener("securitypolicyviolation", event => {' +
    '  top.postMessage(event.violatedDirective, "*");' +
    '});</scr' + 'ipt>' +
    '<iframe src="https://blocked-frame.test/fail.html"></iframe>'
  );
  frame.contentDocument.close();
  return "queued";
})()
"#,
        )
        .expect("child document.open CSP setup should evaluate"),
        "queued"
    );

    for label in [
        "initial-empty outer realm retirement",
        "written outer Document realm materialization",
    ] {
        assert!(
            vm.run_one_child_frame_task_executor_turn(
                ChildFrameSemanticTurnKind::RealmMaterialization,
                &loader,
            )
            .await
            .expect("child realm materialization should use the selected-task dispatcher"),
            "{label} must remain a visible selected child-family turn"
        );
    }
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("nested child navigation commit should use the selected-task dispatcher"),
        "the nested frame navigation should reach the CSP gate after both outer realm turns"
    );
    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("the child CSP report should dispatch to the top Window")
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__childFrameCspEvents)")
            .expect("child frame CSP events should be observable"),
        r#"["frame-src"]"#
    );
}
#[tokio::test]
async fn child_document_write_inline_classic_obeys_inherited_response_csp() {
    let mut vm = new_storage_test_vm("https://child-document-write-csp.test/");
    vm.set_response_content_security_policies(&["script-src 'nonce-allowed'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__childDocumentWriteCspEvents = [];
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.open();
  frame.contentDocument.write(`
    <script>parent.__childDocumentWriteCspEvents.push("blocked");<\/script>
    <script nonce="allowed">parent.__childDocumentWriteCspEvents.push("allowed");<\/script>
  `);
  frame.contentDocument.close();
  return __childDocumentWriteCspEvents.join("|");
})()
"#,
        )
        .expect("child document.write CSP probe should evaluate"),
        "allowed"
    );
}
#[test]
fn child_document_write_nested_write_preserves_parser_insertion_point() {
    let mut vm = new_storage_test_vm("https://child-document-write-insertion.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write(`<!doctype html>
    <html><body><main id="written-main">
      <p id="before-write">before</p>
      <script>document.write("<section id='nested-write'>nested</section>");<\/script>
      <template id="written-template"><span>template</span></template>
      <table id="written-table"><tbody><tr><td>cell</td></tr></tbody></table>
    </main></body></html>`);
  childDocument.close();
  return JSON.stringify({
    order: Array.from(childDocument.getElementById("written-main").children)
      .map(element => element.id || element.localName),
    nestedParent: childDocument.getElementById("nested-write").parentElement.id,
    bodyOrder: Array.from(childDocument.body.children)
      .map(element => element.id || element.localName)
  });
})()
"#,
        )
        .expect("nested child document.write parser insertion probe should evaluate");

    assert_eq!(
        result,
        r#"{"order":["before-write","script","nested-write","written-template","written-table"],"nestedParent":"written-main","bodyOrder":["written-main"]}"#
    );
}
#[test]
fn child_document_write_script_restores_character_chunked_tail() {
    let mut vm = new_storage_test_vm("https://child-write-character-tail.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  const source =
    "<script>document.write(\"<i id='a'>Filler Text</i>\")<\/script>" +
    "<b id=b>Filler Text</b>";
  for (const character of source) {
    childDocument.write(character);
  }
  childDocument.close();
  return Array.from(childDocument.body.children)
    .map(element => [element.localName, element.id, element.textContent].join(":"))
    .join("|");
})()
"#,
        )
        .expect("a written script must restore the character-chunked child parser tail");

    assert_eq!(result, "i:a:Filler Text|b:b:Filler Text");
}
#[test]
fn child_document_close_inside_written_script_preserves_insertion_stack() {
    let mut vm = new_storage_test_vm("https://child-write-script-close-stack.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  const source =
    "<script>document.write('<table><plaintext>Filler '); document.close();<\/script>";
  for (const character of source) {
    childDocument.write(character);
  }
  const children = childDocument.body.children;
  return [
    children.length,
    children[0]?.localName,
    children[0]?.textContent,
    children[1]?.localName,
  ].join("|");
})()
"#,
        )
        .expect("document.close() in a written script must preserve its parser insertion stack");

    assert_eq!(result, "2|plaintext|Filler |table");
}
#[test]
fn pending_initial_child_navigation_keeps_initial_empty_window_surface() {
    let mut vm = new_storage_test_vm("https://child-pending-window.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "pending-window";
  frame.src = "https://child-pending-window.test/target.html";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("pending child Window setup should evaluate");

    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "the target navigation must remain pending before its typed commit turn"
    );
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const child = document.getElementById("pending-window").contentWindow;
  return [
    child.location.href,
    child.navigation.currentEntry?.url ?? "null",
    child.document.URL,
    child.document.readyState,
  ].join("|");
})()
"#,
        )
        .expect("pending child Window state should evaluate"),
        "about:blank|about:blank|about:blank|complete",
        "a planned target must not be projected as committed while the initial empty Document is still live"
    );

    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()[0];
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_current_url(child_handle)
            .is_some_and(|url| moli_url::is_about_blank(&url)),
        "native current-Document state must agree with the initial empty Window surface"
    );
}
#[tokio::test]
async fn pending_child_navigation_does_not_materialize_initial_empty_preload_realm() {
    let mut vm = new_storage_test_vm("https://child-preload-lazy.test/");
    vm.set_stored_document_start_scripts(&[crate::DocumentStartScript {
        registry_key: Some("initial-empty-lazy-preload".to_owned()),
        devtools_session: None,
        source: "globalThis.__documentStartUrl = document.URL;".to_owned(),
        world_name: None,
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    }]);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "pending-child-navigation";
  frame.srcdoc = "<!doctype html><title>committed child</title>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("pending child navigation setup should evaluate");

    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "the srcdoc navigation must publish a commit turn"
    );
    assert!(
        vm.has_pending_child_document_lifecycle(),
        "a reserved srcdoc navigation commit must keep child lifecycle work pending"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::DocumentLifecycle
        ),
        "the internal initial empty Document must not publish parser-finished lifecycle work"
    );
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "protocol inventory must not expose a realm for the unobserved initial empty Document"
    );

    run_child_navigation_commit_and_host_load_for_test(&mut vm, "committed child preload document")
        .await;
    let child_realms = vm.live_child_default_runtime_realm_inventory();
    assert_eq!(
        child_realms.len(),
        1,
        "the committed child document must expose exactly one default realm"
    );
    let child_context_id = child_realms[0].context_id;
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, "__documentStartUrl")
            .expect("committed child preload URL should evaluate"),
        "about:srcdoc",
        "document-start replay must observe the committed child Document"
    );
}
#[tokio::test]
async fn child_default_realm_record_tracks_owner_frame_realm_id() {
    let mut vm = new_storage_test_vm("https://child-owner-realm.test/");

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

    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child frame owner record",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child frame owner record");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };

    assert_eq!(
        vm.child_frame_realm_store
            .owner_realm_id_for_context_id(child_context_id),
        Some(owner_realm_id)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .frame_owner_current_child_snapshot(child_handle)
            .and_then(|snapshot| snapshot.realm_id),
        Some(owner_realm_id)
    );

    let result = vm
        .eval_in_child_default_context(child_context_id, "globalThis === self")
        .expect("child owner realm id should enter the child default context");
    assert_eq!(result, "true");
    assert_eq!(
        vm.eval("document.location.href")
            .expect("main document location should remain observable"),
        "https://child-owner-realm.test/",
        "materializing a child realm must not bind its Location to the main Document"
    );
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, "document.location.href")
            .expect("child document location should evaluate in its owner realm"),
        "about:blank"
    );
}
#[tokio::test]
async fn child_document_script_owner_hooks_select_current_realm() {
    use super::child_document_script_owner_hooks::{
        ChildDocumentScriptOwnerHooks, ChildDocumentScriptRealmSelection,
    };
    use crate::frame_owner_model::FrameRealmId;

    let mut vm = new_storage_test_vm("https://child-script-hooks-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child script hook realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child script hook realm setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child script hook realm setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let script_handle = DomHandle::new(9100);

    let current = ChildDocumentScriptOwnerHooks::new(&mut vm).select_current_realm(
        child_handle,
        None,
        script_handle,
        "test_select_current_realm",
    );
    assert_eq!(
        current,
        ChildDocumentScriptRealmSelection::Current(owner_realm_id)
    );

    let stale = ChildDocumentScriptOwnerHooks::new(&mut vm).select_current_realm(
        child_handle,
        Some(FrameRealmId(owner_realm_id.0 + 1)),
        script_handle,
        "test_select_current_realm",
    );
    assert!(matches!(
        stale,
        ChildDocumentScriptRealmSelection::StaleRealm { .. }
    ));
}
#[tokio::test]
async fn function_constructor_frame_script_job_returns_child_realm_function() {
    let mut vm = new_storage_test_vm("https://child-function-job.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child function job setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child function-constructor job setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child function-constructor job setup",
    );
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let job = vm
        ._context_host
        .borrow()
        .frame_owner_child_function_constructor_script_job(
            child_handle,
            Vec::new(),
            "return globalThis === self ? 27 : -1".to_owned(),
        )
        .expect("child owner should build FunctionConstructor job");
    let function = vm
        .function_from_frame_script_job(job)
        .expect("FunctionConstructor job should return a child realm function");
    let result = vm
        .call_frame_function_for_test(owner_realm_id, &function)
        .expect("child realm function should be callable");
    assert_eq!(result, "27");
}
#[tokio::test]
async fn child_srcdoc_inline_classic_script_runs_as_frame_script_job() {
    let mut vm = new_storage_test_vm("https://child-inline-classic-driver.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childClassicDriverEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childClassicDriverFrame = frame;
  frame.onload = () => globalThis.__childClassicDriverEvents.push("load");
  frame.srcdoc = `
    <script id="inline-classic">
      parent.__childClassicDriverEvents.push("child:" + (globalThis === self));
      parent.__childClassicDriverEvents.push("current:" + document.currentScript.id);
      globalThis.__childClassicDriverValue = 41;
    <\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child inline classic driver setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child inline classic srcdoc should commit before parser work",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child inline classic script should execute from DocumentScriptReady",
    )
    .await;

    assert_eq!(
        vm.eval("__childClassicDriverEvents.join('|')")
            .expect("child inline classic driver events before HostLoad should evaluate"),
        "child:true|current:inline-classic",
        "DocumentScriptReady should execute the inline classic script without firing iframe load"
    );
    for transition in ["DOMContentLoaded", "complete"] {
        assert!(
            vm.run_child_frame_task_source_once_for_test(
                ChildFrameSemanticTurnKind::DocumentLifecycle
            )
            .await,
            "child inline classic should run its {transition} lifecycle transition before HostLoad"
        );
    }
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "iframe load should dispatch from the later HostLoad source turn"
    );
    assert_eq!(
        vm.eval("__childClassicDriverEvents.join('|')")
            .expect("child inline classic driver events after HostLoad should evaluate"),
        "child:true|current:inline-classic|load"
    );
    assert_eq!(
        vm.eval("String(__childClassicDriverFrame.contentDocument.currentScript)")
            .expect("child inline classic currentScript cleanup should evaluate"),
        "null"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child inline classic script should materialize a child realm");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childClassicDriverValue)"
        )
        .expect("child inline classic side effect should be visible in child realm"),
        "41"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childClassicDriverValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
}
#[tokio::test]
async fn child_lifecycle_queues_only_the_ready_sibling_for_host_load() {
    // Production PageVms install their ResourceRequestClient before child parser work can
    // publish a concrete fetch-start task. Keep this low-level fixture on the
    // same capability topology while leaving the external script unsettled so
    // the sibling-lifecycle assertion does not depend on a network response.
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm =
        new_storage_test_vm_with_loader("https://child-host-load-ready-sibling.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childHostLoadSiblingEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));

  const blocked = document.createElement("iframe");
  blocked.onload = () => globalThis.__childHostLoadSiblingEvents.push("blocked-load");
  blocked.srcdoc = `
    <script src="https://child-host-load-ready-sibling.test/blocking.js"><\/script>
    <script>parent.__childHostLoadSiblingEvents.push("blocked-after-inline");<\/script>
  `;
  body.appendChild(blocked);

  const ready = document.createElement("iframe");
  ready.onload = () => globalThis.__childHostLoadSiblingEvents.push("ready-load");
  ready.srcdoc = `<p>ready sibling</p>`;
  body.appendChild(ready);
})()
"#,
    )
    .expect("child host-load sibling setup should evaluate");

    for child in ["blocked", "ready"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            &format!("{child} sibling should commit before lifecycle selection"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
        "blocked sibling should start its typed classic fetch before lifecycle selection",
    )
    .await;
    for label in [
        "remaining blocked-sibling realm prerequisite",
        "remaining ready-sibling realm prerequisite",
    ] {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{label} must consume one visible child-family turn"
        );
    }
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
        "ready sibling should enter interactive before HostLoad"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "ready sibling should dispatch DOMContentLoaded before HostLoad"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "ready sibling should apply complete before HostLoad"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad should receive only the ready sibling lifecycle action"
    );
    assert_eq!(
        vm.eval("__childHostLoadSiblingEvents.join('|')")
            .expect("child host-load sibling events should evaluate"),
        "ready-load",
        "ready sibling iframe load should dispatch while the earlier blocked child stays pending"
    );

    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "blocked sibling must not have a HostLoad delivery action"
    );
    assert_eq!(
        vm.eval("__childHostLoadSiblingEvents.join('|')")
            .expect("child host-load sibling events after blocked pump should evaluate"),
        "ready-load",
        "blocked child should still not dispatch load without its external classic script"
    );
}
#[tokio::test]
async fn child_document_replacement_discards_modulepreload_before_realm_admission() {
    let (mut vm, modulepreload_source) =
        new_child_modulepreload_page_test_vm("https://child-modulepreload-replacement.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "replace-pre-realm-modulepreload";
  frame.srcdoc = `<link rel="modulepreload" href="/retired.mjs">`;
  body.appendChild(frame);
})()
"#,
    )
    .expect("replacement modulepreload fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let (child_handle, retired_owner) = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("replacement fixture should retain one child frame");
        let owner = host
            .current_child_document_task_owner(child_handle)
            .expect("first srcdoc should install an exact Document owner");
        (child_handle, owner)
    };
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        1
    );
    assert!(vm.has_pending_child_frame_realm_materialization());

    vm.eval(
        "document.getElementById('replace-pre-realm-modulepreload').srcdoc = \
         '<!doctype html><p id=\"replacement\">replacement</p>';",
    )
    .expect("replacement srcdoc should queue");
    let replacement_commit = vm
        .run_one_child_navigation_commit_body_for_test()
        .expect("replacement navigation body should succeed")
        .expect("replacement should retain one exact navigation task");
    assert!(
        matches!(
            replacement_commit.action.target_effect,
            crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner
        ),
        "replacement should commit before the stale realm task is selected"
    );
    let current_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("replacement should install a current child Document");
    assert_ne!(retired_owner, current_owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0,
        "the replacement boundary must retire pre-realm work for the old Document"
    );
    let stale = vm
        .run_one_child_realm_materialization_body_for_test()
        .expect("child realm materialization body should succeed")
        .expect("the old Document's durable task must consume one stale owner turn");
    assert!(matches!(
        stale.action.target_effect,
        crate::page_task_queue::PageChildRealmMaterializationTargetEffect::IgnoredStaleOwner { .. }
    ));
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_none(),
        "a scriptless replacement must not eagerly create a realm task after retiring the stale exact-Document reservation"
    );
    assert!(
        !modulepreload_source.has_ready_task(),
        "retired pre-realm work must not reach the typed executable source"
    );
}
#[tokio::test]
async fn child_window_object_event_listener_uses_object_relevant_realm() {
    let mut vm = new_storage_test_vm("https://child-object-listener-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__objectListenerFrame = frame;
})()
"#,
    )
    .expect("child object listener realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child object listener realm setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child object listener realm setup",
    );
    let child_handle = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        realm.child_handle
    };
    vm.eval(
        r#"
(() => {
  const listener = globalThis.__objectListenerFrame.contentWindow.Function("return {}")();
  listener.handleEvent = function() {};
  globalThis.__objectListenerFrame.contentWindow.addEventListener(
    "object-listener-realm",
    listener
  );
})()
"#,
    )
    .expect("parent should register child object listener on child window");

    let identities = vm
        ._context_host
        .borrow()
        .child_window_event_callback_identities_for_test(child_handle, "object-listener-realm");
    assert_eq!(
        identities.len(),
        1,
        "object EventListener should register exactly one callback record"
    );
    let (relevant, incumbent) = identities[0];
    assert_eq!(
        relevant.map(|identity| identity.dispatch_scope()),
        Some(crate::native_bridge::OwnerDispatchScope::Child(
            child_handle
        )),
        "callback-interface relevant realm should come from the listener object"
    );
    assert_eq!(
        incumbent.map(|identity| identity.dispatch_scope()),
        Some(crate::native_bridge::OwnerDispatchScope::Top),
        "object EventListener callback relevant realm should come from the listener object, \
         while incumbent realm should come from the registration call"
    );
}
#[tokio::test]
async fn child_window_event_handler_property_uses_child_relevant_realm() {
    let mut vm = new_storage_test_vm("https://child-handler-property-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child handler property realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child handler property realm setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child handler property realm setup",
    );
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  onload = function() {};
})()
"#,
    )
    .expect("child onload property assignment should evaluate");

    let identities = vm
        ._context_host
        .borrow()
        .child_window_event_callback_identities_for_test(child_handle, "load");
    assert_eq!(
        identities.len(),
        1,
        "child event-handler property should register exactly one callback record"
    );
    let (relevant, incumbent) = identities[0];
    assert_eq!(relevant, incumbent);
    assert_eq!(
        relevant.map(|identity| identity.dispatch_scope()),
        Some(crate::native_bridge::OwnerDispatchScope::Child(
            child_handle
        )),
        "child window event-handler property should register in the child callback realm"
    );
}
#[tokio::test]
async fn child_window_event_handler_mutation_preserves_registration_snapshot_semantics() {
    let mut vm = new_storage_test_vm("https://child-handler-mutation.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childHandlerMutationTrace = "";
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  body.appendChild(document.createElement("iframe"));
})()
"#,
    )
    .expect("child handler mutation setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child handler mutation setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child handler mutation setup");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .map(|realm| realm.owner_realm_id)
        .expect("child handler mutation realm should exist");

    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const trace = [];
  let dispatchNumber = 0;
  addEventListener("click", () => {
    dispatchNumber += 1;
    trace.push("before:" + dispatchNumber);
    if (dispatchNumber === 1) {
      onclick = () => {
        trace.push("replacement:1");
        return false;
      };
    } else if (dispatchNumber === 2) {
      onclick = null;
      onclick = () => trace.push("readded:3");
    }
  });
  onclick = () => trace.push("stale");
  addEventListener("click", () => trace.push("after:" + dispatchNumber));

  for (let index = 0; index < 3; index += 1) {
    const result = dispatchEvent(new Event("click", { cancelable: true }));
    trace.push("result:" + result);
  }
  parent.__childHandlerMutationTrace = trace.join(",");
})()
"#,
    )
    .expect("child handler mutation dispatch should evaluate");

    assert_eq!(
        vm.eval("globalThis.__childHandlerMutationTrace")
            .expect("child handler mutation trace should evaluate"),
        "before:1,replacement:1,after:1,result:false,\
         before:2,after:2,result:true,\
         before:3,after:3,readded:3,result:true",
        "non-null replacement must keep the existing registration slot, while remove and re-add \
         must create a registration excluded from the active dispatch snapshot"
    );
}
#[tokio::test]
async fn child_frame_realm_location_navigation_queues_child_navigation() {
    let mut vm = new_storage_test_vm("https://child-location-navigation.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child location navigation setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child location navigation setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child location navigation setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  location.href = "data:text/html,<!doctype html><p>child</p>";
})()
"#,
    )
    .expect("child location navigation should evaluate");

    assert!(
        !vm.has_pending_location_navigation(),
        "child location navigation must not queue top-level pending navigation"
    );
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(child_handle)
        .expect("child location navigation should queue pending child navigation");
    match pending {
        crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) => {
            assert_eq!(url.as_str(), "data:text/html,<!doctype html><p>child</p>");
        }
        other => panic!("child location navigation should use URL bootstrap, got {other:?}"),
    }
    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "child navigation should wake the dedicated commit source"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not claim or commit pending child navigation"
    );
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_pending_live_navigation_for_test(child_handle)
            .is_some(),
        "attempting HostLoad must leave the pending navigation untouched"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should consume the pending child navigation"
    );
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_pending_live_navigation_for_test(child_handle)
            .is_none(),
        "NavigationCommit should clear the pending navigation after commit"
    );
}
#[tokio::test]
async fn empty_srcdoc_keeps_document_url_separate_from_parent_fallback_base() {
    let mut vm = new_storage_test_vm("https://empty-srcdoc.test/path/page.html");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "empty-srcdoc";
  frame.srcdoc = "";
  body.appendChild(frame);
})()
"#,
    )
    .expect("empty srcdoc setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "empty srcdoc should commit about:srcdoc then HostLoad",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const doc = document.getElementById("empty-srcdoc").contentDocument;
  const relative = doc.createElement("a");
  relative.href = "asset.js";
  return [
    doc.URL,
    doc.baseURI,
    relative.href,
    doc.defaultView.navigation.entries().length,
    doc.defaultView.navigation.currentEntry.url
  ].join("|");
})()
"#,
        )
        .expect("empty srcdoc URL and base URL should evaluate"),
        "about:srcdoc|https://empty-srcdoc.test/path/page.html|https://empty-srcdoc.test/path/asset.js|1|about:srcdoc"
    );
}
#[tokio::test]
async fn committed_srcdoc_navigation_uses_document_url_and_appends_history_entry() {
    let mut vm = new_storage_test_vm("https://srcdoc-commit-history.test/path/page.html");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "srcdoc-commit-history";
  globalThis.__srcdocCommitHistorySource = URL.createObjectURL(
    new Blob(["source"], { type: "text/html" })
  );
  frame.src = __srcdocCommitHistorySource;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("committed child setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "ordinary child document should commit before srcdoc replacement",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.getElementById("srcdoc-commit-history");
  return [
    frame.contentDocument.URL === __srcdocCommitHistorySource,
    frame.contentWindow.navigation.entries().length
  ].join("|");
})()
"#,
        )
        .expect("ordinary child history should evaluate"),
        "true|1"
    );

    vm.eval(r#"document.getElementById("srcdoc-commit-history").srcdoc = "replacement""#)
        .expect("srcdoc replacement should queue");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "srcdoc replacement should commit before inspection",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.getElementById("srcdoc-commit-history");
  const doc = frame.contentDocument;
  return [
    doc.URL,
    doc.baseURI,
    frame.contentWindow.location.href,
    frame.contentWindow.navigation.entries().length,
    frame.contentWindow.navigation.entries()[0].url === __srcdocCommitHistorySource,
    frame.contentWindow.navigation.entries()[1].url
  ].join("|");
})()
"#,
        )
        .expect("srcdoc replacement URL and history should evaluate"),
        "about:srcdoc|https://srcdoc-commit-history.test/path/page.html|about:srcdoc|2|true|about:srcdoc"
    );

    vm.eval(r#"document.getElementById("srcdoc-commit-history").removeAttribute("srcdoc")"#)
        .expect("removing srcdoc should queue the src navigation");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "removing srcdoc should commit the original blob source",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.getElementById("srcdoc-commit-history");
  const doc = frame.contentDocument;
  return [
    doc !== null,
    doc.URL === __srcdocCommitHistorySource,
    doc.location.protocol,
    doc.body.textContent
  ].join("|");
})()
"#,
        )
        .expect("restored blob document should remain same-origin"),
        "true|true|blob:|source"
    );
}
#[tokio::test]
async fn srcdoc_replacement_keeps_old_owner_until_navigation_commit() {
    let mut vm = new_storage_test_vm("https://srcdoc-owner-transition.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "srcdoc-owner-transition";
  body.appendChild(frame);
})()
"#,
    )
    .expect("srcdoc transition setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "srcdoc transition setup should install initial about:blank",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "srcdoc transition initial child document",
    );
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .child_handle;
    let old_owner = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .map(|snapshot| {
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            )
        })
        .expect("initial about:blank should have a document owner");
    vm.eval(
        r#"
document.getElementById("srcdoc-owner-transition").srcdoc =
  "<!doctype html><p id='replacement'>replacement</p>";
"#,
    )
    .expect("srcdoc replacement should queue");
    let owner_before_commit = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .map(|snapshot| {
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            )
        })
        .expect("old document should remain installed while srcdoc navigation is pending");
    assert_eq!(old_owner, owner_before_commit);
    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "srcdoc setter should wake NavigationCommit instead of replacing inline"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "navigation scheduling must leave no old document-owned HostLoad delivery action"
    );

    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let new_owner = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .map(|snapshot| {
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            )
        })
        .expect("srcdoc commit should install a replacement owner");
    assert_ne!(old_owner, new_owner);
    assert_eq!(old_owner.scheduler_lane_id, new_owner.scheduler_lane_id);
    assert!(
        vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::DocumentLifecycle
        ),
        "replacement commit should expose document-owned lifecycle work before HostLoad delivery"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "replacement must not queue HostLoad before its complete transition"
    );
    assert_eq!(
        vm.eval(
            "document.getElementById('srcdoc-owner-transition').contentDocument.body.textContent"
        )
        .expect("replacement srcdoc should be observable after commit"),
        "replacement"
    );
}
#[tokio::test]
async fn child_window_onload_property_navigation_queues_child_navigation() {
    let mut vm = new_storage_test_vm("https://child-onload-navigation.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child onload navigation setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child onload navigation setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child onload navigation setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };

    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  onload = () => {
    location.href = "data:text/html,<!doctype html><p>child-onload</p>";
  };
})()
"#,
    )
    .expect("child onload property assignment should evaluate");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        let global = scope.get_current_context().global(scope);
        let event_ctor = global
            .get(scope, crate::util::v8str(scope, "Event").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .expect("Event constructor should be installed");
        let event = event_ctor
            .new_instance(scope, &[crate::util::v8str(scope, "load").into()])
            .expect("load Event should construct");
        unsafe { &mut *runtime_ptr }.dispatch_child_window_event(
            scope,
            child_handle,
            "load",
            event,
        );
        Ok(())
    })
    .expect("child load dispatch should complete");

    assert!(
        !vm.has_pending_location_navigation(),
        "child onload navigation must not queue top-level pending navigation"
    );
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(child_handle)
        .expect("child onload navigation should queue pending child navigation");
    match pending {
        crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) => {
            assert_eq!(
                url.as_str(),
                "data:text/html,<!doctype html><p>child-onload</p>"
            );
        }
        other => panic!("child onload navigation should use URL bootstrap, got {other:?}"),
    }
}
#[tokio::test]
async fn child_window_object_event_listener_invokes_callback_object() {
    let mut vm = new_storage_test_vm("https://child-object-listener-dispatch.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__objectListenerDispatchFrame = frame;
})()
"#,
    )
    .expect("child object listener dispatch setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child object listener dispatch setup",
    )
    .await;
    vm.live_child_default_runtime_realm_inventory();

    let result = vm
        .eval(
            r#"
(() => {
  const frame = globalThis.__objectListenerDispatchFrame;
  const listener = frame.contentWindow.Function("return { calls: [] }")();
  listener.handleEvent = function(event) {
    this.calls.push(event.type + ":" + (this === listener));
  };
  frame.contentWindow.addEventListener("object-listener-dispatch", listener);
  frame.contentWindow.dispatchEvent(new Event("object-listener-dispatch"));
  return JSON.stringify(listener.calls);
})()
"#,
        )
        .expect("child object listener dispatch should evaluate");
    assert_eq!(result, r#"["object-listener-dispatch:true"]"#);
}
#[tokio::test]
async fn child_window_same_realm_event_listener_dispatches_on_current_stack() {
    let mut vm = new_storage_test_vm("https://child-same-realm-listener.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child same-realm listener setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child same-realm listener setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child same-realm listener setup");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;

    let result = vm
        .eval_in_frame_realm(
            owner_realm_id,
            r#"
(() => {
  globalThis.__sameRealmListenerEvents = [];
  globalThis.addEventListener("same-realm-listener", function(event) {
    globalThis.__sameRealmListenerEvents.push(event.type + ":" + (this === event.currentTarget));
  });
  globalThis.dispatchEvent(new Event("same-realm-listener"));
  return JSON.stringify(globalThis.__sameRealmListenerEvents);
})()
"#,
        )
        .expect("child same-realm listener should dispatch in child frame realm");
    assert_eq!(result, r#"["same-realm-listener:true"]"#);
}
#[tokio::test]
async fn child_window_parent_realm_listener_uses_callback_relevant_realm() {
    let mut vm = new_storage_test_vm("https://child-parent-listener-route.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__parentRouteFrame = frame;
})()
"#,
    )
    .expect("child parent listener route setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child parent listener route setup",
    )
    .await;
    vm.eval(
        r#"
(() => {
  globalThis.__parentRouteEvents = [];
  globalThis.__parentRouteFrame.contentWindow.addEventListener(
    "parent-route",
    function(event) {
      globalThis.__parentRouteEvents.push(event.type + ":" + (this === event.currentTarget));
    }
  );
})()
"#,
    )
    .expect("parent listener should register on child window");
    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__parentRouteFrame.contentWindow.dispatchEvent(new Event("parent-route"));
  return JSON.stringify(globalThis.__parentRouteEvents);
})()
"#,
        )
        .expect("parent listener dispatch should evaluate");
    assert_eq!(result, r#"["parent-route:true"]"#);
}
#[test]
fn child_window_function_is_available_on_first_exposure() {
    let mut vm = new_storage_test_vm("https://child-function-request.test/");

    let first_result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__requestedChildFunctionWindow = frame.contentWindow;
  return [
    typeof globalThis.__requestedChildFunctionWindow.Function,
    globalThis.__requestedChildFunctionWindow.Function("return 33")(),
    globalThis.__requestedChildFunctionWindow === frame.contentWindow
  ].join("|");
})()
"#,
        )
        .expect("first contentWindow access should expose the real child realm");
    assert_eq!(
        first_result, "function|33|true",
        "first exposure must return the stable WindowProxy backed by the real child realm"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .window_execution_context_registry_counts_for_test(),
        (2, 2),
        "an exposed prebootstrapped child realm must be registered before its owner turn"
    );
    assert!(
        vm.has_pending_child_frame_realm_materialization(),
        "first exposure should enqueue the dedicated child realm-materialization source"
    );
    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed"),
        "the owner source should claim one live child realm request"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        1,
        "the dedicated owner source should materialize the requested child FrameRealm"
    );

    let result = vm
        .eval("globalThis.__requestedChildFunctionWindow.Function('return 34')()")
        .expect("follow-up contentWindow.Function call should retain the child realm");
    assert_eq!(result, "34");
}
#[test]
fn child_document_open_preserves_prebootstrapped_execution_context() {
    let mut vm = new_storage_test_vm("https://prebootstrap-document-open.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "prebootstrap-document-open";
  body.appendChild(frame);
  void frame.contentWindow.Function;
})()
"#,
    )
    .expect("first child Window exposure should prebootstrap a context");
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()
        .into_iter()
        .next()
        .expect("child browsing context should exist");
    let initial_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("initial child document owner should exist");
    let initial_context_token = vm
        .prebootstrapped_child_default_contexts
        .borrow()
        .get(&child_handle)
        .expect("child context should await owner promotion")
        .runtime_observable_context_token;

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("prebootstrap-document-open");
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write("<body><p id='replacement'>replacement</p></body>");
  childDocument.close();
  return [
    childWindow === frame.contentWindow,
    childDocument === frame.contentDocument,
    childDocument.getElementById("replacement").textContent
  ].join("|");
})()
"#,
        )
        .expect("document.open should update the existing child LocalWindow context");
    assert_eq!(result, "true|true|replacement");

    while vm
        .run_child_realm_materialization_body_for_test()
        .expect("document.open child realm owner turn should succeed")
    {
        // Preserve the production ChildFrameTask FIFO if script work sits
        // between two exact-Document materialization tasks.
    }

    let replacement_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("replacement child document owner should exist");
    assert_eq!(
        replacement_owner.local_window_id,
        initial_owner.local_window_id
    );
    assert_ne!(replacement_owner.document_id, initial_owner.document_id);
    assert!(
        vm.prebootstrapped_child_default_contexts
            .borrow()
            .is_empty(),
        "document.open owner processing should promote the exposed child context"
    );
    let promoted_context = vm
        .child_frame_realm_store
        .iter_by_execution_context_id()
        .next()
        .map(|(_, context)| context)
        .expect("promoted child context should exist");
    assert_eq!(
        promoted_context.local_window_id,
        initial_owner.local_window_id
    );
    assert_eq!(
        promoted_context.runtime_observable_context_token, initial_context_token,
        "document.open must not replace the LocalWindow realm"
    );
}
#[test]
fn child_document_open_context_preflight_failure_preserves_current_document() {
    let mut vm = new_storage_test_vm("https://document-open-preflight.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "document-open-preflight";
  body.appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.body.innerHTML = "<button id='old-child'>old</button>";
  globalThis.__preflightChildDocument = childDocument;
  globalThis.__preflightOldChild = childDocument.getElementById("old-child");
  globalThis.__preflightListenerRuns = 0;
  __preflightOldChild.addEventListener(
    "preflight-probe",
    () => __preflightListenerRuns++,
  );
  return "ready";
})()
"#,
        )
        .expect("child Document preflight fixture should evaluate");
    assert_eq!(setup, "ready");
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()
        .into_iter()
        .next()
        .expect("child browsing context should exist");
    let initial_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("child Document owner should exist");
    vm._context_host
        .borrow_mut()
        .force_child_default_context_preflight_failure_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  __preflightChildDocument.open();
  __preflightOldChild.dispatchEvent(new Event("preflight-probe"));
  return [
    __preflightChildDocument.getElementById("old-child") === __preflightOldChild,
    __preflightOldChild.textContent,
    __preflightListenerRuns,
  ].join("|");
})()
"#,
        )
        .expect("failed replacement preflight should return the existing child Document");

    assert_eq!(result, "true|old|1");
    assert_eq!(
        vm._context_host
            .borrow()
            .current_child_document_task_owner(child_handle),
        Some(initial_owner),
        "context materialization failure must not rotate the child Document owner",
    );
}
#[test]
fn child_document_open_revalidates_owner_after_descendant_unload_reentry() {
    let mut vm = new_storage_test_vm("https://document-open-unload-reentry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.body.innerHTML = "<p id='before'>before</p>";

  const descendant = childDocument.createElement("iframe");
  childDocument.body.appendChild(descendant);
  descendant.contentWindow.addEventListener("unload", () => {
    childDocument.open();
    childDocument.write("<p id='inner-open'>inner</p>");
    childDocument.close();
  });

  childDocument.open();
  childDocument.write("<p id='outer-open'>outer</p>");
  childDocument.close();

  return [
    childDocument.getElementById("before") === null,
    childDocument.getElementById("inner-open") === null,
    childDocument.getElementById("outer-open").textContent,
  ].join("|");
})()
"#,
        )
        .expect("reentrant descendant unload document.open should not stale-commit an owner plan");

    assert_eq!(
        result, "true|true|outer",
        "the outer document.open must resume on the current owner after unload-script reentry",
    );
}
#[test]
fn detached_first_exposure_retires_prebootstrapped_child_realm() {
    let mut vm = new_storage_test_vm("https://stale-child-function-request.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  const heldWindow = frame.contentWindow;
  frame.remove();
  return typeof heldWindow.Function;
})()
"#,
        )
        .expect("stale contentWindow materialization request setup should evaluate");
    assert_eq!(
        result, "function",
        "the real child realm remains usable by the current stack until owner retirement"
    );
    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed"),
        "the owner source should consume and stale-drop the detached realm request"
    );
    assert!(
        !vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed"),
        "stale-drop must consume exactly one durable request"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        0,
        "the owner source must not materialize a child FrameRealm for a detached owner request"
    );
    assert!(
        vm.prebootstrapped_child_default_contexts
            .borrow()
            .is_empty(),
        "the owner source must detach and release an unclaimed realm for a removed LocalWindow"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .window_execution_context_registry_counts_for_test(),
        (1, 1),
        "stale-drop must retire the unclaimed child context binding and realm registration"
    );
}
#[tokio::test]
async fn child_default_bridge_ref_is_released_on_child_context_teardown() {
    let mut vm = new_storage_test_vm("https://child-bridge-ref-regression.test/");
    let initial_bridge_ref_count = vm._context_host.borrow().bridge_ref_count_for_test();

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "bridge-ref-child";
  body.appendChild(frame);
  return typeof frame.contentWindow.Function;
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "function");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child bridge-ref teardown setup");
    let (child_context_ptr, child_realm_token): (
        *const v8::Global<v8::Context>,
        crate::native_bridge::RuntimeObservableContextToken,
    ) = {
        let context = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child default context should be tracked");
        (
            &context.context as *const _,
            context.runtime_observable_context_token,
        )
    };
    let retained_child_cache = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(
            child_context_ptr,
            |scope, _runtime_ptr| {
                Ok(crate::native_bridge::identity::retain_context_wrapper_cache_for_test(scope))
            },
        )
        .expect("child wrapper cache should be retainable for regression testing");
    let child_wrappers = vm
        .eval_string_in_context_ptr_runtime_turn(
            child_context_ptr,
            r#"
(() => {
  void (document.body || document.documentElement);
  return "child-wrappers";
})()
"#,
            false,
        )
        .expect("child wrapper cache setup should evaluate");
    assert_eq!(child_wrappers, "child-wrappers");
    let child_wrapper_count =
        retained_child_cache.strong_wrapper_entry_count_for_realm(child_realm_token);
    assert!(
        child_wrapper_count >= 1,
        "child context wrappers should populate its default-world cache partition: {child_wrapper_count}"
    );
    let top_realm_token = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            Ok(
                crate::native_bridge::current_runtime_observable_context_token(scope)
                    .expect("top default context should have a runtime token"),
            )
        })
        .expect("top default context token should be readable");
    let top_wrapper_count =
        retained_child_cache.strong_wrapper_entry_count_for_realm(top_realm_token);
    assert!(
        top_wrapper_count >= 1,
        "the shared default-world cache should contain live top-context wrappers"
    );
    assert!(
        vm.child_default_frame_id_for_execution_context_id(child_context_id)
            .is_some(),
        "child default execution context should map to a live frame"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count + 1,
        "creating a child default context should retain exactly one V8 bridge ref-count token"
    );

    let removed = vm
        .eval(
            r#"
(() => {
  document.getElementById("bridge-ref-child").remove();
  return "removed";
})()
"#,
        )
        .expect("child frame removal should evaluate");
    assert_eq!(removed, "removed");
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "removed iframe should not keep a live child default context"
    );
    assert_eq!(
        retained_child_cache.strong_wrapper_entry_count_for_realm(child_realm_token),
        0,
        "destroying a child default context must retire its strong wrapper entries"
    );
    assert!(
        retained_child_cache.strong_wrapper_entry_count_for_realm(top_realm_token)
            >= top_wrapper_count,
        "destroying a child context must preserve wrappers owned by the live top realm"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count,
        "child context teardown must drop its JsContextHostBridgeRef instead of retaining it until ScriptVm drop"
    );
}
#[tokio::test]
async fn child_default_global_keeps_native_function_constructor() {
    let mut vm = new_storage_test_vm("https://child-native-function.test/");

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

    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child native Function setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child native Function setup");

    let result = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  const make = Function("return 7");
  return JSON.stringify({
    functionResult: make(),
    evalResult: eval("1 + 2"),
    globalThisIsSelf: globalThis === self
  });
})()
"#,
        )
        .expect("child default Function/eval intrinsics should evaluate");
    assert_eq!(
        result,
        r#"{"functionResult":7,"evalResult":3,"globalThisIsSelf":true}"#
    );
}
#[tokio::test]
async fn first_exposed_child_realm_survives_lifecycle_bootstrap() {
    let mut vm = new_storage_test_vm("https://child-window-function-sync.test/");

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = "<body>replacement child realm</body>";
  body.appendChild(frame);
  globalThis.__childFunctionWindow = frame.contentWindow;
  return typeof globalThis.__childFunctionWindow.Function;
})()
"#,
        )
        .expect("child Function sync setup should evaluate");
    assert_eq!(
        created, "function",
        "first WindowProxy exposure must use the real child realm"
    );

    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "child Function sync setup should initialize through NavigationCommit then HostLoad",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child default execution context should be created");
    let child_context_result = vm
        .eval_in_child_default_context(child_context_id, "Function('return 4')()")
        .expect("child native Function should evaluate in child realm");
    assert_eq!(child_context_result, "4");

    let result = vm
        .eval(
            r#"
(() => {
  const make = globalThis.__childFunctionWindow.Function("return 9");
  return JSON.stringify({
    constructorType: typeof globalThis.__childFunctionWindow.Function,
    result: make()
  });
})()
"#,
        )
        .expect("synced child Function should evaluate from contentWindow wrapper");
    assert_eq!(result, r#"{"constructorType":"function","result":9}"#);
}
#[tokio::test]
async fn child_realm_sync_repairs_window_proxy_created_after_materialization() {
    let mut vm = new_storage_test_vm("https://child-window-function-late-wrapper.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__lateChildFunctionFrame = frame;
})()
"#,
    )
    .expect("late child Function wrapper setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "late child Function wrapper setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "late child Function wrapper setup",
    );
    let child_context_result = vm
        .eval_in_child_default_context(child_context_id, "Function('return 12')()")
        .expect("child native Function should evaluate before wrapper creation");
    assert_eq!(child_context_result, "12");

    vm.eval("globalThis.__lateChildFunctionWindow = __lateChildFunctionFrame.contentWindow")
        .expect("late contentWindow wrapper should evaluate");

    let result = vm
        .eval("__lateChildFunctionWindow.Function('return 13')()")
        .expect("late WindowProxy wrapper should sync child Function constructor");
    assert_eq!(result, "13");
}
