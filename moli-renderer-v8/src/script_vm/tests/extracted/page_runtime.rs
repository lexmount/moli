use super::*;

#[tokio::test(flavor = "current_thread")]
async fn connected_modulepreload_csp_hash_admission_preserves_integrity_metadata() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    const SOURCE: &str = "globalThis.sriExecutions = (globalThis.sriExecutions || 0) + 1;";
    const INTEGRITY: &str =
        "sha384-T7tuz8k7Hz0eBaWUPKiAEECRmaKHLJ1eRz7NF4VdK1fN++IaKD3hEk0SETOP+8aJ";
    for case in ["match", "mismatch", "missing", "tampered"] {
        let hits = Arc::new(AtomicUsize::new(0));
        let server_hits = Arc::clone(&hits);
        let server = StaticHttpServer::spawn_with_responder(
            1,
            "Content-Type: text/javascript\r\n",
            move |_, _| {
                server_hits.fetch_add(1, Ordering::SeqCst);
                if case == "tampered" {
                    "globalThis.sriExecutions = 1000;"
                } else {
                    SOURCE
                }
                .to_owned()
            },
        )
        .await;
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let document_url = server.base_url().join("page.html").unwrap();
        let module_url = server.base_url().join("module.js").unwrap();
        let mut vm = new_parsed_page_task_executor_test_vm(
            document_url.as_str(),
            "<!doctype html><html><head></head><body></body></html>",
            &loader,
        );
        vm.set_response_content_security_policies(&[format!(
            "script-src '{INTEGRITY}'; script-src-attr 'unsafe-inline'"
        )]);
        let integrity = match case {
            "missing" => None,
            "mismatch" => {
                Some("sha384-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            }
            _ => Some(INTEGRITY),
        };
        vm.eval(&format!(
            r#"
          globalThis.hashPreloadEvents = [];
          const link = document.createElement('link'); link.rel = 'modulepreload';
          link.href = {url};
          const integrity = {integrity};
          if (integrity !== null) link.integrity = integrity;
          link.addEventListener('load', event => hashPreloadEvents.push(event.type));
          link.addEventListener('error', event => hashPreloadEvents.push(event.type));
          document.head.append(link); 'queued';
        "#,
            url = serde_json::to_string(module_url.as_str()).unwrap(),
            integrity = serde_json::to_string(&integrity).unwrap()
        ))
        .unwrap();
        vm.prime_document_lifecycle_and_publish_stylesheets();
        let fetched = matches!(case, "match" | "tampered");
        if fetched {
            wait_for_one_page_resource_completion_selected_task_executor_test_turn(
                &mut vm, &loader, case,
            )
            .await;
        } else {
            let key = crate::module_runtime::ModuleMapKey::java_script(module_url);
            let entry = vm
                .document_runtime
                .native_module_entry_id(&key)
                .expect("CSP rejection settles the Module Map");
            assert!(
                vm.document_runtime
                    .native_module_failure(entry)
                    .unwrap()
                    .message()
                    .contains("Content Security Policy"),
                "{case}"
            );
        }
        assert!(
            vm.run_one_native_module_owner_event_task_executor_turn(&loader)
                .await
                .unwrap(),
            "{case}: the module-map terminal must notify its link client"
        );
        assert!(
            vm.run_one_dom_manipulation_task_executor_turn(
                PageDomManipulationTestFamily::ConnectedStyleEvent,
                &loader
            )
            .await
            .unwrap(),
            "{case}: the link event uses the selected Page task path"
        );
        assert_eq!(
            vm.eval("hashPreloadEvents.join('|')").unwrap(),
            if case == "match" { "load" } else { "error" },
            "{case}"
        );
        assert_eq!(
            vm.eval("String(globalThis.sriExecutions || 0)").unwrap(),
            "0",
            "modulepreload must not execute source"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            usize::from(fetched),
            "{case}: CSP must reject before sending a request"
        );
        if fetched {
            assert_eq!(server.finish_targets().await, ["/module.js"]);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_csp_admission_queues_events_and_rejects_replaced_document() {
    for replace in [false, true] {
        let mut vm = new_storage_test_vm("https://child-module-csp-event.test/");
        vm.set_fetch_subresource_interception(
            crate::types::SubresourceInterceptionClasses::for_resource_type(
                crate::types::SubresourceResourceType::CspReport,
            ),
        );
        vm.eval(r#"
          globalThis.__queuedModuleCspEvents = [];
          globalThis.__queuedModuleCspFrame = document.createElement('iframe');
          (document.body || document.documentElement || document).appendChild(__queuedModuleCspFrame);
          void __queuedModuleCspFrame.contentWindow;
          'frame-ready';
        "#).unwrap();
        assert_initial_about_blank_child_completed_synchronously_for_test(
            &mut vm,
            "queued module CSP event",
        )
        .await;
        let context_id =
            materialize_single_child_default_realm_for_test(&mut vm, "queued module CSP event");
        let handle = vm
            .child_frame_realm_store
            .get(&context_id)
            .unwrap()
            .child_handle;
        vm.eval(r#"
          __queuedModuleCspFrame.contentWindow.addEventListener('securitypolicyviolation', event => {
            __queuedModuleCspEvents.push(event.blockedURI);
          });
          'listener-ready';
        "#).unwrap();
        let document_url = vm
            ._context_host
            .borrow()
            .child_browsing_context_current_url(handle)
            .unwrap();
        let violation = test_window_csp_report_violation(
            &document_url,
            &Url::parse("https://report.test/csp").unwrap(),
        );
        vm.queue_captured_module_csp_violation(
            crate::native_bridge::OwnerDispatchScope::Child(handle),
            violation,
        );
        assert_eq!(
            vm.eval("__queuedModuleCspEvents.length").unwrap(),
            "0",
            "admission must not enter child JavaScript while the modulator is borrowed"
        );
        if replace {
            vm.eval("__queuedModuleCspFrame.srcdoc = '<p>replacement</p>'; 'queued'")
                .unwrap();
            assert_eq!(
                vm.run_next_child_frame_semantic_turn_for_test().await,
                Some(ChildFrameSemanticTurnKind::NavigationCommit)
            );
        }
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
        assert_eq!(
            vm.eval("__queuedModuleCspEvents.length").unwrap(),
            if replace { "0" } else { "1" }
        );
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
        assert_eq!(
            vm.eval("__queuedModuleCspEvents.length").unwrap(),
            if replace { "0" } else { "1" },
            "each queued event is consumed exactly once"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn detached_keepalive_redirect_reports_source_document_csp_without_v8() {
    let mut vm = new_storage_test_vm("https://detached-fetch-csp-owner.test/");
    vm.set_fetch_subresource_interception(
        crate::types::SubresourceInterceptionClasses::for_resource_type(
            crate::types::SubresourceResourceType::CspReport,
        ),
    );
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__detachedFetchCspFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child Fetch CSP frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Fetch CSP registration setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child Fetch CSP registration setup",
    );
    let child_context_ptr = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child Fetch CSP realm record");
        &realm.context as *const _
    };
    let report_url = Url::parse("https://detached-fetch-csp-owner.test/report").unwrap();
    let report_only_request =
        Url::parse("https://detached-fetch-csp-owner.test/report-only-source").unwrap();
    let enforce_request =
        Url::parse("https://detached-fetch-csp-owner.test/enforce-source").unwrap();
    let report_only_policy = crate::document_runtime::DocumentPolicyContainer {
        response_content_security_report_only_policies: vec![
            format!("connect-src 'none'; report-uri {report_url}").into(),
        ],
        ..Default::default()
    };
    let enforce_policy = crate::document_runtime::DocumentPolicyContainer {
        response_content_security_policies: vec![
            format!("connect-src 'none'; report-uri {report_url}").into(),
        ],
        ..Default::default()
    };
    let (report_only_fetch, enforce_fetch) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            Ok((
                register_pending_window_fetch_with_connect_policy_for_test(
                    scope,
                    host,
                    true,
                    report_only_policy,
                    report_only_request.clone(),
                ),
                register_pending_window_fetch_with_connect_policy_for_test(
                    scope,
                    host,
                    true,
                    enforce_policy,
                    enforce_request.clone(),
                ),
            ))
        })
        .expect("child keepalive Fetches should capture their source Document policy");
    assert_eq!(report_only_fetch.4, enforce_fetch.4);

    vm.eval("__detachedFetchCspFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert!(!report_only_fetch.3.is_cancelled());
    assert!(!enforce_fetch.3.is_cancelled());
    let replacement_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("replacement child Fetch CSP realm");
    vm.eval_in_child_default_context(
        replacement_context_id,
        r#"
        globalThis.__replacementFetchCspEvents = 0;
        self.addEventListener("securitypolicyviolation", () => {
          globalThis.__replacementFetchCspEvents += 1;
        });
        "listening"
        "#,
    )
    .expect("replacement child should install its CSP listener");

    let report_only_final = Url::parse("https://report-only-redirect-target.test/final").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: report_only_fetch.0,
        request_url: report_only_request.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: true,
        response_filter: None,
        network_error_text: None,
        result: Ok(redirected_fetch_response(
            &report_only_request,
            report_only_final,
        ))
        .into(),
    })
    .expect("detached report-only keepalive should complete without V8");
    let enforce_final = Url::parse("https://enforce-redirect-target.test/final").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: enforce_fetch.0,
        request_url: enforce_request.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new().into(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: true,
        response_filter: None,
        network_error_text: None,
        result: Ok(redirected_fetch_response(&enforce_request, enforce_final)).into(),
    })
    .expect("detached enforcing keepalive should fail without entering V8");

    assert_eq!(
        vm.eval_in_child_default_context(
            replacement_context_id,
            "String(globalThis.__replacementFetchCspEvents)",
        )
        .expect("replacement child CSP event count"),
        "0"
    );
    let reports = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(reports.len(), 2);
    assert!(
        reports
            .iter()
            .all(|(_, identity, retained_v8, credentials)| {
                *identity == report_only_fetch.4
                    && !retained_v8
                    && *credentials == moli_fetch::RequestCredentialsMode::SameOrigin
            })
    );
    let fetch_records = vm
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                if record.resource_type() == crate::types::SubresourceResourceType::Fetch =>
            {
                Some(record)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(fetch_records.len(), 2);
    assert!(fetch_records.iter().any(|record| {
        record.url() == &report_only_request
            && matches!(
                record.outcome(),
                crate::types::SubresourceNetworkOutcome::Success { .. }
            )
    }));
    assert!(fetch_records.iter().any(|record| {
        record.url() == &enforce_request
            && matches!(
                record.outcome(),
                crate::types::SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("Content Security Policy")
            )
    }));
}
#[tokio::test(flavor = "current_thread")]
async fn child_csp_report_keeps_exact_violation_document_without_v8_after_navigation() {
    let mut vm = new_storage_test_vm("https://child-owner-csp-report.test/");
    vm.set_fetch_subresource_interception(
        crate::types::SubresourceInterceptionClasses::for_resource_type(
            crate::types::SubresourceResourceType::CspReport,
        ),
    );
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundCspReportFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child CSP report frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child CSP report acceptance setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child CSP report acceptance setup",
    );
    let (child_handle, child_context_ptr) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child CSP report realm record");
        (realm.child_handle, &realm.context as *const _)
    };
    let document_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("child CSP report document owner");
    let document_url = vm
        ._context_host
        .borrow()
        .child_browsing_context_current_url(child_handle)
        .expect("child CSP report document URL");
    let report_url = Url::parse("https://csp-report-owner.test/report").unwrap();
    let violation = test_window_csp_report_violation(&document_url, &report_url);
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
        unsafe { &mut *host_ptr }
            .dispatch_child_content_security_policy_violation_event_best_effort(
                scope,
                child_handle,
                &violation,
            );
        Ok(())
    })
    .expect("child CSP violation should dispatch");

    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    let (internal_id, identity, retained_v8_context, credentials_mode) = accepted[0];
    assert_eq!(
        identity.owner(),
        crate::native_bridge::WindowDocumentOwner::Frame(document_owner)
    );
    assert_eq!(
        identity.dispatch_scope(),
        crate::native_bridge::OwnerDispatchScope::Child(child_handle)
    );
    assert!(!retained_v8_context);
    assert_eq!(
        credentials_mode,
        moli_fetch::RequestCredentialsMode::SameOrigin
    );

    vm.eval("__ownerBoundCspReportFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted,
        "accepted report must survive replacement with its original Document identity"
    );

    let fields =
        crate::content_security_policy::ContentSecurityPolicyViolationEventFields::from(&violation);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        crate::network_host::send_content_security_policy_reports_for_window(
            scope,
            unsafe { &mut *host_ptr },
            document_owner,
            Some(child_handle),
            &fields,
            &violation.report_uri_endpoints,
            &violation.report_to_endpoints,
        );
        Ok(())
    })
    .expect("stale exact-owner report attempt should be consumed");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted,
        "retired Document owner must not bind a new report to the replacement child"
    );

    let body_source_id = 70_000 + internal_id;
    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        skip_fetch_security_validation: false,
        response_filter: None,
        internal_id,
        request_url: report_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: vec![(
            "Content-Type".to_owned(),
            "application/csp-report".to_owned(),
        )]
        .into(),
        request_body: Some("report".to_owned()),
        body_source_id,
        network_request_headers: None,
        head: moli_fetch::ResponseHead {
            status_text: None,
            final_url: report_url.clone(),
            status: 204,
            headers: Vec::new(),
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            cache_state: Default::default(),
            negotiated_http_version: None,
        },
    })
    .expect("accepted CSP report should stream without its retired V8 context");
    vm.append_streaming_async_subresource_fetch_chunk(
        body_source_id,
        b"unobservable report response".to_vec(),
    );
    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("accepted CSP report should finish without its retired V8 context");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test()
            .is_empty()
    );
    assert_eq!(
        vm.take_network_output()
            .into_items()
            .filter(|item| matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url() == &report_url
            ))
            .count(),
        1,
        "report terminal must remain network-observable after Document retirement"
    );
}
#[test]
fn dom_wrapper_expando_survives_renderer_document_isolate_garbage_collection() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-expando-retention.test/",
        "<!doctype html><main></main>",
    );

    let seeded = vm
        .eval(
            r#"
(() => {
  document.body.__array = [];
  document.body.__object = { ok: true };
  return "seeded";
})()
"#,
        )
        .expect("wrapper expando setup should evaluate");
    assert_eq!(seeded, "seeded");

    vm.collect_renderer_document_isolate_garbage()
        .expect("document isolate garbage collection should run");

    let retained = vm
        .eval(
            r#"
(() => {
  return Array.isArray(document.body.__array) && document.body.__object.ok
    ? "retained"
    : "missing";
})()
"#,
        )
        .expect("wrapper expando probe should evaluate");
    assert_eq!(retained, "retained");
}
#[test]
fn renderer_document_isolate_memory_pressure_escalates_at_one_third_of_heap_limit() {
    assert!(!renderer_document_isolate_critical_pressure_required(0, 0));
    assert!(!renderer_document_isolate_critical_pressure_required(
        33, 100
    ));
    assert!(renderer_document_isolate_critical_pressure_required(
        34, 100
    ));
    assert!(renderer_document_isolate_critical_pressure_required(
        100, 100
    ));
}

#[test]
fn renderer_document_isolate_moderate_pressure_requires_more_than_8_mib_growth() {
    const EIGHT_MIB: usize = 8 * 1024 * 1024;
    const LIMIT: usize = 512 * 1024 * 1024;
    let base = RendererDocumentIsolateMemoryPressureInput {
        heap_size_limit: LIMIT,
        ..RendererDocumentIsolateMemoryPressureInput::default()
    };

    assert_eq!(
        renderer_document_isolate_memory_pressure_action(base),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: EIGHT_MIB,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: EIGHT_MIB + 1,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: 20 * 1024 * 1024 + EIGHT_MIB,
                last_maintenance_used_memory: 20 * 1024 * 1024,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: 20 * 1024 * 1024 + EIGHT_MIB + 1,
                last_maintenance_used_memory: 20 * 1024 * 1024,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    // Heap shrinkage below the prior baseline must not underflow or trigger GC.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: 4 * 1024 * 1024,
                last_maintenance_used_memory: 20 * 1024 * 1024,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    // External memory growth counts toward the 8 MiB threshold even when JS heap is flat.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: 2 * 1024 * 1024,
                external_memory: EIGHT_MIB,
                last_maintenance_used_memory: 2 * 1024 * 1024,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: 2 * 1024 * 1024,
                external_memory: EIGHT_MIB + 1,
                last_maintenance_used_memory: 2 * 1024 * 1024,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    // Unreclaimed native-backed finalizer bytes trigger moderate pressure above 8 MiB,
    // while already-finalized allocations (live_native_bytes == 0) do not trigger redundant GC.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                allocated_native_bytes_since_maintenance: EIGHT_MIB + 1,
                live_native_bytes: 0,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                allocated_native_bytes_since_maintenance: EIGHT_MIB + 1,
                live_native_bytes: EIGHT_MIB + 1,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    // Burst of unreclaimed finalizer registrations triggers immediately at 256.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                finalizer_registrations_since_maintenance: 255,
                live_finalizers: 255,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                finalizer_registrations_since_maintenance: 256,
                live_finalizers: 256,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    // Direct v8::Weak finalizer bursts are caught via global handle growth.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_global_handles_size: 1024 + 256 * std::mem::size_of::<usize>(),
                last_maintenance_used_global_handles_size: Some(1024),
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    // Newly detached contexts trigger moderate maintenance once.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                detached_contexts: 1,
                last_maintenance_detached_contexts: 0,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                detached_contexts: 1,
                last_maintenance_detached_contexts: 1,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    // Smaller pending native activity falls back to a bounded collection on the 4th tick,
    // while a truly idle isolate stays silent even after many ticks.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                allocated_native_bytes_since_maintenance: 1024,
                live_native_bytes: 1024,
                finalizer_registrations_since_maintenance: 1,
                live_finalizers: 1,
                ticks_with_pending_native_activity: 2,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                allocated_native_bytes_since_maintenance: 1024,
                live_native_bytes: 1024,
                finalizer_registrations_since_maintenance: 1,
                live_finalizers: 1,
                ticks_with_pending_native_activity: 3,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                ticks_with_pending_native_activity: 10,
                ..base
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::None
    );
    // A retained native wrapper, 0-byte finalizer, detached context, or direct
    // weak handle above initial baseline stays idle when no isolate activity
    // occurred, while subsequent isolate activity enables the bounded fallback.
    for retained_case in [
        RendererDocumentIsolateMemoryPressureInput {
            live_native_bytes: 1024 * 1024,
            live_finalizers: 1,
            ticks_with_pending_native_activity: 3,
            ..base
        },
        RendererDocumentIsolateMemoryPressureInput {
            live_finalizers: 1,
            ticks_with_pending_native_activity: 3,
            ..base
        },
        RendererDocumentIsolateMemoryPressureInput {
            detached_contexts: 1,
            last_maintenance_detached_contexts: 1,
            ticks_with_pending_native_activity: 3,
            ..base
        },
        RendererDocumentIsolateMemoryPressureInput {
            used_global_handles_size: 1032,
            initial_used_global_handles_size: Some(1024),
            last_maintenance_used_global_handles_size: Some(1032),
            ticks_with_pending_native_activity: 3,
            ..base
        },
    ] {
        assert_eq!(
            renderer_document_isolate_memory_pressure_action(
                RendererDocumentIsolateMemoryPressureInput {
                    has_isolate_activity_since_maintenance: false,
                    ..retained_case
                }
            ),
            RendererDocumentIsolateMemoryPressureAction::None
        );
        assert_eq!(
            renderer_document_isolate_memory_pressure_action(
                RendererDocumentIsolateMemoryPressureInput {
                    has_isolate_activity_since_maintenance: true,
                    ..retained_case
                }
            ),
            RendererDocumentIsolateMemoryPressureAction::Moderate
        );
    }
    // Critical escalation remains active even when growth since last maintenance is <= 8 MiB.
    assert_eq!(
        renderer_document_isolate_memory_pressure_action(
            RendererDocumentIsolateMemoryPressureInput {
                used_heap_size: 34,
                heap_size_limit: 100,
                last_maintenance_used_memory: 34,
                ..RendererDocumentIsolateMemoryPressureInput::default()
            }
        ),
        RendererDocumentIsolateMemoryPressureAction::Critical
    );
}

#[test]
fn renderer_document_isolate_maintenance_skips_idle_moderate_pressure_and_triggers_after_8_mib_growth()
 {
    let mut vm = new_parsed_test_vm(
        "https://idle-memory-pressure.test/",
        "<!doctype html><body></body>",
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None,
        "fresh idle page below 8 MiB heap growth must not trigger moderate memory pressure"
    );

    vm.eval(
        r#"
(() => {
  window.__retained = [];
  for (let i = 0; i < 160000; i++) {
    window.__retained.push({ id: i, payload: "item-" + i + "-" + "x".repeat(32) });
  }
  return String(window.__retained.length);
})()
"#,
    )
    .expect("large retained allocation should evaluate");

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-growth maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate,
        "heap growth exceeding 8 MiB must trigger moderate memory pressure once"
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("subsequent idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None,
        "subsequent idle maintenance ticks must not keep firing moderate memory pressure"
    );

    vm.eval(
        r#"
(() => {
  window.__small = new Array(1024).fill("small");
  return String(window.__small.length);
})()
"#,
    )
    .expect("small allocation should evaluate");

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("small allocation maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None,
        "sub-threshold heap growth must not trigger moderate memory pressure"
    );
}

async fn drain_v8_foreground_gc_tasks_until(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    mut condition: impl FnMut() -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        while vm
            .run_one_oldest_ready_page_task_executor_turn(loader)
            .await
            .expect("production Page-task dispatcher should drain V8 foreground tasks")
        {}
        if condition() {
            return;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return;
        };
        let arrived = tokio::time::timeout(remaining, vm.wait_for_v8_foreground_task_arrival())
            .await
            .unwrap_or(false);
        if !arrived {
            return;
        }
    }
}

async fn drain_v8_foreground_gc_tasks_to_quiescence(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) -> bool {
    let mut ran_foreground_task = false;
    loop {
        while vm
            .run_one_oldest_ready_page_task_executor_turn(loader)
            .await
            .expect("V8 GC foreground tasks should drain cleanly")
        {
            ran_foreground_task = true;
        }
        let quiet_window = if ran_foreground_task {
            std::time::Duration::from_millis(50)
        } else {
            std::time::Duration::from_secs(2)
        };
        if !tokio::time::timeout(quiet_window, vm.wait_for_v8_foreground_task_arrival())
            .await
            .unwrap_or(false)
        {
            break;
        }
    }
    ran_foreground_task
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_reclaims_native_backed_blobs_without_large_js_heap_growth()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://blob-memory-pressure.test/",
        &loader,
    );
    let owner_id = vm.resource_owner_id;

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    assert_eq!(crate::blob::live_blob_count_for_owner_for_test(owner_id), 0);
    vm.eval(
        r#"
(() => {
  window.__blobSeed = "x".repeat(1024 * 1024);
  for (let i = 0; i < 12; i++) {
    new Blob([window.__blobSeed]);
  }
  return "created";
})()
"#,
    )
    .expect("repeated Blob creation should evaluate");

    let live_blobs_after_alloc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_alloc > 0,
        "expected uncollected Blob backing entries before maintenance (after={live_blobs_after_alloc})"
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-blob maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate,
        "native-backed Blob allocations exceeding 8 MiB must trigger moderate memory pressure even when JS heap growth is small"
    );

    drain_v8_foreground_gc_tasks_until(&mut vm, &loader, || {
        crate::blob::live_blob_count_for_owner_for_test(owner_id) < live_blobs_after_alloc
    })
    .await;

    let live_blobs_after_gc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_gc < live_blobs_after_alloc,
        "moderate memory pressure must reclaim unreferenced native-backed Blobs (after_alloc={live_blobs_after_alloc}, after_gc={live_blobs_after_gc})"
    );

    for idle_tick in 0..6 {
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("subsequent idle maintenance check should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "once unreferenced Blobs are reclaimed, subsequent idle tick {idle_tick} must stay quiet"
        );
    }
}

#[test]
fn renderer_document_isolate_maintenance_tracks_external_array_buffer_memory_growth() {
    let mut vm = new_parsed_test_vm(
        "https://external-memory-pressure.test/",
        "<!doctype html><body></body>",
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    vm.eval(
        r#"
(() => {
  window.__externalBuffers = [];
  for (let i = 0; i < 10; i++) {
    const view = new Uint8Array(1024 * 1024);
    view[0] = i;
    window.__externalBuffers.push(view);
  }
  return String(window.__externalBuffers.length);
})()
"#,
    )
    .expect("ArrayBuffer allocation should evaluate");

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-external-memory maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate,
        "external memory growth exceeding 8 MiB must trigger moderate memory pressure"
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("subsequent idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None,
        "retained external buffers must not trigger repeated idle moderate pressure"
    );
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_reclaims_sub_threshold_blob_via_bounded_fallback_and_stays_idle()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://sub-threshold-blob-pressure.test/",
        &loader,
    );
    let owner_id = vm.resource_owner_id;

    // Negative control: a completely idle page stays silent across many ticks.
    for tick in 0..8 {
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("idle maintenance check should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "fresh idle page must remain silent on tick {tick}"
        );
    }

    assert_eq!(crate::blob::live_blob_count_for_owner_for_test(owner_id), 0);
    vm.eval(
        r#"
(() => {
  window.__smallSeed = "y".repeat(512 * 1024);
  new Blob([window.__smallSeed]);
  return "created";
})()
"#,
    )
    .expect("sub-threshold Blob creation should evaluate");

    let live_blobs_after_alloc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_alloc > 0,
        "expected uncollected Blob before bounded fallback"
    );

    for skipped_tick in 0..3 {
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("pre-fallback maintenance check should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "sub-threshold native wrapper must wait for bounded fallback on tick {skipped_tick}"
        );
    }

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("bounded fallback maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate,
        "sub-threshold unreferenced Blob must trigger moderate memory pressure on the 4th tick"
    );

    drain_v8_foreground_gc_tasks_until(&mut vm, &loader, || {
        crate::blob::live_blob_count_for_owner_for_test(owner_id) < live_blobs_after_alloc
    })
    .await;

    let live_blobs_after_gc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_gc < live_blobs_after_alloc,
        "bounded fallback must reclaim the unreferenced Blob (after_alloc={live_blobs_after_alloc}, after_gc={live_blobs_after_gc})"
    );

    for idle_tick in 0..6 {
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("post-reclaim idle maintenance check should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "reclaimed idle page must stay quiet on post-fallback tick {idle_tick}"
        );
    }
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_reclaims_retained_blob_after_release_without_idle_churn()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://retained-then-released-blob.test/",
        &loader,
    );
    let owner_id = vm.resource_owner_id;

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    assert_eq!(crate::blob::live_blob_count_for_owner_for_test(owner_id), 0);
    vm.eval(
        r#"
(() => {
  const seed = "z".repeat(1024 * 1024);
  window.__retainedBlobs = [];
  for (let i = 0; i < 10; i++) {
    window.__retainedBlobs.push(new Blob([seed]));
  }
  return String(window.__retainedBlobs.length);
})()
"#,
    )
    .expect("retained Blob creation should evaluate");

    let live_blobs_after_alloc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert_eq!(live_blobs_after_alloc, 10);

    // First maintenance observes >8 MiB newly allocated native bytes and runs once.
    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial post-alloc maintenance should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );

    // Wait for V8's spawned GC foreground tasks from that moderate pass to arrive
    // and drain to quiescence; they must not look like new script activity that
    // re-arms idle maintenance.
    assert!(
        drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await,
        "initial moderate pass must dispatch V8 GC foreground tasks"
    );

    // While retained and completely idle (including read-only heap diagnostics
    // and keep-alive queries), repeated ticks must not churn GC.
    for idle_tick in 0..6 {
        let _ = vm
            .renderer_document_isolate_heap_usage()
            .expect("read-only heap diagnostics should succeed");
        let _ = vm
            .has_event_loop_keep_alive_work()
            .expect("read-only keep-alive query should succeed");
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("idle retained-blob maintenance should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "retained Blobs on an idle page must not cause repeated GC on tick {idle_tick}"
        );
    }

    // Releasing the Blobs without allocating anything new counts as isolate activity,
    // enabling the bounded fallback to reclaim them.
    vm.eval("window.__retainedBlobs = null; 'released'")
        .expect("releasing retained Blobs should evaluate");

    let mut triggered_moderate = false;
    for _ in 0..4 {
        if vm
            .notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-release maintenance should succeed")
            == RendererDocumentIsolateMemoryPressureAction::Moderate
        {
            triggered_moderate = true;
            break;
        }
    }
    assert!(
        triggered_moderate,
        "releasing retained native-backed Blobs must trigger bounded moderate reclamation"
    );

    drain_v8_foreground_gc_tasks_until(&mut vm, &loader, || {
        crate::blob::live_blob_count_for_owner_for_test(owner_id) < live_blobs_after_alloc
    })
    .await;

    let live_blobs_after_release_gc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_release_gc < live_blobs_after_alloc,
        "post-release moderate maintenance must reclaim dropped Blobs (after_alloc={live_blobs_after_alloc}, after_gc={live_blobs_after_release_gc})"
    );

    for idle_tick in 0..6 {
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("post-release idle maintenance should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "post-release idle page must stay quiet on tick {idle_tick}"
        );
    }
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_tracks_registry_network_body_sources_and_early_consumption()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://registry-body-source-pressure.test/",
        &loader,
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    // Part 1: >8 MiB of unconsumed registry body sources must trigger immediate
    // Moderate pressure and reclaim all backing entries via V8 finalizers.
    let unconsumed_ids = vm
        .with_default_context_scope(|scope, _| {
            let mut ids = Vec::with_capacity(10);
            for _ in 0..10 {
                let (_source, id) = crate::network_host::register_test_network_body_source_in_scope(
                    scope,
                    vec![b'r'; 1024 * 1024],
                );
                ids.push(id);
            }
            Ok(ids)
        })
        .expect("registering unconsumed body sources should succeed");
    assert!(
        unconsumed_ids
            .iter()
            .all(|&id| crate::network_host::test_network_body_source_is_live(id)),
        "all unconsumed registry body sources should be live before maintenance"
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-body-source maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate,
        "unconsumed registry network body sources exceeding 8 MiB must trigger moderate pressure"
    );

    drain_v8_foreground_gc_tasks_until(&mut vm, &loader, || {
        unconsumed_ids
            .iter()
            .all(|&id| !crate::network_host::test_network_body_source_is_live(id))
    })
    .await;

    assert!(
        unconsumed_ids
            .iter()
            .all(|&id| !crate::network_host::test_network_body_source_is_live(id)),
        "moderate maintenance must reclaim all unconsumed registry network body sources"
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-reclaim idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    // Part 2: Explicitly consuming >8 MiB of registry body sources releases
    // their NativeMemoryLease immediately even if the JS source wrappers remain
    // retained, avoiding false-positive >8 MiB moderate triggers.
    let retained_consumed_ids = vm
        .with_default_context_scope(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let retained = v8::Array::new(scope, 10);
            let mut ids = Vec::with_capacity(10);
            for idx in 0..10u32 {
                let (source, id) = crate::network_host::register_test_network_body_source_in_scope(
                    scope,
                    vec![b'c'; 1024 * 1024],
                );
                let _ = retained.set_index(scope, idx, source.into());
                ids.push(id);
            }
            let key = crate::util::v8str(scope, "__retainedConsumedBodySources");
            let _ = global.set(scope, key.into(), retained.into());
            Ok(ids)
        })
        .expect("registering retained body sources should succeed");

    for &id in &retained_consumed_ids {
        assert!(
            crate::network_host::consume_test_network_body_source(id).is_some(),
            "explicit consumption should take the buffered bytes"
        );
        assert!(!crate::network_host::test_network_body_source_is_live(id));
    }

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-consumed maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None,
        "explicitly consumed registry body sources must release native byte leases immediately"
    );
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_recalibrates_baselines_after_async_moderate_gc() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://async-recalibration.test/",
        &loader,
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    // Allocate >12 MiB of unreferenced JS heap garbage, trigger async Moderate GC,
    // and drain V8 foreground tasks so the post-GC heap drops well below the pre-GC peak.
    vm.eval(
        r#"
(() => {
  let tmp = [];
  for (let i = 0; i < 240000; i++) {
    tmp.push({ id: i, payload: "garbage-" + i + "-" + "x".repeat(32) });
  }
  return String(tmp.length);
})()
"#,
    )
    .expect("transient heap allocation should evaluate");

    let pre_gc_peak = vm
        .renderer_document_isolate_heap_usage()
        .expect("pre-gc heap usage")
        .used_size;

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("first moderate pass should trigger"),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );

    assert!(
        drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await,
        "async moderate pass must run V8 GC foreground tasks"
    );

    let post_gc_valley = vm
        .renderer_document_isolate_heap_usage()
        .expect("post-gc heap usage")
        .used_size;
    assert!(
        post_gc_valley + 8 * 1024 * 1024 < pre_gc_peak,
        "expected async moderate GC to reclaim transient heap (valley={post_gc_valley}, peak={pre_gc_peak})"
    );

    // Allocate ~9 MiB of external memory: total tracked memory stays below or near
    // the pre-GC peak, yet exceeds post-GC valley by >8 MiB. Recalibration must catch it.
    vm.eval(
        r#"
(() => {
  window.__postValleyBuffers = [];
  for (let i = 0; i < 9; i++) {
    const view = new Uint8Array(1024 * 1024);
    view[0] = i;
    window.__postValleyBuffers.push(view);
  }
  return String(window.__postValleyBuffers.length);
})()
"#,
    )
    .expect("post-valley external allocation should evaluate");

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-recalibration maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate,
        "new >8 MiB growth after async Moderate GC must not be hidden by the pre-GC peak baseline"
    );

    assert!(drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await);

    // Detached context recalibration: after leaving the initial-empty document,
    // navigating an iframe replaces its Window while reusing its WindowProxy,
    // detaching the outgoing child v8::Context (`detach_global()`). Once async
    // Moderate GC collects that detached context and lowers the baseline back to
    // 0, a second replacement navigation must trigger Moderate on the next tick.
    vm.eval(
        r#"
(() => {
  window.__recalFrame = document.createElement("iframe");
  document.body.appendChild(window.__recalFrame);
  window.__recalFrame.srcdoc = "<p>committed-base</p>";
  return "base";
})()
"#,
    )
    .expect("initial child srcdoc navigation should evaluate");
    while vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("base child srcdoc commit should settle")
    {}

    vm.eval(
        r#"
(() => {
  void window.__recalFrame.contentDocument.body.textContent;
  window.__recalFrame.srcdoc = "<p>nav-1</p>";
  return "nav-1";
})()
"#,
    )
    .expect("first replacement child navigation should evaluate");
    while vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("first replacement child Window should settle")
    {}
    let detached_after_nav1 = vm
        .renderer_document_isolate_heap_usage()
        .expect("heap usage after first replacement navigation")
        .number_of_detached_contexts;
    assert!(
        detached_after_nav1 >= 1,
        "reused-WindowProxy child navigation must produce a detached context (got {detached_after_nav1})"
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("first detached iframe maintenance should trigger"),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
    assert!(drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await);
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("heap usage after first detached-context GC")
            .number_of_detached_contexts,
        0,
        "async Moderate GC must collect the first detached child context"
    );

    vm.eval(
        r#"
(() => {
  void window.__recalFrame.contentDocument.body.textContent;
  window.__recalFrame.srcdoc = "<p>nav-2</p>";
  return "nav-2";
})()
"#,
    )
    .expect("second replacement child navigation should evaluate");
    while vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("second replacement child Window should settle")
    {}
    let detached_after_nav2 = vm
        .renderer_document_isolate_heap_usage()
        .expect("heap usage after second replacement navigation")
        .number_of_detached_contexts;
    assert!(
        detached_after_nav2 >= 1,
        "second replacement navigation must produce a new detached context (got {detached_after_nav2})"
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("second detached iframe maintenance must trigger after recalibration"),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_detects_first_tick_direct_weak_wrappers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://first-tick-direct-weak.test/",
        &loader,
    );

    let handles_before = vm
        .renderer_document_isolate_heap_usage()
        .expect("initial heap usage")
        .used_global_handles_size;

    // Create direct v8::Weak::with_finalizer wrappers (CompressionStream codec
    // state) BEFORE the first maintenance tick; the initial main-world global
    // handles baseline must detect them instead of absorbing them into the
    // baseline.
    vm.eval(
        r#"
(() => {
  for (let i = 0; i < 32; i++) {
    new CompressionStream("gzip");
  }
  return "created";
})()
"#,
    )
    .expect("direct weak CompressionStream allocation should evaluate");

    let handles_after = vm
        .renderer_document_isolate_heap_usage()
        .expect("post-alloc heap usage")
        .used_global_handles_size;
    assert!(
        handles_after > handles_before,
        "direct weak wrappers should grow used_global_handles_size above initial main-world baseline (before={handles_before}, after={handles_after})"
    );

    let mut triggered_moderate = false;
    for _ in 0..4 {
        if vm
            .notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("first-tick direct weak maintenance should succeed")
            == RendererDocumentIsolateMemoryPressureAction::Moderate
        {
            triggered_moderate = true;
            break;
        }
    }
    assert!(
        triggered_moderate,
        "direct v8::Weak wrappers created before the first maintenance tick must trigger bounded reclamation (before={handles_before}, after={handles_after})"
    );

    assert!(drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await);

    for idle_tick in 0..6 {
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("post-direct-weak idle maintenance should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "idle page after reclaiming direct weak wrappers must stay quiet on tick {idle_tick}"
        );
    }
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_reclaims_blob_released_by_finalization_registry_foreground_task()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://finalization-registry-release.test/",
        &loader,
    );
    let owner_id = vm.resource_owner_id;

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    assert_eq!(crate::blob::live_blob_count_for_owner_for_test(owner_id), 0);
    vm.eval(
        r#"
(() => {
  const seed = "f".repeat(1024 * 1024);
  window.__retainedBlobs = [];
  for (let i = 0; i < 10; i++) {
    window.__retainedBlobs.push(new Blob([seed]));
  }
  window.__fr = new FinalizationRegistry(() => {
    window.__retainedBlobs = null;
  });
  (() => {
    let target = { tag: "release-trigger" };
    window.__fr.register(target, "drop-blobs");
    target = null;
  })();
  return String(window.__retainedBlobs.length);
})()
"#,
    )
    .expect("retained Blobs and FinalizationRegistry setup should evaluate");

    let live_blobs_after_alloc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert_eq!(live_blobs_after_alloc, 10);

    // First maintenance pass observes >8 MiB newly allocated native bytes and
    // triggers async Moderate GC while the Blobs are still strongly retained.
    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial post-alloc maintenance should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );

    // Draining V8 foreground tasks finishes the Moderate GC (collecting the
    // transient target) and then runs V8's FinalizationRegistry cleanup
    // foreground task, releasing `window.__retainedBlobs = null` via author JS
    // without allocating anything new.
    assert!(
        drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await,
        "moderate pass must run V8 GC and FinalizationRegistry cleanup foreground tasks"
    );

    let live_blobs_after_cleanup = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert_eq!(
        live_blobs_after_cleanup, live_blobs_after_alloc,
        "Blobs must still be alive right after FinalizationRegistry released them, awaiting the next GC"
    );

    let mut triggered_moderate = false;
    for _ in 0..4 {
        if vm
            .notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-FinalizationRegistry maintenance should succeed")
            == RendererDocumentIsolateMemoryPressureAction::Moderate
        {
            triggered_moderate = true;
            break;
        }
    }
    assert!(
        triggered_moderate,
        "FinalizationRegistry cleanup JS releasing retained Blobs must arm the bounded fallback"
    );

    drain_v8_foreground_gc_tasks_until(&mut vm, &loader, || {
        crate::blob::live_blob_count_for_owner_for_test(owner_id) < live_blobs_after_alloc
    })
    .await;

    let live_blobs_after_reclaim = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_reclaim < live_blobs_after_alloc,
        "bounded fallback after FinalizationRegistry cleanup must reclaim released Blobs (before={live_blobs_after_alloc}, after={live_blobs_after_reclaim})"
    );
    assert_eq!(
        vm.eval("String(window.__retainedBlobs)")
            .expect("probe after reclaim should evaluate"),
        "null"
    );

    // Non-vacuous idle negative control: retain a live Blob so
    // `has_live_native_wrappers_above_baseline()` stays true, sync baselines,
    // and verify repeated empty microtask checkpoints never bump
    // `activity_generation` or re-enter 60s Moderate GC churn.
    vm.eval(
        r#"
(() => {
  window.__idleRetainedBlob = new Blob(["r".repeat(64 * 1024)]);
  return "retained";
})()
"#,
    )
    .expect("idle retained Blob setup should evaluate");
    let _ = drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await;
    vm.collect_renderer_document_isolate_garbage()
        .expect("post-test baseline sync should succeed");
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        1,
        "retained Blob must remain live so the empty-checkpoint negative control is non-vacuous"
    );
    for idle_tick in 0..6 {
        vm.perform_owner_lane_task_microtask_checkpoints()
            .expect("empty microtask checkpoint should succeed");
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("post-reclaim idle maintenance should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "idle page holding a retained Blob with empty checkpoints must remain quiet on tick {idle_tick}"
        );
    }
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        1,
        "retained Blob must remain live across all idle ticks"
    );
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_reclaims_blob_released_by_wasm_promise_reaction_checkpoint()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://wasm-promise-reaction-release.test/",
        &loader,
    );
    let owner_id = vm.resource_owner_id;

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    assert_eq!(crate::blob::live_blob_count_for_owner_for_test(owner_id), 0);
    vm.eval(
        r#"
(() => {
  const seed = "w".repeat(1024 * 1024);
  window.__retainedBlobs = [];
  for (let i = 0; i < 10; i++) {
    window.__retainedBlobs.push(new Blob([seed]));
  }
  WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])).then(() => {
    window.__retainedBlobs = null;
  });
  return String(window.__retainedBlobs.length);
})()
"#,
    )
    .expect("retained Blobs and async WebAssembly.compile setup should evaluate");

    let live_blobs_after_alloc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert_eq!(live_blobs_after_alloc, 10);

    // Wait for V8's background Wasm compilation to enqueue its foreground
    // continuation without executing it yet.
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            vm.wait_for_v8_foreground_task_arrival(),
        )
        .await
        .unwrap_or(false),
        "async WebAssembly.compile must post a V8 foreground continuation"
    );

    // Complete a collection while Blobs are still retained, resetting all
    // since-maintenance counters and the activity generation baseline.
    vm.collect_renderer_document_isolate_garbage()
        .expect("preceding GC while Blobs are retained should succeed");
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        live_blobs_after_alloc,
        "retained Blobs must survive the preceding collection"
    );

    // Drain the queued V8 foreground continuation and its task-end microtask
    // checkpoint: the Wasm Promise resolves and its `.then(...)` reaction clears
    // `window.__retainedBlobs = null` without any further host eval.
    assert!(
        drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await,
        "Wasm foreground continuation must execute"
    );
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        live_blobs_after_alloc,
        "Blobs must remain pending reclamation until a follow-up GC runs"
    );

    let mut triggered_moderate = false;
    for _ in 0..4 {
        if vm
            .notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-Wasm-reaction maintenance should succeed")
            == RendererDocumentIsolateMemoryPressureAction::Moderate
        {
            triggered_moderate = true;
            break;
        }
    }
    assert!(
        triggered_moderate,
        "Promise reaction following async Wasm foreground continuation must arm the bounded fallback"
    );

    drain_v8_foreground_gc_tasks_until(&mut vm, &loader, || {
        crate::blob::live_blob_count_for_owner_for_test(owner_id) < live_blobs_after_alloc
    })
    .await;

    let live_blobs_after_reclaim = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_reclaim < live_blobs_after_alloc,
        "bounded fallback after async Wasm Promise reaction must reclaim released Blobs (before={live_blobs_after_alloc}, after={live_blobs_after_reclaim})"
    );
    assert_eq!(
        vm.eval("String(window.__retainedBlobs)")
            .expect("probe after Wasm reclaim should evaluate"),
        "null"
    );
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_reclaims_blob_released_by_atomics_wait_async_checkpoint()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://atomics-wait-async-release.test/",
        &loader,
    );
    let owner_id = vm.resource_owner_id;

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    vm.eval(
        r#"
(() => {
  const seed = "a".repeat(1024 * 1024);
  window.__retainedBlobs = [];
  for (let i = 0; i < 10; i++) {
    window.__retainedBlobs.push(new Blob([seed]));
  }
  const view = new Int32Array(new SharedArrayBuffer(4));
  Atomics.waitAsync(view, 0, 0, 5).value.then(() => {
    window.__retainedBlobs = null;
  });
  return String(window.__retainedBlobs.length);
})()
"#,
    )
    .expect("retained Blobs and Atomics.waitAsync setup should evaluate");

    let live_blobs_after_alloc = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert_eq!(live_blobs_after_alloc, 10);

    // Wait for V8's delayed AsyncWaiterTimeoutTask to arrive without running it yet.
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            vm.wait_for_v8_foreground_task_arrival(),
        )
        .await
        .unwrap_or(false),
        "Atomics.waitAsync timeout must post a V8 foreground continuation"
    );

    // Complete a collection while Blobs are still retained, resetting all
    // since-maintenance counters and the activity generation baseline.
    vm.collect_renderer_document_isolate_garbage()
        .expect("preceding GC while Blobs are retained should succeed");
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        live_blobs_after_alloc
    );

    // Drain the queued Atomics.waitAsync timeout continuation and its task-end
    // microtask checkpoint; the `.then(...)` reaction clears `__retainedBlobs`.
    assert!(
        drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await,
        "Atomics.waitAsync timeout foreground continuation must execute"
    );
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        live_blobs_after_alloc
    );

    let mut triggered_moderate = false;
    for _ in 0..4 {
        if vm
            .notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-Atomics.waitAsync maintenance should succeed")
            == RendererDocumentIsolateMemoryPressureAction::Moderate
        {
            triggered_moderate = true;
            break;
        }
    }
    assert!(
        triggered_moderate,
        "Promise reaction following Atomics.waitAsync continuation must arm the bounded fallback"
    );

    drain_v8_foreground_gc_tasks_until(&mut vm, &loader, || {
        crate::blob::live_blob_count_for_owner_for_test(owner_id) < live_blobs_after_alloc
    })
    .await;

    let live_blobs_after_reclaim = crate::blob::live_blob_count_for_owner_for_test(owner_id);
    assert!(
        live_blobs_after_reclaim < live_blobs_after_alloc,
        "bounded fallback after Atomics.waitAsync reaction must reclaim released Blobs (before={live_blobs_after_alloc}, after={live_blobs_after_reclaim})"
    );
    assert_eq!(
        vm.eval("String(window.__retainedBlobs)")
            .expect("probe after Atomics.waitAsync reclaim should evaluate"),
        "null"
    );

    // Non-vacuous idle negative control: even with the lazy promise activity
    // hook installed on this isolate AND a live retained Blob keeping
    // `has_live_native_wrappers_above_baseline()` true, repeated empty
    // microtask checkpoints must not bump `activity_generation` or re-enter
    // 60s Moderate GC churn.
    vm.eval(
        r#"
(() => {
  window.__idleRetainedBlob = new Blob(["r".repeat(64 * 1024)]);
  return "retained";
})()
"#,
    )
    .expect("idle retained Blob setup should evaluate");
    let _ = drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await;
    vm.collect_renderer_document_isolate_garbage()
        .expect("post-test baseline sync should succeed");
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        1,
        "retained Blob must remain live so the lazy-promise-hook negative control is non-vacuous"
    );
    for idle_tick in 0..6 {
        vm.perform_owner_lane_task_microtask_checkpoints()
            .expect("empty microtask checkpoint should succeed");
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("post-reclaim idle maintenance should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "idle page holding a retained Blob with lazy promise hook installed must remain quiet on tick {idle_tick}"
        );
    }
    assert_eq!(
        crate::blob::live_blob_count_for_owner_for_test(owner_id),
        1,
        "retained Blob must remain live across all idle ticks"
    );
}

#[tokio::test]
async fn renderer_document_isolate_maintenance_keeps_recalibration_armed_across_skipped_ticks_until_async_gc_finishes()
 {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://async-recalibration-across-skipped-ticks.test/",
        &loader,
    );

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("initial idle maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::None
    );

    // Allocate >12 MiB of transient JS heap garbage.
    vm.eval(
        r#"
(() => {
  let tmp = [];
  for (let i = 0; i < 240000; i++) {
    tmp.push({ id: i, payload: "delayed-recal-" + i + "-" + "x".repeat(32) });
  }
  return String(tmp.length);
})()
"#,
    )
    .expect("transient heap allocation should evaluate");

    let pre_gc_peak = vm
        .renderer_document_isolate_heap_usage()
        .expect("pre-gc heap usage")
        .used_size;

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("first moderate pass should trigger"),
        RendererDocumentIsolateMemoryPressureAction::Moderate
    );

    // Simulate one or more maintenance ticks being skipped BEFORE V8's async
    // Moderate GC foreground tasks have finished.
    for skipped_tick in 0..2 {
        assert_eq!(
            vm.notify_renderer_document_isolate_memory_pressure_with_action()
                .expect("pre-completion skipped tick should succeed"),
            RendererDocumentIsolateMemoryPressureAction::None,
            "tick {skipped_tick} before async GC completes should be skipped"
        );
    }

    // Now let the pending async Moderate GC finish and drop heap usage into the
    // post-GC valley.
    assert!(
        drain_v8_foreground_gc_tasks_to_quiescence(&mut vm, &loader).await,
        "async moderate pass must still run V8 GC foreground tasks after skipped ticks"
    );

    let post_gc_valley = vm
        .renderer_document_isolate_heap_usage()
        .expect("post-gc heap usage")
        .used_size;
    assert!(
        post_gc_valley + 8 * 1024 * 1024 < pre_gc_peak,
        "expected async moderate GC to reclaim transient heap (valley={post_gc_valley}, peak={pre_gc_peak})"
    );

    // Allocate ~9 MiB of external memory: total tracked memory stays below or
    // near the pre-GC peak, yet exceeds the post-GC valley by >8 MiB.
    vm.eval(
        r#"
(() => {
  window.__postSkippedValleyBuffers = [];
  for (let i = 0; i < 9; i++) {
    const view = new Uint8Array(1024 * 1024);
    view[0] = i;
    window.__postSkippedValleyBuffers.push(view);
  }
  return String(window.__postSkippedValleyBuffers.length);
})()
"#,
    )
    .expect("post-valley external allocation should evaluate");

    assert_eq!(
        vm.notify_renderer_document_isolate_memory_pressure_with_action()
            .expect("post-delayed-recalibration maintenance check should succeed"),
        RendererDocumentIsolateMemoryPressureAction::Moderate,
        "recalibration must remain armed across skipped ticks until async Moderate GC finishes"
    );
}
#[tokio::test]
async fn child_javascript_url_nested_frame_uses_inherited_frame_src_policy() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-javascript-url-frame-csp.test/",
        &loader,
    );
    vm.document_runtime
        .set_response_content_security_policies(&["frame-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__childJavascriptUrlFrameCspEvents = [];
  addEventListener("message", event => {
    __childJavascriptUrlFrameCspEvents.push(String(event.data));
  });
  const source = `javascript:
    addEventListener("securitypolicyviolation", event => {
      parent.postMessage(event.violatedDirective, "*");
    });
    const nested = document.createElement("iframe");
    nested.src = "https://blocked-frame.test/fail.html";
    document.body.appendChild(nested);
  `;
  const frame = document.createElement("iframe");
  frame.src = encodeURI(source.replaceAll("\n", ""));
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
})()
"#,
        )
        .expect("child javascript URL CSP setup should evaluate"),
        "queued"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("javascript URL navigation commit should use the selected-task dispatcher")
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL should execute after its initial child realm materializes",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("nested frame navigation should reach the inherited CSP gate")
    );
    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("child CSP report should dispatch to the top Window")
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__childJavascriptUrlFrameCspEvents)")
            .expect("child javascript URL CSP events should be observable"),
        r#"["frame-src"]"#
    );
}
#[tokio::test]
async fn child_nested_external_document_write_preserves_input_order() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm =
        new_storage_test_vm_with_loader("https://child-nested-external-write.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.open();

  const nestedSource =
    '<script src="data:text/javascript,document.write(%22w%22)%3Bdocument.write(%22o%22)%3B"><\/script>r';
  const nestedLiteral = JSON.stringify(nestedSource).replace("</script>", "<\\/script>");
  childDocument.write(
    "<script>document.write(" + nestedLiteral + "); document.write('k');<\/script>e"
  );
  childDocument.write("d");
  childDocument.close();
})()
"#,
    )
    .expect("nested external child document.write fixture should evaluate");

    while vm
        .run_next_child_frame_semantic_turn_for_test()
        .await
        .is_some()
    {
        // Drain the data-script source load, nested writes, and parser resume.
    }

    let result = vm
        .eval(
            r#"
document.querySelector("iframe").contentDocument.body.textContent
"#,
        )
        .expect("the nested external writer must restore all parent parser input");

    assert_eq!(result, "worked");
}
#[tokio::test]
async fn child_pending_external_classic_blocks_later_inline_and_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm =
        new_storage_test_vm_with_loader("https://child-external-classic-block.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childClassicBlockEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicBlockEvents.push("load");
  frame.srcdoc = `
    <script src="https://child-external-classic-block.test/blocking.js"><\/script>
    <script>parent.__childClassicBlockEvents.push("after-inline");<\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child external classic block setup should evaluate");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "srcdoc bootstrap should commit from the explicit NavigationCommit source turn"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "no child document-script ready work should run while the external classic source is pending"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad
        )
        .await,
        "the exact child authority should start the pending classic-script request"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad should not dispatch while external classic source is pending"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "blocked documents must not produce HostLoad delivery work"
    );

    assert_eq!(
        vm.eval("__childClassicBlockEvents.join('|')")
            .expect("child external classic block events should evaluate"),
        "",
        "pending external classic script must block later parser-connected inline script and load"
    );
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::RealmMaterialization),
        "the production-shaped child must consume its realm prerequisite"
    );
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        None,
        "owner pump should observe no HostLoad wake while lifecycle readiness is blocked"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "blocked lifecycle should continue to expose no HostLoad delivery action"
    );
    assert_eq!(
        vm.take_pending_child_frame_tree_events().len(),
        1,
        "the actual attachment projection should be drained before testing scheduler/output isolation"
    );
}
#[tokio::test]
async fn child_detach_retires_document_modulator_at_owner_boundary() {
    let mut vm = new_storage_test_vm("https://child-detach-owner-transition.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "detach-owner-transition";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child detach transition setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child detach transition setup should install initial about:blank",
    )
    .await;
    let context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child detach transition initial document",
    );
    let (child_handle, realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let task_owner = vm
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
        .expect("child document should expose an owner");
    let _ = vm
        .child_document_modulator_store
        .take_or_create_document_modulator(task_owner.document_owner(), realm_id);

    vm.eval("document.getElementById('detach-owner-transition').remove()")
        .expect("child detach should evaluate");
    assert!(
        !vm.child_document_modulator_store
            .contains_execution_context(task_owner.document_owner()),
        "detach transition must retire ScriptVm modulator state at the enclosing owner boundary"
    );
}
#[cfg(feature = "devtools")]
#[tokio::test(flavor = "current_thread")]
async fn runtime_protocol_retains_connected_style_event_bodies_without_numeric_termination() {
    const STYLE_COUNT: usize = 129;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader(
        "https://runtime-connected-style-owner-turn.test/",
        &loader,
    );
    let expression = format!(
        r#"
globalThis.__runtimeConnectedStyleLoadCount = 0;
const root = document.documentElement || document.appendChild(document.createElement("html"));
const head = document.head || root.appendChild(document.createElement("head"));
for (let index = 0; index < {STYLE_COUNT}; index += 1) {{
  const style = document.createElement("style");
  style.textContent = `:root{{--runtime-connected-style-${{index}}:${{index}}}}`;
  style.addEventListener("load", () => {{
    globalThis.__runtimeConnectedStyleLoadCount += 1;
  }});
  head.appendChild(style);
}}
globalThis.__runtimeConnectedStyleLoadCount;
"#
    );
    let message = serde_json::json!({
        "id": 17,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "awaitPromise": false,
            "returnByValue": true,
        }
    });

    let messages = vm
        .dispatch_inspector_protocol_message(&message.to_string())
        .expect("Runtime.evaluate should queue connected stylesheet events");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(17))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!(0),
        "the protocol execution turn must not inline-dispatch connected stylesheet events"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "the typed source should expose the first connected stylesheet event body"
    );
    assert_eq!(
        vm.eval("String(globalThis.__runtimeConnectedStyleLoadCount)")
            .expect("first connected stylesheet event count should evaluate"),
        "1",
        "one body application should dispatch exactly one connected stylesheet event"
    );

    let mut dispatched = 1;
    while vm.apply_next_connected_style_event_body_for_test() {
        dispatched += 1;
    }
    assert_eq!(
        dispatched, STYLE_COUNT,
        "the typed source must retain bodies beyond the removed 128-item protocol drain"
    );
    assert_eq!(
        vm.eval("String(globalThis.__runtimeConnectedStyleLoadCount)")
            .expect("final connected stylesheet event count should evaluate"),
        STYLE_COUNT.to_string()
    );
}
#[test]
fn cached_connected_modulepreload_never_acquires_a_load_delay() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        concat!(
            "<!doctype html><html><head>",
            "<link rel='modulepreload' href='/slow-module.mjs'>",
            "</head><body></body></html>",
        ),
    );
    let owner = vm
        .current_main_document_task_owner()
        .expect("modulepreload fixture must retain a current Document owner");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.replace_document_resource_runtime(&loader);
    vm.document_runtime.insert_native_module_source(
        crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://example.test/slow-module.mjs").expect("module URL"),
        ),
        crate::module_runtime::ModuleSource::text("export {};".to_owned()),
    );
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false)
    );

    vm.queue_initial_connected_style_loads_for_current_owner();

    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "modulepreload network admission must retain only owner identity, not a load-delay token"
    );

    vm.prime_document_lifecycle_and_publish_stylesheets();
    let ready = vm
        .take_next_connected_style_event_body_for_test()
        .expect("cached modulepreload terminal should publish its link event")
        .into_ready();
    assert_eq!(
        ready.load_event_binding(),
        None,
        "a ready modulepreload terminal must not acquire a short-lived stylesheet lease"
    );
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "modulepreload terminal publication must leave the Document load gate untouched"
    );
}
#[tokio::test]
async fn in_flight_connected_modulepreload_does_not_delay_window_load() {
    let (module_url, request_rx, release_tx, server) = spawn_gated_module_resource_server().await;
    let document_url = module_url.replace("/slow-module.mjs", "/page.html");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let markup = format!(
        concat!(
            "<!doctype html><html><head>",
            "<link id='preload' rel='modulepreload' href='{module_url}'>",
            "</head><body></body></html>",
        ),
        module_url = module_url,
    );
    let mut vm = new_parsed_page_task_executor_test_vm(&document_url, &markup, &loader);
    vm.exec(
        r#"
        globalThis.__modulepreloadLifecycleEvents = [];
        document.getElementById("preload").addEventListener("load", () => {
          __modulepreloadLifecycleEvents.push(`link:${document.readyState}`);
        });
        window.addEventListener("load", () => {
          __modulepreloadLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("modulepreload lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("modulepreload fixture must retain a current Document owner");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_and_publish_stylesheets();
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .expect("modulepreload request should reach the server")
        .expect("modulepreload request channel should remain open");
    assert!(request.starts_with("GET /slow-module.mjs HTTP/1.1"));
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "an in-flight modulepreload request must not hold the Document load gate"
    );

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
        Some(true),
        "window load must become ready while modulepreload remains in flight"
    );
    assert!(
        vm.dispatch_main_document_window_load_lifecycle(owner)
            .expect("main Window load lifecycle should apply")
            .is_none()
    );
    assert_eq!(
        vm.eval("__modulepreloadLifecycleEvents.join('|')")
            .expect("pre-terminal lifecycle event trace"),
        "window:complete",
        "window load must run before the pending modulepreload terminal"
    );

    release_tx.send(()).expect("release modulepreload response");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "modulepreload network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__modulepreloadLifecycleEvents.join('|')")
            .expect("network-terminal lifecycle event trace"),
        "window:complete",
        "the network terminal may only queue the later link event"
    );
    assert!(
        vm.has_ready_native_module_owner_actions(),
        "the module-map terminal must publish its joined link-client notification"
    );
    assert!(
        vm.run_one_native_module_owner_event_task_executor_turn(&loader)
            .await
            .expect("modulepreload owner-notification turn"),
        "the joined link client must be notified in a later selected task"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ConnectedStyleEvent,
            &loader,
        )
        .await
        .expect("modulepreload link-event turn")
    );
    assert_eq!(
        vm.eval("__modulepreloadLifecycleEvents.join('|')")
            .expect("complete modulepreload lifecycle event trace"),
        "window:complete|link:complete"
    );
    server.await.expect("modulepreload server should finish");
}
#[tokio::test(flavor = "current_thread")]
async fn connected_modulepreload_invalid_as_dispatches_link_error_event() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );
    vm.exec(
        r#"
        globalThis.__connectedModulepreloadInvalidAsEvents = [];
        const link = document.createElement("link");
        link.rel = "modulepreload";
        link.href = "/bad.bin";
        link.setAttribute("as", "image");
        link.addEventListener("load", () => {
          globalThis.__connectedModulepreloadInvalidAsEvents.push("load");
        });
        link.addEventListener("error", () => {
          globalThis.__connectedModulepreloadInvalidAsEvents.push("error");
        });
        document.head.appendChild(link);
        "#,
        None,
    )
    .expect("runtime-inserted invalid modulepreload should append");
    let owner = vm
        .current_main_document_task_owner()
        .expect("invalid modulepreload fixture must retain a current Document owner");

    vm.prime_document_lifecycle_and_publish_stylesheets();
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "invalid modulepreload admission must retain only event identity before its error task"
    );

    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "connected invalid modulepreload should dispatch through its document-owned link lane"
    );
    assert_eq!(
        vm.runtime_observable_lifecycle_errors_for_testing(),
        vec!["<link rel=modulepreload> has an invalid `as` value image".to_owned()]
    );
    assert!(
        !vm.has_ready_native_module_owner_actions(),
        "main connected link errors must not be duplicated through the child/module owner lane"
    );

    assert_eq!(
        vm.eval("globalThis.__connectedModulepreloadInvalidAsEvents.join(',')")
            .expect("events should be readable"),
        "error"
    );
}
