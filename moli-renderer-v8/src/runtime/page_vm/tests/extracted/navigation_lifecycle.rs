use super::*;

#[tokio::test]
async fn page_vm_child_script_event_created_nested_navigation_precedes_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-event-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childScriptEventNestedEvents.push("module:" + (globalThis === self));
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_source,
            module_followup_sources,
            events_after_script_event,
            nested_script_ready_source,
            events_after_nested_script,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-event-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childScriptEventNestedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>parent.__childScriptEventNestedEvents.push("before:" + (globalThis === self));<\/script>
    <script id="event-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("event-module").addEventListener("load", () => {{
        parent.__childScriptEventNestedEvents.push("script-load");
        const nested = document.createElement("iframe");
        nested.srcdoc = "<script>parent.parent.__childScriptEventNestedEvents.push('nested-script:' + (globalThis === self));<\\/script>";
        document.body.appendChild(nested);
      }});
      parent.__childScriptEventNestedEvents.push("after:" + String(globalThis.__childScriptEventNestedValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let startup_sources =
                    drive_child_frame_task_sources_until_resource_completion_ready(
                        &mut page_vm,
                        8,
                    )
                    .await;
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::DocumentScriptReady),
                    "child module startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
                );
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
                    "child module startup should start the parser module root fetch from its typed source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childScriptEventNestedEvents.join('|')")?,
                    "before:true|after:undefined",
                    "parser should continue past the child module while the root fetch is pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child parser module completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child parser module completion sender should remain open"
                    );
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child script-event module terminal fanout",
                )
                .await;
                let mut module_followup_sources = Vec::new();
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childScriptEventNestedEvents.join('|')")?,
                    "before:true|after:undefined",
                    "ModuleScriptTerminal should queue graph-ready work without running the module or nested iframe"
                );
                module_followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child script-event module execution",
                    )
                    .await,
                );
                let events_after_script_event = page_vm
                    .vm_mut()
                    .eval("__childScriptEventNestedEvents.join('|')")?;

                module_followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "nested iframe navigation created during child script event",
                    )
                    .await,
                );
                module_followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child script-event DOMContentLoaded transition",
                    )
                    .await,
                );
                let nested_script_ready_source = Some(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "nested iframe ready work created during child script event",
                    )
                    .await,
                );
                let events_after_nested_script = page_vm
                    .vm_mut()
                    .eval("__childScriptEventNestedEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    completion_source,
                    module_followup_sources,
                    events_after_script_event,
                    nested_script_ready_source,
                    events_after_nested_script,
                ))
            })
            .await
            .expect("child script event nested ready-work source test should run");

        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            module_followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "module execution, nested navigation commit, and DOMContentLoaded should remain separate child-frame turns after the typed terminal fanout"
        );
        assert_eq!(
            events_after_script_event, "before:true|after:undefined|module:true|script-load",
            "module evaluation should dispatch the script load event before nested parser work"
        );
        assert_eq!(
            nested_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "nested parser ready work should enter DocumentScriptReady after its owner transaction commits"
        );
        assert_eq!(
            events_after_nested_script,
            "before:true|after:undefined|module:true|script-load|nested-script:true"
        );

        server
            .await
            .expect("child script event nested ready-work server should finish");
    })
    .await;
}
#[tokio::test]
async fn page_vm_resync_child_commits_navigation_before_document_script_ready() {
    let page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();

    let resync = run_on_page_vm_local_executor(local_executor, async move {
        let mut page_vm = page_vm;
        page_vm.vm_mut().eval_with_child_record_sync(
            r#"
(() => {
  globalThis.__resyncReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => __resyncReadyEvents.push("frame-load");
  frame.srcdoc = `<script>parent.__resyncReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
  body.appendChild(frame);
})()
"#,
        )?;
        let host_load_pending_after_resync = page_vm
            .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "resynced child navigation commit",
        )
        .await;
        run_expected_child_realm_materialization_for_wait(&mut page_vm, "resynced child realm")
            .await;
        let script_ready_source = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await;
        let events_after_script_ready = page_vm.vm_mut().eval("__resyncReadyEvents.join('|')")?;
        let host_load_source = Some(
            run_child_interactive_domcontentloaded_then_host_load_for_wait(
                &mut page_vm,
                "resynced child iframe load",
            )
            .await,
        );
        let events_after_host_load = page_vm.vm_mut().eval("__resyncReadyEvents.join('|')")?;

        Ok::<_, anyhow::Error>((
            host_load_pending_after_resync,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ))
    });
    let (
        host_load_pending_after_resync,
        script_ready_source,
        events_after_script_ready,
        host_load_source,
        events_after_host_load,
    ) = run_page_vm_async_test(resync)
        .await
        .expect("resync ready-work source test should run");

    assert!(
        !host_load_pending_after_resync,
        "ScriptVm resync should not wake HostLoad while parser-ready work is pending"
    );
    assert_eq!(
        script_ready_source,
        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
        "ScriptVm resync should expose parser-ready work after the child navigation commits"
    );
    assert_eq!(
        events_after_script_ready, "child-script:true",
        "DocumentScriptReady should run the child script without dispatching iframe load"
    );
    assert_eq!(
        host_load_source,
        Some(ChildFrameSemanticTurnKind::HostLoad),
        "iframe load should remain a later HostLoad source"
    );
    assert_eq!(
        events_after_host_load, "child-script:true|frame-load",
        "HostLoad should dispatch iframe load after the script-ready turn"
    );
}
#[tokio::test]
async fn page_vm_nested_frame_finish_releases_parent_document_lifecycle() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__nestedLifecycleEvents = [];
  const outer = document.createElement("iframe");
  outer.onload = () => __nestedLifecycleEvents.push("outer-load");
  outer.srcdoc = `<!doctype html><body><script>
    parent.__nestedLifecycleEvents.push("outer-script");
    const nested = document.createElement("iframe");
    nested.onload = () => parent.parent.__nestedLifecycleEvents.push("nested-load");
    nested.srcdoc = \`<!doctype html><script>parent.parent.__nestedLifecycleEvents.push("nested-script")<\\/script>\`;
    document.body.appendChild(nested);
  <\/script></body>`;
  document.body.appendChild(outer);
})()
"#,
        )?;

        let mut turns = Vec::new();
        for _ in 0..32 {
            let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                break;
            };
            let events = page_vm
                .vm_mut()
                .eval("__nestedLifecycleEvents.join('|')")?;
            let outer_loaded = events.contains("outer-load");
            turns.push((source, events));
            if outer_loaded {
                break;
            }
        }

        let nested_load_turn = turns
            .iter()
            .position(|(_, events)| events.contains("nested-load"))
            .expect("nested frame should finish");
        let outer_load_turn = turns
            .iter()
            .position(|(_, events)| events.contains("outer-load"))
            .expect("parent frame should finish after its descendant");
        assert_eq!(turns[nested_load_turn].0, ChildFrameSemanticTurnKind::HostLoad);
        assert_eq!(turns[outer_load_turn].0, ChildFrameSemanticTurnKind::HostLoad);
        assert!(
            nested_load_turn < outer_load_turn,
            "parent load must remain blocked until the exact descendant frame finishes: {turns:?}"
        );
        assert!(
            !turns[nested_load_turn].1.contains("outer-load"),
            "descendant HostLoad must only enqueue the parent lifecycle wake: {turns:?}"
        );
        assert!(
            turns[nested_load_turn + 1..outer_load_turn]
                .iter()
                .any(|(source, _)| *source == ChildFrameSemanticTurnKind::DocumentLifecycle),
            "parent complete must be a later DocumentLifecycle turn before its HostLoad: {turns:?}"
        );
        assert_eq!(
            turns[outer_load_turn].1,
            "outer-script|nested-script|nested-load|outer-load"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("nested lifecycle source-turn proof should run");
}
#[tokio::test]
async fn page_vm_host_load_commits_nested_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-load-creates-nested.html",
            "HTTP/1.1 200 OK",
            r#"
<script>
parent.__hostLoadNestedEvents.push("child-script");
onload = () => {
  parent.__hostLoadNestedEvents.push("child-load");
  const nested = document.createElement("iframe");
  nested.onload = () => parent.__hostLoadNestedEvents.push("nested-load");
  nested.srcdoc = `<script>parent.parent.__hostLoadNestedEvents.push("nested-script:" + (globalThis === self));<\/script>`;
  document.body.appendChild(nested);
};
</script>
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            bootstrap_source,
            completion_source,
            host_load_pending_after_completion,
            script_ready_source,
            events_after_script_ready,
            interactive_source,
            host_load_source,
            events_after_host_load,
            nested_script_ready_source,
            events_after_nested_script_ready,
            nested_interactive_source,
            nested_host_load_source,
            events_after_nested_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child-load-creates-nested.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__hostLoadNestedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;

                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(arrived, "child document completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                let host_load_pending_after_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child host-load realm prerequisite",
                )?;

                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;
                let interactive_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child window load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "nested child navigation created during HostLoad",
                )
                .await;
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "nested child realm prerequisite",
                )?;
                let nested_script_ready_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_nested_script_ready = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;
                let nested_interactive_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let nested_host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "nested child iframe load",
                    )
                    .await,
                );
                let events_after_nested_host_load = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    bootstrap_source,
                    completion_source,
                    host_load_pending_after_completion,
                    script_ready_source,
                    events_after_script_ready,
                    interactive_source,
                    host_load_source,
                    events_after_host_load,
                    nested_script_ready_source,
                    events_after_nested_script_ready,
                    nested_interactive_source,
                    nested_host_load_source,
                    events_after_nested_host_load,
                ))
            })
            .await
            .expect("host-load nested ready-work source test should run");

        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "bootstrap child frame turn should commit navigation before lifecycle delivery"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildDocument
        );
        assert!(
            !host_load_pending_after_completion,
            "child document completion should not leave HostLoad pending before document-script ready work runs"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "child document completion follow-up should run document-script ready work"
        );
        assert_eq!(events_after_script_ready, "child-script");
        assert_eq!(
            interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "child parser EOF should become interactive before window load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "child window load should dispatch on HostLoad"
        );
        assert_eq!(
            events_after_host_load, "child-script|child-load",
            "HostLoad should create the nested frame without running its script inline"
        );
        assert_eq!(
            nested_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "nested parser work created during HostLoad should follow its navigation commit"
        );
        assert_eq!(
            events_after_nested_script_ready, "child-script|child-load|nested-script:true",
            "nested script should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            nested_interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "nested parser EOF should become interactive before nested HostLoad"
        );
        assert_eq!(
            nested_host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "nested iframe load should still remain a separate HostLoad turn"
        );
        assert_eq!(
            events_after_nested_host_load,
            "child-script|child-load|nested-script:true|nested-load"
        );

        server
            .await
            .expect("host-load nested ready-work server should finish");
    })
    .await;
}
#[tokio::test(flavor = "current_thread")]
async fn main_runtime_module_tla_releases_lifecycle_at_evaluation_start() {
    #[derive(Debug)]
    enum RuntimeModuleTurn {
        RuntimeScriptAdmission,
        ParserAsyncModuleAdmission,
        ModuleReaction,
        NativeModuleOwner,
        DynamicModuleJob,
        RuntimeScriptContinuation,
        RuntimeOwnedModuleContinuation,
        ParserOwnedModuleContinuation,
        PostParseWork,
    }

    async fn run_one_runtime_module_turn(
        page_vm: &mut PageVm,
        loader: &crate::network::ResourceRequestClient,
    ) -> anyhow::Result<Option<RuntimeModuleTurn>> {
        if page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ModuleReaction,
                loader,
            )
            .await?
        {
            return Ok(Some(RuntimeModuleTurn::ModuleReaction));
        }
        if let Some(outcome) = page_vm
            .run_page_main_document_runtime_body_for_test(loader)
            .await?
        {
            let turn = match outcome.action.kind() {
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission => {
                    RuntimeModuleTurn::RuntimeScriptAdmission
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::ParserAsyncModuleAdmission => {
                    RuntimeModuleTurn::ParserAsyncModuleAdmission
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation => {
                    RuntimeModuleTurn::RuntimeScriptContinuation
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::DynamicModuleJob => {
                    RuntimeModuleTurn::DynamicModuleJob
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation => {
                    RuntimeModuleTurn::RuntimeOwnedModuleContinuation
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation => {
                    RuntimeModuleTurn::ParserOwnedModuleContinuation
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent => {
                    RuntimeModuleTurn::NativeModuleOwner
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork => {
                    RuntimeModuleTurn::PostParseWork
                }
            };
            return Ok(Some(turn));
        }
        Ok(None)
    }

    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/runtime-tla.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .document_runtime
            .note_dom_content_loaded_dispatched();
        page_vm
            .vm_mut()
            .eval(
                r#"
(() => {
  globalThis.__runtimeTlaEvents = [];
  const script = document.createElement("script");
  script.type = "module";
  script.onload = () => __runtimeTlaEvents.push("script-load");
  script.onerror = () => __runtimeTlaEvents.push("script-error");
  script.textContent = `
    globalThis.__runtimeTlaStarted = 1;
    await new Promise(resolve => { globalThis.__resolveRuntimeTla = resolve; });
    globalThis.__runtimeTlaFinished = 1;
  `;
  document.body.appendChild(script);
})()
"#,
            )
            .expect("install runtime TLA module");
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main document owner");

        let mut reached_evaluation_start = false;
        let mut progress_actions = Vec::new();
        for _ in 0..32 {
            let progressed = run_one_runtime_module_turn(&mut page_vm, &loader).await?;
            let started = page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaStarted)")?;
            let load_delay = page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner);
            let events = page_vm.vm_mut().eval("__runtimeTlaEvents.join('|')")?;
            if started == "1" && load_delay == Some(false) {
                assert_eq!(
                    events, "",
                    "inline module evaluation must not dispatch script load or error"
                );
                reached_evaluation_start = true;
                break;
            }
            let Some(progressed) = progressed else {
                let ready_module_continuation =
                    page_vm.has_ready_runtime_owned_module_script_continuation_work();
                let ready_dynamic_module_job = page_vm.vm_mut().has_ready_dynamic_module_job();
                let runnable_runtime_work = page_vm
                    .vm_mut()
                    .has_runnable_runtime_script_work_now();
                panic!(
                    "runtime module owner loop stalled before evaluation-start completion: started={started} load_delay={load_delay:?} events={events:?} progress_actions={progress_actions:?} ready_module_continuation={ready_module_continuation} ready_dynamic_module_job={ready_dynamic_module_job} runnable_runtime_work={runnable_runtime_work}",
                );
            };
            progress_actions.push(progressed);
        }
        assert!(
            reached_evaluation_start,
            "runtime TLA evaluation start must complete the script and release its lifecycle binding"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaFinished)")?,
            "undefined",
            "the module body should still be suspended on its TLA promise"
        );
        let run_count_at_evaluation_start = page_vm
            .report
            .runs
            .iter()
            .filter(|run| run.url().as_str().contains("runtime-tla"))
            .count();
        assert_eq!(
            run_count_at_evaluation_start, 1,
            "evaluation start must apply runtime module script completion exactly once"
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__resolveRuntimeTla(); 'resolved'")?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ModuleReaction, &loader)
                .await?,
            "runtime TLA fulfillment should enqueue one typed module reaction"
        );
        for _ in 0..32 {
            let progressed = run_one_runtime_module_turn(&mut page_vm, &loader).await?;
            if page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaFinished)")?
                == "1"
            {
                break;
            }
            assert!(
                progressed.is_some(),
                "runtime module owner loop stalled after TLA resolve"
            );
        }
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaFinished)")?,
            "1"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url().as_str().contains("runtime-tla"))
                .count(),
            run_count_at_evaluation_start,
            "TLA fulfillment must not redispatch runtime module script completion"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__runtimeTlaEvents.join('|')")?,
            "",
            "inline module fulfillment must not dispatch script load or error"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime TLA lifecycle test should run");
}
#[tokio::test(flavor = "current_thread")]
async fn dcl_lifecycle_yields_parser_deferred_source_wait_to_page_vm_resource_queue() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-arrival.html").expect("document URL"),
        );
        let script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "deferred-arrival",
            Url::parse("https://example.com/deferred-arrival.js").expect("script URL"),
            ScriptSource::External,
            ("onload", "globalThis.__deferredArrivalLoaded = true"),
        );
        let (source_ready_tx, source_ready_rx) = tokio::sync::oneshot::channel();
        let source_load = SharedScriptSourceLoad::spawn_for_test(async move {
            source_ready_rx
                .await
                .expect("test should release the deferred source");
            Ok("globalThis.__deferredArrivalExecuted = true;".to_owned())
        });
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("defer test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    script,
                    Some(source_load),
                    None,
                    Default::default(),
                )
                .expect("pending classic defer should be accepted")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("defer queue should seal");
        let lifecycle_driver = page_vm
            .vm()
            .resume_post_parse_lifecycle_driver_for_existing_queue(
                PageVmInitStage::DomContentLoaded,
            );

        let advance = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = &mut page_vm;
            vm.as_mut()
                .expect("page vm must retain a ScriptVm")
                .advance_post_parse_lifecycle(
                    &loader,
                    page_task_queue,
                    report,
                    lifecycle_driver,
                    None,
                )
                .await
                .expect("DCL driver should report its blocked state")
        };
        assert!(
            matches!(advance, PostParseLifecycleAdvance::AwaitProgress),
            "the document driver must yield future resource progress to PageVm arbitration"
        );

        page_vm
            .vm_mut()
            .eval(
                "globalThis.__dclBlockedTimerRan = false; \
                 setTimeout(() => { globalThis.__dclBlockedTimerRan = true; }, 0);",
            )
            .expect("ready timer should be registered");
        assert!(
            page_vm.vm().has_ready_timeout(),
            "the regression requires a ready timer competing with the deferred source"
        );
        let progress_arrived = {
            let progress_wait =
                page_vm.wait_for_lifecycle_blocking_page_work_arrival_without_timeout(false);
            tokio::pin!(progress_wait);
            tokio::select! {
                biased;
                result = &mut progress_wait => {
                    panic!("a DCL-ineligible timer must not satisfy the lifecycle wait: {result}")
                }
                _ = std::future::ready(()) => {}
            }
            source_ready_tx
                .send(())
                .expect("deferred source receiver should remain alive");
            progress_wait.await
        };
        assert!(
            progress_arrived,
            "deferred source terminal must wake the PageVm resource queue"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__dclBlockedTimerRan)")
                .expect("timer marker should remain readable"),
            "false",
            "waiting for DCL progress must not execute a generic timer task"
        );
        let deferred_source = run_next_resource_completion_as_typed_page_turn(&mut page_vm)
            .expect("deferred source terminal should apply");
        assert_eq!(
            deferred_source.action.source,
            RendererOwnerResourceActivitySource::MainParserDeferredClassicSource
        );

        let advance = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = &mut page_vm;
            vm.as_mut()
                .expect("page vm must retain a ScriptVm")
                .advance_post_parse_lifecycle(
                    &loader,
                    page_task_queue,
                    report,
                    lifecycle_driver,
                    None,
                )
                .await
                .expect("resource completion should resume the DCL driver")
        };
        let PostParseLifecycleAdvance::PageOwnedTask(mut task) = advance else {
            panic!("ready parser-deferred work should become a page-owned task");
        };
        assert!(
            task.take_work_for_execution()
                .main_parser_deferred_scripts_owner()
                .is_some(),
            "the resumed owner turn must execute the exact parser-deferred queue"
        );
    })
    .await;
}
