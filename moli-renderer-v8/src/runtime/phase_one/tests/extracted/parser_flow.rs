use super::*;

#[test]
fn main_document_parser_derives_scripting_state_from_runtime_and_response_sandbox() {
    const MARKUP: &str = "<!doctype html><noscript><main id='fallback'></main></noscript>";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
            for (script_execution_disabled, policy, fallback_expected) in [
                (false, None, false),
                (true, None, true),
                (false, Some("script-src 'none'"), false),
                (false, Some("sandbox"), true),
                (false, Some("sandbox allow-scripts"), false),
            ] {
                let env = default_test_page_vm_env_config_with(|env| {
                    env.script_execution_disabled = script_execution_disabled;
                    env.document_policy_container
                        .response_content_security_policies =
                        policy.into_iter().map(str::to_owned).collect();
                });
                let page_vm =
                    parse_phase_one_html_into_page_vm_for_test_with_env(MARKUP, env).await;
                let fallback_present = page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .element_handle_by_id("fallback")
                    .is_some();
                assert_eq!(
                    fallback_present, fallback_expected,
                    "unexpected noscript parse for disabled={script_execution_disabled}, policy={policy:?}"
                );
            }

            for (policy, script_expected) in
                [("sandbox", false), ("sandbox allow-scripts", true)]
            {
                let env = default_test_page_vm_env_config_with(|env| {
                    env.document_policy_container
                        .response_content_security_policies = vec![policy.to_owned()];
                });
                let page_vm = parse_phase_one_html_into_page_vm_for_test_with_env(
                    "<!doctype html><script>document.documentElement.setAttribute('data-script-ran', 'yes')</script>",
                    env,
                )
                .await;
                let dom_host = page_vm.vm().document_runtime.dom_host();
                let document_element = dom_host
                    .document_element_handle()
                    .expect("parser should create the document element");
                assert_eq!(
                    dom_host.get_attribute(document_element, "data-script-ran").is_some(),
                    script_expected,
                    "unexpected parser script execution for policy={policy:?}"
                );
            }
        }));
}
#[test]
fn parser_body_onerror_attribute_replaces_prior_window_handler_before_compilation() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><script>
globalThis.priorBodyErrorHandlerRan = false;
window.onerror = () => {
  globalThis.priorBodyErrorHandlerRan = true;
  return true;
};
</script></head><body onerror="{">
<script>for(;) {}</script>
<script>
document.body.setAttribute('data-error-state', [
  globalThis.priorBodyErrorHandlerRan,
  window.onerror === null
].join('|'));
</script>
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-error-state"),
                Some("false|true"),
                "parser body handler registration must clear the prior Window handler before compiling invalid source"
            );
        }));
}
#[test]
fn parser_clients_claim_pre_meta_pending_descriptors_before_gate_drain() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut cache = BufferedDocumentPreloadState::default();
    cache.append_to_main_document_scan(
        &final_url,
        r#"
                <script src="/before.js"></script>
                <link rel="stylesheet" href="/before.css">
                <meta http-equiv="content-security-policy" content="default-src 'self'">
                <script src="/after.js"></script>
                <link rel="stylesheet" href="/after.css">
            "#,
        &loader,
    );
    assert_eq!(cache.pending_preload_counts_for_test(), (2, 2));

    cache.claim_pending_script_preload_for_parser(&prepared_external_classic(
        "https://example.test/before.js",
    ));
    let stylesheet_candidate =
        moli_stylesheet_blocking::DocumentOwnedBlockingStylesheetCandidate::Link {
            node_id: NodeId::new(11),
            url: Url::parse("https://example.test/before.css").expect("before stylesheet"),
            options: crate::stylesheet_blocking::StylesheetFetchOptions::default(),
        };
    cache.claim_pending_stylesheet_preloads_for_parser(&[
        DocumentOwnedBlockingStylesheetDiscoveryInput::from(&stylesheet_candidate),
    ]);
    assert_eq!(cache.pending_preload_counts_for_test(), (1, 1));

    cache.note_parser_processed_meta_csp(1);
    assert_eq!(
        preload_request_urls(cache.take_pending_script_preloads_for_test()),
        vec![Url::parse("https://example.test/after.js").expect("after script")]
    );
    assert_eq!(
        cache
            .take_pending_stylesheet_preloads()
            .into_iter()
            .map(|request| request.url)
            .collect::<Vec<_>>(),
        vec![Url::parse("https://example.test/after.css").expect("after stylesheet")]
    );
}
#[test]
fn parse_time_driver_state_can_select_live_stream_backend_for_testing() {
    let final_url = Url::parse("https://example.test/").expect("test url");
    let state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);

    assert!(
        state
            .parser_session
            .stream_handle()
            .borrow()
            .is_parser_stream_backend_for_testing()
    );
}
#[test]
fn parser_step_without_script_handoff_consumes_live_backend_dom() {
    let final_url = Url::parse("https://example.test/").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
    let driver = ParserDriver {
        loader: &loader,
        final_url: &state.final_url,
        parser_session: &mut state.parser_session,
        scheduler: &mut state.scheduler,
        buffered_document_preloads: &mut state.buffered_document_preloads,
        service_worker_preload_context: state.service_worker_preload_context.as_ref(),
        input_closed: &state.input_closed,
    };

    let crate::parser::ParserPumpOutcome {
        result,
        discovered_async_prefetch_scripts: _,
        discovered_preload_link_candidates: _,
        discovered_blocking_stylesheet_inputs: _,
    } = driver
        .parser_session
        .stream_handle()
        .borrow_mut()
        .pump_parser_step("<!doctype html><html><body><main>live parser step</main></body></html>");
    let parser_stream_snapshot = state
        .parser_session
        .stream_handle()
        .borrow()
        .snapshot_parser_stream_document();
    assert!(
        matches!(result, ParserPumpStep::InputDrained),
        "expected non-script html to drain without a parser handoff"
    );

    assert!(
        parser_stream_snapshot.parse_errors().is_empty(),
        "plain html parser step should not record parse errors"
    );
    let body = parser_stream_snapshot.document_body_handle().expect("body");
    assert_eq!(
        parser_stream_snapshot.text_content(body).as_deref(),
        Some("live parser step")
    );
    assert!(
        state
            .parser_session
            .stream_handle()
            .borrow()
            .is_parser_stream_backend_for_testing()
    );
}
#[test]
fn parser_step_with_inline_svg_script_surfaces_shared_script_handoff() {
    let final_url = Url::parse("https://example.test/").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
    let driver = ParserDriver {
        loader: &loader,
        final_url: &state.final_url,
        parser_session: &mut state.parser_session,
        scheduler: &mut state.scheduler,
        buffered_document_preloads: &mut state.buffered_document_preloads,
        service_worker_preload_context: state.service_worker_preload_context.as_ref(),
        input_closed: &state.input_closed,
    };

    let crate::parser::ParserPumpOutcome {
        result,
        discovered_async_prefetch_scripts: _,
        discovered_preload_link_candidates: _,
        discovered_blocking_stylesheet_inputs: _,
    } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
        "<!doctype html><html><body><svg><script>window.svgAnswer = 42;</script></svg><div>late</div></body></html>",
    );
    let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
        panic!("expected parser step to stop at inline SVG script handoff");
    };
    let ParserScriptHandoff::BlockingClassic {
        node_id: handle,
        start_line: _,
        start_column: _,
        blocking_signatures_before: _,
        script: _,
    } = *handoff
    else {
        panic!("expected inline SVG script to use the blocking classic handoff");
    };
    let parser_stream_snapshot = state
        .parser_session
        .stream_handle()
        .borrow()
        .snapshot_parser_stream_document();
    let script = parser_stream_snapshot
        .node(handle)
        .and_then(Node::as_element)
        .expect("SVG script element at handoff");

    assert!(script.is_script_element());
    assert_eq!(script.wrapper_prototype_name(), "SVGScriptElement");
    assert!(parser_stream_snapshot.node_is_parser_created(handle));
    assert_eq!(
        parser_stream_snapshot.script_text(handle).as_deref(),
        Some("window.svgAnswer = 42;")
    );
    assert!(
        state
            .parser_session
            .stream_handle()
            .borrow()
            .is_parser_stream_backend_for_testing()
    );
}
#[test]
fn parser_merged_root_attributes_hide_nonce_content_values() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><body>
<body nonce="body-secret">
<html nonce="html-secret">
</body></html>"#,
        )
        .await;

        let snapshot = page_vm.vm().snapshot_live_document();
        let html = snapshot
            .document_element_handle()
            .expect("document element");
        let body = snapshot.document_body_handle().expect("body");
        let html = snapshot
            .node(html)
            .and_then(Node::as_element)
            .expect("html element");
        let body = snapshot
            .node(body)
            .and_then(Node::as_element)
            .expect("body element");

        assert_eq!(html.attribute("nonce"), Some(""));
        assert_eq!(html.cryptographic_nonce(), Some("html-secret"));
        assert_eq!(body.attribute("nonce"), Some(""));
        assert_eq!(body.cryptographic_nonce(), Some("body-secret"));
    }));
}
#[test]
fn parser_defined_autonomous_custom_element_reaches_parser_handoff() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader: &'static ResourceRequestClient =
            Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
        let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone())));
        let _js_runtime = crate::JsRuntime::initialize();
        let mut driver = ParserDriver {
            loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };

        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_preload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
            r#"<!doctype html><script>customElements.define("x-sync", class extends HTMLElement {});</script>"#,
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at inline customElements.define handoff");
        };

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let mut page_vm = PageVm::new(
            PageId::new_for_testing(103),
            local_executor.clone(),
            loader,
            &default_test_page_vm_env_config(),
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
        let outcome = super::access::run_named_owner_local_task(
            local_executor,
            "phase-one parser custom element definition handoff channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver
                    .handle_parse_time_script_handoff(page_vm, *handoff, None)
                    .await
            },
        )
        .await
        .expect("customElements.define handoff should complete");
        assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

        let parser_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("custom-element parser test requires a main document owner");
        let result = driver.pump_parse_step_with_signals(
            &mut page_vm,
            parser_document_owner,
            "<body><x-sync id='candidate' data-probe='yes'></x-sync></body>",
        );
        let crate::live_document_parser::LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(
            handoff,
        ) = result
        else {
            panic!("expected parser-created custom element handoff after define()");
        };
        let handoff = &*handoff;
        assert_eq!(handoff.local_name, "x-sync");
        assert_eq!(handoff.namespace, "http://www.w3.org/1999/xhtml");
        assert_eq!(handoff.prefix, None);
        assert_eq!(handoff.parent_at_creation, None);
        assert_eq!(handoff.owner_document.index(), 0);
        assert!(
            handoff
                .attributes
                .iter()
                .any(|attribute| attribute.name() == "id" && attribute.value() == "candidate")
        );
        assert!(handoff.attributes.iter().any(|attribute| {
            attribute.name() == "data-probe" && attribute.value() == "yes"
        }));
    }));
}
#[test]
fn move_before_does_not_prepare_empty_parser_scripts_from_moved_text() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><body>
<script id="html-move"></script>
<svg><script id="svg-move"></script></svg>
<script id="html-append"></script>
<svg><script id="svg-append"></script></svg>
<script>
globalThis.__htmlMoveRan = false;
globalThis.__svgMoveRan = false;
globalThis.__htmlAppendRan = false;
globalThis.__svgAppendRan = false;

for (const [id, flag] of [
  ['html-move', '__htmlMoveRan'],
  ['svg-move', '__svgMoveRan'],
]) {
  const text = document.createTextNode(`globalThis.${flag} = true;`);
  document.body.appendChild(text);
  document.getElementById(id).moveBefore(text, null);
}
for (const [id, flag] of [
  ['html-append', '__htmlAppendRan'],
  ['svg-append', '__svgAppendRan'],
]) {
  document.getElementById(id).appendChild(
    document.createTextNode(`globalThis.${flag} = true;`)
  );
}
</script>
</body></html>"#,
        )
        .await;

        let result = page_vm
            .evaluate_expression(
                r#"JSON.stringify({
  moved: [globalThis.__htmlMoveRan, globalThis.__svgMoveRan],
  appended: [globalThis.__htmlAppendRan, globalThis.__svgAppendRan],
})"#,
            )
            .expect("atomic script move result should evaluate");
        assert_eq!(
            result.get("value").and_then(serde_json::Value::as_str),
            Some(r#"{"moved":[false,false],"appended":[true,true]}"#),
            "moveBefore must suppress script preparation without changing ordinary child insertion"
        );
    }));
}
#[test]
fn empty_script_inside_template_does_not_crash_observer_delivery() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
new MutationObserver(() => {}).observe(document.body, { childList: true });
</script>
<div id="host"><template><span>Content</span><script></script></template></div>
<script>
document.body.setAttribute('data-result', 'done');
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one template script parser step local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(
                result,
                Some("done"),
                "template-contained scripts are inert and must not crash observer delivery before the following parser script"
            );
        }));
}
#[test]
fn declarative_shadow_respects_custom_element_disabled_shadow_feature() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
class ShadowDisabledElement extends HTMLElement {
  static get disabledFeatures() { return ['shadow']; }
}
customElements.define('shadow-disabled', ShadowDisabledElement);
</script>
<shadow-disabled><template shadowrootmode="open"><span>Content</span></template></shadow-disabled>
<script>
const element = document.querySelector('shadow-disabled');
document.body.setAttribute('data-result', [
  element instanceof ShadowDisabledElement,
  !!element.querySelector('template'),
  !element.shadowRoot
].join('|'));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one disabled shadow parser step local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(
                result,
                Some("true|true|true"),
                "custom elements with disabledFeatures containing shadow should reject parser-created declarative shadow roots"
            );
        }));
}

#[test]
fn quirks_stylesheet_parser_client_claims_mode_neutral_scanner_descriptor() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut cache = BufferedDocumentPreloadState::default();
    cache.append_to_main_document_scan(
        &final_url,
        r#"<link rel="stylesheet" href="/app.css">"#,
        &loader,
    );
    assert_eq!(cache.pending_preload_counts_for_test(), (0, 1));

    let stylesheet_candidate =
        moli_stylesheet_blocking::DocumentOwnedBlockingStylesheetCandidate::Link {
            node_id: NodeId::new(11),
            url: Url::parse("https://example.test/app.css").expect("stylesheet URL"),
            options: crate::stylesheet_blocking::StylesheetFetchOptions::default()
                .with_quirks_mode_mime_compatibility(true),
        };
    cache.claim_pending_stylesheet_preloads_for_parser(&[
        DocumentOwnedBlockingStylesheetDiscoveryInput::from(&stylesheet_candidate),
    ]);

    assert_eq!(cache.pending_preload_counts_for_test(), (0, 0));
}
