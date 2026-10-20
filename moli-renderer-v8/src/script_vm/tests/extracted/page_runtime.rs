use super::*;

#[tokio::test(flavor = "current_thread")]
async fn detached_keepalive_redirect_reports_source_document_csp_without_v8() {
    let mut vm = new_storage_test_vm("https://detached-fetch-csp-owner.test/");
    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
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
        response_content_security_report_only_policies: vec![format!(
            "connect-src 'none'; report-uri {report_url}"
        )],
        ..Default::default()
    };
    let enforce_policy = crate::document_runtime::DocumentPolicyContainer {
        response_content_security_policies: vec![format!(
            "connect-src 'none'; report-uri {report_url}"
        )],
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
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
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

    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
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
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
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

    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
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
