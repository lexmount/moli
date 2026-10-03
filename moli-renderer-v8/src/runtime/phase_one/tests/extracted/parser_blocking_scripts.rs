use super::*;

#[tokio::test]
async fn main_parser_classic_pending_script_cancels_after_document_open() {
    use crate::parser_script::context::ParserClassicScriptDocumentOwnerState;

    let mut harness = new_phase_one_page_vm_harness_for_test();
    let captured_owner = harness
        .page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let mut script =
        prepared_external_classic("https://main-classic-stable-owner.test/parser-blocking.js");
    script.source_kind = crate::types::ScriptSourceKind::Inline;
    script.source = ScriptSource::Inline("window.__stableOwner = true".to_owned());
    let script_handle = script.node_id;
    let mut runner = PendingParsingBlockingClassicScriptRunner::from_parser_blocking_script(
        parser_blocking_pending::main_parser_blocking_classic_script_item(
            captured_owner,
            crate::parser_script::payload::ParserPreparedClassicScript::new(
                crate::parser_script::payload::ParserClassicScriptMetadata::new(script_handle, 1),
                script,
            ),
            HashSet::new(),
            None,
        ),
    );
    assert_eq!(
        runner
            .current_parser_blocking_context()
            .expect("captured parser classic context")
            .parser_classic_document_task_owner(),
        captured_owner
    );

    harness
        .page_vm
        .vm_mut()
        .eval("document.open(); 'replaced'")
        .expect("document.open should rotate the main document owner");
    let replacement_owner = harness
        .page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(replacement_owner, captured_owner);

    assert!(matches!(
        parser_blocking_execution::resolve_main_parser_blocking_classic_after_runtime_gate(
            &mut harness.state.parser_session,
            &mut harness.page_vm,
            &mut runner,
            "stale owner test must not execute",
        )
        .await
        .expect("stale owner should produce a typed parser cancellation outcome"),
        parser_blocking_execution::MainParserBlockingExecutionOutcome::StoppedCurrentDocument
    ));
    assert_eq!(
        runner
            .current_parser_blocking_context()
            .expect("stale PendingScript should retain its captured context")
            .parser_classic_document_task_owner(),
        captured_owner,
        "replacement currentness must not rewrite the PendingScript owner"
    );
}
#[tokio::test]
async fn navigation_terminal_retires_selected_and_late_main_parser_continuations() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let PhaseOnePageVmHarness {
                page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            let replacement_state = ParseTimeDriverState::new_with_scripting_enabled_for_test(
                Url::parse("https://example.test/replacement-state")
                    .expect("replacement state URL"),
            );
            let state = std::mem::replace(state, replacement_state);
            let mut runtime = ConcurrentParseTimeRuntime::new_parser_owner(
                (*loader).clone(),
                PageVmInitStage::Load,
                state,
                page_vm,
            );
            let producer = runtime
                .page_vm
                .vm()
                .document_runtime
                .main_parser_continuation_producer()
                .expect("active phase one should expose its parser continuation producer");
            let request_client = runtime
                .page_vm
                .main_document_resource_loader()
                .request_client()
                .clone();

            assert_eq!(
                producer
                    .request()
                    .expect("first parser continuation request"),
                crate::page_task_queue::MainParserContinuationRequest::Enqueued
            );
            let selected_task_ran = runtime
                .page_vm
                .run_exact_selected_page_task_for_test(
                    crate::runtime::page_vm::PageSelectedTaskTestSelector::MainParserContinuation,
                    &request_client,
                )
                .await
                .expect("selected parser continuation should run");
            assert!(selected_task_ran);
            assert!(
                runtime
                    .page_vm
                    .vm()
                    .document_runtime
                    .has_main_parser_continuation_admission(),
                "the selected task should grant its active phase-one driver one admission"
            );

            runtime
                .page_vm
                .vm_mut()
                .eval("location.href = 'https://example.test/next'")
                .expect("location assignment should evaluate");
            let mut page_vm = runtime.into_navigation_triggered_page_vm();
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .main_parser_continuation_producer()
                    .is_none(),
                "a Page awaiting navigation must not retain an active phase-one producer"
            );
            assert!(
                !page_vm
                    .vm_mut()
                    .document_runtime
                    .take_main_parser_continuation_admission(),
                "the navigation terminal must clear an already-selected admission"
            );

            assert_eq!(
                producer
                    .request()
                    .expect("late parser continuation request"),
                crate::page_task_queue::MainParserContinuationRequest::Enqueued
            );
            let late_task_ran = page_vm
                .run_exact_selected_page_task_for_test(
                    crate::runtime::page_vm::PageSelectedTaskTestSelector::MainParserContinuation,
                    &request_client,
                )
                .await
                .expect("late parser continuation should stale-reject normally");
            assert!(late_task_ran);
            assert!(
                !page_vm
                    .vm_mut()
                    .document_runtime
                    .take_main_parser_continuation_admission(),
                "a late task from the retired producer must not recreate an admission"
            );
        })
        .await;
}
#[test]
fn buffered_module_script_scan_waits_for_native_module_map_admission() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut cache = BufferedDocumentPreloadState::default();

    cache.append_to_main_document_scan(
        &final_url,
        r#"<script type="module" src="/entry.mjs"></script>"#,
        &loader,
    );

    assert!(
        cache.entries.is_empty(),
        "module scripts must not start in the legacy SharedScriptSourceLoad cache"
    );
    assert_eq!(
        preload_request_urls(cache.take_pending_script_preloads_for_test()),
        vec![Url::parse("https://example.test/entry.mjs").expect("module url")],
        "PageVm bootstrap must receive the scanned module and register it in the native module map"
    );
}
#[test]
fn main_parser_finish_waits_for_phase_one_continuation_boundary() {
    run_phase_one_large_stack_test(
        "phase-one-main-parser-finish-after-continuation-boundary",
        main_parser_finish_waits_for_phase_one_continuation_boundary_inner,
    );
}
#[test]
fn main_parser_blocking_completion_settles_script_reactions_before_load() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                ready_preload_entry_for_script(
                    &blocking_script,
                    r#"
window.__mainParserClassicCheckpointEvents = ['script'];
queueMicrotask(() => window.__mainParserClassicCheckpointEvents.push('script-microtask'));
"#,
                ),
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "main parser classic checkpoint ordering test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><body>
<script src="/blocking.js" onload="window.__mainParserClassicCheckpointEvents.push('load'); queueMicrotask(() => window.__mainParserClassicCheckpointEvents.push('load-microtask'))"></script>
<script>window.__mainParserClassicCheckpointEvents.push('later-inline');</script>
</body></html>"#,
                            None,
                        )
                        .await
                },
            )
            .await
            .expect("main parser classic checkpoint ordering test should run");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "ready parser-blocking preload should execute without a source boundary"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicCheckpointEvents.join('|')")
                    .expect("main parser classic checkpoint events should evaluate"),
                "script|script-microtask|load|load-microtask|later-inline",
                "classic-script evaluation reactions must settle before the element load body, whose reactions must settle before parser continuation"
            );
        }));
}
#[test]
fn main_parser_blocking_load_event_document_open_is_ignored_during_parser_execution() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            let initial_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("initial main document owner");
            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                ready_preload_entry_for_script(
                    &blocking_script,
                    "window.__mainParserClassicReplacementEvents = ['external'];",
                ),
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "main parser classic completion replacement test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><body>
<script src="/blocking.js" onload="window.__mainParserClassicReplacementEvents.push('load'); document.open();"></script>
<script>window.__mainParserClassicReplacementEvents.push('later-inline');</script>
</body></html>"#,
                            None,
                        )
                        .await
                },
            )
            .await
            .expect("main parser classic completion replacement test should run");

            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "document.open() from a parser-blocking load event must be ignored while the parser script nesting scope is active"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicReplacementEvents.join('|')")
                    .expect("replacement completion events should evaluate"),
                "external|load|later-inline",
                "the parser must continue after ignoring document.open() from the parser-blocking load event"
            );
            assert_eq!(
                page_vm.vm().current_main_document_task_owner(),
                Some(initial_owner),
                "ignored document.open() must not rotate the main Document owner"
            );
        }));
}
#[test]
fn main_parser_blocking_source_failure_uses_the_shared_completion_event_flow() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader: _,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let script_node = page_vm
                .vm_mut()
                .document_runtime
                .dom_host_mut()
                .create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
            {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                assert!(dom_host.set_attribute(script_node, "src", "/missing.js"));
                assert!(dom_host.set_attribute(
                    script_node,
                    "onerror",
                    "window.__mainParserClassicFailureEvents.push('error:' + (document.currentScript === null)); queueMicrotask(() => window.__mainParserClassicFailureEvents.push('error-microtask'))"
                ));
                assert!(dom_host.append_child(body, script_node));
            }
            let _host_handle = page_vm
                .vm_mut()
                .document_runtime
                .bind_parser_owned_script_handle_for_node(script_node);
            page_vm
                .vm_mut()
                .eval("window.__mainParserClassicFailureEvents = []")
                .expect("source failure event state should initialize");

            let task_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("main document task owner should exist");
            let target = crate::document_script_scheduler::MainDocumentClassicScriptTarget::new(
                task_owner,
                script_node,
            );
            let failure: super::parser_blocking_task::MainParserBlockingClassicScriptSourceFailureAction =
                crate::parser_script::action::ParserClassicScriptSourceFailureAction::new(
                    target,
                    crate::parser_script::payload::ParserClassicScriptSourceFailure {
                        metadata: crate::parser_script::payload::ParserClassicScriptMetadata::new(
                            script_node,
                            1,
                        ),
                        script_url: Url::parse("https://example.test/missing.js")
                            .expect("script URL"),
                        error: "network failure".to_owned(),
                        prepared_script: None,
                        source_network_result: None,
                    },
                    None,
                );
            let mut pending_runner =
                PendingParsingBlockingClassicScriptRunner::new_parser_blocking(Vec::new());
            let parser_bridge =
                crate::document_runtime::ParserConnectedScriptBridge::for_session(
                    &state.parser_session,
                );
            let mut owner = super::parser_blocking_document_script::MainParserBlockingDocumentScriptOwner::new(
                &mut page_vm,
                &mut pending_runner,
                parser_bridge,
                "test source failure",
            );

            let outcome = crate::document_script_scheduler::ParserClassicDocumentScriptExecutionOwner::new(
                &mut owner,
            )
            .run_source_failure(failure)
            .await
            .expect("main parser classic source failure should complete");

            assert_eq!(
                outcome,
                crate::document_script_scheduler::DocumentScriptExecutionOutcome::Progressed,
                "source failure should be consumed by the shared completion owner"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicFailureEvents.join('|')")
                    .expect("source failure events should evaluate"),
                "error:true|error-microtask",
                "source failure must dispatch error with currentScript cleared and settle its reactions before parser continuation"
            );
        }));
}
#[test]
fn parser_blocking_stylesheet_gate_retains_external_source_load() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);

            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    r#"<!doctype html><html><head><link rel="stylesheet" href="/app.css"><script src="/blocking.js"></script><script defer src="/later.js"></script></head></html>"#,
                    None,
                )
                .await
                .expect("parser step should reach the stylesheet gate");

            let ParserStepAdvanceOutcome::BlockedOnStylesheet(pending) = outcome else {
                panic!("stylesheet gate should be checked before parser-blocking source loading");
            };
            let pending_script = pending.script();
            assert_eq!(
                parser_blocking_classic_script_for_test(pending_script)
                    .expect("pending script")
                    .url
                    .as_str(),
                "https://example.test/blocking.js"
            );
            assert_eq!(
                parser_blocking_classic_metadata_for_test(pending_script)
                    .expect("pending metadata")
                    .start_line(),
                1
            );
            assert!(
                matches!(
                    parser_blocking_classic_source_load_for_test(pending_script),
                    Some(PendingParserBlockingSourceLoad::ParserDiscovered(_))
                ),
                "stylesheet-gated parser-blocking scripts must retain their preparation-time source load"
            );
        }));
}
#[test]
fn parser_blocking_script_disabled_does_not_start_parser_discovered_source_load() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let env = default_test_page_vm_env_config_with(|env| {
                env.document_settings.script_execution_disabled = true;
            });
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor.clone(),
                loader,
                &env,
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
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one script-disabled parser-blocking test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        driver.advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><head><script src="/blocking.js"></script></head></html>"#,
                            None,
                        ),
                    )
                    .await
                    .expect("script-disabled parser-blocking handoff must not wait on network")
                },
            )
            .await
            .expect("script-disabled parser-blocking test should run on owner lane");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "disabled script execution must not create a parser-discovered source-load boundary"
            );
            assert!(
                !driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/blocking.js")),
                "the parser-discovered blocking script should not be inserted into the preload cache when script execution is disabled"
            );
        }));
}
#[test]
fn parser_blocking_csp_block_does_not_start_parser_discovered_source_load() {
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
            let env = default_test_page_vm_env_config_with(|env| {
                env.document_policy_container
                    .response_content_security_policies =
                    vec!["script-src 'none'".to_owned()];
            });
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor.clone(),
                loader,
                &env,
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
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one csp-blocked parser-blocking test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        driver.advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><head><script src="/blocking.js"></script></head></html>"#,
                            None,
                        ),
                    )
                    .await
                    .expect("CSP-blocked parser-blocking handoff must not wait on network")
                },
            )
            .await
            .expect("CSP-blocked parser-blocking test should run on owner lane");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "CSP-blocked parser-blocking scripts must not create a source-load boundary"
            );
            assert!(
                !driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/blocking.js")),
                "the CSP-blocked parser-discovered script should not be inserted into the preload cache"
            );
        }));
}
#[test]
fn parser_blocking_strict_dynamic_matching_integrity_can_start_source_load() {
    let mut page_vm = new_phase_one_page_vm_for_test();
    let integrity = "sha256-wIc3KtqOuTFEu6t17sIBuOswgkV406VJvhSk79Gw6U0=";
    page_vm
        .vm_mut()
        .set_response_content_security_policies(&[format!(
            "script-src 'strict-dynamic' '{integrity}'"
        )]);
    let mut script = prepared_external_classic("https://example.test/external-script.js");

    assert!(
        !parser_blocking_script_can_start_external_source_load(&page_vm, &script),
        "strict-dynamic must block a parser-inserted host source without trusted metadata"
    );

    script.fetch_metadata.integrity = Some(integrity.to_owned());
    assert!(
        parser_blocking_script_can_start_external_source_load(&page_vm, &script),
        "a matching integrity hash source must authorize the parser-blocking source load"
    );
}
#[test]
fn parser_owner_boundary_with_live_backend_queues_document_turn_before_runtime_work() {
    let final_url = Url::parse("https://example.test/").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
    let mut owner = ParseTimeOwner::Parser;
    let mut parser_step_ready = false;
    let mut pending_parsing_blocking_wait = PendingParsingBlockingWait::None;
    let _js_runtime = crate::JsRuntime::initialize();

    state
        .parser_session
        .queue_arrived_chunk("<!doctype html><html><body><div>ok</div></body></html>".to_owned());
    state.input_closed = true;

    let mut driver = ParserDriver {
        loader: &loader,
        final_url: &state.final_url,
        parser_session: &mut state.parser_session,
        scheduler: &mut state.scheduler,
        buffered_document_preloads: &mut state.buffered_document_preloads,
        service_worker_preload_context: state.service_worker_preload_context.as_ref(),
        input_closed: &state.input_closed,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let local_executor = JsLocalExecutor::new();
    let mut page_vm = PageVm::new(
        PageId::new_for_testing(1),
        local_executor,
        &loader,
        &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
            root_frame_id: None,
            main_document_commit: None,
            top_level_storage_key: None,
            document_start_scripts: vec![],
            runtime_bindings: vec![],
            runtime_inspector_session_restore_snapshots: vec![],
            runtime_isolated_worlds: vec![],
            permission_overrides: vec![],
            extra_http_headers: Default::default(),
            navigator_identity: Default::default(),
            document_policy_container: Default::default(),
            document_default_language: None,
            document_last_modified: None,
            document_settings: Default::default(),
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            indexed_db_manager: None,
            storage_bucket_store: None,
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            layout_configuration: moli_page_types::LayoutConfiguration {
                policy: moli_page_types::LayoutPolicy::default(),
                scrollbars_hidden: false,
            },
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        },
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
        crate::dom::native::DomHost::from_dom(NativeDom::new(
            Url::parse("https://example.test/").expect("test url"),
        )),
        Instant::now(),
    )
    .expect("page vm");
    let parser_document_owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("parser owner-step test requires a main document owner");
    let progress = runtime
        .block_on(async {
            driver
                .drive_owner_step(
                    &mut owner,
                    &mut parser_step_ready,
                    &mut pending_parsing_blocking_wait,
                    parser_document_owner,
                    &mut page_vm,
                )
                .await
        })
        .expect("initial parser owner step should stop at the document-turn boundary");

    assert_eq!(progress, OwnerStepProgress::Continue);
    assert_eq!(owner, ParseTimeOwner::Document);
    assert!(
        page_vm
            .vm()
            .document_runtime
            .pending_main_parser_script()
            .is_none(),
        "initial parser boundary should not invent a pending parser-blocking script"
    );
    assert!(
        !state.parser_session.is_empty() || state.parser_session.has_script_input(),
        "initial parser boundary should preserve staged parser work"
    );
    assert!(
        !pending_parsing_blocking_wait.is_pending(),
        "parser owner should hand off a before-parser-step document turn, not a blocking wait"
    );
    assert!(
        state
            .parser_session
            .current_chunk_is_non_empty_for_testing(),
        "planning the parser boundary should stage the first parser chunk"
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
fn parser_blocking_script_sees_parser_created_style_sources() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader: &'static ResourceRequestClient = Box::leak(Box::new(
            ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
        ));
        let state = Box::leak(Box::new(
            ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone()),
        ));
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

        let html = r#"<!doctype html><style>div { color: red } .foo { color: lime }</style><body>
<div id="target"></div>
<script>
const style = getComputedStyle(target);
const before = style.color;
target.classList.add('foo');
document.body.setAttribute('data-result', `${before}|${style.color}`);
</script>
</body>"#;
        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_modulepreload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver
            .parser_session
            .stream_handle()
            .borrow_mut()
            .pump_parser_step(html);
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at inline script handoff");
        };

        let parser_dom_host = driver
            .parser_session
            .stream_handle()
            .borrow_mut()
            .take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let mut page_vm = PageVm::new(
            PageId::new_for_testing(101),
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
            "phase-one parser-created style sync script handoff channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver
                    .handle_parse_time_script_handoff(page_vm, *handoff, None)
                    .await
            },
        )
        .await
        .expect("script handoff should complete");
        assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

        let snapshot = page_vm.vm().snapshot_live_document();
        let body = snapshot.document_body_handle().expect("body");
        let result = snapshot
            .node(body)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute("data-result"));
        assert_eq!(result, Some("rgb(255, 0, 0)|rgb(0, 255, 0)"));
    }));
}
#[test]
fn phase_transition_syncs_parser_created_style_sources_for_later_reads() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone());
            let _js_runtime = crate::JsRuntime::initialize();
            let html =
                r#"<!doctype html><style>div { color: red } .foo { color: lime }</style><body><div id="target"></div></body>"#;
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = state.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            assert!(
                matches!(result, ParserPumpStep::InputDrained),
                "non-script parser input should drain"
            );
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let page_vm = PageVm::new(
                PageId::new_for_testing(102),
                local_executor.clone(),
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let runtime = ConcurrentParseTimeRuntime::new_parser_owner(
                loader.clone(),
                crate::renderer::PageVmInitStage::Load,
                state,
                page_vm,
            );
            let (mut page_vm, _, _, _) = super::scaffold::run_phase_one_local_task(
                &local_executor,
                "phase-one parser-created final style sync handoff",
                async move {
                    runtime
                        .into_phase_two_execution(
                            Instant::now(),
                            super::loop_protocol::ParseTimePhaseTransitionReason::ParserCompleted,
                        )
                        .await
                },
            )
            .await
            .expect("phase transition should complete");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
                        getComputedStyle(target).color,
                        (target.classList.add('foo'), getComputedStyle(target).color)
                    ])"#,
                )
                .expect("style read should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"["rgb(255, 0, 0)","rgb(0, 255, 0)"]"#)
            );
        }));
}
#[test]
fn parser_connected_head_script_does_not_push_later_head_tokens_into_body() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone())));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = "<!doctype html><html><head><script>const body = document.createElement('body');document.documentElement.appendChild(body);const early = document.createElement('div');early.id = 'early';body.appendChild(early);</script><meta charset='utf-8'><title>x</title></head><body><main>late</main></body></html>";
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at inline script handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: Default::default(),
                    navigator_identity: Default::default(),
                    document_policy_container: Default::default(),
                    document_default_language: None,
                    document_last_modified: None,
                    document_settings: Default::default(),
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_configuration: moli_page_types::LayoutConfiguration {
                        policy: moli_page_types::LayoutPolicy::default(),
                        scrollbars_hidden: false,
                    },
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
},
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let handoff = handoff.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one inline script handoff local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("inline script handoff should execute");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "inline parser-connected script should not navigate in this fixture"
            );

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one inline script continuation local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser should continue after the inline script");
            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "parser should finish the remaining buffered html after the script"
            );

            let snapshot = page_vm.vm().snapshot_live_document();
            let serialized = snapshot.serialize_document();
            assert!(
                serialized.to_ascii_lowercase().contains("<!doctype html>"),
                "doctype should survive parser-connected script execution: {serialized}"
            );

            let head = snapshot.document_head_handle().expect("head should exist");
            let body = snapshot.document_body_handle().expect("body should exist");
            let head_children = snapshot.child_ids(head).collect::<Vec<_>>();
            let body_children = snapshot.child_ids(body).collect::<Vec<_>>();

            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("meta"))
                }),
                "later <meta> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("title"))
                }),
                "later <title> should stay under <head>: {serialized}"
            );
            assert!(
                body_children.iter().all(|handle| {
                    !snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("meta") || element.is_html_element("title")
                        })
                }),
                "<body> should not receive later head-only tokens: {serialized}"
            );
        }));
}
#[test]
fn parser_connected_external_head_script_with_live_head_and_body_mutation_keeps_later_head_tokens_in_head()
 {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = Box::leak(Box::new(
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
            ));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone())));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let external_script = "globalThis.__runtimeHeadMutationStage='start';const body = document.createElement('body');globalThis.__runtimeHeadMutationStage='body-created';document.documentElement.appendChild(body);globalThis.__runtimeHeadMutationStage='body-appended';const early = document.createElement('div');early.id = 'early';body.appendChild(early);const style = document.createElement('style');style.textContent = '.runtime-style { color: red; }';document.head.appendChild(style);const script = document.createElement('script');script.textContent = 'window.__runtimeHeadMutation = true;';document.head.appendChild(script);globalThis.__runtimeHeadMutationStage='complete';";
            let encoded_script = url::form_urlencoded::byte_serialize(external_script.as_bytes())
                .collect::<String>()
                .replace('+', "%20");
            let html = format!(
                "<!doctype html><html><head><script src=\"data:text/javascript,{encoded_script}\"></script><meta charset='utf-8'><title>x</title></head><body><main>late</main></body></html>"
            );
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(&html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at external script handoff");
            };
            let ParserScriptHandoff::BlockingClassic { script, .. } = handoff.as_ref() else {
                panic!("expected external parser-blocking classic handoff");
            };
            driver.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(script).expect("preload key"),
                ready_preload_entry_for_script(script, external_script),
            );

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: Default::default(),
                    navigator_identity: Default::default(),
                    document_policy_container: Default::default(),
                    document_default_language: None,
                    document_last_modified: None,
                    document_settings: Default::default(),
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_configuration: moli_page_types::LayoutConfiguration {
                        policy: moli_page_types::LayoutPolicy::default(),
                        scrollbars_hidden: false,
                    },
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
},
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let handoff = handoff.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one external script handoff local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("external parser-connected script should execute");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "external parser-connected script should not navigate in this fixture"
            );
            let before_resume = page_vm.vm().snapshot_live_document();
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__runtimeHeadMutationStage)")
                    .expect("runtime head mutation stage should evaluate"),
                "complete",
                "external parser-blocking script should complete before parser resume"
            );
            let before_resume_head = before_resume
                .document_head_handle()
                .expect("head should exist before parser resume");
            let before_resume_body = before_resume
                .document_body_handle()
                .expect("runtime body should exist before parser resume");
            assert!(
                before_resume
                    .child_ids(before_resume_head)
                    .any(|handle| before_resume
                        .node(handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("style"))),
                "runtime style should exist before parser resume: {}",
                before_resume.serialize_document()
            );
            assert!(
                before_resume
                    .child_ids(before_resume_body)
                    .any(|handle| before_resume
                        .node(handle)
                        .and_then(Node::as_element)
                        .and_then(Element::id)
                        .is_some_and(|id| id == "early")),
                "runtime body node should exist before parser resume: {}",
                before_resume.serialize_document()
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__runtimeHeadMutation === true)")
                    .expect("runtime head script marker should evaluate"),
                "true",
                "runtime-inserted script should execute before parser resume"
            );

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one external script continuation local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser should continue after the external script");
            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "parser should finish the remaining buffered html after the script"
            );

            let snapshot = page_vm.vm().snapshot_live_document();
            let serialized = snapshot.serialize_document();
            assert!(
                serialized.to_ascii_lowercase().contains("<!doctype html>"),
                "doctype should survive external parser-connected script execution: {serialized}"
            );

            let head = snapshot.document_head_handle().expect("head should exist");
            let body = snapshot.document_body_handle().expect("body should exist");
            let head_children = snapshot.child_ids(head).collect::<Vec<_>>();
            let body_children = snapshot.child_ids(body).collect::<Vec<_>>();

            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("meta"))
                }),
                "later <meta> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("title"))
                }),
                "later <title> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("style"))
                }),
                "runtime-inserted <style> should stay under <head>: {serialized}"
            );
            assert!(
                body_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .and_then(Element::id)
                        .is_some_and(|id| id == "early")
                }),
                "runtime-inserted body node should stay under <body>: {serialized}"
            );
            assert!(
                body_children.iter().all(|handle| {
                    !snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("meta") || element.is_html_element("title")
                        })
                }),
                "<body> should not receive later head-only tokens: {serialized}"
            );
        }));
}
#[test]
fn empty_parser_blocking_script_drains_observers_before_declarative_shadow() {
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
let gotHost = false;
new MutationObserver((records) => {
  for (const record of records) {
    for (const node of record.addedNodes) {
      if (node.id === 'host') {
        gotHost = true;
        node.attachShadow({ mode: 'closed' });
      }
    }
  }
}).observe(document.body, { childList: true, subtree: true });
</script>
<div id="host"><script></script><template shadowrootmode="open"><span>Content</span></template></div>
<script>
const host = document.querySelector('#host');
document.body.setAttribute('data-result', [
  gotHost,
  !!host.querySelector('template'),
  !host.shadowRoot
].join('|'));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one empty script parser step local task channel closed",
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
                "empty parser-blocking scripts still create a microtask checkpoint before later declarative shadow parsing"
            );
            let empty_script_remained_startable = snapshot
                .script_handles()
                .into_iter()
                .filter(|handle| {
                    snapshot
                        .direct_text_content(*handle)
                        .is_some_and(|text| text.is_empty())
                })
                .any(|handle| {
                    snapshot
                        .node(handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| !element.script_already_started())
                });
            assert!(
                empty_script_remained_startable,
                "empty parser-blocking scripts must not commit already-started; later text insertion can still start them"
            );
        }));
}
#[test]
fn parser_nonce_hiding_queues_attribute_reaction_after_connected() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><body>
<script>
window.nonceEvents = [];
class ParserNonceElement extends HTMLElement {
  static get observedAttributes() { return ['nonce']; }
  attributeChangedCallback(name, oldValue, newValue) {
    window.nonceEvents.push(`attribute:${name}:${oldValue}:${newValue}`);
  }
  connectedCallback() {
    window.nonceEvents.push('connected');
  }
}
customElements.define('parser-nonce-element', ParserNonceElement);
</script>
<parser-nonce-element nonce="secret"></parser-nonce-element>
<script>
const element = document.querySelector('parser-nonce-element');
document.body.setAttribute('data-events', window.nonceEvents.join('|'));
document.body.setAttribute('data-content-nonce', element.getAttribute('nonce'));
document.body.setAttribute('data-idl-nonce', element.nonce);
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
            body_element.attribute("data-events"),
            Some("attribute:nonce:null:secret|connected|attribute:nonce:secret:"),
            "nonce hiding must enqueue its attribute reaction after the parser connection reaction"
        );
        assert_eq!(body_element.attribute("data-content-nonce"), Some(""));
        assert_eq!(body_element.attribute("data-idl-nonce"), Some("secret"));
    }));
}
#[test]
fn parser_created_customized_builtin_direct_constructs_from_is_attribute() {
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
window.builtInEvents = [];
window.WptParserButton = class extends HTMLButtonElement {
  constructor() {
    super();
    let writeResult = 'missing';
    try {
      document.write('<b id="bad-built-in-write">bad</b>');
      writeResult = 'ok';
    } catch (error) {
      writeResult = error.name;
    }
    window.builtInEvents.push([
      'constructor',
      this.localName,
      this.hasAttribute('is'),
      this.hasAttribute('data-token'),
      !!document.getElementById('after-button'),
      writeResult
    ].join('|'));
  }
  connectedCallback() {
    window.builtInEvents.push([
      'connected',
      this.getAttribute('is'),
      this.getAttribute('data-token'),
      !!document.getElementById('after-button'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-parser-button', window.WptParserButton, { extends: 'button' });
</script>
<button is="wpt-parser-button" data-token="owned"></button><span id="after-button"></span>
<script>
const element = document.querySelector('button');
document.body.setAttribute('data-events', window.builtInEvents.join('||'));
document.body.setAttribute('data-instance', String(element instanceof window.WptParserButton));
document.body.setAttribute('data-local-name', element.localName);
document.body.setAttribute('data-is', element.getAttribute('is') || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-bad-write', String(!!document.getElementById('bad-built-in-write')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser customized built-in direct channel closed",
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
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-events"),
                Some("constructor|button|false|false|false|InvalidStateError||connected|wpt-parser-button|owned|false|true"),
                "parser-created customized built-ins should synchronously construct from the token is= attribute"
            );
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-local-name"), Some("button"));
            assert_eq!(body_element.attribute("data-is"), Some("wpt-parser-button"));
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-bad-write"), Some("false"));
        }));
}
#[test]
fn parser_created_customized_builtin_uses_existing_element_validation() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<script>
window.onerror = () => true;

class MyCustomParagraph extends HTMLParagraphElement {
  constructor() {
    super();
    this.textContent = 'PASS';
  }
}
customElements.define('custom-p', MyCustomParagraph, { extends: 'p' });
</script>
<p id="targetp" is="custom-p"></p>
<script>
const targetp = document.getElementById('targetp');
document.body.setAttribute('data-p', [
  !!targetp,
  targetp.localName,
  targetp instanceof MyCustomParagraph,
  targetp instanceof HTMLParagraphElement,
  targetp.childNodes.length,
  targetp.textContent
].join('|'));

class MyCustomVideo extends HTMLVideoElement {
  constructor() {
    super();
    throw new Error('boom');
  }
}
customElements.define('custom-video', MyCustomVideo, { extends: 'video' });
</script>
<video id="targetvideo" is="custom-video"> <source></source> </video>
<script>
const targetvideo = document.getElementById('targetvideo');
document.body.setAttribute('data-video', [
  !!targetvideo,
  targetvideo.localName,
  targetvideo instanceof MyCustomVideo,
  targetvideo instanceof HTMLVideoElement,
  targetvideo.children.length
].join('|'));

class MyCustomForm extends HTMLFormElement {
  constructor() {
    super();
    throw new Error('boom');
  }
}
customElements.define('custom-form', MyCustomForm, { extends: 'form' });
</script>
<form id="targetform" is="custom-form"> <label></label><input> </form>
<script>
const targetform = document.getElementById('targetform');
document.body.setAttribute('data-form', [
  !!targetform,
  targetform.localName,
  targetform instanceof MyCustomForm,
  targetform instanceof HTMLFormElement,
  targetform.children.length
].join('|'));

class MyInputAttrs extends HTMLInputElement {
  constructor() {
    super();
    this.setAttribute('foo', 'bar');
  }
}
customElements.define('my-input-attr', MyInputAttrs, { extends: 'input' });
</script>
<input id="customized-input-attr" is="my-input-attr">
<script>
const input = document.getElementById('customized-input-attr');
document.body.setAttribute('data-input', [
  input instanceof MyInputAttrs,
  input instanceof HTMLInputElement,
  input.getAttribute('foo')
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
                body_element.attribute("data-p"),
                Some("true|p|true|true|1|PASS"),
                "parser-created customized built-ins should allow constructor child mutations on the existing element"
            );
            assert_eq!(
                body_element.attribute("data-video"),
                Some("true|video|true|true|1"),
                "throwing parser-created customized built-ins should preserve the specialized custom prototype"
            );
            assert_eq!(
                body_element.attribute("data-form"),
                Some("true|form|true|true|2"),
                "throwing parser-created customized form built-ins should keep parser children on the existing element"
            );
            assert_eq!(
                body_element.attribute("data-input"),
                Some("true|true|bar"),
                "parser-created customized built-ins should allow constructor attribute mutations on the existing element"
            );
        }));
}
#[test]
fn parser_owner_body_stylesheet_pause_preserves_unconsumed_tail_on_live_page_vm() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let parser_dom_host = state
            .parser_session
            .stream_handle()
            .borrow_mut()
            .take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let mut page_vm = PageVm::new(
            PageId::new_for_testing(2),
            local_executor,
            &loader,
            &PageVmEnvConfig {
                root_frame_id: None,
                main_document_commit: None,
                top_level_storage_key: None,
                web_storage: crate::RendererWebStorageHandles::ephemeral(),
                document_start_scripts: vec![],
                runtime_bindings: vec![],
                runtime_inspector_session_restore_snapshots: vec![],
                runtime_isolated_worlds: vec![],
                permission_overrides: vec![],
                extra_http_headers: Default::default(),
                navigator_identity: Default::default(),
                document_policy_container: Default::default(),
                document_default_language: None,
                document_last_modified: None,
                document_settings: Default::default(),
                network_offline: false,
                blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
                storage_bucket_store: None,
                fetch_subresource_interception_enabled: false,
                fetch_subresource_interception_resource_type: None,
                layout_configuration: moli_page_types::LayoutConfiguration {
                    policy: moli_page_types::LayoutPolicy::default(),
                    scrollbars_hidden: false,
                },
                wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
                reserved_service_worker_client_id: None,
            },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        let mut driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };

        let outcome = driver
            .advance_parser_step(
                &mut page_vm,
                concat!(
                    "<!doctype html><html><body>",
                    "<main id=phase-one-before>before</main>",
                    "<link rel=stylesheet href='/style.css'>",
                    "<script src='/after-pause.js'></script>",
                    "<footer id=phase-one-after>after</footer>",
                    "</body></html>"
                ),
                None,
            )
            .await
            .expect("parser step should reach its stylesheet boundary");

        assert!(matches!(
            outcome,
            ParserStepAdvanceOutcome::BlockedOnStylesheetParserPause
        ));
        let live_dom = page_vm.vm().document_runtime.dom_host();
        assert!(live_dom.element_handle_by_id("phase-one-before").is_some());
        assert!(
            live_dom.element_handle_by_id("phase-one-after").is_none(),
            "phase-one must retain the parser tail until the body stylesheet settles"
        );
        assert!(
            driver
                .buffered_document_preloads
                .entries
                .contains_key(&classic_preload_key("https://example.test/after-pause.js")),
            "a stylesheet parser pause must scan the unconsumed tail for preloadable scripts"
        );
    }));
}
