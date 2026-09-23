use super::*;

#[test]
fn main_parser_blocking_completion_dispatches_load_before_later_inline_script() {
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
                    "window.__mainParserClassicCompletionEvents = ['external'];",
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
                "main parser classic completion ordering test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><body>
<script id="external" src="/blocking.js" onload="window.__mainParserClassicCompletionEvents.push('load:' + (document.currentScript === null))"></script>
<script>window.__mainParserClassicCompletionEvents.push('later-inline');</script>
</body></html>"#,
                            None,
                        )
                        .await
                },
            )
            .await
            .expect("main parser classic completion ordering test should run");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "ready parser-blocking preload should execute without a source boundary"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicCompletionEvents.join('|')")
                    .expect("main parser classic completion events should evaluate"),
                "external|load:true|later-inline",
                "PendingScript completion must dispatch external load with currentScript cleared before the parser executes the later inline script"
            );
        }));
}
#[test]
fn parser_step_with_inline_script_surfaces_handoff_on_live_backend() {
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
        "<!doctype html><html><head><script>window.answer = 42;</script></head><body><div>late</div></body></html>",
    );
    let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
        panic!("expected parser step to stop at inline script handoff");
    };
    let ParserScriptHandoff::BlockingClassic {
        node_id: handle,
        start_line: _,
        start_column: _,
        blocking_signatures_before: _,
        script: _script,
    } = *handoff
    else {
        panic!("expected parser step to stop at inline blocking classic handoff");
    };
    let parser_stream_snapshot = state
        .parser_session
        .stream_handle()
        .borrow()
        .snapshot_parser_stream_document();

    assert!(
        parser_stream_snapshot.node_is_parser_created(handle),
        "handoff script should still be parser-created on the parser-stream backend"
    );
    assert_eq!(
        parser_stream_snapshot.script_text(handle).as_deref(),
        Some("window.answer = 42;")
    );
    assert!(
        parser_stream_snapshot.document_body_handle().is_none(),
        "later body content should remain hidden at the script handoff boundary"
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
fn inline_script_handoff_is_classified_as_blocking_classic_on_live_backend() {
    let final_url = Url::parse("https://example.test/").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut state =
        ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone());
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
        .pump_parser_step(
            "<!doctype html><html><head><script>window.answer = 42;</script></head></html>",
        );
    let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
        panic!("expected parser step to surface an inline script handoff");
    };
    let ParserScriptHandoff::BlockingClassic {
        node_id: handle,
        start_line: _,
        start_column: _,
        blocking_signatures_before: _,
        script,
    } = *handoff
    else {
        panic!("inline parser-visible classic should already be prepared as blocking classic");
    };

    assert_eq!(script.node_id, NodeId::new(handle.index()));
    assert_eq!(script.kind, crate::types::ScriptKind::Classic);
    assert_eq!(script.mode, crate::types::ScriptMode::Normal);
    assert_eq!(script.source_kind, crate::types::ScriptSourceKind::Inline);
    assert_eq!(script.url, final_url);
    assert_eq!(script.initiator_url, final_url);
}
#[test]
fn no_execution_datablock_handoff_consumes_parser_prepare_state_on_live_backend() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let mut driver = ParserDriver {
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
            discovered_blocking_stylesheet_inputs: _,
            discovered_preload_link_candidates: _,
        } = driver
            .parser_session
            .stream_handle()
            .borrow_mut()
            .pump_parser_step(
            "<!doctype html><html><head><script type=\"text/plain\" src=\"/data.txt\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at data-block script handoff");
        };
        let ParserScriptHandoff::NoExecution {
            node_id: handle, ..
        } = handoff.as_ref()
        else {
            panic!("data-block parser script should surface as no-execution handoff");
        };
        let handle = *handle;

        let parser_dom_host = driver
            .parser_session
            .stream_handle()
            .borrow_mut()
            .take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let mut page_vm = PageVm::new(
            PageId::new_for_testing(104),
            local_executor,
            &loader,
            &default_test_page_vm_env_config(),
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");

        let before = page_vm.vm().snapshot_live_document();
        let before_element = before
            .node(handle)
            .and_then(Node::as_element)
            .expect("script element should exist before handoff");
        assert!(before_element.script_parser_inserted_for_prepare());
        assert!(
            !before_element.script_async(),
            "DOM should not classify data-block scripts before prepare"
        );

        let outcome = driver
            .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
            .await
            .expect("data-block handoff should resolve without executing V8");
        assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

        let after = page_vm.vm().snapshot_live_document();
        let after_element = after
            .node(handle)
            .and_then(Node::as_element)
            .expect("script element should exist after handoff");
        assert!(
            !after_element.script_parser_inserted_for_prepare(),
            "inert data-block prepare should consume parser-inserted state"
        );
        assert!(
            after_element.script_async(),
            "inert data-block prepare should expose force-async for later reactivation"
        );
        assert!(
            !after_element.script_already_started(),
            "inert data-block prepare must leave the script startable"
        );
    }));
}
#[test]
fn external_async_handoff_marks_parser_stream_already_started_on_live_backend() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state =
            ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let mut driver = ParserDriver {
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
            discovered_async_prefetch_scripts,
        discovered_preload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
            "<!doctype html><html><head><script async src=\"/async.js\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at external async script handoff");
        };
        let handle = match handoff.as_ref() {
            ParserScriptHandoff::AsyncPostParse { node_id, .. }
            | ParserScriptHandoff::NonAsyncPostParse { node_id, .. }
            | ParserScriptHandoff::ImportMap { node_id, .. }
            | ParserScriptHandoff::NoExecution { node_id, .. }
            | ParserScriptHandoff::PreparationFailure { node_id, .. } => *node_id,
            ParserScriptHandoff::BlockingClassic { .. } => {
                panic!("non-blocking async handoff should not carry a blocking-classic payload")
            }
        };

        // In the single-DOM model, the PageVm owns the parser's DomHost.
        // Simulate bootstrap by giving the parser's DomHost to the PageVm.
        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
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
            navigation_history_source: None,
        reserved_service_worker_client_id: None,
            },
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        for mut script in discovered_async_prefetch_scripts {
            bind_parser_owned_script_handle(&mut page_vm, &mut script);
            let _ = driver
                .scheduler
                .on_parser_discovered_async_candidate_with_shared_load(
                    script,
                    Some(SharedScriptSourceLoad::ready_err(
                        "synthetic async source terminal",
                    )),
                );
        }
        let outcome = driver
            .handle_parse_time_script_handoff(&mut page_vm, *handoff.clone(), None)
            .await
            .expect("external async handoff should resolve without executing V8");

        assert!(
            matches!(outcome, ScriptHandoffOutcome::NoNavigation),
            "external async handoff should resolve without navigation (credit granted internally)"
        );

        // In the single-DOM model, the already-started bit is on the runtime's DomHost.
        let runtime_snapshot = page_vm.vm().snapshot_live_document();
        assert!(
            runtime_snapshot
                .node(handle)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.script_already_started()),
            "runtime live document should record async handoff ownership after mutation"
        );
        let expected_handle = format!("parser-script-native-{}", handle.index());
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .resolve_host_script_handle(&expected_handle),
            Some(handle),
            "async parser-owned handoff should register a parser-owned host script handle"
        );
    });
}
#[test]
fn blocking_classic_handoff_registers_parser_owned_handle_on_live_backend() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let mut driver = ParserDriver {
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
            discovered_blocking_stylesheet_inputs,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
            "<!doctype html><html><head><link rel=\"stylesheet\" href=\"/app.css\"><script src=\"/app.js\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at blocking classic handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            node_id: handle, ..
        } = handoff.as_ref()
        else {
            panic!("expected parser-blocking classic to classify as blocking classic");
        };
        let handle = *handle;

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
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
            navigation_history_source: None,
        reserved_service_worker_client_id: None,
            },
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        activate_standalone_main_parser_continuation_for_test(&mut page_vm);
        page_vm
            .vm_mut()
            .document_runtime
            .note_discovered_document_owned_blocking_stylesheet_inputs(
                discovered_blocking_stylesheet_inputs.iter(),
            );

        let outcome = driver
            .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
            .await
            .expect("blocking classic handoff should resolve on the live backend");
        assert!(
            matches!(outcome, ScriptHandoffOutcome::BlockedOnStylesheet(_)),
            "blocking classic handoff should stay stylesheet-gated in this fixture"
        );
        let expected_handle = format!("parser-script-native-{}", handle.index());
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .resolve_host_script_handle(&expected_handle),
            Some(handle),
            "blocking classic handoff should register a parser-owned host script handle"
        );
        let runtime_snapshot = page_vm.vm().snapshot_live_document();
        assert!(
            runtime_snapshot
                .node(handle)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.script_already_started()),
            "blocking classic handoff should mark already-started on the live runtime DOM"
        );
    });
}
#[test]
fn non_async_post_parse_handoff_registers_pending_before_source_and_seals_without_waiting() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let mut driver = ParserDriver {
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
            "<!doctype html><html><head><script defer src=\"/defer.js\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at defer handoff");
        };
        let ParserScriptHandoff::NonAsyncPostParse {
            node_id: handle,
            script,
            ..
        } = handoff.as_ref()
        else {
            panic!("expected defer script to classify as non-async post-parse");
        };
        let handle = *handle;
        let script = script.clone();
        let (source_tx, source_rx) = tokio::sync::oneshot::channel();
        driver.buffered_document_preloads.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("defer preload key"),
            BufferedScriptPreloadEntry {
                request: preload_request_for_script(
                    &script,
                    moli_fetch::RequestResourceType::ParserBlockingScript,
                ),
                load: SharedScriptSourceLoad::spawn_for_test(async move {
                    source_rx.await.expect("defer source result")
                }),
            },
        );

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
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
            navigation_history_source: None,
        reserved_service_worker_client_id: None,
            },
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");

        let outcome = driver
            .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
            .await
            .expect("defer handoff should resolve without executing V8");
        assert!(
            matches!(outcome, ScriptHandoffOutcome::NoNavigation),
            "defer handoff should remain a scheduling-only step"
        );
        let expected_handle = format!("parser-script-native-{}", handle.index());
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .resolve_host_script_handle(&expected_handle),
            Some(handle),
            "non-async parser-owned handoff should register a parser-owned host script handle"
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser-deferred document owner");
        let parser_owner =
            crate::module_script_continuation::MainParserDocumentOwner::new(task_owner);
        assert!(
            page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .has_load_blocking_document_script_work(parser_owner),
            "handoff must install the PendingScript before its pending source finishes"
        );
        assert!(
            !page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .has_after_parsing_script(parser_owner),
            "parser-deferred work cannot execute before EOF"
        );
        assert!(
            page_vm
                .seal_main_parser_deferred_scripts(task_owner)
                .is_some(),
            "EOF seal should synchronously arm parser-deferred work"
        );
        assert!(
            !page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .next_after_parsing_script_is_ready(parser_owner),
            "EOF must not wait for or inline-apply the source result"
        );

        source_tx
            .send(Ok("globalThis.__mainDeferred = 1;".to_owned()))
            .expect("defer source receiver should remain alive");
        if !page_vm.page_resource_completion_queue().has_ready_completion() {
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                page_vm.wait_for_page_resource_completion_for_test(),
            )
            .await
            .expect("defer source completion should arrive");
        }
        let mut source = page_vm.page_resource_completion_queue();
        let completion = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut source)
            .expect("defer source completion should apply")
            .expect("defer source completion should make progress");
        assert_eq!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::MainParserDeferredClassicSource
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .next_after_parsing_script_is_ready(parser_owner),
            "typed source completion should release the original PendingScript"
        );
    });
}
#[test]
fn parser_owned_external_module_handoff_starts_pending_script_tree_root_fetch() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let mut driver = ParserDriver {
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
            "<!doctype html><html><head><script type=\"module\" src=\"/module.mjs\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at module handoff");
        };
        let ParserScriptHandoff::NonAsyncPostParse {
            node_id: handle,
            ..
        } = handoff.as_ref()
        else {
            panic!("expected parser-owned external module to classify as non-async post-parse");
        };
        let handle = *handle;

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
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
            navigation_history_source: None,
        reserved_service_worker_client_id: None,
            },
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");

        let outcome = driver
            .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
            .await
            .expect("module handoff should resolve without executing V8");
        assert!(
            matches!(outcome, ScriptHandoffOutcome::NoNavigation),
            "module handoff should remain a scheduling-only step"
        );
        let expected_handle = format!("parser-script-native-{}", handle.index());
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .resolve_host_script_handle(&expected_handle),
            Some(handle),
            "parser-owned external module handoff should register a parser-owned host script handle"
        );
        let root_key = crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://example.test/module.mjs").expect("root url"),
        );
        let root_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("parser handoff should eagerly start module tree root fetch");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(root_entry),
            crate::module_runtime::ModuleMapEntryState::Fetching
        );
    });
}
#[test]
fn parser_owned_module_handoff_starts_external_root_pending_tree_without_source_load() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let mut driver = ParserDriver {
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
            "<!doctype html><html><head><script type=\"module\" src=\"/pending.mjs\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at module handoff");
        };

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
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

        let outcome = driver
            .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
            .await
            .expect("module handoff should start root pending script tree");
        assert!(
            matches!(outcome, ScriptHandoffOutcome::NoNavigation),
            "module handoff should remain a scheduling-only step"
        );

        let pending_root_key = crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://example.test/pending.mjs").expect("root url"),
        );
        let pending_dep_key = crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://example.test/pending-dep.mjs").expect("dependency url"),
        );
        let pending_root_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&pending_root_key)
            .expect("parser handoff should start external root graph fetch immediately");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(pending_root_entry),
            crate::module_runtime::ModuleMapEntryState::Fetching
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_id(&pending_dep_key)
                .is_none(),
            "dependency cannot be discovered until the root graph fetch completes"
        );

        tokio::task::yield_now().await;
        assert!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_id(&pending_dep_key)
                .is_none(),
            "module graph dependency discovery must wait for native module fetch completion"
        );
    });
}
#[test]
fn parser_owned_module_handoff_starts_loaded_source_pending_tree_dependencies() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        let mut driver = ParserDriver {
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
            "<!doctype html><html><head><script type=\"module\" src=\"/ready.mjs\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at module handoff");
        };

        let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
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
        page_vm.vm_mut().document_runtime.insert_native_module_source(
            crate::module_runtime::ModuleMapKey::java_script(
                Url::parse("https://example.test/ready.mjs").expect("root url"),
            ),
            crate::module_runtime::ModuleSource::text(
                "import './ready-dep.mjs'; globalThis.readyModuleShouldNotRunYet = true;"
                    .to_owned(),
            ),
        );

        let outcome = driver
            .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
            .await
            .expect("module handoff should start loaded-source pending script tree");
        assert!(
            matches!(outcome, ScriptHandoffOutcome::NoNavigation),
            "module handoff should remain a scheduling-only step"
        );

        let root_key = crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://example.test/ready.mjs").expect("root url"),
        );
        let dep_key = crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://example.test/ready-dep.mjs").expect("dependency url"),
        );
        let root_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("loaded module source should install the root module entry");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(root_entry),
            crate::module_runtime::ModuleMapEntryState::Compiled
        );
        let dep_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&dep_key)
            .expect("loaded module handoff should discover static dependencies immediately");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(dep_entry),
            crate::module_runtime::ModuleMapEntryState::Fetching
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.readyModuleShouldNotRunYet)")
                .expect("read module side effect before page task"),
            "undefined",
            "prestarted loaded-source graph must not evaluate before its ordered page task"
        );
    });
}
#[test]
fn parser_owned_inline_importmap_handoff_registers_parser_owned_handle_on_live_backend() {
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
            "<!doctype html><html><head><script type=\"importmap\">{\"imports\":{\"fixture\":\"/module.mjs\"}}</script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at inline importmap handoff");
        };
        let ParserScriptHandoff::ImportMap { node_id: handle, .. } = handoff.as_ref() else {
            panic!("expected parser-owned inline importmap registration handoff");
        };
        let handle = *handle;

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
            navigation_history_source: None,
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
            "phase-one inline importmap handoff local task channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver
                    .handle_parse_time_script_handoff(page_vm, *handoff, None)
                    .await
            },
        )
        .await
        .expect("inline importmap handoff should resolve without executing external fetch");
        assert!(
            matches!(outcome, ScriptHandoffOutcome::NoNavigation),
            "inline importmap handoff should remain a current-turn scheduling step"
        );
        let expected_handle = format!("parser-script-native-{}", handle.index());
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .resolve_host_script_handle(&expected_handle),
            Some(handle),
            "parser-owned inline importmap handoff should register a parser-owned host script handle before current-turn execution"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .document_runtime
                .resolve_module_specifier(
                    "fixture",
                    &Url::parse("https://example.test/").expect("base url"),
                )
                .expect("registered import map should resolve fixture"),
            Url::parse("https://example.test/module.mjs").expect("mapped url"),
            "dedicated import-map handoff should register before later module preparation"
        );
    }));
}
#[test]
fn external_blocking_classic_handoff_is_stylesheet_gated_on_live_backend() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state =
            ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
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
            discovered_blocking_stylesheet_inputs,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
            "<!doctype html><html><head><link rel=\"stylesheet\" href=\"/app.css\"><script src=\"/app.js\"></script></head></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at external classic script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            node_id: _handle,
            start_line: _,
            start_column: _,
            blocking_signatures_before,
            script: _script,
        } = *handoff
        else {
            panic!("external parser-blocking classic should already be prepared");
        };
        assert!(
            !blocking_signatures_before.is_empty(),
            "parser-owned blocking classic handoff should carry blocking stylesheet signatures discovered before the script"
        );
        let parser_stream_snapshot = state.parser_session.stream_handle().borrow().snapshot_parser_stream_document();

        let mut live_runtime = DocumentRuntime::new_networked(
            &parser_stream_snapshot.clone().into_document(),
            &loader,
        );
        let stylesheet_gated =
            blocking_classic_is_stylesheet_gated_for_testing(
            &mut live_runtime,
            &discovered_blocking_stylesheet_inputs,
            &blocking_signatures_before,
        );

        assert!(
            stylesheet_gated,
            "stylesheet discovered before a parser-blocking classic script should gate execution on the parser-stream backend"
        );
    });
}
#[test]
fn parser_owner_style_import_handoff_is_stylesheet_gated_on_live_page_vm() {
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
            navigation_history_source: None,
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
                "<!doctype html><html><head><style id='blocking-style'>@import url('/style.css');</style><script>window.afterStyle = true;</script></head></html>",
                None,
            )
            .await
            .expect("parser step should complete");

        assert!(
            matches!(outcome, ParserStepAdvanceOutcome::BlockedOnStylesheet(_)),
            "parser-created style import should gate parser-blocking script on live PageVm"
        );
        let style = page_vm
            .vm()
            .document_runtime
            .dom_host()
            .element_handle_by_id("blocking-style")
            .expect("parser-created style owner");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .pending_style_import_binding_for_test(style),
            Some((1, true)),
            "the parser-discovered import must bind its live stylesheet root before the script gate is released"
        );
    }));
}
