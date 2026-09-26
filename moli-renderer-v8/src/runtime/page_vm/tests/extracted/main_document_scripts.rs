use super::*;

#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_classic_owns_load_delay_from_discovery_through_settlement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async.html").expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async test requires an owner");
        let script_url =
            Url::parse("https://example.com/main-parser-async.js").expect("script URL");
        let mut script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            9060,
            "main-parser-async-classic",
            script_url,
            ScriptSource::External,
            ("async", ""),
        );
        script.mode = ScriptMode::Async;
        assert!(
            page_vm
                .vm_mut()
                .document_runtime
                .dom_host_mut()
                .set_script_already_started(script.node_id, true)
        );
        let shared_load =
            SharedScriptSourceLoad::ready_ok("globalThis.__mainParserAsyncClassicExecuted = 1;");
        let mut scheduler = crate::document_script_scheduler::DocumentScriptScheduler::new();
        let resource_task_runner = page_vm.resource_task_runner();

        assert!(
            scheduler.accept_parser_discovered_async_candidate(
                script.clone(),
                &loader,
                page_vm
                    .vm()
                    .current_main_document_resource_loader()
                    .unwrap()
                    .fetch_context()
                    .request_origin(),
                resource_task_runner,
                Some(shared_load),
                None,
                |_| {
                    page_vm
                        .vm_mut()
                        .accept_main_document_script_load_delay_binding(
                            owner,
                            crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
                        )
                        .expect("parser discovery should bind classic lifecycle ownership")
                },
            )
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "acceptance must delay load before source readiness is observed"
        );
        assert!(scheduler.claim_existing_parse_time_async_handoff(script.node_id));
        let ready = scheduler
            .parse_time_turn(
                crate::document_script_scheduler::ParseTimeTurnTrigger::BeforeParserStep {
                    default_chunk_bytes: 4096,
                },
            )
            .ready_task
            .expect("ready shared source should produce a parser async task");
        let crate::document_script_scheduler::ParseTimeDocumentScriptTask::ClassicAsyncScript(
            ready,
        ) = ready
        else {
            panic!("ready shared source should produce classic execution work");
        };
        let (script, binding) = ready.into_parts();
        let binding = binding.expect("parse-time work must retain its discovery binding");
        assert_eq!(binding.owner(), owner);

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::document_script_work(
                    PageOwnedDocumentScriptWork::parser_async_script(
                        DocumentScriptExecutionLane::ParseTimeAsync,
                        script,
                        Some(binding),
                    ),
                ),
            )
            .await
            .expect("parser async classic execution turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserAsyncClassicExecuted)")?,
            "1"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "execution must enqueue, not inline-apply, lifecycle settlement"
        );
        page_vm
            .drain_deferred_page_tasks_on_named_owner_local_task()
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "parser progress may publish follow-ups but must not apply settlement inline"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async classic",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async classic lifecycle test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_module_owns_load_delay_until_evaluation_start() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async-module.html").expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async module test requires an owner");
        let module_url =
            Url::parse("https://example.com/main-parser-async-module.mjs").expect("module URL");
        let mut script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9061, module_url.clone());
        script.mode = ScriptMode::Async;

        assert!(
            page_vm
                .vm_mut()
                .accept_main_parser_async_module_script(owner, &script)?
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "module acceptance must bind lifecycle ownership before graph work"
        );
        page_vm.target_stage = PageVmInitStage::DomContentLoaded;
        assert!(
            !page_vm.has_pending_module_script_for_target_stage(),
            "a watched parser-origin async module must not enter the parser-deferred DCL gate"
        );
        assert!(
            page_vm.has_pending_module_fetch_for_target_stage(),
            "the pending graph should remain visible to resource-completion observation"
        );
        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__mainParserAsyncModuleExecuted = 1; export const value = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)?.is_some(),
            "async module graph completion should apply"
        );
        assert!(
            page_vm.has_ready_parser_owned_document_script_action(),
            "watching a completed async graph should queue owner work"
        );
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader).await?,
            "async module graph-ready work should start evaluation"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserAsyncModuleExecuted)")?,
            "1"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "evaluation start must leave settlement for its own lifecycle turn"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async module",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async module lifecycle test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_inline_module_is_watched_before_graph_start() {
    run_page_vm_async_test(async move {
        let loader_owner =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let loader = loader_owner.handle();
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async-inline-module.html")
                .expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async inline module test requires an owner");
        let mut script = prepared_inline_module_for_page_vm_test(
            &page_vm,
            9063,
            "globalThis.__mainParserAsyncInlineModuleExecuted = 1; export const value = 1;",
        );
        script.mode = ScriptMode::Async;

        assert!(
            page_vm
                .vm_mut()
                .accept_main_parser_async_module_script(owner, &script)?
        );
        assert!(
            page_vm.has_ready_parser_owned_document_script_action(),
            "an immediately-ready graph must notify the PendingScript watch installed before graph start"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true)
        );
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader)
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserAsyncInlineModuleExecuted)")?,
            "1"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async inline module",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async inline module lifecycle test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn main_document_lifecycle_sets_interactive_before_defer_and_loads_later() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-lifecycle.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval(
                r#"
                globalThis.__mainLifecycleEvents = [];
                document.addEventListener("readystatechange", () => {
                  __mainLifecycleEvents.push("ready:" + document.readyState);
                });
                document.addEventListener("DOMContentLoaded", () => {
                  __mainLifecycleEvents.push("dcl:" + document.readyState);
                });
                window.addEventListener("load", () => {
                  __mainLifecycleEvents.push("load:" + document.readyState);
                });
                "installed";
                "#,
            )
            .expect("main lifecycle listeners should install");
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main lifecycle test requires an owner");
        let defer = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            7001,
            "main-lifecycle-defer",
            Url::parse("https://example.com/main-lifecycle-defer.js").expect("defer script URL"),
            ScriptSource::Loaded(
                "__mainLifecycleEvents.push('defer:' + document.readyState);".to_owned(),
            ),
            ("data-lifecycle-test", "defer"),
        );
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(owner, defer, None, None, Default::default(),)
                .expect("parser-deferred PendingScript acceptance should succeed")
        );
        let defer_marker = page_vm
            .seal_main_parser_deferred_scripts(owner)
            .expect("parser-deferred work should install into after-parsing order");
        let interactive = page_vm
            .vm_mut()
            .finish_current_main_document_parsing(owner)
            .expect("parser EOF should produce one interactive action");

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_interactive(interactive),
            )
            .await
            .expect("interactive lifecycle turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("interactive events"),
            "ready:interactive"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, defer_marker)
            .await
            .expect("main defer script should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("defer events"),
            "ready:interactive|defer:interactive"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_domcontentloaded(owner),
            )
            .await
            .expect("main DOMContentLoaded turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("DOMContentLoaded events"),
            "ready:interactive|defer:interactive|dcl:interactive"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_window_load(owner),
            )
            .await
            .expect("main complete/load turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("complete/load events"),
            "ready:interactive|defer:interactive|dcl:interactive|ready:complete|load:complete"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_document_replacement_during_interactive_retires_old_lifecycle_actions() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-lifecycle-replacement.html")
                .expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval(
                r#"
                globalThis.__mainReplacementLifecycleEvents = [];
                document.addEventListener("readystatechange", () => {
                  __mainReplacementLifecycleEvents.push("ready:" + document.readyState);
                  if (document.readyState === "interactive") {
                    document.open();
                  }
                });
                "installed";
                "#,
            )
            .expect("replacement listener should install");
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main lifecycle test requires an owner");
        let interactive = page_vm
            .vm_mut()
            .finish_current_main_document_parsing(retired_owner)
            .expect("parser EOF should produce interactive action");

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_interactive(interactive),
            )
            .await
            .expect("interactive replacement turn should run");
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement must install a current owner");
        assert_ne!(retired_owner, current_owner);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainReplacementLifecycleEvents.join('|') + ':' + document.readyState")
                .expect("replacement state"),
            "ready:interactive:loading"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_domcontentloaded(retired_owner),
            )
            .await
            .expect("stale DOMContentLoaded work should be consumed");
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_window_load(retired_owner),
            )
            .await
            .expect("stale load work should be consumed");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainReplacementLifecycleEvents.join('|') + ':' + document.readyState")
                .expect("stale lifecycle state"),
            "ready:interactive:loading",
            "retired owner work must not mutate or dispatch lifecycle events on the replacement"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn empty_main_parser_deferred_seal_does_not_arm_owner_source() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/empty-parser-deferred.html").expect("document URL"),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("empty parser-deferred test requires a document owner");

        assert!(
            page_vm
                .seal_main_parser_deferred_scripts(task_owner)
                .is_none(),
            "empty EOF seal must not create a parser-deferred marker"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .main_parser_deferred_scripts_owner()
                .is_none(),
            "empty EOF seal must not arm a lifecycle source that can block DCL"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_dispatches_load_before_releasing_next_pending_script() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-order.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferEvents = []")
            .expect("defer event state should initialize");
        let first = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "first-defer",
            Url::parse("https://example.com/first-defer.js").expect("first script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferEvents.push('first-exec:' + document.currentScript.id);"
                    .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferEvents.push('first-load:' + (document.currentScript === null))",
            ),
        );
        let second = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            2,
            "second-defer",
            Url::parse("https://example.com/second-defer.js").expect("second script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferEvents.push('second-exec:' + document.currentScript.id);"
                    .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferEvents.push('second-load:' + (document.currentScript === null))",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("defer test requires a document owner");
        for script in [first, second] {
            assert!(
                page_vm
                    .vm_mut()
                    .claim_main_parser_deferred_script(
                        task_owner,
                        script,
                        None,
                        None,
                        Default::default(),
                    )
                    .expect("loaded classic defer should be accepted")
            );
        }
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "accepted classic-defer PendingScripts must own the main lifecycle gate"
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("defer queue should seal");
        assert!(
            !page_vm.has_pending_module_script_for_target_stage(),
            "classic defer lifecycle ownership must not enter the runtime-owned module lane"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "first classic defer")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferEvents.join('|')")
                .expect("first defer events should evaluate"),
            "first-exec:first-defer|first-load:true",
            "the first PendingScript must dispatch load with currentScript cleared before the next parser-deferred slot is released"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "the second PendingScript must retain its own lifecycle token"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "second classic defer")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferEvents.join('|')")
                .expect("all defer events should evaluate"),
            "first-exec:first-defer|first-load:true|second-exec:second-defer|second-load:true"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "consuming the final PendingScript must release the lifecycle gate"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_settles_script_reactions_before_load() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-checkpoint-order.html")
                .expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferCheckpointEvents = []")
            .expect("defer checkpoint state should initialize");
        let script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "checkpoint-defer",
            Url::parse("https://example.com/checkpoint-defer.js").expect("script URL"),
            ScriptSource::Loaded(
                r#"
globalThis.__mainParserDeferCheckpointEvents.push('script');
queueMicrotask(() => globalThis.__mainParserDeferCheckpointEvents.push('script-microtask'));
"#
                .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferCheckpointEvents.push('load'); queueMicrotask(() => globalThis.__mainParserDeferCheckpointEvents.push('load-microtask'))",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("defer checkpoint test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    script,
                    None,
                    None,
                    Default::default(),
                )
                .expect("loaded classic defer should be accepted")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("defer queue should seal");

        run_and_finish_ready_parser_deferred_task_for_test(
            &mut page_vm,
            &loader,
            "classic defer checkpoint ordering",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferCheckpointEvents.join('|')")
                .expect("defer checkpoint events should evaluate"),
            "script|script-microtask|load|load-microtask",
            "classic-defer evaluation reactions must settle before load and the selected parser task must then settle load reactions"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_load_replacement_retires_old_pending_queue() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-replacement.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferReplacementEvents = []")
            .expect("replacement event state should initialize");
        let first = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "replacement-defer",
            Url::parse("https://example.com/replacement-defer.js")
                .expect("replacement script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferReplacementEvents.push('execute');".to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferReplacementEvents.push('load'); document.open(); document.write(\"<!doctype html><html><body><main id='replacement'>replacement</main></body></html>\"); document.close();",
            ),
        );
        let stale = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            2,
            "stale-defer",
            Url::parse("https://example.com/stale-defer.js").expect("stale script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferReplacementEvents.push('stale-execute');".to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferReplacementEvents.push('stale-load')",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement test requires a document owner");
        for script in [first, stale] {
            assert!(
                page_vm
                    .vm_mut()
                    .claim_main_parser_deferred_script(
                        task_owner,
                        script,
                        None,
                        None,
                        Default::default(),
                    )
                    .expect("classic defer should be accepted")
            );
        }
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("replacement defer queue should seal");

        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "replacement classic defer",
        )
        .await;
        assert_ne!(
            page_vm.vm().current_main_document_task_owner(),
            Some(task_owner),
            "document.open from the load handler must rotate the document owner"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferReplacementEvents.join('|')")
                .expect("replacement events should evaluate"),
            "execute|load",
            "old parser-deferred work must stop after its completion event replaces the document"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.getElementById('replacement') !== null)")
                .expect("replacement document should evaluate"),
            "true"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .main_parser_deferred_scripts_owner()
                .is_none(),
            "replacement must disarm the retired parser-deferred owner source"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_execution_can_replace_the_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-execution-replacement.html")
                .expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferExecutionReplacementEvents = []")
            .expect("replacement event state should initialize");
        let script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "replacement-defer",
            Url::parse("https://example.com/replacement-defer.js")
                .expect("replacement script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferExecutionReplacementEvents.push('execute'); document.open(); document.write(\"<!doctype html><html><body><main id='replacement'>replacement</main></body></html>\"); document.close(); globalThis.__mainParserDeferExecutionReplacementEvents.push('after-open');"
                    .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferExecutionReplacementEvents.push('load')",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    script,
                    None,
                    None,
                    Default::default(),
                )
                .expect("classic defer should be accepted")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("replacement defer queue should seal");

        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "replacement classic defer execution",
        )
        .await;
        assert_ne!(
            page_vm.vm().current_main_document_task_owner(),
            Some(task_owner),
            "document.open from deferred execution must rotate the document owner"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferExecutionReplacementEvents.join('|')")
                .expect("replacement events should evaluate"),
            "execute|after-open",
            "replacement must prevent the retired script element load event from dispatching"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.getElementById('replacement') !== null)")
                .expect("replacement document should evaluate"),
            "true"
        );
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_source_failure_dispatches_typed_error_without_refetch() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut page_resource_queue, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/defer-failure.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferFailureEvents = []")
            .expect("failure event state should initialize");
        let failed = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "failed-defer",
            Url::parse("https://defer-failure.test/missing.js").expect("failed script URL"),
            ScriptSource::External,
            (
                "onerror",
                "globalThis.__mainParserDeferFailureEvents.push('error:' + (document.currentScript === null))",
            ),
        );
        let failed_node_id = failed.node_id;
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("failure test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    failed,
                    Some(crate::planning::SharedScriptSourceLoad::ready_err(
                        "prepared source failure",
                    )),
                    None,
                    Default::default(),
                )
                .expect("failed classic defer should be accepted before source start")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("failed defer queue should seal without waiting");
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "source failure must retain lifecycle ownership until its owner turn dispatches error"
        );
        loop {
            let wake = owner_wake_rx
                .recv()
                .await
                .expect("ready source failure must keep the Page owner wake route open");
            if wake.source_for_test()
                == crate::page_task_queue::RendererOwnerWakeSource::NetworkingTask
            {
                break;
            }
        }
        let source_failure = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)
            .expect("source terminal should arbitrate")
            .expect("source terminal should apply");
        assert_eq!(
            source_failure.action.source,
            RendererOwnerResourceActivitySource::MainParserDeferredClassicSource
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferFailureEvents.join('|')")
                .expect("pre-turn failure events should evaluate"),
            "",
            "resource completion must only update PendingScript state"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "failed classic defer")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferFailureEvents.join('|')")
                .expect("failure events should evaluate"),
            "error:true"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "source-failure completion must release its exact lifecycle token"
        );
        let run = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.node_id() == failed_node_id)
            .expect("source failure owner turn should record one failed run");
        assert!(matches!(run.outcome(), ScriptRunOutcome::Failed(_)));
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_parser_after_parsing_queue_orders_module_before_ready_classic_defer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/ordered-module.mjs").expect("module URL");
        let module =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9101, module_url.clone());
        let (classic_node, classic_handle) = {
            let runtime = &mut page_vm.vm_mut().document_runtime;
            let body = runtime
                .snapshot_document()
                .document_body_handle()
                .expect("test document body");
            let script_node = runtime.dom_host_mut().create_element("script");
            assert!(runtime.dom_host_mut().append_child(body, script_node));
            let handle = runtime.bind_document_write_owned_script_handle_for_node(script_node);
            (script_node, handle)
        };
        let mut classic = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9102,
            "globalThis.__orderedClassicDefer = 1;",
        );
        classic.node_id = classic_node;
        classic.host_script_handle = Some(classic_handle);
        classic.mode = ScriptMode::Defer;

        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("mixed parser-deferred test requires a current document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    classic.clone(),
                    None,
                    None,
                    Default::default(),
                )
                .expect("classic defer PendingScript should be accepted")
        );
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    module.clone(),
                    None,
                    None,
                    Default::default(),
                )
                .expect("module defer PendingScript should start its graph after acceptance")
        );
        let _marker = page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("mixed parser-deferred batch should install");
        page_vm
            .page_task_queue
            .extend_post_parse_work([PostParsePageOwnedWork::lifecycle_work(
                PostParseLifecycleWork::test_domcontentloaded(),
            )]);
        assert!(
            poll_post_parse_document_processing_action_for_test(&mut page_vm).is_none(),
            "pending earlier module must hold DCL without exposing later classic work"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedClassicDefer)")
                .expect("read later classic side effect while module is pending"),
            "undefined",
            "later ready classic defer must stay behind the earlier module PendingScript"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "mixed after-parsing order must not materialize broad ready work"
        );

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__orderedModuleDefer = 1; export const value = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("module graph completion should run")
                .is_some()
        );
        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "earlier module release")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedModuleDefer)")
                .expect("read earlier module side effect"),
            "1"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedClassicDefer)")
                .expect("read later classic before its owner turn"),
            "undefined",
            "one parser owner turn must release only the current document-order slot"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "later classic release")
            .await;
        let classic_run = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.node_id() == classic.node_id)
            .unwrap_or_else(|| {
                panic!(
                    "later classic owner turn should report a run: {:?}",
                    page_vm.report.runs
                )
            });
        assert!(
            matches!(classic_run.outcome(), ScriptRunOutcome::Executed),
            "later classic owner turn should execute, got {classic_run:?}"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedClassicDefer)")
                .expect("read later classic side effect"),
            "1"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .main_parser_deferred_scripts_owner()
                .is_none(),
            "consuming the final ordered slot must disarm the document owner source"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "mixed classic/module order must release every lifecycle token"
        );
        assert!(matches!(
            poll_post_parse_document_processing_action_for_test(&mut page_vm),
            Some(crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.is_domcontentloaded_task()
        ));
    })
    .await;
}
