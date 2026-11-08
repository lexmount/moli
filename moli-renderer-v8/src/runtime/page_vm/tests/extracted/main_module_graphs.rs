use super::*;

#[test]
fn runtime_module_graph_wait_does_not_block_domcontentloaded_on_load_target() {
    let mut page_vm = test_page_vm();
    page_vm.target_stage = PageVmInitStage::Load;
    let mut script = prepared_inline_module_for_page_vm_test(
        &page_vm,
        9062,
        "import { value } from 'late'; globalThis.__lateValue = value;",
    );
    script.mode = ScriptMode::Async;
    page_vm
        .vm_mut()
        .document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .requeue_failed_script_front(
            DynamicScriptOwnerId::from_u64(1),
            script,
            "failed to resolve module specifier `late`".to_owned(),
            DynamicScriptFailureKind::ModuleResolve,
            Some(ModuleFailurePolicy::GraphFailure),
            None,
            None,
        );

    assert!(
        page_vm
            .vm_mut()
            .has_pending_runtime_owned_module_script_graph(),
        "the terminal graph failure must remain pending until its error owner turn"
    );
    assert!(
        !page_vm.has_pending_module_script_for_target_stage(),
        "runtime async module work must not become a DCL wait merely because the final target is Load"
    );

    page_vm
        .vm_mut()
        .document_runtime
        .note_dom_content_loaded_dispatched();
    assert!(
        page_vm.has_pending_module_script_for_target_stage(),
        "after DCL the same terminal graph work must delay Load until error dispatch settles it"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_module_graph_failure_settles_exact_load_delay() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async-module-failure.html")
                .expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async module failure test requires an owner");
        let module_url =
            Url::parse("https://example.com/main-parser-async-failure.mjs").expect("module URL");
        let mut script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9062, module_url.clone());
        script.mode = ScriptMode::Async;

        assert!(
            page_vm
                .vm_mut()
                .accept_main_parser_async_module_script(owner, &script)?
        );
        enqueue_parser_owned_module_script_fetch_error_for_test(
            &mut page_vm,
            0,
            &module_url,
            "async module graph failed",
        );
        assert!(run_next_main_module_fetch_terminal_for_test(&mut page_vm)?.is_some());
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader).await?,
            "graph failure should run through the watched PendingScript owner"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "failure reporting must be ordered before lifecycle settlement"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async module graph failure",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        assert!(
            page_vm.report.runs.iter().any(|run| {
                run.url() == &module_url && matches!(run.outcome(), ScriptRunOutcome::Failed(_))
            }),
            "graph failure should remain an observable failed script run"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async module failure lifecycle test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_module_tla_releases_lifecycle_at_evaluation_start() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-tla.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/main-parser-tla.mjs").expect("module URL");
        let script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9050, module_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "the accepted module PendingScript must own a lifecycle token before fetch starts"
        );
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, parser_module_work)
            .await
            .expect("parser module owner should wait for its graph");

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            concat!(
                "globalThis.__mainParserTlaStarted = 1;",
                "await new Promise(resolve => { globalThis.__resolveMainParserTla = resolve; });",
                "globalThis.__mainParserTlaFinished = 1; export const value = 1;",
            ),
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("TLA module graph completion should apply")
                .is_some()
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "main parser TLA evaluation start",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserTlaStarted)")
                .expect("read TLA start"),
            "1"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserTlaFinished)")
                .expect("read suspended TLA body"),
            "undefined"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "the TLA promise must not retain parser-deferred lifecycle ownership"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url() == &module_url)
                .count(),
            1,
            "starting module evaluation must apply script completion exactly once"
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__resolveMainParserTla(); 'resolved'")
            .expect("resolve main parser TLA");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            1,
            "main parser TLA fulfillment",
        )
        .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserTlaFinished)")
                .expect("read fulfilled TLA body"),
            "1"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url() == &module_url)
                .count(),
            1,
            "TLA fulfillment must not redispatch script completion"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_module_tla_rejection_reports_without_duplicate_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-tla-rejection.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval(
                "globalThis.__mainParserTlaErrors = []; addEventListener('error', event => { __mainParserTlaErrors.push((event.error?.constructor?.name ?? 'none') + ':' + event.message); event.preventDefault(); });",
            )
            .expect("install main TLA error observer");
        let module_url = Url::parse("https://example.com/main-parser-tla-rejection.mjs")
            .expect("module URL");
        let script = prepared_external_module_for_page_vm_test_with_node(
            &page_vm,
            9051,
            module_url.clone(),
        );
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser module owner should wait for its graph");
        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            concat!(
                "await new Promise((_, reject) => {",
                "  globalThis.__rejectMainParserTla = reject;",
                "}); export const value = 1;",
            ),
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("TLA rejection graph completion should apply")
                .is_some()
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "main parser TLA rejection evaluation start",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "pending rejection must not retain parser lifecycle ownership"
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__rejectMainParserTla(new TypeError('main TLA rejected')); 'rejected'")
            .expect("reject main parser TLA");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            1,
            "main parser TLA rejection",
        )
        .await;
        let error = page_vm
            .vm_mut()
            .eval("__mainParserTlaErrors.join('|')")
            .expect("read TLA rejection event");
        assert!(
            error.contains("TypeError:") && error.contains("main TLA rejected"),
            "TLA rejection must retain typed window-error metadata: {error}"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url() == &module_url)
                .count(),
            1,
            "TLA rejection must not redispatch or rerecord script completion"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn parser_module_graph_failure_leaves_shared_sibling_fetch_for_joined_script_waiter() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let first_root_url =
            Url::parse("https://example.com/first-root.mjs").expect("first root URL");
        let second_root_url =
            Url::parse("https://example.com/second-root.mjs").expect("second root URL");
        let bad_url = Url::parse("https://example.com/bad.mjs").expect("bad module URL");
        let shared_url = Url::parse("https://example.com/shared.mjs").expect("shared module URL");
        let shared_key = ModuleMapKey::java_script(shared_url.clone());
        let first_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9201, first_root_url.clone());
        let second_script = prepared_external_module_for_page_vm_test_with_node(
            &page_vm,
            9202,
            second_root_url.clone(),
        );

        let first_parser_module_work =
            install_parser_module_defer_work(&mut page_vm, first_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                first_parser_module_work,
            )
            .await
            .expect("first module defer page task should watch loading tree");

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &first_root_url,
            r#"
import "./bad.mjs";
import "./shared.mjs";
export const first = 1;
"#,
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("first root completion should run")
                .is_some(),
            "first root completion should be consumed"
        );

        let second_parser_module_work =
            install_parser_module_defer_work(&mut page_vm, second_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                second_parser_module_work,
            )
            .await
            .expect("second module defer page task should watch loading tree");
        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            3,
            &second_root_url,
            r#"
import "./shared.mjs";
export const second = 2;
"#,
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("second root completion should run")
                .is_some(),
            "second root completion should be consumed"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "second graph should be waiting on the first graph's shared sibling fetch"
        );
        let shared_fetch_target = page_vm
            .vm()
            .current_main_parser_module_graph_fetch_target(2)
            .expect("shared fetch must retain its producer-captured target before owner failure");

        enqueue_parser_owned_module_script_fetch_error_for_test(
            &mut page_vm,
            1,
            &bad_url,
            "bad dependency failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("bad dependency completion should run")
                .is_some(),
            "bad dependency failure should be consumed"
        );

        let shared_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&shared_key)
            .expect("cancelled shared sibling fetch should remain in the module map");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(shared_entry),
            ModuleMapEntryState::Fetching,
            "owner failure must not globally fail a shared module map entry while its network fetch is still pending"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "joined parser graph should keep waiting for the shared fetch network completion"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "ordered graph failure must stay in the parser PendingScript instead of the broad ready lane"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "bad dependency failure",
        )
        .await;
        let first_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &first_root_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("first parser module script should fail after bad dependency");
        assert!(
            first_failure.contains("bad dependency failed"),
            "first script should report the real dependency failure: {first_failure}"
        );

        enqueue_parser_owned_module_script_fetch_completion_for_target_for_test(
            &mut page_vm,
            shared_fetch_target,
            &shared_url,
            "export const shared = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("shared dependency completion should run")
                .is_some(),
            "shared dependency completion should be consumed"
        );
        assert!(
            page_vm.vm_mut().has_ready_native_module_owner_actions(),
            "shared module-map terminal must publish its joined-client owner event"
        );
        run_next_native_module_owner_event_for_test(
            &mut page_vm,
            &loader,
            "shared parser module-map completion",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "joined parser graph should resume through the native module owner turn"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "joined parser module graph completion",
        )
        .await;
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "joined parser module graph completion",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "joined parser graph should no longer be pending after its ready script executes"
        );
        let second_run = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &second_root_url)
            .expect("second parser module script should run after joined fetch wakeup");
        assert!(
            matches!(second_run.outcome(), ScriptRunOutcome::Executed),
            "second parser module script should execute after joined fetch wakeup: {:?}",
            second_run.outcome()
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn stale_main_parser_module_terminal_drops_after_pending_script_retirement() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let old_task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("original main document task owner should exist");
        let old_owner = MainParserDocumentOwner::new(old_task_owner);
        let module_url =
            Url::parse("https://example.com/stale-ready-module.mjs").expect("module URL");
        let module_script =
            prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let replacement_script = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9200,
            "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        );
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                classic_defer_work(replacement_script),
            )
            .await
            .expect("replacement script task should run");
        let stale_pending_script_id =
            crate::document_script_scheduler::ParserPendingScriptId::new(
                old_owner,
                &module_script,
            );
        let stale_continuation = ModuleScriptContinuation::new_parser(
            module_script,
            stale_pending_script_id,
        )
        .with_completed_graph(ModuleGraphHandle {
            root_entry: ModuleEntryId::for_test(1),
            entries: vec![ModuleEntryId::for_test(1)],
        });
        let main_task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main document task owner should exist");
        let main_owner = MainParserDocumentOwner::new(main_task_owner);
        assert_ne!(main_task_owner.document_id, old_task_owner.document_id);
        assert_eq!(
            main_task_owner.scheduler_lane_id,
            old_task_owner.scheduler_lane_id,
            "main document replacement should retain the browsing-context scheduler lane"
        );
        assert_eq!(
            main_task_owner.local_window_id,
            old_task_owner.local_window_id,
            "document.open() should retain the main LocalWindow"
        );
        assert_ne!(
            main_owner, old_owner,
            "document replacement should install a new document-scoped parser owner"
        );
        let stale_work = stale_continuation.into_main_document_graph_ready_work();
        let accepted = page_vm
            .vm_mut()
            .document_runtime
            .parser_module_document_scripts_mut()
            .notify_module_script_graph_ready_work(stale_work);
        assert!(
            !accepted,
            "terminal must fail closed after its original PendingScript is retired"
        );
        assert!(
            !page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .has_load_blocking_document_script_work(old_owner),
            "retired owner must not be rematerialized by a stale terminal"
        );
        assert!(
            !page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .has_load_blocking_document_script_work(main_owner),
            "stale terminal must not enter the replacement document scheduler"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "stale terminal must not synthesize a ready action without its PendingScript"
        );

        assert!(
            !run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader)
                .await
                .expect("stale parser module ready lane should remain idle"),
            "retired PendingScript terminal must not reach owner-specific execution"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "stale ready action should be removed from the ready lane"
        );
        assert!(
            page_vm.report.runs.iter().all(|run| run.url() != &module_url),
            "stale main parser module ready action must not produce a script run: {:?}",
            page_vm.report.runs
        );
    })
    .await;
}
#[test]
fn module_graph_network_result_records_staged_response_started_with_cache_state() {
    let mut page_vm = test_page_vm_with_document_url(
        Url::parse("https://example.com/page.html").expect("document URL"),
    );
    let request_url = Url::parse("https://example.com/module.js").expect("request URL");
    let response = crate::types::NavigationResponse::from_head_and_text_body(
        moli_fetch::ResponseHead {
            status_text: None,
            final_url: request_url.clone(),
            status: 200,
            headers: vec![("content-type".to_owned(), b"text/javascript".to_vec())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: true,
            cache_state: moli_fetch::ResponseCacheState::Local,
            negotiated_http_version: None,
        },
        "export default 1;".to_owned(),
    );
    let completion = ModuleGraphFetchCompletion {
        load_id: 7,
        requester: ModuleGraphFetchRequester::DynamicImport,
        ordering: ModuleGraphFetchOrdering::Runtime,
        request_url: request_url.clone(),
        result: Err("source is not used by this test".to_owned()),
        network_result: None,
    };

    page_vm
        .vm_mut()
        .record_module_graph_subresource_network_result(&completion, &Ok(response));

    let items: Vec<_> = page_vm
        .vm_mut()
        .take_network_output()
        .into_items()
        .collect();
    assert_eq!(
        items.len(),
        3,
        "module fetch should record staged network output"
    );
    let ScriptNetworkOutputItem::SubresourceRequestStarted(request) = &items[0] else {
        panic!("first item should be requestStarted: {items:?}");
    };
    assert_eq!(request.url(), &request_url);
    assert_eq!(request.resource_type(), SubresourceResourceType::Script);

    let ScriptNetworkOutputItem::SubresourceResponseStarted(response) = &items[1] else {
        panic!("second item should be responseStarted: {items:?}");
    };
    assert_eq!(response.final_url(), &request_url);
    assert!(
        response.from_cache(),
        "module responseStarted must preserve fetch cache state"
    );

    let ScriptNetworkOutputItem::SubresourceBodyFinished(body) = &items[2] else {
        panic!("third item should be bodyFinished: {items:?}");
    };
    assert!(matches!(
        body.result(),
        SubresourceBodyFinishedResult::Ready(_)
    ));
}
