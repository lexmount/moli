use super::*;

#[test]
fn child_parser_script_document_close_drains_restored_parent_input() {
    let mut vm = new_storage_test_vm("https://child-parser-close.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write(
    '<script>document.close();<\/script>' +
    '<main id="tail-after-parser-close">tail</main>'
  );
  return String(!!childDocument.getElementById("tail-after-parser-close"));
})()
"#,
        )
        .expect("document.close() from a parser script must drain the restored parent input");

    assert_eq!(result, "true");
}
#[tokio::test]
async fn srcdoc_child_parser_write_script_drains_restored_parent_input() {
    let mut vm = new_storage_test_vm("https://srcdoc-write-drain.test/");
    vm.eval(
        r##"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc =
    '<body><script>document.title = "script-ran"; ' +
    "document.write('<b id=\"inserted\">i</b>'); " +
    '</' + 'script>' +
    '<main id="tail-after-write">tail</main></body>';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"##,
    )
    .expect("srcdoc write frame fixture should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "srcdoc write frame should commit before its parser script",
    )
    .await;
    while vm.has_ready_child_frame_semantic_turn_for_test(
        ChildFrameSemanticTurnKind::RealmMaterialization,
    ) {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "srcdoc write child realm should materialize before its parser script"
        );
    }
    while vm.has_ready_child_frame_semantic_turn_for_test(
        ChildFrameSemanticTurnKind::DocumentScriptReady,
    ) {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "srcdoc write parser script should run on DocumentScriptReady"
        );
    }
    run_child_document_lifecycle_and_host_load_for_test(&mut vm, "srcdoc write frame").await;

    let result = vm
        .eval(
            r#"
(() => {
  const childDocument = document.querySelector("iframe").contentDocument;
  return [
    childDocument.title,
    !!childDocument.getElementById("inserted"),
    !!childDocument.getElementById("tail-after-write")
  ].join("|");
})()
"#,
        )
        .expect("a finite child parser must drain the parent input restored after a write script");

    assert_eq!(result, "script-ran|true|true");
}
#[tokio::test]
async fn child_parser_resumes_write_queued_during_external_script_block() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm = new_storage_test_vm_with_loader("https://child-parent-write-block.test/", &loader);

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script src="data:text/javascript,globalThis.__childDataScriptRan%3Dtrue"><\/script>
    <main id="tail-after-block">tail</main>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("parent write block frame fixture should evaluate");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "parent write block frame should commit"
    );
    while vm.has_ready_child_frame_semantic_turn_for_test(
        ChildFrameSemanticTurnKind::RealmMaterialization,
    ) {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "parent write block child realm should materialize"
        );
    }

    vm.eval(
        r#"
(() => {
  const frame = document.querySelector("iframe");
  frame.contentDocument.write('<b id="from-parent">p</b>');
})()
"#,
    )
    .expect("a parent write during the child parser suspension should evaluate");

    while vm
        .run_next_child_frame_semantic_turn_for_test()
        .await
        .is_some()
    {
        // Drain the data-script source load, its execution, and the parser
        // resume that must consume the parent-queued write input.
    }
    let mut lifecycle_turns = 0;
    while lifecycle_turns < 8
        && vm
            .run_child_frame_task_source_once_for_test(
                ChildFrameSemanticTurnKind::DocumentLifecycle,
            )
            .await
    {
        lifecycle_turns += 1;
    }
    let _ = vm
        .run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
        .await;

    let result = vm
        .eval(
            r#"
(() => {
  const childDocument = document.querySelector("iframe").contentDocument;
  return [
    !!childDocument.getElementById("from-parent"),
    !!childDocument.getElementById("tail-after-block")
  ].join("|");
})()
"#,
        )
        .expect("resuming a child parser with parent-queued write input must drain all input");

    assert_eq!(result, "true|true");
}
#[tokio::test]
async fn parser_classic_frame_script_job_executes_in_child_realm() {
    let mut vm = new_storage_test_vm("https://child-classic-job.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser classic job setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child parser-classic job setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child parser-classic job setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let job = vm
        ._context_host
        .borrow()
        .frame_owner_child_parser_classic_script_job(
            child_handle,
            None,
            "globalThis.__parserClassicFrameJob = globalThis === self ? 31 : -1;".to_owned(),
        )
        .expect("child owner should build ParserClassic job");
    vm.exec_frame_script_job(job)
        .expect("ParserClassic job should execute in child realm");
    let observed = vm
        .eval_in_frame_realm(owner_realm_id, "String(globalThis.__parserClassicFrameJob)")
        .expect("child parser classic job side effect should be visible in child realm");
    assert_eq!(observed, "31");
    let parent_observed = vm
        .eval("String(globalThis.__parserClassicFrameJob)")
        .expect("parent realm should evaluate");
    assert_eq!(parent_observed, "undefined");
}
#[tokio::test]
async fn child_inline_classic_script_moved_from_original_document_is_skipped() {
    let mut vm = new_storage_test_vm("https://child-inline-classic-moved.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childClassicMovedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicMovedEvents.push("load");
  frame.srcdoc = `
    <script id="stale-inline">
      parent.__childClassicMovedEvents.push("stale:" + document.currentScript.id);
      globalThis.__staleInlineRan = true;
    <\/script>
    <script id="after-stale-inline">
      parent.__childClassicMovedEvents.push("after:" + document.currentScript.id);
      globalThis.__afterStaleInlineRan = true;
    <\/script>
  `;
  globalThis.__childClassicMovedFrame = frame;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child moved inline classic setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child moved inline classic srcdoc should commit before moving the pending script",
    )
    .await;
    vm.eval(
        r#"
(() => {
  const stale = __childClassicMovedFrame.contentDocument.getElementById("stale-inline");
  (document.body || document.documentElement || document).appendChild(document.adoptNode(stale));
})()
"#,
    )
    .expect("pending child inline classic script should move after srcdoc commit");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "moved stale inline classic work should be consumed by DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("__childClassicMovedEvents.join('|')")
            .expect("child moved inline classic events after stale turn should evaluate"),
        "",
        "stale moved script should be dropped without running and without firing iframe load"
    );
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "parser continuation after moved stale script should run from DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("__childClassicMovedEvents.join('|')")
            .expect("child moved inline classic events before HostLoad should evaluate"),
        "after:after-stale-inline",
        "parser continuation should run the next inline classic script without firing iframe load"
    );
    for transition in ["DOMContentLoaded", "complete"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("moved child classic should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "iframe load should dispatch from the later HostLoad source turn",
    )
    .await;

    assert_eq!(
        vm.eval("__childClassicMovedEvents.join('|')")
            .expect("child moved inline classic events should evaluate"),
        "after:after-stale-inline|load"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child moved inline classic script should materialize a child realm");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__staleInlineRan) + '|' + String(globalThis.__afterStaleInlineRan)"
        )
        .expect("child moved inline classic side effects should evaluate"),
        "undefined|true"
    );
}
#[tokio::test]
async fn child_external_classic_script_load_executes_as_frame_script_job() {
    let (script_url, request_path_rx, server) =
        spawn_child_external_classic_frame_script_job_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://child-external-classic-driver.test/",
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childExternalClassicJobEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childExternalClassicJobFrame = frame;
  frame.onload = () => globalThis.__childExternalClassicJobEvents.push("load");
  frame.srcdoc = `
    <script id="external-classic" src="{script_url}"><\/script>
    <script id="after-external-classic">
      parent.__childExternalClassicJobEvents.push(
        "inline-current:" + document.currentScript.id
      );
      parent.__childExternalClassicJobEvents.push(
        "inline:" + globalThis.__childExternalClassicValue
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
    ))
    .expect("child external classic frame job setup should evaluate");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child external classic srcdoc should commit before source loading",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
            &loader,
        )
        .await
        .expect("child classic source-load task should use the selected-task dispatcher"),
        "child external classic source load should start from the classic source-load turn"
    );

    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("pending child external classic events should evaluate"),
        "",
        "pending external classic script must block later inline script and child load"
    );

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child external classic completion",
    )
    .await;
    assert!(
        vm.has_pending_child_frame_realm_materialization(),
        "the accepted external script must retain one exact-realm prerequisite"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader,
        )
        .await
        .expect("child realm materialization should use the selected-task dispatcher"),
        "the typed realm turn must materialize the committed Document realm"
    );
    assert!(
        !vm.has_pending_child_frame_realm_materialization(),
        "the exact committed realm request must be settled after its typed turn"
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "realm completion must promote the external script into typed DocumentScriptReady",
    )
    .await;

    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic events before HostLoad should evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load",
        "the parser-blocking load event must ignore document.open() and finish without firing iframe load"
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child parser continuation should run from the next DocumentScriptReady turn",
    )
    .await;
    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic parser-continuation events should evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load|inline-current:after-external-classic|inline:73",
        "ignored document.open() must leave the parser owner alive for the following inline script"
    );
    for transition in ["DOMContentLoaded", "complete"] {
        run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
            &mut vm,
            &loader,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child external classic should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "iframe load should dispatch from the later HostLoad source turn",
    )
    .await;

    assert_eq!(
        request_path_rx
            .await
            .expect("child external classic server should report request path"),
        "/child-classic.js"
    );
    server
        .await
        .expect("child external classic test server should finish");

    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic events should evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load|inline-current:after-external-classic|inline:73|load"
    );
    assert_eq!(
        vm.eval("String(__childExternalClassicJobFrame.contentDocument.currentScript)")
            .expect("child external classic currentScript cleanup should evaluate"),
        "null"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child external classic script should materialize a child realm");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let (task_owner, owner, script_handle) = {
        let host = vm._context_host.borrow();
        let owner = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child owner should expose a current owner snapshot");
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref() == Some("external-classic")
            })
            .expect("external classic script handle should exist");
        (
            crate::frame_owner_model::FrameDocumentTaskOwner::new(
                owner.scheduler_lane_id,
                owner.local_window_id,
                owner.document_id,
            ),
            crate::frame_owner_model::FrameDocumentOwner::new(
                owner.local_window_id,
                owner.document_id,
            ),
            script_handle,
        )
    };
    let stale_task_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
        task_owner.scheduler_lane_id,
        task_owner.local_window_id,
        crate::frame_owner_model::DocumentId(task_owner.document_id.0 + 1),
    );
    assert!(
        super::child_document_event::ChildDocumentEventOwner::new(&mut vm)
            .dispatch_script_element_event_for_parts_selected_task_body(
                stale_task_owner,
                owner_realm_id,
                script_handle,
                crate::frame_owner_model::FrameDocumentScriptElementEventKind::Load,
            )
            .is_err(),
        "script element event helper with stale owner token should not dispatch"
    );
    let stale_event = crate::frame_owner_model::FrameDocumentScriptElementEvent {
        child_handle,
        owner: crate::frame_owner_model::FrameDocumentOwner::new(
            owner.local_window_id,
            crate::frame_owner_model::DocumentId(owner.document_id.0 + 1),
        ),
        script_handle,
        kind: crate::frame_owner_model::FrameDocumentScriptElementEventKind::Load,
    };
    assert!(
        super::child_document_event::ChildDocumentEventOwner::new(&mut vm)
            .dispatch_script_element_event(stale_event)
            .is_err(),
        "script element event with stale owner token should not dispatch"
    );
    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic events should still evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load|inline-current:after-external-classic|inline:73|load",
        "stale owner-token event must not fire the script load listener again"
    );
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childExternalClassicValue)"
        )
        .expect("child external classic side effect should be visible in child realm"),
        "73"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childExternalClassicValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
}
#[tokio::test]
async fn child_module_producer_boundaries_require_exact_task_owner() {
    let mut vm = new_storage_test_vm("https://child-module-attribution.test/");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "module-attribution-child";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child module attribution fixture should install an iframe");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child module attribution",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child module attribution");
    let (child_handle, realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child module attribution realm should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let owner = current_single_child_document_owner_for_test(&vm, "child module attribution");
    let request_url =
        Url::parse("https://child-module-attribution.test/root.js").expect("module URL");

    let (exact_target, exact_network_attribution) = vm
        ._context_host
        .borrow()
        .capture_child_module_fetch_producer_for_child(
            child_handle,
            owner,
            realm_id,
            request_url.clone(),
        )
        .expect("exact child module owner should capture producer attribution");
    assert_eq!(exact_target.child_handle(), child_handle);
    assert_eq!(exact_target.task_owner(), owner);
    assert_eq!(exact_target.realm_id(), realm_id);
    assert_eq!(exact_network_attribution.request_url(), &request_url);
    assert_eq!(
        exact_network_attribution.document_url().as_str(),
        "about:blank"
    );
    assert!(exact_network_attribution.frame_id().is_some());

    let stale_lane_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
        crate::frame_owner_model::FrameSchedulerLaneId(owner.scheduler_lane_id.0 + 1),
        owner.local_window_id,
        owner.document_id,
    );
    assert!(
        vm._context_host
            .borrow()
            .capture_child_module_fetch_producer_for_child(
                child_handle,
                stale_lane_owner,
                realm_id,
                request_url.clone(),
            )
            .is_none(),
        "producer attribution must not collapse task owner to local-window/document IDs"
    );
    assert!(
        vm._context_host
            .borrow()
            .current_child_module_fetch_target_for_realm(stale_lane_owner, realm_id)
            .is_none(),
        "dependency target lookup must reject the same lane-only stale owner"
    );
    assert!(
        vm._context_host
            .borrow()
            .capture_child_module_fetch_producer_for_child(
                child_handle,
                owner,
                crate::frame_owner_model::FrameRealmId(realm_id.0 + 1),
                request_url.clone(),
            )
            .is_none(),
        "producer lookup must reject an exact task owner paired with the wrong realm"
    );

    let pending_script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: request_url.clone(),
        base_url: request_url.clone(),
        initiator_url: request_url,
        host_script_handle: None,
    };
    let document_owner = owner.document_owner();
    vm._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_module_script(document_owner, &pending_script);
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(document_owner),
        1
    );
    assert!(
        !vm._context_host
            .borrow_mut()
            .cancel_child_document_script_work_if_current(child_handle, stale_lane_owner),
        "a stale module-start failure must not cancel replacement Document work"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(document_owner),
        1,
        "rejecting stale failure cleanup must preserve the current Document's module work"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .cancel_child_document_script_work_if_current(child_handle, owner),
        "the same failure cleanup must still cancel work for its exact current Document"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(document_owner),
        0
    );
}
#[tokio::test]
async fn child_parser_module_ready_lane_evaluates_compiled_graph_in_frame_realm() {
    let mut vm = new_storage_test_vm("https://child-parser-module-eval.test/");

    vm.eval_with_child_record_sync(
        r#"
(() => {
  globalThis.__childParserModuleEvalEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childParserModuleEvalFrame = frame;
  frame.srcdoc = "";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser module eval setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "empty srcdoc child should commit about:srcdoc before HostLoad",
    )
    .await;
    assert_eq!(
        vm.eval("__childParserModuleEvalFrame.contentDocument.URL")
            .expect("empty srcdoc child URL should evaluate"),
        "about:srcdoc"
    );

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child parser module eval should materialize a child realm");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const marker = document.createElement("script");
  marker.id = "eval-module-target";
  marker.type = "application/json";
  marker.addEventListener("load", () => parent.__childParserModuleEvalEvents.push("script-load"));
  marker.addEventListener("error", () => parent.__childParserModuleEvalEvents.push("script-error"));
  body.appendChild(marker);
  parent.__childParserModuleEvalEvents.push("realm-ready");
})()
"#,
    )
    .expect("child parser module eval child-realm setup should evaluate");
    assert_eq!(
        vm.eval("__childParserModuleEvalEvents.join('|')")
            .expect("child parser module setup events should evaluate"),
        "realm-ready"
    );
    let (document_owner, script_handle) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child frame should expose a current owner snapshot");
        let document_owner = crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        );
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref()
                    == Some("eval-module-target")
            })
            .expect("eval module marker script handle should exist");
        (document_owner, script_handle)
    };
    let task_owner = vm
        ._context_host
        .borrow()
        .current_child_module_route_task_owner(document_owner, owner_realm_id)
        .expect("child frame should expose current document task owner");
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-parser-module-eval.test/module.js").expect("module url"),
    );
    let metadata = crate::module_runtime::ModuleFetchMetadata::default();
    let source = crate::module_runtime::ModuleSource::text(
        r#"parent.__childParserModuleEvalEvents.push("module:" + (globalThis === self));
globalThis.__childParserModuleEvalValue = 144;"#
            .to_owned(),
    );
    let (record, identity) = vm
        .compile_native_module_record_for_frame_realm(
            owner_realm_id,
            key.clone(),
            &source,
            key.url(),
            &metadata,
        )
        .expect("child frame module record should compile");
    let mut document_modulator = vm
        .child_document_modulator_store
        .take_or_create_document_modulator(task_owner.document_owner(), owner_realm_id);
    let root_entry = document_modulator.insert_compiled_record_with_metadata(
        key.clone(),
        record,
        identity,
        metadata,
    );
    let tasks = vm
        .child_document_modulator_store
        .restore_document_modulator(task_owner, owner_realm_id, document_modulator);
    vm.push_child_module_terminal_batch_to_frame_lane(tasks);
    let script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: key.url().clone(),
        base_url: key.url().clone(),
        initiator_url: key.url().clone(),
        host_script_handle: None,
    };
    let pending_script_id = vm
        ._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_and_watch_module_script(task_owner.document_owner(), &script)
        .pending_script_id();
    let work = crate::document_script_scheduler::DocumentModuleGraphReadyWork::new(
        task_owner,
        owner_realm_id,
        pending_script_id,
        script,
        script_handle,
        key,
        moli_module_script_tree::ModuleTreeId(1),
        crate::frame_owner_model::DocumentLoadDelayTokenId(1),
        crate::module_runtime::ModuleGraphHandle {
            root_entry,
            entries: vec![root_entry],
        },
    );

    assert!(
        super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(&mut vm)
            .notify_module_script_graph_ready_work(work),
        "child graph-ready work should queue DocumentScriptReady"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "DocumentScriptReady should evaluate the completed child module graph"
    );
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childParserModuleEvalValue)"
        )
        .expect("child parser module side effect should be visible in child realm"),
        "144"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childParserModuleEvalValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
    assert_eq!(
        vm.eval("__childParserModuleEvalEvents.join('|')")
            .expect("child parser module eval events should evaluate"),
        "realm-ready|module:true|script-load"
    );
}
#[tokio::test]
async fn parser_discovered_child_modulepreloads_wait_for_one_realm_turn_and_promote_fifo() {
    let (mut vm, modulepreload_source) =
        new_child_modulepreload_page_test_vm("https://child-modulepreload-pre-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "pre-realm-modulepreloads";
  frame.srcdoc = `
    <link rel="modulepreload" href="/first.mjs">
    <link rel="modulepreload" href="/second.mjs">
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("pre-realm modulepreload fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "the child commit should discover both modulepreload links"
    );

    let (child_handle, owner, realm_id_before_materialization) = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("modulepreload fixture should retain one child frame");
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("modulepreload fixture should install a child Document");
        (
            child_handle,
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            ),
            snapshot.realm_id,
        )
    };
    assert!(
        realm_id_before_materialization.is_some(),
        "parser discovery must reserve one exact realm identity"
    );
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "reserved realm identity must not grant execution or protocol visibility before its typed turn"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        2,
        "both Document-owned starts should survive until exact-realm admission"
    );
    assert!(vm.has_pending_child_frame_realm_materialization());
    assert!(
        !modulepreload_source.has_ready_task(),
        "pre-realm work must not enter the typed executable source early"
    );

    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_some(),
        "one realm-materialization turn should bind all starts for the same child Document"
    );
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_none(),
        "two preload links must not manufacture a second realm turn"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0
    );

    let first = modulepreload_source
        .pop_front()
        .map(|(_, task)| task.into_task());
    let second = modulepreload_source
        .pop_front()
        .map(|(_, task)| task.into_task());
    let exhausted = modulepreload_source.pop_front();
    let first = first.expect("first preload should be promoted");
    let second = second.expect("second preload should be promoted");
    assert_eq!(first.owner(), owner);
    assert_eq!(second.owner(), owner);
    assert_eq!(first.realm_id(), second.realm_id());
    assert_eq!(first.target().child_handle(), child_handle);
    assert_eq!(
        first.request().module_key().url().as_str(),
        "https://child-modulepreload-pre-realm.test/first.mjs"
    );
    assert_eq!(
        second.request().module_key().url().as_str(),
        "https://child-modulepreload-pre-realm.test/second.mjs"
    );
    assert!(exhausted.is_none(), "promotion must not clone a start task");
}
#[tokio::test]
async fn parser_discovered_child_modulepreload_error_waits_for_the_same_realm_boundary() {
    let mut vm = new_storage_test_vm("https://child-modulepreload-error-realm.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__preRealmModulepreloadErrors = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <link rel="modulepreload" href="/invalid.bin" as="image"
      onerror="parent.__preRealmModulepreloadErrors.push('link-error')">
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("pre-realm modulepreload error fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        1,
        "invalid and fetchable modulepreloads must share the exact-realm admission boundary"
    );
    assert!(
        !vm.run_child_modulepreload_event_action_body_for_test(),
        "the link error must not be executable before its realm exists"
    );

    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0
    );
    let _predecessors = run_child_modulepreload_event_after_predecessors_for_test(
        &mut vm,
        "realm materialization should promote the exact-Document link error",
    )
    .await;
    assert_eq!(
        vm.eval("__preRealmModulepreloadErrors.join('|')")
            .expect("modulepreload error events should evaluate"),
        "link-error"
    );
}
#[test]
fn child_module_terminal_warning_exposes_followup_progress() {
    let mut vm = new_storage_test_vm("https://child-module-terminal-warning.test/");
    let task_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
        crate::frame_owner_model::FrameSchedulerLaneId(1),
        crate::frame_owner_model::LocalWindowId(2),
        crate::frame_owner_model::DocumentId(3),
    );
    let realm_id = crate::frame_owner_model::FrameRealmId(4);
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-module-terminal-warning.test/root.mjs").expect("module url"),
    );
    let mut batch = crate::frame_owner_model::FrameDocumentModuleTerminalBatch::default();
    batch.push_warning(
        crate::frame_owner_model::FrameDocumentModuleTerminalWarningRecord::new(
            task_owner,
            realm_id,
            crate::frame_owner_model::FrameDocumentModuleTerminalWarning::ParserRootTerminalWithoutOwnerWork {
                key,
                successful: false,
                parser_root_client_count: 1,
            },
        ),
    );

    let followup = vm.push_child_module_terminal_batch_to_frame_lane(batch);

    assert!(followup.made_progress());
    assert!(followup.terminal_warning_was_recorded());
    assert!(!followup.module_script_terminal_was_queued());
    assert!(!followup.modulepreload_event_action_was_queued());
    assert!(!followup.dynamic_import_owner_action_was_queued());
    assert!(
        vm.runtime_observable_lifecycle_errors_for_testing()
            .iter()
            .any(|warning| warning.contains("produced no owner-local terminal work"))
    );
}
#[tokio::test]
async fn child_module_reaction_target_rejects_a_replaced_realm_with_the_same_document_owner() {
    let mut vm = new_storage_test_vm("https://child-module-reaction-realm.test/");
    vm.eval_with_child_record_sync(
        "const root = document.documentElement || document.appendChild(document.createElement('html')); \
         const body = document.body || root.appendChild(document.createElement('body')); \
         const frame = document.createElement('iframe'); body.appendChild(frame);",
    )
    .expect("child module-reaction fixture should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child module-reaction fixture",
    )
    .await;

    let retired_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "retired child realm");
    let (child_handle, retired_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&retired_context_id)
            .expect("retired child realm record");
        (realm.child_handle, realm.owner_realm_id)
    };
    let document_owner = current_single_child_document_owner_for_test(&vm, "child reaction owner");
    let retired_target =
        crate::page_task_queue::RendererPageModuleReactionTarget::ChildParserModule {
            document_owner,
            realm_id: retired_realm_id,
        };
    assert!(vm.module_reaction_target_is_current(retired_target));

    vm.retire_child_frame_realm_for_test(child_handle);
    let current_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "replacement child realm");
    let current_realm_id = vm
        .child_frame_realm_store
        .get(&current_context_id)
        .expect("replacement child realm record")
        .owner_realm_id;
    assert_ne!(retired_realm_id, current_realm_id);
    assert_eq!(
        current_single_child_document_owner_for_test(&vm, "replacement child reaction owner"),
        document_owner,
        "realm rematerialization must preserve the Document owner in this collision fixture"
    );
    assert!(
        !vm.module_reaction_target_is_current(retired_target),
        "the old realm identity must not authorize work against the replacement realm"
    );
    assert!(vm.module_reaction_target_is_current(
        crate::page_task_queue::RendererPageModuleReactionTarget::ChildParserModule {
            document_owner,
            realm_id: current_realm_id,
        }
    ));
}
#[tokio::test]
async fn child_parser_module_evaluation_start_dispatches_load_before_tla_reaction() {
    let mut vm = new_storage_test_vm("https://child-parser-module-tla.test/");

    vm.eval_with_child_record_sync(
        r#"
(() => {
  globalThis.__childParserModuleTlaEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childParserModuleTlaFrame = frame;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser module TLA setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child parser module TLA setup",
    )
    .await;

    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child parser module TLA setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const marker = document.createElement("script");
  marker.id = "pending-module-target";
  marker.type = "application/json";
  marker.addEventListener("load", () => parent.__childParserModuleTlaEvents.push("script-load"));
  marker.addEventListener("error", () => parent.__childParserModuleTlaEvents.push("script-error"));
  body.appendChild(marker);
  parent.__childParserModuleTlaEvents.push("realm-ready");
})()
"#,
    )
    .expect("child parser module TLA child-realm setup should evaluate");
    assert_eq!(
        vm.eval("__childParserModuleTlaEvents.join('|')")
            .expect("child parser module TLA setup events should evaluate"),
        "realm-ready"
    );
    let (task_owner, script_handle) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child frame should expose a current owner snapshot");
        let document_owner = crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        );
        let task_owner = host
            .current_child_module_route_task_owner(document_owner, owner_realm_id)
            .expect("child frame should expose current document task owner");
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref()
                    == Some("pending-module-target")
            })
            .expect("pending module marker script handle should exist");
        (task_owner, script_handle)
    };
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-parser-module-tla.test/module.js").expect("module url"),
    );
    let metadata = crate::module_runtime::ModuleFetchMetadata::default();
    let source = crate::module_runtime::ModuleSource::text(
        r#"parent.__childParserModuleTlaEvents.push("module-start:" + (globalThis === self));
await new Promise(resolve => {
  globalThis.__resolveChildParserModuleTla = resolve;
  parent.__childParserModuleTlaEvents.push("module-pending");
});
globalThis.__childParserModuleTlaValue = 377;
parent.__childParserModuleTlaEvents.push("module-after");"#
            .to_owned(),
    );
    let (record, identity) = vm
        .compile_native_module_record_for_frame_realm(
            owner_realm_id,
            key.clone(),
            &source,
            key.url(),
            &metadata,
        )
        .expect("child frame TLA module record should compile");
    let mut document_modulator = vm
        .child_document_modulator_store
        .take_or_create_document_modulator(task_owner.document_owner(), owner_realm_id);
    let root_entry = document_modulator.insert_compiled_record_with_metadata(
        key.clone(),
        record,
        identity,
        metadata,
    );
    let tasks = vm
        .child_document_modulator_store
        .restore_document_modulator(task_owner, owner_realm_id, document_modulator);
    vm.push_child_module_terminal_batch_to_frame_lane(tasks);
    let script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: key.url().clone(),
        base_url: key.url().clone(),
        initiator_url: key.url().clone(),
        host_script_handle: None,
    };
    let pending_script_id = vm
        ._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_and_watch_module_script(task_owner.document_owner(), &script)
        .pending_script_id();
    let work = crate::document_script_scheduler::DocumentModuleGraphReadyWork::new(
        task_owner,
        owner_realm_id,
        pending_script_id,
        script,
        script_handle,
        key,
        moli_module_script_tree::ModuleTreeId(1),
        crate::frame_owner_model::DocumentLoadDelayTokenId(1),
        crate::module_runtime::ModuleGraphHandle {
            root_entry,
            entries: vec![root_entry],
        },
    );

    assert!(
        super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(&mut vm)
            .notify_module_script_graph_ready_work(work),
        "child TLA graph-ready work should queue DocumentScriptReady"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "DocumentScriptReady should start child TLA module evaluation"
    );
    assert_eq!(
        vm.eval("__childParserModuleTlaEvents.join('|')")
            .expect("child parser module TLA pending events should evaluate"),
        "realm-ready|module-start:true|module-pending|script-load",
        "module load should dispatch once evaluation starts without waiting for TLA settlement"
    );

    vm.eval_in_frame_realm(
        owner_realm_id,
        "globalThis.__resolveChildParserModuleTla(); 'ok'",
    )
    .expect("child parser module TLA resolver should run");
    assert_eq!(
        vm.run_page_module_reaction_body_for_test()
            .expect("child parser module TLA reaction should run"),
        Some(
            crate::page_task_queue::PageModuleReactionTargetEffect::AppliedToCurrentOwner(
                crate::page_task_queue::PageModuleReactionCurrentEffect::ModuleStateUpdated,
            )
        ),
        "resolving child parser module TLA should queue one exact-owner reaction"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "DocumentScriptReady should finish fulfilled child TLA module evaluation"
    );

    assert_eq!(
        vm.eval("__childParserModuleTlaEvents.join('|')")
            .expect("child parser module TLA fulfilled events should evaluate"),
        "realm-ready|module-start:true|module-pending|script-load|module-after",
        "the fulfilled reaction should continue evaluation without dispatching script load again"
    );
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childParserModuleTlaValue)"
        )
        .expect("child parser module TLA side effect should be visible in child realm"),
        "377"
    );
}
#[tokio::test]
async fn child_parser_module_graph_failure_dispatches_error_from_scheduler_lane() {
    let mut vm = new_storage_test_vm("https://child-parser-module-failure.test/");

    vm.eval_with_child_record_sync(
        r#"
(() => {
  globalThis.__childParserModuleFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childParserModuleFailureFrame = frame;
  frame.srcdoc = "";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser module failure setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "child parser module failure setup should initialize through NavigationCommit then HostLoad",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child parser module failure should materialize a child realm");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const marker = document.createElement("script");
  marker.id = "failed-module-target";
  marker.type = "application/json";
  marker.addEventListener("load", () => parent.__childParserModuleFailureEvents.push("script-load"));
  marker.addEventListener("error", () => parent.__childParserModuleFailureEvents.push("script-error"));
  body.appendChild(marker);
  parent.__childParserModuleFailureEvents.push("realm-ready");
})()
"#,
    )
    .expect("child parser module failure child-realm setup should evaluate");
    assert_eq!(
        vm.eval("__childParserModuleFailureEvents.join('|')")
            .expect("child parser module failure setup events should evaluate"),
        "realm-ready"
    );
    let (task_owner, script_handle) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child frame should expose a current owner snapshot");
        let document_owner = crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        );
        let task_owner = host
            .current_child_module_route_task_owner(document_owner, owner_realm_id)
            .expect("child frame should expose current document task owner");
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref()
                    == Some("failed-module-target")
            })
            .expect("failed module marker script handle should exist");
        (task_owner, script_handle)
    };
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-parser-module-failure.test/module.js").expect("module url"),
    );
    let script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: key.url().clone(),
        base_url: key.url().clone(),
        initiator_url: key.url().clone(),
        host_script_handle: None,
    };
    let pending_script_id = vm
        ._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_module_script(task_owner.document_owner(), &script);
    assert!(
        vm._context_host
            .borrow_mut()
            .child_document_script_schedulers_mut()
            .watch_module_script(pending_script_id)
            .watched(),
        "child failed parser module should be watched before terminal failure"
    );
    let work = crate::document_script_scheduler::DocumentModuleGraphFailedWork::new(
        task_owner,
        owner_realm_id,
        pending_script_id,
        script,
        script_handle,
        key,
        None,
        crate::frame_owner_model::DocumentLoadDelayTokenId(1),
        crate::module_runtime::ModuleLoadError::new(
            crate::module_runtime::ModuleLoadStage::Fetch,
            "network failed",
        ),
    );
    super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(&mut vm)
        .notify_module_script_graph_failed_action(work);

    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "child graph-failed work should dispatch from DocumentScriptReady"
    );
    assert!(
        !vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .has_ready_work(),
        "child graph-failed work should be consumed by the document script ready source"
    );
    assert_eq!(
        vm.eval("__childParserModuleFailureEvents.join('|')")
            .expect("child parser module failure events should evaluate"),
        "realm-ready|script-error"
    );
}
#[tokio::test]
async fn child_inline_parser_module_executes_from_registered_pending_script() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("http://child-inline-module.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childInlineParserModuleEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>parent.__childInlineParserModuleEvents.push("before");<\/script>
    <script type="module">
      parent.__childInlineParserModuleEvents.push(
        "module:" + (globalThis === self) + ":" + import.meta.url + ":" + import.meta.resolve("./x")
      );
      globalThis.__childInlineParserModuleValue = 42;
    <\/script>
    <script>parent.__childInlineParserModuleEvents.push("after:" + String(globalThis.__childInlineParserModuleValue));<\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child inline parser module setup should evaluate");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "srcdoc navigation should install the child document before parser work"
    );
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first child parser classic script should execute",
    )
    .await;
    let parser_module_owner = {
        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child inline module handoff should materialize a child realm");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child inline module realm should exist")
            .child_handle;
        let snapshot = vm
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child inline module frame should expose its document owner");
        crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        )
    };
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(parser_module_owner),
        1,
        "inline module PendingScript must exist before graph start"
    );
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::ParserModuleRootStart,
        "inline module graph should start before the parser continues past its element",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "parser should continue past the deferred inline module after graph start",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "graph-ready inline module should execute from DocumentScriptReady"
    );

    assert_eq!(
        vm.eval("__childInlineParserModuleEvents.join('|')")
            .expect("child inline parser module events should evaluate"),
        "before|after:undefined|module:true:http://child-inline-module.test/:http://child-inline-module.test/x"
    );
}
#[tokio::test]
async fn child_external_parser_module_executes_from_document_ready_lane() {
    let (script_url, request_path_rx, server) =
        spawn_child_external_parser_module_ready_lane_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://child-parser-module-driver.test/",
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childExternalParserModuleEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childExternalParserModuleFrame = frame;
  frame.addEventListener("load", () => {{
    globalThis.__childExternalParserModuleEvents.push("frame-load");
  }});
  frame.srcdoc = `
    <script>parent.__childExternalParserModuleEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childExternalParserModuleEvents.push("script-load");
      }});
      parent.__childExternalParserModuleEvents.push("after:" + String(globalThis.__childExternalParserModuleValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
    ))
    .expect("child external parser module setup should evaluate");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child external parser module srcdoc should commit before parser work",
    )
    .await;
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child inline parser script should run from DocumentScriptReady",
    )
    .await;
    let parser_module_owner = {
        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child parser module handoff should materialize a child realm");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child parser module realm should exist")
            .child_handle;
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child parser module frame should expose its document owner");
        crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        )
    };
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(parser_module_owner),
        1,
        "parser handoff must register PendingScript before the root-fetch source runs"
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::ParserModuleRootStart,
        "ParserModuleRootStart should begin fetch before the parser continues past the element",
    )
    .await;
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "parser should continue past module-defer script from DocumentScriptReady after fetch start",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("pre-terminal child HostLoad probe should use the selected-task dispatcher"),
        "registered PendingScript should block HostLoad before root-fetch starts"
    );
    assert_eq!(
        vm.eval("__childExternalParserModuleEvents.join('|')")
            .expect("child external parser module pre-completion events should evaluate"),
        "before:true|after:undefined",
        "parser should continue past module-defer script while root module fetch is pending"
    );

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child external parser module completion",
    )
    .await;
    assert!(
        vm.run_one_child_module_script_terminal_executor_turn(&loader)
            .await
            .expect("child module terminal should use the selected-task dispatcher"),
        "child parser module root completion should fan out from the typed terminal source"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            &loader,
        )
        .await
        .expect("child module execution should use the selected-task dispatcher"),
        "child parser module script should execute from DocumentScriptReady"
    );

    assert_eq!(
        request_path_rx
            .await
            .expect("child external parser module server should report request path"),
        "/child-parser-module.js"
    );
    server
        .await
        .expect("child external parser module test server should finish");

    assert_eq!(
        vm.eval("__childExternalParserModuleEvents.join('|')")
            .expect("child external parser module events should evaluate"),
        "before:true|after:undefined|module:true|script-load",
        "script load should dispatch before iframe load"
    );
    for transition in ["DOMContentLoaded", "complete"] {
        run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
            &mut vm,
            &loader,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child parser module should run its {transition} lifecycle turn"),
        )
        .await;
    }
    assert!(
        vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("child HostLoad should use the selected-task dispatcher"),
        "iframe load should dispatch from the later HostLoad source turn"
    );
    assert_eq!(
        vm.eval("__childExternalParserModuleEvents.join('|')")
            .expect("child external parser module events should evaluate after HostLoad"),
        "before:true|after:undefined|module:true|script-load|frame-load"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child external parser module should materialize a child realm");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childExternalParserModuleValue)"
        )
        .expect("child parser module side effect should be visible in child realm"),
        "188"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childExternalParserModuleValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
}
#[tokio::test]
async fn child_dynamic_import_root_fetch_uses_child_import_map_and_initiator_url() {
    let mut vm = new_storage_test_vm("https://parent-dynamic-owner.test/page.html");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-owner.test/nested/frame.html">
    <script type="importmap">
      {"imports":{"mapped-root":"/mapped/dynamic-root.js"}}
    <\/script>
    <script nonce="child-dynamic-import-nonce">
      parent.__childDynamicImportOwnerReady = true;
      import('mapped-root');
    <\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic import owner setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child dynamic import srcdoc should commit before parser work",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child dynamic import setup inline script should run from DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__childDynamicImportOwnerReady)")
            .expect("child dynamic import owner ready flag should evaluate"),
        "true"
    );
    for transition in ["DOMContentLoaded", "complete"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child dynamic import setup should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "child dynamic import setup frame should finish from a later HostLoad turn",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child default execution context should be created");
    let child_realm = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist");
    let child_handle = child_realm.child_handle;
    let child_realm_id = child_realm.owner_realm_id;
    let child_initiator_url = vm
        .child_browsing_context_module_request_initiator_url(child_handle)
        .expect("child document should expose a module request initiator URL");
    let dynamic_import_owner = vm
        .with_frame_realm_scope_and_checkpoint_for_test(child_realm_id, move |scope, host_ptr| {
            unsafe { &*host_ptr }
                .current_dynamic_module_import_owner(scope, Some(child_handle))
                .ok_or_else(|| anyhow::anyhow!("child dynamic import owner is unavailable"))
        })
        .expect("child dynamic import must bind its current execution context");
    assert_eq!(
        child_initiator_url.as_str(),
        "https://parent-dynamic-owner.test/",
        "srcdoc inherits its parent's origin even when <base> changes module URL resolution"
    );

    let (_child_handle, task_owner, realm_id) = dynamic_import_owner
        .child_parts()
        .expect("dynamic import owner should identify the child document");
    assert!(
        !vm.child_document_modulator_store
            .contains_execution_context(task_owner.document_owner()),
        "a child classic script should not eagerly create module state"
    );
    assert!(
        matches!(
            vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
            super::native_module::MainNativeModuleSelectedTaskApplication::Applied(_)
        ),
        "the classic-script import() callback should enqueue a graph job"
    );
    assert!(
        vm.child_document_modulator_store
            .contains_execution_context(task_owner.document_owner()),
        "the first child dynamic import graph start should lazily create its document modulator"
    );
    let root_fetch = vm
        .take_child_dynamic_module_import_fetch(task_owner.document_owner(), realm_id, 1)
        .expect("child dynamic import graph start should retain its root fetch");
    let root_fetch = root_fetch.inflight.request_for_test();

    assert_eq!(
        root_fetch.source_url().as_str(),
        "https://child-dynamic-owner.test/mapped/dynamic-root.js"
    );
    assert_eq!(root_fetch.initiator_url_for_test(), &child_initiator_url);
    assert_eq!(root_fetch.nonce(), Some("child-dynamic-import-nonce"));
}
#[tokio::test]
async fn child_dynamic_import_unexpected_complete_followup_without_graph_records_warning() {
    use crate::frame_owner_model::{
        FrameDocumentDynamicImportGraphAdvanceFollowup,
        FrameDocumentDynamicImportUnexpectedCompleteWarning, FrameDocumentTaskOwner,
    };

    let mut vm = new_storage_test_vm("https://parent-dynamic-owner-action.test/page.html");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic import owner action setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "empty child dynamic-import warning setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "empty child dynamic-import warning setup",
    );
    let (task_owner, realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        let owner = vm
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot(realm.child_handle)
            .expect("child frame should expose a current owner snapshot");
        (
            FrameDocumentTaskOwner::new(
                owner.scheduler_lane_id,
                owner.local_window_id,
                owner.document_id,
            ),
            realm.owner_realm_id,
        )
    };
    let outcome = vm.apply_child_dynamic_import_followup(
        FrameDocumentDynamicImportGraphAdvanceFollowup::RecordUnexpectedCompleteWarning(
            FrameDocumentDynamicImportUnexpectedCompleteWarning::new(
                task_owner.document_owner(),
                realm_id,
            ),
        ),
    );
    assert!(outcome.made_progress());
    assert!(
        outcome.terminal_warning_was_recorded(),
        "warning-only follow-up should not require a pending module graph"
    );
    assert!(
        !outcome.dynamic_import_owner_action_was_queued(),
        "warning-only follow-up must not enqueue a dynamic import owner action"
    );
}
#[tokio::test]
async fn child_external_classic_source_error_dispatches_before_later_inline() {
    let (script_url, request_path_rx, server) =
        spawn_child_external_classic_source_error_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://child-external-classic-source-error.test/",
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childExternalClassicSourceErrorEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childExternalClassicSourceErrorEvents.push("load");
  frame.srcdoc = `
    <script>
      document.addEventListener('DOMContentLoaded', () => {{
        parent.__childExternalClassicSourceErrorEvents.push('dcl');
      }});
    <\/script>
    <script id="missing-classic" src="{script_url}"
      onerror="parent.__childExternalClassicSourceErrorEvents.push('script-error')"><\/script>
    <script>
      parent.__childExternalClassicSourceErrorEvents.push('after-inline');
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
    ))
    .expect("child external classic source error setup should evaluate");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child external classic source-error srcdoc should commit before parser work",
    )
    .await;
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "initial inline parser script should run before the external source load starts",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
            &loader,
        )
        .await
        .expect("child classic source-load task should use the selected-task dispatcher"),
        "child external classic source-error load should start from the classic source-load turn"
    );
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("pre-terminal child HostLoad probe should use the selected-task dispatcher"),
        "HostLoad should report no progress while parser-blocking classic source is pending"
    );

    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("pending child external classic source error events should evaluate"),
        "",
        "pending failed external classic source must block later inline script and child load"
    );

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child external classic source-error completion",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            &loader,
        )
        .await
        .expect("child script-error task should use the selected-task dispatcher"),
        "child external classic source failure should dispatch script error from DocumentScriptReady"
    );
    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("child external classic source error events before HostLoad should evaluate"),
        "script-error",
        "first DocumentScriptReady should dispatch the source error without firing iframe load"
    );
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("blocked child HostLoad probe should use the selected-task dispatcher"),
        "HostLoad should not dispatch while source-failure parser continuation is still queued"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            &loader,
        )
        .await
        .expect("child parser continuation should use the selected-task dispatcher"),
        "source-failure parser continuation should run from the next DocumentScriptReady turn"
    );
    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("child external classic source error continuation events should evaluate"),
        "script-error|after-inline",
        "parser continuation should execute the following inline script without firing iframe load"
    );
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &loader,
        )
        .await
        .expect("child DCL task should use the selected-task dispatcher"),
        "DOMContentLoaded should dispatch from its own lifecycle turn"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &loader,
        )
        .await
        .expect("child complete task should use the selected-task dispatcher"),
        "document complete should dispatch from its own lifecycle turn"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("child HostLoad task should use the selected-task dispatcher"),
        "iframe load should dispatch from the later HostLoad source turn"
    );

    assert_eq!(
        request_path_rx
            .await
            .expect("child external classic source error server should report request path"),
        "/missing-classic.js"
    );
    server
        .await
        .expect("child external classic source error test server should finish");

    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("child external classic source error events should evaluate"),
        "script-error|after-inline|dcl|load"
    );
}
#[tokio::test]
async fn child_inline_classic_throw_reports_to_child_window_and_continues() {
    let mut vm = new_storage_test_vm("https://child-inline-classic-throw.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childInlineClassicThrowEvents = [];
  window.addEventListener("error", event => {
    globalThis.__childInlineClassicThrowEvents.push(
      "parent-listener-error:" + event.message
    );
  });
  window.onerror = (message) => {
    globalThis.__childInlineClassicThrowEvents.push("parent-error:" + message);
    return true;
  };
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childInlineClassicThrowEvents.push("load");
  frame.srcdoc = `
    <script>
      globalThis.onerror = function(message, source, line, column, error) {
        parent.__childInlineClassicThrowEvents.push(
          "child-error:" + message + ":" + (error && error.message) + ":" + (this === window)
        );
        return true;
      };
      document.addEventListener('DOMContentLoaded', () => {
        parent.__childInlineClassicThrowEvents.push('dcl');
      });
    <\/script>
    <script>
      throw new Error('child-boom');
    <\/script>
    <script>
      parent.__childInlineClassicThrowEvents.push('after-inline:' + (globalThis === window));
    <\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child inline classic throw setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child inline classic error srcdoc should commit before parser work",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first child inline classic setup script should run from DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events after setup should evaluate"),
        "",
        "first setup script should not fire child load or errors"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "throwing child inline classic script should run from the next DocumentScriptReady turn"
    );
    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events after error should evaluate"),
        "child-error:Uncaught Error: child-boom:child-boom:true",
        "throwing script should report to the child window without firing iframe load"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad should not dispatch while parser continuation work is still queued"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "parser continuation after the throw should run from DocumentScriptReady"
    );
    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events after parser continuation should evaluate"),
        "child-error:Uncaught Error: child-boom:child-boom:true|after-inline:true",
        "parser continuation should run the following inline script without firing iframe load"
    );
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "DOMContentLoaded should dispatch from its own lifecycle turn after inline error recovery"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "complete should apply on its own lifecycle turn before iframe load"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "iframe load should dispatch from the later HostLoad source turn"
    );

    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events should evaluate"),
        "child-error:Uncaught Error: child-boom:child-boom:true|after-inline:true|dcl|load"
    );
}
#[test]
fn parser_discovered_modulepreload_invalid_as_warns_each_link_once() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        concat!(
            "<!doctype html><html><head>",
            "<link id='first' rel='modulepreload' href='/bad.bin' as='image'>",
            "<link id='second' rel='modulepreload' href='/bad.bin' as='IMAGE'>",
            "</head><body></body></html>",
        ),
    );
    let link_handles = ["first", "second"].map(|id| {
        vm.document_runtime
            .get_element_by_id(id)
            .expect("parser modulepreload link should exist")
    });

    assert!(
        vm.accept_parser_discovered_native_modulepreloads(link_handles),
        "parser-discovered invalid modulepreload should produce observable owner progress"
    );
    assert!(
        !vm.accept_parser_discovered_native_modulepreloads(link_handles),
        "replaying the same exact candidates should not repeat the warning"
    );

    assert_eq!(
        vm.runtime_observable_lifecycle_errors_for_testing(),
        vec![
            "<link rel=modulepreload> has an invalid `as` value image".to_owned(),
            "<link rel=modulepreload> has an invalid `as` value image".to_owned()
        ]
    );
}
#[tokio::test(flavor = "current_thread")]
async fn parser_discovered_modulepreload_invalid_as_dispatches_link_error_events() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        concat!(
            "<!doctype html><html><head>",
            "<link id='first' rel='modulepreload' href='/bad.bin' as='image'>",
            "<link id='second' rel='modulepreload' href='/bad.bin' as='IMAGE'>",
            "</head><body></body></html>",
        ),
    );
    vm.exec(
        r#"
        globalThis.__modulepreloadInvalidAsEvents = [];
        for (const link of document.querySelectorAll("link")) {
          link.addEventListener("load", () => {
            globalThis.__modulepreloadInvalidAsEvents.push(`${link.id}:load`);
          });
          link.addEventListener("error", () => {
            globalThis.__modulepreloadInvalidAsEvents.push(`${link.id}:error`);
          });
        }
        "#,
        None,
    )
    .expect("event listeners should install");
    let link_handles = ["first", "second"].map(|id| {
        vm.document_runtime
            .get_element_by_id(id)
            .expect("parser modulepreload link should exist")
    });

    assert!(
        vm.accept_parser_discovered_native_modulepreloads(link_handles),
        "parser-discovered invalid modulepreloads should queue link error tasks"
    );
    while vm.has_ready_native_module_owner_actions() {
        vm.drain_ready_native_module_owner_actions()
            .expect("parser-discovered invalid modulepreload owner event should dispatch");
    }
    vm.queue_initial_connected_style_loads_for_current_owner();
    assert!(
        !vm.apply_connected_style_lifecycle_bodies_for_test(),
        "initial connected scan must not dispatch duplicate parser invalid-as errors"
    );

    assert_eq!(
        vm.eval("globalThis.__modulepreloadInvalidAsEvents.join(',')")
            .expect("events should be readable"),
        "first:error,second:error"
    );
}

#[tokio::test]
async fn child_parser_preloads_wait_for_one_exact_realm_before_starting_fetches() {
    let (mut vm, _) = new_child_modulepreload_page_test_vm("https://parser-preloads.test/");
    vm.eval(r#"
      const root = document.documentElement || document.appendChild(document.createElement('html'));
      const body = document.body || root.appendChild(document.createElement('body'));
      const frame = document.createElement('iframe');
      frame.srcdoc = '<link rel="preload" as="style" href="data:text/css,body{}"><link rel="preload" as="style" href="data:text/css,p{}">';
      body.append(frame);
    "#).expect("insert parser preloads");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert!(vm.live_child_default_runtime_realm_inventory().is_empty());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_parser_preload_count_for_test(),
        2
    );
    assert!(!vm.document_runtime.has_pending_style_loads());
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("materialize preload realm")
            .is_some()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_parser_preload_count_for_test(),
        0
    );
    assert!(vm.document_runtime.has_pending_style_loads());
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("one shared realm turn")
            .is_none()
    );
}

#[tokio::test]
async fn child_parser_preloads_are_discarded_before_realm_admission_after_navigation() {
    let (mut vm, _) =
        new_child_modulepreload_page_test_vm("https://parser-preloads-replaced.test/");
    vm.eval(
        r#"
      const root = document.documentElement || document.appendChild(document.createElement('html'));
      const body = document.body || root.appendChild(document.createElement('body'));
      const frame = document.createElement('iframe');
      frame.id = 'preload-frame';
      frame.srcdoc = '<link rel="preload" as="style" href="data:text/css,body{}">';
      body.append(frame);
    "#,
    )
    .expect("insert parser preload");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_parser_preload_count_for_test(),
        1
    );
    vm.eval(
        "document.getElementById('preload-frame').srcdoc = '<!doctype html><p>replacement</p>'; ",
    )
    .expect("replace child before realm turn");
    vm.run_one_child_navigation_commit_body_for_test()
        .expect("replacement navigation")
        .expect("replacement commit");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_parser_preload_count_for_test(),
        0
    );
    let stale = vm
        .run_one_child_realm_materialization_body_for_test()
        .expect("stale realm turn")
        .expect("old reservation remains scheduler-visible");
    assert!(matches!(
        stale.action.target_effect,
        crate::page_task_queue::PageChildRealmMaterializationTargetEffect::IgnoredStaleOwner { .. }
    ));
    assert!(!vm.document_runtime.has_pending_style_loads());
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("no replacement preload work")
            .is_none()
    );
}

#[tokio::test]
async fn child_parser_preloads_do_not_rebind_to_a_replacement_realm() {
    let (mut vm, _) = new_child_modulepreload_page_test_vm("https://parser-preloads-realm.test/");
    vm.eval(
        r#"
      const root = document.documentElement || document.appendChild(document.createElement('html'));
      const body = document.body || root.appendChild(document.createElement('body'));
      const frame = document.createElement('iframe');
      frame.id = 'preload-frame';
      frame.srcdoc = '<link rel="preload" as="style" href="data:text/css,body{}">';
      body.append(frame);
    "#,
    )
    .expect("insert parser preload");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let child = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()[0];
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_parser_preload_count_for_test(),
        1
    );
    vm.eval("void document.getElementById('preload-frame').contentWindow.Function")
        .expect("expose the reserved child realm");
    let original = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child)
        .unwrap();
    vm._context_host
        .borrow_mut()
        .clear_child_default_execution_context_id(child);
    vm.eval("void document.getElementById('preload-frame').contentWindow.Function")
        .expect("expose a replacement child realm");
    let replacement = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child)
        .unwrap();
    assert_eq!(original.document_id, replacement.document_id);
    assert_ne!(original.realm_id, replacement.realm_id);
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("materialize replacement realm")
            .is_some()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_parser_preload_count_for_test(),
        0
    );
    assert!(
        !vm.document_runtime.has_pending_style_loads(),
        "preload reserved for the original realm must not start in its replacement"
    );
}
