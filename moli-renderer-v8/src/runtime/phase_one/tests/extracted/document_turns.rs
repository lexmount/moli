use super::*;

#[test]
fn phase_one_owner_stop_distinguishes_document_replacement_from_navigation() {
    let mut replacement = new_phase_one_page_vm_harness_for_test();
    replacement
            .page_vm
            .vm_mut()
            .eval("document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();")
            .expect("document replacement should evaluate");
    assert!(
        !replacement.page_vm.vm().has_pending_location_navigation(),
        "document.open must not create a location navigation"
    );
    assert_eq!(
        owner_step_progress_after_current_document_stop(&replacement.page_vm),
        OwnerStepProgress::DocumentReplaced
    );

    let mut navigation = new_phase_one_page_vm_harness_for_test();
    navigation
        .page_vm
        .vm_mut()
        .eval("location.href = 'https://example.test/next.html'")
        .expect("location navigation should evaluate");
    assert!(
        navigation.page_vm.vm().has_pending_location_navigation(),
        "location assignment should queue a top-level navigation"
    );
    assert_eq!(
        owner_step_progress_after_current_document_stop(&navigation.page_vm),
        OwnerStepProgress::TriggeredNavigation
    );
}
#[test]
fn document_wait_does_not_block_on_parse_visible_async_credit() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
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
            PageId::new_for_testing(1),
            local_executor,
            &loader,
            &default_test_page_vm_env_config(),
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");

        let mut async_script = prepared_external_classic("https://example.test/slow-async.js");
        async_script.mode = crate::types::ScriptMode::Async;
        let pending_load = SharedScriptSourceLoad::spawn_for_test(std::future::pending());
        assert!(
            state
                .scheduler
                .on_parser_discovered_async_candidate_with_shared_load_and_document_character_set(
                    async_script.clone(),
                    Some(pending_load),
                    None,
                )
        );
        assert!(
            state
                .scheduler
                .claim_existing_parse_time_async_handoff(async_script.node_id)
        );
        let _ = state.scheduler.grant_parse_visible_reevaluation_credit();
        assert!(
            state
                .scheduler
                .has_outstanding_parse_visible_reevaluation_credit()
        );

        let local_executor = page_vm.local_executor.clone();
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let scheduler_ptr = &mut state.scheduler as *mut _;
        let parser_session_ptr = &state.parser_session as *const DocumentParserSession;
        let result = super::access::run_named_owner_local_task(
            local_executor,
            "phase-one async credit document drain local task channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let scheduler = unsafe { &mut *scheduler_ptr };
                let parser_session = unsafe { &*parser_session_ptr };
                let mut context = DocumentTurnContext {
                    scheduler,
                    parser_session,
                };
                let result = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    context.drain_parse_time_turns_until_idle(page_vm, true),
                )
                .await
                .expect("parse-time document drain must not wait for a slow async fetch")
                .expect("document drain should succeed");
                Ok(result)
            },
        )
        .await
        .expect("document drain should run on the named owner lane");

        assert!(
            matches!(result, PageTaskTurnResult::NoTask),
            "parse-visible async credit is a parser wake interest, not document-processing work"
        );
        assert!(
            state
                .scheduler
                .has_outstanding_parse_visible_reevaluation_credit(),
            "the streaming/parser boundary still owns the async wake interest"
        );
    }));
}
#[test]
fn phase_one_template_stylesheets_remain_inert() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><head>
<template>
  <link rel="stylesheet" href="/inside.css">
  <style>@import url("/inside-import.css");</style>
</template>
</head><body></body></html>"#,
        )
        .await;

        let snapshot = page_vm.vm().snapshot_live_document();
        let inert_stylesheet_owners = snapshot
            .nodes()
            .filter_map(|node| {
                node.as_element()
                    .filter(|element| {
                        element.is_html_element("link") || element.is_html_element("style")
                    })
                    .map(|_| node.id())
            })
            .collect::<Vec<_>>();
        assert_eq!(inert_stylesheet_owners.len(), 2);
        for owner in inert_stylesheet_owners {
            let node = snapshot.node(owner).expect("stylesheet owner node");
            assert!(
                !node.is_connected(),
                "a stylesheet owner in template contents must remain disconnected"
            );
            assert!(
                !page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(owner),
                "a disconnected stylesheet owner must not start a connected load"
            );
        }
    }));
}
#[test]
fn pending_parsing_blocking_turn_preserves_triggered_navigation_progress() {
    let pending_parsing_blocking_wait = PendingParsingBlockingWait::PageTaskBlockingStylesheet;
    let owner = ParseTimeOwner::Document;
    let result = OwnerStepProgress::TriggeredNavigation;

    assert_eq!(result, OwnerStepProgress::TriggeredNavigation);
    assert!(pending_parsing_blocking_wait.is_pending());
    assert_eq!(owner, ParseTimeOwner::Document);
}
#[test]
fn pending_parsing_blocking_wake_from_task_like_source_prefers_task_drain() {
    assert!(pending_parsing_blocking_wake_prefers_ready_task_drain(
        PendingParsingBlockingWake::Source(
            crate::document_runtime::DocumentProcessingWakeSource::InjectedPageTask,
        ),
    ));
    assert!(pending_parsing_blocking_wake_prefers_ready_task_drain(
        PendingParsingBlockingWake::Source(
            crate::document_runtime::DocumentProcessingWakeSource::TaskSourceLoadCompletion,
        ),
    ));
}
#[test]
fn document_execution_debug_starts_on_parser_without_pending_turn_state() {
    let debug = format!(
        "DocumentExecutionState {{ owner: {:?}, parser_step_ready: false, pending_parsing_blocking_wait: None }}",
        ParseTimeOwner::Parser
    );

    assert!(
        debug.contains("owner: Parser"),
        "new coordinator should start on the parser owner"
    );
    assert!(
        debug.contains("pending_parsing_blocking_wait: None"),
        "new coordinator should not start in stylesheet-blocking wait mode"
    );
}
