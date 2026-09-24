use super::*;
use moli_dom::native::Node;

#[test]
fn data_block_completes_in_parser_turn_without_page_task() {
    super::tests::run_phase_one_large_stack_test("phase-one-data-block-completion", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test URL");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("test loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state
                .parser_session
                .stream_handle()
                .borrow_mut()
                .take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(106),
                local_executor,
                loader,
                &super::tests::default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("phase-one PageVm");
            let mut driver = ParserDriver {
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let html = r#"<!doctype html><html><body>
<script>
window.dataBlockObserverRan = false;
new MutationObserver(() => {
  window.dataBlockObserverRan = true;
}).observe(document.body, { childList: true });
</script>
<div id="before-data-block"></div>
<script type="application/ld+json">{"name":"moli"}</script>
<script>
document.body.setAttribute(
  "data-observer-ran-before-following-script",
  String(window.dataBlockObserverRan)
);
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one data-block parser test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser turn should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("document body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| {
                    element.attribute("data-observer-ran-before-following-script")
                });
            assert_eq!(
                result,
                Some("true"),
                "a non-executable parser script must still establish its parser microtask checkpoint before parsing continues"
            );
            assert!(
                !page_vm
                    .vm_mut()
                    .document_runtime
                    .has_parser_owned_pre_domcontentloaded_page_tasks(),
                "JSON-LD is parser-local completion, not a future Page task"
            );
            assert!(
                page_vm.report.runs.iter().any(|run| {
                    run.kind() == crate::types::ScriptKind::DataBlock
                        && matches!(
                            run.outcome(),
                            crate::types::ScriptRunOutcome::Skipped(
                                crate::types::ScriptSkipReason::UnsupportedType(_)
                            )
                        )
                }),
                "the synchronous terminal must remain visible in the internal script report"
            );
        }));
    });
}

#[test]
fn parser_owned_import_map_is_not_registered_when_script_execution_is_disabled() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let env = super::tests::default_test_page_vm_env_config_with(|env| {
            env.script_execution_disabled = true;
        });
        let mut page_vm = super::tests::parse_phase_one_html_into_page_vm_for_test_with_env(
            r#"<!doctype html><html><head><script type="importmap">{"imports":{"fixture":"/module.mjs"}}</script></head><body></body></html>"#,
            env,
        )
        .await;
        let base_url = Url::parse("https://example.test/").expect("base url");

        assert!(
            page_vm
                .vm_mut()
                .document_runtime
                .resolve_module_specifier("fixture", &base_url)
                .is_err(),
            "a disabled parser-owned import map must not alter module resolution"
        );
        let document = page_vm.vm().document_runtime.dom_host().document_handle();
        let scripts = page_vm
            .vm()
            .document_runtime
            .dom_host()
            .script_handles_in_subtree(document);
        assert_eq!(scripts.len(), 1);
        assert!(
            page_vm
                .vm()
                .document_runtime
                .dom_host()
                .node(scripts[0])
                .and_then(Node::as_element)
                .is_some_and(|element| element.script_already_started()),
            "the disabled import map must remain inert if scripting is enabled later"
        );
    }));
}
