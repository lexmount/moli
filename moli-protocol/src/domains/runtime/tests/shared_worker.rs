use super::*;

use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsGetRealmsCommand,
    DevToolsProtocol, DevToolsSessionId, DevToolsTargetId, RuntimeExecutionContextEvent,
};

async fn wait_until_runtime_expression_true(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    command_id_base: u64,
    expression: &str,
    description: &str,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    for attempt in 0..256 {
        let command_id = command_id_base + attempt;
        let mut command = json!({
            "id": command_id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "returnByValue": true
            }
        });
        if let Some(session_id) = session_id {
            command["sessionId"] = json!(session_id);
        }

        ctx.process_async(command).await;
        let response = take_response_by_id(ctx, command_id);
        if response["result"]["result"]["value"] == json!(true) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for {description}: {response:?}");
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        tokio::task::yield_now().await;
    }
    panic!("timed out waiting for {description}");
}

fn record_shared_worker_runtime_context(ctx: &mut TestContext, session_id: &str, context_id: i64) {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(context_id),
        realm_id: None,
        frame_id: None,
        origin: None,
        name: None,
        is_default: None,
        context_type: Some("worker".to_owned()),
        grant_universal_access: None,
    };
    ctx.conn
        .shared_worker_target_for_session_mut(Some(session_id))
        .expect("shared worker target session should exist")
        .record_runtime_execution_context_created_event(&event);
}

#[tokio::test]
async fn runtime_enable_on_shared_worker_session_without_renderer_context_delays_buffered_logs() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");
    record_shared_worker_console(&mut ctx, "SID-shared-worker", "warn: boot warning");

    ctx.process_async(json!({
        "id": 71,
        "method": "Runtime.enable",
        "sessionId": "SID-shared-worker"
    }))
    .await;

    ctx.expect_result(71, json!({}), Some("SID-shared-worker"));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Runtime.executionContextCreated")),
        "Runtime.enable must not synthesize a worker execution context before renderer context creation: {:?}",
        ctx.sent
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Runtime.consoleAPICalled")),
        "Runtime.enable must not emit Runtime.consoleAPICalled before a real renderer context id exists: {:?}",
        ctx.sent
    );
}

#[tokio::test]
async fn runtime_enable_on_shared_worker_session_replays_real_worker_context() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");
    record_shared_worker_console(&mut ctx, "SID-shared-worker", "warn: boot warning");
    record_shared_worker_runtime_context(&mut ctx, "SID-shared-worker", 81_081);

    ctx.process_async(json!({
        "id": 72,
        "method": "Runtime.enable",
        "sessionId": "SID-shared-worker"
    }))
    .await;

    ctx.expect_result(72, json!({}), Some("SID-shared-worker"));
    let context_event = ctx.take_first_matching("shared worker execution context", |message| {
        message["method"] == json!("Runtime.executionContextCreated")
    });
    assert_eq!(context_event["sessionId"], "SID-shared-worker");
    assert_eq!(context_event["params"]["context"]["id"], json!(81_081));
    assert_eq!(
        context_event["params"]["context"]["origin"],
        json!("https://example.test")
    );
    assert_eq!(context_event["params"]["context"]["name"], json!("worker"));
    assert_eq!(
        context_event["params"]["context"]["auxData"],
        json!({
            "isDefault": true,
            "type": "worker"
        })
    );

    let console_event = ctx.take_first_matching("shared worker runtime console", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
    });
    assert_eq!(console_event["sessionId"], "SID-shared-worker");
    assert_eq!(console_event["params"]["type"], "warning");
    assert_eq!(
        console_event["params"]["args"],
        json!([{
            "type": "string",
            "value": "boot warning"
        }])
    );
    assert_eq!(console_event["params"]["executionContextId"], json!(81_081));
}

#[tokio::test]
async fn get_realms_on_shared_worker_target_waits_for_renderer_context() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: Some(DevToolsTargetId::from("TID-shared-worker")),
                browser_context_id: None,
            },
            realm_type: Some("shared-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert!(
        result.realms.is_empty(),
        "getRealms must not synthesize a shared worker realm before renderer context creation: {:?}",
        result.realms
    );
}

#[tokio::test]
async fn get_realms_on_shared_worker_target_returns_real_renderer_realm() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");
    record_shared_worker_runtime_context(&mut ctx, "SID-shared-worker", 81_081);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: Some(DevToolsTargetId::from("TID-shared-worker")),
                browser_context_id: None,
            },
            realm_type: Some("shared-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert_eq!(result.realms.len(), 1);
    let realm = &result.realms[0];
    assert_eq!(
        realm.target_id.as_ref().map(DevToolsTargetId::as_str),
        Some("TID-shared-worker")
    );
    assert_eq!(realm.context_id, Some(81_081));
    assert_eq!(
        realm.realm_id.as_ref().map(|realm| realm.as_str()),
        Some("shared-worker-TID-shared-worker")
    );
    assert_eq!(realm.frame_id, None);
    assert_eq!(realm.origin.as_deref(), Some("https://example.test"));
    assert_eq!(realm.name.as_deref(), Some("worker"));
    assert_eq!(realm.context_type.as_deref(), Some("shared-worker"));
}

#[tokio::test]
async fn get_realms_global_enumeration_waits_for_shared_worker_renderer_context() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: None,
                browser_context_id: None,
            },
            realm_type: Some("shared-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert!(
        result.realms.iter().all(|realm| {
            realm.target_id.as_ref().map(DevToolsTargetId::as_str) != Some("TID-shared-worker")
        }),
        "global getRealms must not synthesize shared worker target realms before renderer context creation: {:?}",
        result.realms
    );
}

#[tokio::test]
async fn get_realms_global_enumeration_includes_shared_worker_real_renderer_realm() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");
    record_shared_worker_runtime_context(&mut ctx, "SID-shared-worker", 81_081);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: None,
                browser_context_id: None,
            },
            realm_type: Some("shared-worker".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("getRealms should succeed") else {
        panic!("expected Realms result");
    };

    assert!(
        result.realms.iter().any(|realm| {
            realm.target_id.as_ref().map(DevToolsTargetId::as_str) == Some("TID-shared-worker")
                && realm.context_id == Some(81_081)
                && realm.context_type.as_deref() == Some("shared-worker")
                && realm.realm_id.as_ref().map(|realm| realm.as_str())
                    == Some("shared-worker-TID-shared-worker")
        }),
        "global getRealms should include shared worker target realms after renderer context creation: {:?}",
        result.realms
    );
}

#[tokio::test]
async fn runtime_evaluate_on_shared_worker_session_does_not_fall_back_to_page_runtime() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");

    ctx.process_async(json!({
        "id": 72,
        "method": "Runtime.evaluate",
        "sessionId": "SID-shared-worker",
        "params": { "expression": "self.name" }
    }))
    .await;

    let response = ctx.take_response_by_id(72);
    assert_eq!(response["sessionId"], "SID-shared-worker");
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(
        response["error"]["message"],
        "SharedWorkerRuntimeUnavailable"
    );
}

#[tokio::test]
async fn runtime_enable_on_dedicated_worker_without_renderer_runtime_still_succeeds() {
    let mut ctx = TestContext::new();
    load_dedicated_worker_target(&mut ctx, "SID-dedicated-worker");

    ctx.process_async(json!({
        "id": 72_001,
        "method": "Runtime.enable",
        "sessionId": "SID-dedicated-worker"
    }))
    .await;

    ctx.expect_result(72_001, json!({}), Some("SID-dedicated-worker"));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Runtime.executionContextCreated")),
        "Runtime.enable must not synthesize a dedicated worker execution context: {:?}",
        ctx.sent
    );
}

#[tokio::test]
async fn runtime_evaluate_on_dedicated_worker_reports_dedicated_runtime_unavailable() {
    let mut ctx = TestContext::new();
    load_dedicated_worker_target(&mut ctx, "SID-dedicated-worker");

    ctx.process_async(json!({
        "id": 72_002,
        "method": "Runtime.evaluate",
        "sessionId": "SID-dedicated-worker",
        "params": { "expression": "self.name" }
    }))
    .await;

    let response = ctx.take_response_by_id(72_002);
    assert_eq!(response["sessionId"], "SID-dedicated-worker");
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(
        response["error"]["message"],
        "DedicatedWorkerRuntimeUnavailable"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_commands_on_real_shared_worker_session_enter_worker_global() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

    ctx.process_async(json!({
        "id": 73,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(73, json!({}), None);

    ctx.process_async(json!({
        "id": 74,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
(() => {
  const source = `
    globalThis.__workerProbe = "worker:" + name + ":" + (
      typeof SharedWorkerGlobalScope !== "undefined" &&
      self instanceof SharedWorkerGlobalScope
    );
    globalThis.__workerObject = {
      answer: 42,
      label: globalThis.__workerProbe
    };
    onconnect = event => {
      const port = event.ports[0];
      port.postMessage("ready:" + globalThis.__workerProbe);
    };
  `;
  const url = "data:text/javascript," + encodeURIComponent(source);
  const worker = new SharedWorker(url, "runtime-probe");
  globalThis.__sharedWorkerRuntimeProbe = worker;
  globalThis.__sharedWorkerRuntimeReady = null;
  worker.port.onmessage = event => {
    globalThis.__sharedWorkerRuntimeReady = event.data;
  };
  worker.port.start();
  return "started";
})()
"#,
            "returnByValue": true
        }
    }))
    .await;
    let start_response = take_response_by_id(&mut ctx, 74);
    assert_eq!(
        start_response["result"]["result"]["value"],
        json!("started")
    );

    wait_until_message(
        &mut ctx,
        None,
        "shared worker target auto-attach",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["type"] == json!("shared_worker")
                && message["params"]["targetInfo"]["title"] == json!("runtime-probe")
        },
    )
    .await;
    let attached = ctx.take_first_matching("shared worker target auto-attach", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("shared_worker")
            && message["params"]["targetInfo"]["title"] == json!("runtime-probe")
    });
    let shared_worker_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("shared worker session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 75,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 75);
    assert_eq!(enable_response["sessionId"], shared_worker_session_id);
    assert_eq!(enable_response["result"], json!({}));
    let context_event = ctx.take_first_matching("shared worker execution context", |message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["sessionId"] == json!(shared_worker_session_id)
    });
    assert_eq!(
        context_event["params"]["context"]["auxData"]["type"],
        json!("worker")
    );

    ctx.process_async(json!({
        "id": 76,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.__workerProbe",
            "returnByValue": true
        }
    }))
    .await;
    let probe_response = take_response_by_id(&mut ctx, 76);
    assert_eq!(
        probe_response["result"]["result"]["value"],
        json!("worker:runtime-probe:true")
    );

    ctx.process_async(json!({
        "id": 77,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.__workerObject"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 77)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("shared worker object id");

    ctx.process_async(json!({
        "id": 78,
        "method": "Runtime.callFunctionOn",
        "sessionId": shared_worker_session_id,
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.label + ':' + this.answer; }",
            "returnByValue": true
        }
    }))
    .await;
    let call_response = take_response_by_id(&mut ctx, 78);
    assert_eq!(
        call_response["result"]["result"]["value"],
        json!("worker:runtime-probe:true:42")
    );

    ctx.process_async(json!({
        "id": 79,
        "method": "Runtime.getProperties",
        "sessionId": shared_worker_session_id,
        "params": {
            "objectId": object_id,
            "ownProperties": true
        }
    }))
    .await;
    let properties_response = take_response_by_id(&mut ctx, 79);
    let properties = properties_response["result"]["result"]
        .as_array()
        .expect("shared worker object properties");
    let answer = properties
        .iter()
        .find(|property| property["name"] == json!("answer"))
        .expect("answer property");
    assert_eq!(answer["value"]["value"], json!(42));
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_moli_diagnostics_reports_shared_worker_runtime_counts() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

    let _shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_010,
        "diagnostics-runtime-counts",
        r#"
onconnect = event => {
  event.ports[0].postMessage('ready');
};
"#,
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_012,
        "method": "HeapProfiler.moliDiagnostics"
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 90_012);
    let isolate_scope = &response["result"]["isolateScope"];
    assert_eq!(isolate_scope["sharedWorkerTargetCount"], json!(1));
    assert_eq!(isolate_scope["sharedWorkerMatchingEntryCount"], json!(1));
    assert_eq!(isolate_scope["sharedWorkerLoadingInstanceCount"], json!(0));
    assert_eq!(isolate_scope["sharedWorkerRunningInstanceCount"], json!(1));
    assert_eq!(isolate_scope["sharedWorkerClientCount"], json!(1));
    assert_eq!(isolate_scope["sharedWorkerLoadingHostCount"], json!(0));
    assert_eq!(
        isolate_scope["sharedWorkerRunningWorkerIsolateCount"],
        json!(1)
    );
    assert!(
        isolate_scope
            .get("sharedWorkerPendingTargetLifecycleEventCount")
            .is_none(),
        "SharedWorker target lifecycle is carried by its concrete renderer output stream, not a legacy pending-event queue"
    );
    assert_eq!(
        isolate_scope["sharedWorkerPendingServiceLaneEventCount"],
        json!(0)
    );
    assert_eq!(
        isolate_scope["sharedWorkerProtocolDispatchRequiresLiveOwnerPageCommand"],
        json!(false)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_moli_diagnostics_splits_document_and_shared_worker_isolates() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body>active</body>").await;

    let _shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_020,
        "diagnostics-document-worker-split",
        r#"
onconnect = event => {
  event.ports[0].postMessage('ready');
};
"#,
    )
    .await;

    let background_url = "data:text/html,<!doctype html><body>background</body>";
    let background_page = ctx
        .conn
        .load_page_via_runtime_async(background_url)
        .await
        .expect("background diagnostics page should load");
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should remain installed");
    let mut background = crate::conn::PageTargetHost::with_url(
        "TID-diagnostics-document-worker-bg".to_owned(),
        Some("SID-diagnostics-document-worker-bg".to_owned()),
        background_url.to_owned(),
    );
    background.replace_loaded_page(Some(background_page));
    browser_context.insert_page_target_host(background);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_022,
        "method": "HeapProfiler.moliDiagnostics"
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 90_022);
    let isolate_scope = &response["result"]["isolateScope"];
    assert_eq!(isolate_scope["documentIsolateModel"], json!("page-vm"));
    assert_eq!(isolate_scope["loadedDocumentPageCount"], json!(2));
    assert_eq!(
        isolate_scope["loadedDocumentRendererOwnerCount"],
        json!(1),
        "this accounting fixture deliberately reuses the active engine for both document snapshots: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedDocumentIsolateCount"],
        json!(2),
        "document isolate diagnostics must count loaded PageVMs, not renderer owners or worker targets: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedWorkerIsolateCount"],
        json!(1),
        "worker isolate diagnostics should count the SharedWorker separately from document isolates: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedLiveV8IsolateCount"],
        json!(3),
        "live V8 isolate total should be two page document isolates plus one SharedWorker isolate: {response:?}"
    );
    assert_eq!(
        isolate_scope["documentContextCount"],
        json!(2),
        "active and background pages should contribute separate document contexts: {response:?}"
    );
    assert_eq!(isolate_scope["sharedWorkerTargetCount"], json!(1));
    assert_eq!(
        isolate_scope["sharedWorkerRunningWorkerIsolateCount"],
        json!(1),
        "SharedWorker must be counted as a separate worker isolate, not a document isolate: {response:?}"
    );
    assert_eq!(
        response["result"]["activeBrowserContext"]["backgroundLoadedPageCount"],
        json!(1)
    );
    assert_eq!(
        response["result"]["activeBrowserContext"]["isolateScope"]["browserContextRuntime"]["sharedWorker"]
            ["runningWorkerIsolateCount"],
        json!(1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn heap_profiler_moli_diagnostics_covers_popup_dedicated_and_shared_worker_combo() {
    let mut ctx = TestContext::new();
    with_loaded_document_for_active_target_async(
        &mut ctx,
        "<!doctype html><body>opener</body>",
        "SID-diagnostics-combo-opener",
        "TID-diagnostics-combo-opener",
    )
    .await;

    let _shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_030,
        "diagnostics-popup-worker-combo",
        r#"
onconnect = event => {
  event.ports[0].postMessage('ready');
};
"#,
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_032,
        "method": "Runtime.evaluate",
        "sessionId": "SID-diagnostics-combo-opener",
        "params": {
            "expression": r#"
(() => {
  globalThis.__lmWorkerDiagnosticsReady = false;
  const worker = new Worker("data:text/javascript,postMessage('ready')");
  worker.onmessage = () => {
    globalThis.__lmWorkerDiagnosticsReady = true;
  };
  globalThis.__lmWorkerDiagnosticsWorker = worker;
})()
"#
        }
    }))
    .await;
    let dedicated_worker_start = take_response_by_id(&mut ctx, 90_032);
    assert_eq!(
        dedicated_worker_start["result"]["result"]["type"],
        json!("undefined")
    );

    wait_until_runtime_expression_true(
        &mut ctx,
        Some("SID-diagnostics-combo-opener"),
        90_033,
        "globalThis.__lmWorkerDiagnosticsReady === true",
        "dedicated worker diagnostics probe to become ready",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_034,
        "method": "Runtime.evaluate",
        "sessionId": "SID-diagnostics-combo-opener",
        "params": {
            "expression": "window.open('about:blank#diagnostics-popup-worker-combo', '_blank') !== null",
            "returnByValue": true
        }
    }))
    .await;
    let open_response = take_response_by_id(&mut ctx, 90_034);
    assert_eq!(
        open_response["result"]["result"]["value"],
        json!(true),
        "window.open should create a popup target: {open_response:?}"
    );
    let target_created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .cloned()
        .unwrap_or_else(|| panic!("popup targetCreated should be emitted: {:?}", ctx.sent));
    let popup_target_id = target_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let attached = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(popup_target_id)
        })
        .cloned()
        .unwrap_or_else(|| panic!("popup target should be auto-attached: {:?}", ctx.sent));
    let popup_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_035,
        "method": "Page.navigate",
        "sessionId": popup_session_id,
        "params": {
            "url": "data:text/html,<!doctype html><body>popup</body>"
        }
    }))
    .await;
    let navigate_response = take_response_by_id(&mut ctx, 90_035);
    assert_eq!(
        navigate_response["result"]["frameId"],
        json!(popup_target_id)
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_036,
        "method": "HeapProfiler.moliDiagnostics"
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 90_036);
    let isolate_scope = &response["result"]["isolateScope"];
    assert_eq!(isolate_scope["loadedDocumentPageCount"], json!(2));
    assert_eq!(
        isolate_scope["loadedDocumentRendererOwnerCount"],
        json!(2),
        "opener and popup must remain independently schedulable: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedDocumentIsolateCount"],
        json!(2),
        "popup PageVM must be counted as a second document isolate: {response:?}"
    );
    assert_eq!(
        isolate_scope["documentContextCount"],
        json!(2),
        "opener and popup should still have separate document contexts: {response:?}"
    );
    assert_eq!(isolate_scope["sharedWorkerTargetCount"], json!(1));
    assert_eq!(
        isolate_scope["dedicatedWorkerRunningWorkerIsolateCount"],
        json!(1),
        "dedicated worker should be counted separately from document isolates: {response:?}"
    );
    assert_eq!(
        isolate_scope["sharedWorkerRunningWorkerIsolateCount"],
        json!(1),
        "SharedWorker should remain a separate worker isolate in the popup combo: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedWorkerIsolateCount"],
        json!(2),
        "worker isolate total should include both dedicated worker and SharedWorker: {response:?}"
    );
    assert_eq!(
        isolate_scope["estimatedLiveV8IsolateCount"],
        json!(4),
        "combo should be two page document isolates plus two worker isolates: {response:?}"
    );
    assert_eq!(
        response["result"]["activeBrowserContext"]["backgroundLoadedPageCount"],
        json!(1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_replays_page_and_shared_worker_contexts_to_their_own_sessions() {
    let mut ctx = TestContext::new();
    with_loaded_document_for_active_target_async(
        &mut ctx,
        "<!doctype html><script>console.warn('page boot warning')</script><body></body>",
        "SID-page",
        "TID-page",
    )
    .await;
    ctx.sent.clear();

    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_020,
        "context-replay-isolation",
        r#"
onconnect = event => {
  event.ports[0].postMessage('ready');
};
"#,
    )
    .await;
    record_shared_worker_console(
        &mut ctx,
        &shared_worker_session_id,
        "warn: worker boot warning",
    );
    record_shared_worker_runtime_context(&mut ctx, &shared_worker_session_id, 90_021);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_022,
        "method": "Runtime.enable",
        "sessionId": "SID-page"
    }))
    .await;
    let page_messages = ctx.take_all();
    assert!(
        page_messages.iter().any(|message| {
            message["id"] == json!(90_022)
                && message["result"] == json!({})
                && message["sessionId"] == json!("SID-page")
        }),
        "page Runtime.enable should return a page-session result: {page_messages:?}"
    );
    let page_context = page_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-page")
                && message["method"] == json!("Runtime.executionContextCreated")
        })
        .unwrap_or_else(|| {
            panic!("page Runtime.enable should replay a page execution context: {page_messages:?}")
        });
    let page_context_id = page_context["params"]["context"]["id"]
        .as_i64()
        .expect("page context id");
    assert_eq!(
        page_context["params"]["context"]["auxData"]["frameId"],
        json!("TID-page")
    );
    assert_ne!(
        page_context["params"]["context"]["auxData"]["type"],
        json!("worker")
    );
    let page_console = page_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-page")
                && message["method"] == json!("Runtime.consoleAPICalled")
        })
        .unwrap_or_else(|| {
            panic!("page Runtime.enable should replay page console output: {page_messages:?}")
        });
    assert_eq!(
        page_console["params"]["args"][0]["value"],
        json!("page boot warning")
    );
    assert_eq!(
        page_console["params"]["executionContextId"],
        json!(page_context_id)
    );
    assert!(
        page_messages.iter().all(|message| {
            message["sessionId"] != json!(shared_worker_session_id)
                && message["params"]["context"]["auxData"]["type"] != json!("worker")
        }),
        "page Runtime.enable must not replay worker context or console output: {page_messages:?}"
    );

    ctx.process_async(json!({
        "id": 90_023,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let worker_messages = ctx.take_all();
    assert!(
        worker_messages.iter().any(|message| {
            message["id"] == json!(90_023)
                && message["result"] == json!({})
                && message["sessionId"] == json!(shared_worker_session_id)
        }),
        "shared worker Runtime.enable should return a worker-session result: {worker_messages:?}"
    );
    let worker_context = worker_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(shared_worker_session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
        })
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.enable should replay a worker execution context: {worker_messages:?}"
            )
        });
    assert_eq!(
        worker_context["params"]["context"]["auxData"]["type"],
        json!("worker")
    );
    let worker_console = worker_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(shared_worker_session_id)
                && message["method"] == json!("Runtime.consoleAPICalled")
        })
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.enable should replay worker console output: {worker_messages:?}"
            )
        });
    assert_eq!(
        worker_console["params"]["args"][0]["value"],
        json!("worker boot warning")
    );
    assert_ne!(
        worker_console["params"]["executionContextId"],
        json!(page_context_id),
        "worker console replay must not use the page execution context id"
    );
    assert_eq!(
        worker_console["params"]["executionContextId"],
        json!(90_021),
        "worker console replay should use the real shared worker execution context id"
    );
    assert!(
        worker_messages.iter().all(|message| {
            message["sessionId"] != json!("SID-page")
                && message["params"]["context"]["auxData"]["frameId"] != json!("TID-page")
        }),
        "shared worker Runtime.enable must not replay page context or console output: {worker_messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_runtime_add_remove_binding_persists_on_target() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_000,
        "binding-target",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_010,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 90_010);
    assert_eq!(enable_response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_011,
        "method": "Runtime.addBinding",
        "sessionId": shared_worker_session_id,
        "params": { "name": "sharedWorkerBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 90_011);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        ctx.conn
            .shared_worker_target_for_session(Some(&shared_worker_session_id))
            .expect("shared worker target should exist")
            .runtime_bindings(&shared_worker_session_id)
            .iter()
            .any(|binding| {
                binding.name == "sharedWorkerBinding" && binding.execution_context_name.is_none()
            }),
        "shared worker addBinding should persist on the target state"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_012,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.sharedWorkerBinding('from-shared-worker'); 'called'",
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 90_012);
    assert_eq!(evaluation["result"]["result"]["value"], json!("called"));
    let binding_called = ctx.take_first_matching("shared worker bindingCalled", |message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["sessionId"] == json!(shared_worker_session_id)
            && message["params"]["name"] == json!("sharedWorkerBinding")
    });
    assert_eq!(
        binding_called["params"]["payload"],
        json!("from-shared-worker")
    );

    ctx.process_async(json!({
        "id": 90_013,
        "method": "Runtime.removeBinding",
        "sessionId": shared_worker_session_id,
        "params": { "name": "sharedWorkerBinding" }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 90_013);
    assert_eq!(remove_binding["result"], json!({}));
    assert!(
        ctx.conn
            .shared_worker_target_for_session(Some(&shared_worker_session_id))
            .expect("shared worker target should exist")
            .runtime_bindings(&shared_worker_session_id)
            .iter()
            .all(|binding| binding.name != "sharedWorkerBinding"),
        "shared worker removeBinding should clear persisted target state"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 90_014,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.sharedWorkerBinding('after-remove'); typeof globalThis.sharedWorkerBinding",
            "returnByValue": true
        }
    }))
    .await;
    let removed = take_response_by_id(&mut ctx, 90_014);
    assert_eq!(removed["result"]["result"]["value"], json!("function"));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!(shared_worker_session_id)
                && message["params"]["name"] == json!("sharedWorkerBinding")
        }),
        "shared worker removeBinding should leave the existing function inert"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_runtime_agent_state_commands_dispatch_after_runtime_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_050,
        "runtime-agent-state",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_060,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 90_060);
    assert_eq!(enable_response["result"], json!({}));

    for (id, method, params) in [
        (
            90_061_u64,
            "Runtime.setCustomObjectFormatterEnabled",
            json!({ "enabled": true }),
        ),
        (
            90_062_u64,
            "Runtime.setMaxCallStackSizeToCapture",
            json!({ "size": 8 }),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": shared_worker_session_id,
            "params": params
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(
            response["result"],
            json!({}),
            "{method} should return V8 Runtime agent success: {response:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_terminate_execution_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_065,
        "runtime-terminate-execution",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_066,
        "method": "Runtime.terminateExecution",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_066);
    assert_eq!(
        response["result"],
        json!({}),
        "shared worker Runtime.terminateExecution should return V8 Runtime agent success: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_heap_profiler_collect_garbage_dispatches_through_v8_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_067,
        "heap-profiler-collect-garbage",
        r#"
onconnect = event => {
  globalThis.__heapProbe = new Array(1024).fill({value: "worker"});
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    let raw = json!({
        "id": 90_068,
        "method": "HeapProfiler.collectGarbage",
        "sessionId": shared_worker_session_id
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "shared worker HeapProfiler.collectGarbage should dispatch to the worker V8 HeapProfiler agent"
    );

    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(
        scheduler_events.is_empty(),
        "shared worker HeapProfiler.collectGarbage should not enqueue scheduler work: {scheduler_events:?}"
    );
    ctx.sent.extend(messages);
    wait_until_message(
        &mut ctx,
        Some(shared_worker_session_id.as_str()),
        "shared worker HeapProfiler.collectGarbage response",
        |message| message["id"] == json!(90_068),
    )
    .await;
    let response = take_response_by_id(&mut ctx, 90_068);
    assert_eq!(
        response["result"],
        json!({}),
        "shared worker HeapProfiler.collectGarbage should return V8 HeapProfiler agent success: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_heap_profiler_sampling_dispatches_through_v8_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_069,
        "heap-profiler-sampling",
        r#"
globalThis.__heapProbe = [];
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 90_070,
        "method": "HeapProfiler.startSampling",
        "sessionId": shared_worker_session_id,
        "params": { "samplingInterval": 1024, "stackDepth": 32 }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_070)["result"], json!({}));

    ctx.process_and_wait_for_response_async(json!({
        "id": 90_071,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "for (let i = 0; i < 200; i++) globalThis.__heapProbe.push({i, value: 'worker-heap-' + i}); 'done'"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_071)["result"]["result"]["value"],
        json!("done")
    );

    ctx.process_and_wait_for_response_async(json!({
        "id": 90_072,
        "method": "HeapProfiler.getSamplingProfile",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let profile = take_response_by_id(&mut ctx, 90_072);
    assert!(
        profile["result"]["profile"]["head"]["callFrame"]["functionName"].is_string(),
        "shared worker HeapProfiler.getSamplingProfile should return V8 sampling profile shape: {profile:?}"
    );

    ctx.process_and_wait_for_response_async(json!({
        "id": 90_073,
        "method": "HeapProfiler.stopSampling",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let stopped = take_response_by_id(&mut ctx, 90_073);
    assert!(
        stopped["result"]["profile"]["head"]["callFrame"]["functionName"].is_string(),
        "shared worker HeapProfiler.stopSampling should return V8 sampling profile shape: {stopped:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_get_isolate_id_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_070,
        "runtime-get-isolate-id",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_071,
        "method": "Runtime.getIsolateId",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_071);
    let isolate_id = response["result"]["id"]
        .as_str()
        .expect("Runtime.getIsolateId should return result.id");
    assert!(
        !isolate_id.is_empty() && isolate_id.chars().all(|ch| ch.is_ascii_hexdigit()),
        "shared worker Runtime.getIsolateId should return V8 isolate id as hex: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_get_exception_details_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_075,
        "runtime-get-exception-details",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_077,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 90_077);
    assert_eq!(enable_response["result"], json!({}));

    ctx.process_async(json!({
        "id": 90_078,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "new Error('moli worker exception details')"
        }
    }))
    .await;
    let evaluated = take_response_by_id(&mut ctx, 90_078);
    let error_object_id = evaluated["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.evaluate should return an Error object handle: {evaluated:?}"
            )
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 90_079,
        "method": "Runtime.getExceptionDetails",
        "sessionId": shared_worker_session_id,
        "params": {
            "errorObjectId": error_object_id
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_079);
    let details_text = response["result"]["exceptionDetails"]["text"]
        .as_str()
        .expect("Runtime.getExceptionDetails should return result.exceptionDetails.text");
    assert!(
        details_text.contains("moli worker exception details"),
        "shared worker Runtime.getExceptionDetails should return V8 exception details for the Error object: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_set_async_call_stack_depth_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        91_100,
        "runtime-set-async-call-stack-depth",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 91_102,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 91_102);
    assert_eq!(enable_response["result"], json!({}));

    ctx.process_async(json!({
        "id": 91_103,
        "method": "Runtime.setAsyncCallStackDepth",
        "sessionId": shared_worker_session_id,
        "params": {
            "maxDepth": 8
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 91_103);
    assert_eq!(response["sessionId"], json!(shared_worker_session_id));
    assert_eq!(
        response["result"],
        json!({}),
        "shared worker Runtime.setAsyncCallStackDepth should return V8 inspector success: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_global_lexical_scope_names_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_080,
        "runtime-global-lexical-scope-names",
        r#"
let __lmWorkerLexicalLet = 1;
const __lmWorkerLexicalConst = 2;
class __LmWorkerLexicalClass {}
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_081,
        "method": "Runtime.globalLexicalScopeNames",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_081);
    let names = response["result"]["names"]
        .as_array()
        .expect("Runtime.globalLexicalScopeNames should return result.names");
    for expected in [
        "__lmWorkerLexicalLet",
        "__lmWorkerLexicalConst",
        "__LmWorkerLexicalClass",
    ] {
        assert!(
            names.iter().any(|name| name.as_str() == Some(expected)),
            "shared worker Runtime.globalLexicalScopeNames should include {expected}: {response:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_query_objects_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_090,
        "runtime-query-objects",
        r#"
class __LmWorkerQueryObjectsThing {
  constructor(value) {
    this.value = value;
  }
}
globalThis.__lmWorkerQueryObjectsThings = [
  new __LmWorkerQueryObjectsThing(1),
  new __LmWorkerQueryObjectsThing(2),
];
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_091,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "__LmWorkerQueryObjectsThing.prototype",
            "objectGroup": "worker-query-prototype"
        }
    }))
    .await;
    let prototype_response = take_response_by_id(&mut ctx, 90_091);
    let prototype_object_id = prototype_response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.evaluate should return a prototype handle: {prototype_response:?}"
            )
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 90_092,
        "method": "Runtime.queryObjects",
        "sessionId": shared_worker_session_id,
        "params": {
            "prototypeObjectId": prototype_object_id,
            "objectGroup": "worker-query-results"
        }
    }))
    .await;
    let query_response = take_response_by_id(&mut ctx, 90_092);
    let objects_id = query_response["result"]["objects"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.queryObjects should return an array handle: {query_response:?}"
            )
        })
        .to_owned();
    let worker_target = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should exist");
    assert!(
        worker_target.has_runtime_remote_object_id(&shared_worker_session_id, &objects_id),
        "shared worker Runtime.queryObjects result handle should be target-local: {query_response:?}"
    );
    assert_eq!(
        worker_target.runtime_remote_object_group(&shared_worker_session_id, &objects_id),
        Some("worker-query-results"),
        "shared worker Runtime.queryObjects should register its result in the explicit objectGroup"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                std::slice::from_ref(&objects_id)
            )
            .is_err(),
        "active page owner should see shared worker queryObjects result as another target's handle"
    );

    ctx.process_async(json!({
        "id": 90_093,
        "method": "Runtime.releaseObjectGroup",
        "sessionId": shared_worker_session_id,
        "params": {
            "objectGroup": "worker-query-results"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_093)["result"], json!({}));
    let worker_target = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should exist");
    assert!(
        !worker_target.has_runtime_remote_object_id(&shared_worker_session_id, &objects_id),
        "shared worker Runtime.releaseObjectGroup should clear queryObjects result handle"
    );
    assert_eq!(
        worker_target.runtime_remote_object_group(&shared_worker_session_id, &objects_id),
        None,
        "shared worker Runtime.releaseObjectGroup should clear queryObjects result group"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_compile_and_run_script_dispatch_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_100,
        "runtime-compile-run-script",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_101,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_101)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_102,
        "method": "Runtime.compileScript",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "({ workerCompiled: 42 })",
            "sourceURL": "moli://runtime-compile-script/shared-worker.js",
            "persistScript": true
        }
    }))
    .await;
    let compile_response = take_response_by_id(&mut ctx, 90_102);
    let script_id = compile_response["result"]["scriptId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.compileScript should return a V8 scriptId: {compile_response:?}"
            )
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 90_103,
        "method": "Runtime.runScript",
        "sessionId": shared_worker_session_id,
        "params": {
            "scriptId": script_id,
            "objectGroup": "worker-compiled-script-results"
        }
    }))
    .await;
    let run_response = take_response_by_id(&mut ctx, 90_103);
    let object_id = run_response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.runScript should return an object handle: {run_response:?}"
            )
        })
        .to_owned();
    let worker_target = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should exist");
    assert!(
        worker_target.has_runtime_remote_object_id(&shared_worker_session_id, &object_id),
        "shared worker Runtime.runScript result handle should be target-local: {run_response:?}"
    );
    assert_eq!(
        worker_target.runtime_remote_object_group(&shared_worker_session_id, &object_id),
        Some("worker-compiled-script-results"),
        "shared worker Runtime.runScript should register its result in the explicit objectGroup"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                std::slice::from_ref(&object_id)
            )
            .is_err(),
        "active page owner should see shared worker runScript result as another target's handle"
    );

    ctx.process_async(json!({
        "id": 90_104,
        "method": "Runtime.releaseObjectGroup",
        "sessionId": shared_worker_session_id,
        "params": {
            "objectGroup": "worker-compiled-script-results"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_104)["result"], json!({}));
    let worker_target = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should exist");
    assert!(
        !worker_target.has_runtime_remote_object_id(&shared_worker_session_id, &object_id),
        "shared worker Runtime.releaseObjectGroup should clear runScript result handle"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_runtime_enable_replays_persisted_bindings_for_new_context() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_100,
        "binding-replay",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.conn
        .shared_worker_target_for_session_mut(Some(&shared_worker_session_id))
        .expect("shared worker target should exist")
        .upsert_runtime_binding_definition(
            &shared_worker_session_id,
            "replayedSharedWorkerBinding".to_owned(),
            None,
        );

    ctx.process_async(json!({
        "id": 90_110,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 90_110);
    assert_eq!(enable_response["result"], json!({}));
    let _ = ctx.take_first_matching("shared worker execution context", |message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["sessionId"] == json!(shared_worker_session_id)
    });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_111,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.replayedSharedWorkerBinding('after-replay'); typeof globalThis.replayedSharedWorkerBinding",
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 90_111);
    assert_eq!(evaluation["result"]["result"]["value"], json!("function"));
    let binding_called =
        ctx.take_first_matching("replayed shared worker bindingCalled", |message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!(shared_worker_session_id)
                && message["params"]["name"] == json!("replayedSharedWorkerBinding")
        });
    assert_eq!(binding_called["params"]["payload"], json!("after-replay"));
    assert!(
        ctx.conn
            .shared_worker_target_for_session(Some(&shared_worker_session_id))
            .expect("shared worker target should exist")
            .runtime_bindings_requiring_replay(&shared_worker_session_id)
            .is_empty(),
        "successful replay should clear the target replay-pending marker"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_runtime_remote_objects_are_target_local_and_release_clears_registry() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_200,
        "remote-object-owner",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_210,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_210)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_211,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "({ owner: 'shared-worker' })"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_211);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("shared worker Runtime.evaluate should return an object handle: {response:?}")
        })
        .to_owned();
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                std::slice::from_ref(&object_id),
            )
            .is_err(),
        "active page owner should see the shared worker handle as belonging to another target"
    );

    ctx.process_async(json!({
        "id": 90_212,
        "method": "Runtime.releaseObject",
        "params": {
            "objectId": object_id
        }
    }))
    .await;
    let active_release = take_response_by_id(&mut ctx, 90_212);
    assert_eq!(active_release["error"]["code"], json!(-32000));
    assert_eq!(
        active_release["error"]["message"],
        json!("Cannot find object with given id")
    );

    ctx.process_async(json!({
        "id": 90_213,
        "method": "Runtime.releaseObject",
        "sessionId": shared_worker_session_id,
        "params": {
            "objectId": object_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_213)["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(None, &[object_id])
            .is_ok(),
        "successful shared worker release should remove the handle from target-local ownership"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_runtime_remote_objects_do_not_cross_peer_worker_targets() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let first_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_240,
        "remote-object-peer-a",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready-a");
};
"#,
    )
    .await;
    let second_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_260,
        "remote-object-peer-b",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready-b");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_270,
        "method": "Runtime.evaluate",
        "sessionId": first_session_id,
        "params": {
            "expression": "({ owner: 'first-shared-worker' })"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_270);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "first shared worker Runtime.evaluate should return an object handle: {response:?}"
            )
        })
        .to_owned();

    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some(&second_session_id),
                std::slice::from_ref(&object_id),
            )
            .is_err(),
        "peer shared worker target should see the first worker handle as belonging to another target"
    );
    ctx.process_async(json!({
        "id": 90_271,
        "method": "Runtime.releaseObject",
        "sessionId": second_session_id,
        "params": {
            "objectId": object_id
        }
    }))
    .await;
    let peer_release = take_response_by_id(&mut ctx, 90_271);
    assert_eq!(peer_release["error"]["code"], json!(-32000));
    assert_eq!(
        peer_release["error"]["message"],
        json!("Cannot find object with given id")
    );

    ctx.process_async(json!({
        "id": 90_272,
        "method": "Runtime.releaseObject",
        "sessionId": first_session_id,
        "params": {
            "objectId": object_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_272)["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some(&second_session_id),
                &[object_id],
            )
            .is_ok(),
        "successful release by the owning worker should clear the handle from target-local ownership"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_profiler_state_is_isolated_per_attached_session() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let first_session_id = start_attached_shared_worker_session(
        &mut ctx,
        92_000,
        "profiler-session-isolation",
        r#"
globalThis.__sharedWorkerProfilerWork = () => {
  let value = 0;
  for (let i = 0; i < 200000; i += 1)
    value += Math.sqrt(i);
  return value > 0;
};
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;
    let target_id = ctx
        .conn
        .shared_worker_target_for_session(Some(&first_session_id))
        .expect("shared worker target should exist")
        .target_id
        .clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 92_010,
        "method": "Target.attachToTarget",
        "params": {"targetId": target_id}
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 92_010)["result"]["sessionId"]
        .as_str()
        .expect("second shared worker session id")
        .to_owned();
    assert_ne!(second_session_id, first_session_id);
    let attached = ctx.take_first_matching("second shared worker attached session", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
            && message["params"]["sessionId"] == json!(second_session_id)
    });
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));
    ctx.sent.clear();

    for (command_id, session_id) in [(92_011, &first_session_id), (92_012, &second_session_id)] {
        ctx.process_async(json!({
            "id": command_id,
            "method": "Profiler.enable",
            "sessionId": session_id
        }))
        .await;
        let response = take_response_by_id(&mut ctx, command_id);
        assert_eq!(response["sessionId"], json!(session_id));
        assert_eq!(response["result"], json!({}));
    }

    ctx.process_async(json!({
        "id": 92_023,
        "method": "Profiler.startPreciseCoverage",
        "sessionId": first_session_id,
        "params": {"callCount": true, "detailed": true}
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 92_023)["result"]["timestamp"].is_number(),
        "first shared worker session should start precise coverage"
    );

    ctx.process_async(json!({
        "id": 92_024,
        "method": "Profiler.takePreciseCoverage",
        "sessionId": second_session_id
    }))
    .await;
    let second_coverage = take_response_by_id(&mut ctx, 92_024);
    assert_eq!(second_coverage["sessionId"], json!(second_session_id));
    assert_eq!(second_coverage["error"]["code"], json!(-32000));
    assert!(
        second_coverage["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Precise coverage has not been started")),
        "second shared worker session must not read peer precise coverage: {second_coverage:?}"
    );

    ctx.process_async(json!({
        "id": 92_013,
        "method": "Profiler.start",
        "sessionId": first_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 92_013)["result"], json!({}));

    ctx.process_async(json!({
        "id": 92_022,
        "method": "Profiler.start",
        "sessionId": second_session_id
    }))
    .await;
    let second_start = take_response_by_id(&mut ctx, 92_022);
    assert_eq!(second_start["sessionId"], json!(second_session_id));
    assert_eq!(
        second_start["result"],
        json!({}),
        "renderer-side shared worker inspector sessions should own independent Profiler agents: {second_start:?}"
    );

    ctx.process_async(json!({
        "id": 92_014,
        "method": "Profiler.stop",
        "sessionId": second_session_id
    }))
    .await;
    let second_stop = take_response_by_id(&mut ctx, 92_014);
    assert_eq!(second_stop["sessionId"], json!(second_session_id));
    assert!(
        second_stop["result"]["profile"]["nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty()),
        "second shared worker session should stop its own profile without touching the first session: {second_stop:?}"
    );

    ctx.process_async(json!({
        "id": 92_021,
        "method": "Profiler.disable",
        "sessionId": second_session_id
    }))
    .await;
    let second_disable = take_response_by_id(&mut ctx, 92_021);
    assert_eq!(second_disable["sessionId"], json!(second_session_id));
    assert_eq!(
        second_disable["result"],
        json!({}),
        "disabling a peer shared worker profiler session must not disable the active session: {second_disable:?}"
    );

    ctx.process_async(json!({
        "id": 92_015,
        "method": "Target.detachFromTarget",
        "params": {"sessionId": second_session_id}
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 92_015)["result"], json!({}));
    assert!(
        ctx.conn
            .shared_worker_target_for_session(Some(&first_session_id))
            .is_some(),
        "detaching the second shared worker session must not detach the first session"
    );

    ctx.process_async(json!({
        "id": 92_016,
        "method": "Runtime.evaluate",
        "sessionId": first_session_id,
        "params": {
            "expression": "globalThis.__sharedWorkerProfilerWork()",
            "returnByValue": true
        }
    }))
    .await;
    let work = take_response_by_id(&mut ctx, 92_016);
    assert_eq!(work["result"]["result"]["value"], json!(true));

    ctx.process_async(json!({
        "id": 92_017,
        "method": "Profiler.stop",
        "sessionId": first_session_id
    }))
    .await;
    let first_stop = take_response_by_id(&mut ctx, 92_017);
    assert_eq!(first_stop["sessionId"], json!(first_session_id));
    assert!(
        first_stop["result"]["profile"]["nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty()),
        "first shared worker session should still return a CPU profile after peer stop/detach: {first_stop:?}"
    );

    ctx.process_async(json!({
        "id": 92_018,
        "method": "Target.detachFromTarget",
        "params": {"sessionId": first_session_id}
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 92_018)["result"], json!({}));

    ctx.process_async(json!({
        "id": 92_019,
        "method": "Target.attachToTarget",
        "params": {"targetId": target_id}
    }))
    .await;
    let replacement_session_id = take_response_by_id(&mut ctx, 92_019)["result"]["sessionId"]
        .as_str()
        .expect("replacement shared worker session id")
        .to_owned();
    assert_ne!(replacement_session_id, first_session_id);
    ctx.process_async(json!({
        "id": 92_020,
        "method": "Profiler.start",
        "sessionId": replacement_session_id
    }))
    .await;
    let replacement_start = take_response_by_id(&mut ctx, 92_020);
    assert_eq!(
        replacement_start["sessionId"],
        json!(replacement_session_id)
    );
    assert_eq!(replacement_start["error"]["code"], json!(-32000));
    assert!(
        replacement_start["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Profiler is not enabled")),
        "newly attached shared worker session must not inherit detached Profiler.enable state: {replacement_start:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_console_profile_events_fan_out_to_attached_profiler_sessions() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let first_session_id = start_attached_shared_worker_session(
        &mut ctx,
        92_100,
        "console-profile-fanout",
        r#"
globalThis.__sharedWorkerConsoleProfileWork = () => {
  let value = 0;
  for (let i = 0; i < 200000; i += 1)
    value += Math.sqrt(i + 1);
  return value > 0;
};
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;
    let target_id = ctx
        .conn
        .shared_worker_target_for_session(Some(&first_session_id))
        .expect("shared worker target should exist")
        .target_id
        .clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 92_110,
        "method": "Target.attachToTarget",
        "params": {"targetId": target_id}
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 92_110)["result"]["sessionId"]
        .as_str()
        .expect("second shared worker session id")
        .to_owned();
    assert_ne!(second_session_id, first_session_id);
    ctx.sent.clear();

    for (command_id, session_id) in [(92_111, &first_session_id), (92_112, &second_session_id)] {
        ctx.process_async(json!({
            "id": command_id,
            "method": "Profiler.enable",
            "sessionId": session_id
        }))
        .await;
        let response = take_response_by_id(&mut ctx, command_id);
        assert_eq!(response["sessionId"], json!(session_id));
        assert_eq!(response["result"], json!({}));
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 92_113,
        "method": "Runtime.evaluate",
        "sessionId": first_session_id,
        "params": {
            "expression": r#"
(() => {
  console.profile("shared-worker-console-profile");
  const result = globalThis.__sharedWorkerConsoleProfileWork();
  console.profileEnd("shared-worker-console-profile");
  return result;
})()
"#,
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 92_113);
    assert_eq!(evaluation["sessionId"], json!(first_session_id));
    assert_eq!(
        evaluation["result"]["result"]["value"],
        json!(true),
        "shared worker console profile evaluation should complete successfully: {evaluation:?}"
    );

    wait_until_messages(
        &mut ctx,
        None,
        "shared worker console profile events for both attached sessions",
        |messages| {
            [
                (&first_session_id, "Profiler.consoleProfileStarted"),
                (&first_session_id, "Profiler.consoleProfileFinished"),
                (&second_session_id, "Profiler.consoleProfileStarted"),
                (&second_session_id, "Profiler.consoleProfileFinished"),
            ]
            .into_iter()
            .all(|(session_id, method)| {
                messages.iter().any(|message| {
                    message["sessionId"] == json!(session_id) && message["method"] == json!(method)
                })
            })
        },
    )
    .await;

    for session_id in [&first_session_id, &second_session_id] {
        let started = ctx.take_first_matching(
            "shared worker Profiler.consoleProfileStarted event",
            |message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Profiler.consoleProfileStarted")
            },
        );
        assert_eq!(
            started["params"]["title"],
            json!("shared-worker-console-profile")
        );
        assert!(
            started["params"]["id"].is_string(),
            "consoleProfileStarted should include a V8 profile id: {started:?}"
        );

        let finished = ctx.take_first_matching(
            "shared worker Profiler.consoleProfileFinished event",
            |message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Profiler.consoleProfileFinished")
            },
        );
        assert_eq!(
            finished["params"]["title"],
            json!("shared-worker-console-profile")
        );
        assert_eq!(
            finished["params"]["id"], started["params"]["id"],
            "console profile finished id should match started id for the same attached session"
        );
        assert!(
            finished["params"]["profile"]["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty()),
            "consoleProfileFinished should include a CPU profile: {finished:?}"
        );
    }

    for (command_id, session_id) in [(92_114, &first_session_id), (92_115, &second_session_id)] {
        ctx.process_async(json!({
            "id": command_id,
            "method": "Profiler.stop",
            "sessionId": session_id
        }))
        .await;
        let stop = take_response_by_id(&mut ctx, command_id);
        assert_eq!(stop["sessionId"], json!(session_id));
        assert_eq!(stop["error"]["code"], json!(-32000));
        assert!(
            stop["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("No recording profiles found")),
            "console.profile must not create a frontend Profiler.start recording: {stop:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn target_close_target_terminates_shared_worker_and_cleans_target_state() {
    let mut ctx = TestContext::new();
    ctx.conn.set_target_discovery_for_owner(
        None,
        crate::conn::CdpTargetFilter::default_target_discovery(),
    );
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_280,
        "target-close-cleanup",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_290,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_290)["result"], json!({}));

    ctx.process_async(json!({
        "id": 90_291,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "new Promise(resolve => setTimeout(() => resolve('too-late'), 1000))",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(90_291)),
        "timer-backed shared worker awaitPromise should still be pending before Target.closeTarget"
    );
    let target_id = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should be attached before target close")
        .target_id
        .clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_292,
        "method": "Target.closeTarget",
        "params": { "targetId": target_id }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_292)["result"],
        json!({ "success": true })
    );
    let failed_await = take_response_by_id(&mut ctx, 90_291);
    assert_eq!(failed_await["sessionId"], json!(shared_worker_session_id));
    assert_eq!(failed_await["error"]["code"], json!(-32000));
    assert_eq!(failed_await["error"]["message"], json!("Target closed"));
    let detached =
        ctx.take_first_matching("shared worker target detach after closeTarget", |message| {
            message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["targetId"] == json!(target_id)
                && message["params"]["sessionId"] == json!(shared_worker_session_id)
        });
    assert_eq!(detached["params"]["targetId"], json!(target_id));
    let destroyed = ctx.take_first_matching(
        "shared worker target destroy after closeTarget",
        |message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(target_id)
        },
    );
    assert_eq!(destroyed["params"]["targetId"], json!(target_id));
    assert_eq!(
        ctx.conn.session_route(Some(&shared_worker_session_id)),
        None
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|context| context.shared_worker_target(&target_id))
            .is_none(),
        "Target.closeTarget should remove the shared worker CDP target state"
    );
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "Target.closeTarget should drain shared worker pending awaits"
    );

    ctx.process_async(json!({
        "id": 90_293,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body instanceof HTMLBodyElement",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_293)["result"]["result"]["value"],
        json!(true),
        "closing a SharedWorker target must not disturb the active page target"
    );

    ctx.process_async(json!({
        "id": 90_294,
        "method": "HeapProfiler.moliDiagnostics"
    }))
    .await;
    let diagnostics = take_response_by_id(&mut ctx, 90_294);
    let isolate_scope = &diagnostics["result"]["isolateScope"];
    assert_eq!(isolate_scope["sharedWorkerTargetCount"], json!(0));
    assert_eq!(isolate_scope["sharedWorkerRunningInstanceCount"], json!(0));
    assert_eq!(
        isolate_scope["sharedWorkerRunningWorkerIsolateCount"],
        json!(0)
    );
    assert_eq!(isolate_scope["sharedWorkerClientCount"], json!(0));

    ctx.process_async(json!({
        "id": 90_295,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "self.name",
            "returnByValue": true
        }
    }))
    .await;
    let stale_session = take_response_by_id(&mut ctx, 90_295);
    assert_eq!(stale_session["error"]["code"], json!(-32001));
    assert_eq!(
        stale_session["error"]["message"],
        json!("Unknown sessionId")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_runtime_object_groups_inherit_and_clear_on_detach() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_300,
        "remote-object-group-owner",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_310,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_310)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_311,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "({ child: { answer: 42 } })",
            "objectGroup": "shared-worker-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_311);
    let parent_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("shared worker Runtime.evaluate should return an object handle: {response:?}")
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 90_312,
        "method": "Runtime.getProperties",
        "sessionId": shared_worker_session_id,
        "params": {
            "objectId": parent_object_id,
            "ownProperties": true
        }
    }))
    .await;
    let properties = take_response_by_id(&mut ctx, 90_312);
    let child_object_id = properties["result"]["result"]
        .as_array()
        .and_then(|properties| {
            properties
                .iter()
                .find(|property| property["name"] == json!("child"))
        })
        .and_then(|property| property["value"]["objectId"].as_str())
        .unwrap_or_else(|| {
            panic!(
                "shared worker Runtime.getProperties should return a child handle: {properties:?}"
            )
        })
        .to_owned();
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                &[parent_object_id.clone(), child_object_id.clone()],
            )
            .is_err(),
        "active page owner should see grouped shared worker handles as another target's handles"
    );

    ctx.process_async(json!({
        "id": 90_313,
        "method": "Target.detachFromTarget",
        "params": {
            "sessionId": shared_worker_session_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_313)["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                &[parent_object_id, child_object_id],
            )
            .is_ok(),
        "shared worker target detach should clear target-local remote object ownership"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_pending_await_fails_on_target_detach() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_400,
        "pending-await-detach",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_410,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_410)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_411,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "new Promise(resolve => setTimeout(() => resolve('too-late'), 250))",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(90_411)),
        "timer-backed shared worker awaitPromise should still be pending before detach"
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&shared_worker_session_id)),
        "shared worker awaitPromise should register against the shared worker target"
    );

    ctx.process_async(json!({
        "id": 90_412,
        "method": "Target.detachFromTarget",
        "params": {
            "sessionId": shared_worker_session_id
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_412)["result"], json!({}));
    let failed_await = take_response_by_id(&mut ctx, 90_411);
    assert_eq!(failed_await["sessionId"], shared_worker_session_id);
    assert_eq!(failed_await["error"]["code"], json!(-32000));
    assert_eq!(failed_await["error"]["message"], json!("Target detached"));
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "detaching the shared worker target should drain the pending await registry"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn real_shared_worker_close_cleans_runtime_state_before_destroy_events() {
    let mut ctx = TestContext::new();
    ctx.conn.set_target_discovery_for_owner(
        None,
        crate::conn::CdpTargetFilter::default_target_discovery(),
    );
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let worker_source = r#"
globalThis.__closeProbe = { child: { answer: 42 } };
onconnect = event => {
  const port = event.ports[0];
  port.onmessage = event => {
    if (event.data === "close") {
      close();
    }
  };
  port.start();
  port.postMessage("ready");
};
"#;
    let shared_worker_session_id =
        start_attached_shared_worker_session(&mut ctx, 90_500, "real-close-cleanup", worker_source)
            .await;

    ctx.process_async(json!({
        "id": 90_510,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_510)["result"], json!({}));

    ctx.process_async(json!({
        "id": 90_511,
        "method": "Runtime.addBinding",
        "sessionId": shared_worker_session_id,
        "params": { "name": "closedWorkerBinding" }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_511)["result"], json!({}));

    ctx.process_async(json!({
        "id": 90_512,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.__closeProbe",
            "objectGroup": "real-close-cleanup-group"
        }
    }))
    .await;
    let close_probe_object_id =
        take_response_by_id(&mut ctx, 90_512)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_else(|| panic!("shared worker close probe should return an object handle"))
            .to_owned();
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                std::slice::from_ref(&close_probe_object_id),
            )
            .is_err(),
        "active page owner should see the shared worker handle as another target's handle"
    );

    ctx.process_async(json!({
        "id": 90_513,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "new Promise(resolve => setTimeout(() => resolve('too-late'), 1000))",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(90_513)),
        "timer-backed shared worker awaitPromise should still be pending before worker close"
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&shared_worker_session_id)),
        "shared worker awaitPromise should register against the shared worker target"
    );
    let closing_target_id = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should still be attached before close")
        .target_id
        .clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_514,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__sharedWorkerRuntimeBindingProbe.port.postMessage('close'); 'closing'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_514)["result"]["result"]["value"],
        json!("closing")
    );

    wait_until_messages(
        &mut ctx,
        None,
        "real shared worker close target teardown",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(90_513))
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Target.detachedFromTarget"))
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Target.targetDestroyed"))
        },
    )
    .await;
    let teardown_messages = ctx.take_all();
    let failed_await_index = teardown_messages
        .iter()
        .position(|message| message["id"] == json!(90_513))
        .expect("pending await should fail when the real shared worker closes");
    let detached_index = teardown_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["targetId"] == json!(closing_target_id)
                && message["params"]["sessionId"] == json!(shared_worker_session_id)
        })
        .unwrap_or_else(|| {
            panic!("real shared worker close should detach target: {teardown_messages:?}")
        });
    let destroyed_index = teardown_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(closing_target_id)
        })
        .unwrap_or_else(|| {
            panic!("real shared worker close should destroy target: {teardown_messages:?}")
        });
    let failed_await = &teardown_messages[failed_await_index];
    assert_eq!(failed_await["sessionId"], json!(shared_worker_session_id));
    assert_eq!(failed_await["error"]["code"], json!(-32000));
    assert_eq!(failed_await["error"]["message"], json!("Target closed"));
    assert!(
        failed_await_index < detached_index && detached_index < destroyed_index,
        "real worker close must fail pending awaits before detached/destroyed events: {teardown_messages:?}"
    );
    assert_eq!(
        ctx.conn.session_route(Some(&shared_worker_session_id)),
        None
    );
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "destroying the shared worker target should drain pending awaits"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                std::slice::from_ref(&close_probe_object_id),
            )
            .is_ok(),
        "destroying the shared worker target should remove target-local remote object ownership"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|context| context.shared_worker_target(&closing_target_id))
            .is_none(),
        "real shared worker close should remove the destroyed target state"
    );

    let replacement_session_id =
        start_attached_shared_worker_session(&mut ctx, 90_520, "real-close-cleanup", worker_source)
            .await;
    assert_ne!(
        replacement_session_id, shared_worker_session_id,
        "fresh shared worker instance should get a fresh target session after close"
    );
    ctx.process_async(json!({
        "id": 90_530,
        "method": "Runtime.enable",
        "sessionId": replacement_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_530)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_531,
        "method": "Runtime.evaluate",
        "sessionId": replacement_session_id,
        "params": {
            "expression": "typeof globalThis.closedWorkerBinding",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_531)["result"]["result"]["value"],
        json!("undefined"),
        "target-local Runtime.addBinding state from the destroyed shared worker must not replay into a fresh target"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn page_navigation_teardown_terminates_shared_worker_and_cleans_target_state() {
    let mut ctx = TestContext::new();
    ctx.conn.set_target_discovery_for_owner(
        None,
        crate::conn::CdpTargetFilter::default_target_discovery(),
    );
    with_loaded_document_for_active_target_async(
        &mut ctx,
        "<!doctype html><body>before</body>",
        "SID-page",
        "TID-page",
    )
    .await;
    let worker_source = r#"
globalThis.__teardownProbe = { child: { answer: 42 } };
onconnect = event => {
  const port = event.ports[0];
  port.start();
  port.postMessage("ready");
};
"#;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_600,
        "navigation-teardown-cleanup",
        worker_source,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_610,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_610)["result"], json!({}));

    ctx.process_async(json!({
        "id": 90_611,
        "method": "Runtime.addBinding",
        "sessionId": shared_worker_session_id,
        "params": { "name": "navigationTeardownBinding" }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_611)["result"], json!({}));

    ctx.process_async(json!({
        "id": 90_612,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.__teardownProbe",
            "objectGroup": "navigation-teardown-cleanup-group"
        }
    }))
    .await;
    let teardown_probe_response = take_response_by_id(&mut ctx, 90_612);
    let teardown_probe_object_id = teardown_probe_response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "shared worker navigation teardown probe should return an object handle: {teardown_probe_response:?}"
            )
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 90_613,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "new Promise(resolve => setTimeout(() => resolve('too-late'), 1000))",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(90_613)),
        "timer-backed shared worker awaitPromise should still be pending before page navigation"
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&shared_worker_session_id)),
        "shared worker awaitPromise should register against the shared worker target"
    );
    let terminating_target_id = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should be attached before navigation teardown")
        .target_id
        .clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_614,
        "method": "Page.navigate",
        "sessionId": "SID-page",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 90_614);
    assert_eq!(navigation["result"]["frameId"], json!("TID-page"));

    wait_until_messages(
        &mut ctx,
        None,
        "shared worker client removal target teardown",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(90_613))
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Target.detachedFromTarget"))
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Target.targetDestroyed"))
        },
    )
    .await;
    let teardown_messages = ctx.take_all();
    let failed_await_index = teardown_messages
        .iter()
        .position(|message| message["id"] == json!(90_613))
        .expect("pending await should fail when the last SharedWorker client is removed");
    let detached_index = teardown_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["targetId"] == json!(terminating_target_id)
                && message["params"]["sessionId"] == json!(shared_worker_session_id)
        })
        .unwrap_or_else(|| {
            panic!("client removal should detach the shared worker target: {teardown_messages:?}")
        });
    let destroyed_index = teardown_messages
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(terminating_target_id)
        })
        .unwrap_or_else(|| {
            panic!("client removal should destroy the shared worker target: {teardown_messages:?}")
        });
    let failed_await = &teardown_messages[failed_await_index];
    assert_eq!(failed_await["sessionId"], json!(shared_worker_session_id));
    assert_eq!(failed_await["error"]["code"], json!(-32000));
    assert_eq!(failed_await["error"]["message"], json!("Target closed"));
    assert!(
        failed_await_index < detached_index && detached_index < destroyed_index,
        "client removal must fail pending awaits before detached/destroyed events: {teardown_messages:?}"
    );
    assert_eq!(
        ctx.conn.session_route(Some(&shared_worker_session_id)),
        None
    );
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "client removal should drain shared worker pending awaits"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                std::slice::from_ref(&teardown_probe_object_id),
            )
            .is_ok(),
        "destroying the shared worker target should remove target-local remote object ownership"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|context| context.shared_worker_target(&terminating_target_id))
            .is_none(),
        "client removal should remove the destroyed shared worker target state"
    );

    let replacement_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_620,
        "navigation-teardown-cleanup",
        worker_source,
    )
    .await;
    assert_ne!(
        replacement_session_id, shared_worker_session_id,
        "new page context should get a fresh shared worker target session after teardown"
    );
    ctx.process_async(json!({
        "id": 90_630,
        "method": "Runtime.enable",
        "sessionId": replacement_session_id
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 90_630)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_631,
        "method": "Runtime.evaluate",
        "sessionId": replacement_session_id,
        "params": {
            "expression": "typeof globalThis.navigationTeardownBinding",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_631)["result"]["result"]["value"],
        json!("undefined"),
        "target-local binding state from the terminated shared worker must not replay into a fresh target"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_target_runtime_survives_owner_page_navigation_with_peer_client() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>shared worker peer page</body>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });
    let active_url = format!("http://{addr}/page?active");
    let peer_url = format!("http://{addr}/page?peer");
    let replacement_url = format!("http://{addr}/page?replacement");

    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &active_url, "SID-page", "TID-page").await;

    let worker_source = r#"
let connectionCount = 0;
globalThis.__runtimeOwnerProbe = "worker:" + name + ":" + (
  typeof SharedWorkerGlobalScope !== "undefined" &&
  self instanceof SharedWorkerGlobalScope
);
onconnect = event => {
  connectionCount += 1;
  const port = event.ports[0];
  port.onmessage = event => {
    if (event.data === "count") {
      port.postMessage("count:" + connectionCount);
    }
  };
  port.start();
  port.postMessage("ready:" + connectionCount);
};
"#;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_700,
        "survive-owner-navigation",
        worker_source,
    )
    .await;
    let shared_worker_target_id = ctx
        .conn
        .shared_worker_target_for_session(Some(&shared_worker_session_id))
        .expect("shared worker target should exist after first client")
        .target_id
        .clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_710,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-1", "url": "about:blank#peer"}
    }))
    .await;
    let created = ctx.take_first_matching("peer targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
    });
    let peer_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("peer target id")
        .to_owned();
    let attached = ctx.take_first_matching("peer attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"].as_str() == Some(peer_target_id.as_str())
    });
    let peer_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("peer target session id")
        .to_owned();
    ctx.expect_result(90_710, json!({ "targetId": peer_target_id }), None);

    ctx.process_async(json!({
        "id": 90_711,
        "method": "Page.navigate",
        "sessionId": peer_session_id,
        "params": {"url": peer_url}
    }))
    .await;
    let peer_navigation = take_response_by_id(&mut ctx, 90_711);
    assert_eq!(peer_navigation["result"]["frameId"], json!(peer_target_id));
    ctx.take_all();

    let source_literal =
        serde_json::to_string(worker_source).expect("worker source should serialize");
    ctx.process_async(json!({
        "id": 90_712,
        "method": "Runtime.evaluate",
        "sessionId": peer_session_id,
        "params": {
            "expression": format!(
                r#"
(() => {{
  const source = {source_literal};
  const url = "data:text/javascript," + encodeURIComponent(source);
  const worker = new SharedWorker(url, "survive-owner-navigation");
  globalThis.__sharedWorkerPeerProbe = worker;
  globalThis.__sharedWorkerPeerMessages = [];
  worker.port.onmessage = event => {{
    globalThis.__sharedWorkerPeerMessages.push(String(event.data));
  }};
  worker.port.start();
  return "peer-started";
}})()
"#
            ),
            "returnByValue": true
        }
    }))
    .await;
    let peer_start = take_response_by_id(&mut ctx, 90_712);
    assert_eq!(
        peer_start["result"]["result"]["value"],
        json!("peer-started")
    );

    wait_until_runtime_expression_true(
        &mut ctx,
        Some(&peer_session_id),
        90_713,
        "globalThis.__sharedWorkerPeerMessages.includes('ready:2')",
        "peer SharedWorker client to connect",
    )
    .await;
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .renderer_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "same-origin pages with the same SharedWorker key should share one running worker isolate"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_714,
        "method": "Page.navigate",
        "sessionId": "SID-page",
        "params": {"url": replacement_url}
    }))
    .await;
    let active_navigation = take_response_by_id(&mut ctx, 90_714);
    assert_eq!(active_navigation["result"]["frameId"], json!("TID-page"));
    let navigation_messages = ctx.take_all();
    assert!(
        navigation_messages.iter().all(|message| {
            let is_shared_worker_teardown = (message["method"]
                == json!("Target.detachedFromTarget")
                || message["method"] == json!("Target.targetDestroyed"))
                && message["params"]["targetId"] == json!(shared_worker_target_id);
            !is_shared_worker_teardown
        }),
        "owner page navigation must not detach/destroy a SharedWorker target with a peer client: {navigation_messages:?}"
    );
    assert!(
        ctx.conn
            .shared_worker_target_for_session(Some(&shared_worker_session_id))
            .is_some(),
        "attached SharedWorker target state must survive owner page navigation while a peer client remains"
    );

    ctx.process_async(json!({
        "id": 90_715,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "globalThis.__runtimeOwnerProbe",
            "returnByValue": true
        }
    }))
    .await;
    let worker_probe = take_response_by_id(&mut ctx, 90_715);
    assert_eq!(
        worker_probe["result"]["result"]["value"],
        json!("worker:survive-owner-navigation:true"),
        "Runtime.evaluate on the attached SharedWorker target must still enter the worker global after owner page navigation"
    );

    ctx.process_async(json!({
        "id": 90_716,
        "method": "Runtime.evaluate",
        "sessionId": peer_session_id,
        "params": {
            "expression": "globalThis.__sharedWorkerPeerProbe.port.postMessage('count'); 'posted'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_716)["result"]["result"]["value"],
        json!("posted")
    );
    wait_until_runtime_expression_true(
        &mut ctx,
        Some(&peer_session_id),
        90_800,
        "globalThis.__sharedWorkerPeerMessages.includes('count:2')",
        "owner page navigation to remove only the owner page client endpoint",
    )
    .await;

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_await_promise_timer_reply_arrives_through_renderer_receiver() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

    ctx.process_async(json!({
        "id": 80,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(80, json!({}), None);

    ctx.process_async(json!({
        "id": 81,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
(() => {
  const source = `
    globalThis.__workerProbe = "deferred:" + name;
    globalThis.__pendingProbeResolve = null;
    onconnect = event => {
      const port = event.ports[0];
      port.onmessage = event => {
        if (event.data === "release" && globalThis.__pendingProbeResolve) {
          const resolve = globalThis.__pendingProbeResolve;
          globalThis.__pendingProbeResolve = null;
          resolve("late:" + globalThis.__workerProbe);
        }
      };
      port.postMessage("ready");
    };
  `;
  const url = "data:text/javascript," + encodeURIComponent(source);
  const worker = new SharedWorker(url, "deferred-runtime-probe");
  globalThis.__sharedWorkerDeferredRuntimeProbe = worker;
  worker.port.start();
  return "started";
})()
"#,
            "returnByValue": true
        }
    }))
    .await;
    let start_response = take_response_by_id(&mut ctx, 81);
    assert_eq!(
        start_response["result"]["result"]["value"],
        json!("started")
    );

    wait_until_message(
        &mut ctx,
        None,
        "shared worker target auto-attach",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["type"] == json!("shared_worker")
                && message["params"]["targetInfo"]["title"] == json!("deferred-runtime-probe")
        },
    )
    .await;
    let attached = ctx.take_first_matching("shared worker target auto-attach", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("shared_worker")
            && message["params"]["targetInfo"]["title"] == json!("deferred-runtime-probe")
    });
    let shared_worker_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("shared worker session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 82,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_response = take_response_by_id(&mut ctx, 82);
    assert_eq!(enable_response["sessionId"], shared_worker_session_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 83,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "new Promise(resolve => { globalThis.__pendingProbeResolve = resolve; })",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;

    assert!(
        !ctx.sent.iter().any(|message| message["id"] == json!(83)),
        "timer-backed shared worker awaitPromise should defer until the worker timer settles: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&shared_worker_session_id)),
        "shared worker awaitPromise should register against the shared worker target"
    );

    ctx.process_async(json!({
        "id": 84,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__sharedWorkerDeferredRuntimeProbe.port.postMessage('release'); 'released'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 84)["result"]["result"]["value"],
        json!("released")
    );

    wait_until_message(
        &mut ctx,
        None,
        "shared worker deferred awaitPromise",
        |message| message["id"] == json!(83),
    )
    .await;
    let response = take_response_by_id(&mut ctx, 83);
    assert_eq!(response["sessionId"], shared_worker_session_id);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("late:deferred:deferred-runtime-probe")
    );
    assert!(
        !ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&shared_worker_session_id)),
        "settled shared worker awaitPromise should clear the target pending-await entry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_await_promise_releases_dispatch_before_session_response() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        91_100,
        "deferred-receiver",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    let raw = json!({
        "id": 91_102,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "new Promise(() => {})",
            "awaitPromise": true,
            "returnByValue": true
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("shared worker Runtime.evaluate awaitPromise should start as a pending command");
    let completed = tokio::time::timeout(std::time::Duration::from_secs(1), pending.wait())
        .await
        .expect("initial Worker Inspector dispatch must not wait for Promise settlement");
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("a Worker session response must not enter the AdapterReply deferred-reply lane");
    };
    assert!(
        outcome.into_parts().0.is_empty(),
        "the unresolved terminal response must remain in the Worker session output journal"
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&shared_worker_session_id)),
        "the unresolved await must remain registered to its exact Worker session"
    );

    ctx.process_async(json!({
        "id": 91_103,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": shared_worker_session_id }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 91_103)["result"], json!({}));
    let failed_await = take_response_by_id(&mut ctx, 91_102);
    assert_eq!(failed_await["error"]["message"], json!("Target detached"));
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_release_object_group_is_target_local() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        90_900,
        "release-object-group",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 90_910,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "({ owner: 'shared-worker-group' })",
            "objectGroup": "shared-worker-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 90_910);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("shared worker Runtime.evaluate should return an object handle: {response:?}")
        })
        .to_owned();
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                std::slice::from_ref(&object_id),
            )
            .is_err(),
        "active page owner should see the grouped shared worker handle as another target's handle"
    );

    ctx.process_async(json!({
        "id": 90_911,
        "method": "Runtime.releaseObjectGroup",
        "sessionId": shared_worker_session_id,
        "params": {
            "objectGroup": "shared-worker-group"
        }
    }))
    .await;
    let release = take_response_by_id(&mut ctx, 90_911);
    assert_eq!(release["sessionId"], json!(shared_worker_session_id));
    assert_eq!(release["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(None, &[object_id])
            .is_ok(),
        "successful shared worker releaseObjectGroup should remove grouped handles from the target-local registry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_discard_console_entries_clears_v8_console_storage() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
    let shared_worker_session_id = start_attached_shared_worker_session(
        &mut ctx,
        91_020,
        "runtime-discard-native-console",
        r#"
onconnect = event => {
  event.ports[0].postMessage("ready");
};
"#,
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 91_021,
        "method": "Runtime.evaluate",
        "sessionId": shared_worker_session_id,
        "params": {
            "expression": "console.warn('worker native boot warning'); 'logged'",
            "returnByValue": true
        }
    }))
    .await;
    let log_response = take_response_by_id(&mut ctx, 91_021);
    assert_eq!(log_response["sessionId"], json!(shared_worker_session_id));
    assert_eq!(log_response["result"]["result"]["value"], json!("logged"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 91_022,
        "method": "Runtime.discardConsoleEntries",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let discard = take_response_by_id(&mut ctx, 91_022);
    assert_eq!(discard["sessionId"], json!(shared_worker_session_id));
    assert_eq!(discard["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 91_023,
        "method": "Runtime.enable",
        "sessionId": shared_worker_session_id
    }))
    .await;
    let enable_messages = ctx.take_all();
    assert!(
        enable_messages.iter().any(|message| {
            message["id"] == json!(91_023)
                && message["sessionId"] == json!(shared_worker_session_id)
                && message["result"] == json!({})
        }),
        "Runtime.enable should complete on the shared worker session: {enable_messages:?}"
    );
    assert!(
        enable_messages.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!(shared_worker_session_id)
        }),
        "Runtime.enable should still replay the shared worker execution context: {enable_messages:?}"
    );
    assert!(
        enable_messages.iter().all(|message| {
            message["method"] != json!("Runtime.consoleAPICalled")
                || message["sessionId"] != json!(shared_worker_session_id)
                || message["params"]["args"].as_array().is_none_or(|args| {
                    !args
                        .iter()
                        .any(|arg| arg["value"] == json!("worker native boot warning"))
                })
        }),
        "Runtime.discardConsoleEntries should clear V8 buffered console storage before Runtime.enable replay: {enable_messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_discard_console_entries_is_target_local() {
    let mut ctx = TestContext::new();
    load_shared_worker_target(&mut ctx, "SID-shared-worker");
    {
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should exist");
        browser_context.set_active_target_id("TID-page");
        browser_context.attach_active_session("SID-page");
    }
    record_shared_worker_console(&mut ctx, "SID-shared-worker", "warn: worker boot warning");

    ctx.process_async(json!({
        "id": 91_010,
        "method": "Runtime.discardConsoleEntries",
        "sessionId": "SID-shared-worker"
    }))
    .await;
    let discard = take_response_by_id(&mut ctx, 91_010);
    assert_eq!(discard["sessionId"], json!("SID-shared-worker"));
    assert_eq!(discard["result"], json!({}));

    ctx.process_async(json!({
        "id": 91_011,
        "method": "Runtime.enable",
        "sessionId": "SID-shared-worker"
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 91_011)["result"], json!({}));
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.consoleAPICalled")
                || message["sessionId"] != json!("SID-shared-worker")
        }),
        "Runtime.discardConsoleEntries should suppress buffered shared worker Runtime console replay: {:?}",
        ctx.sent
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-page")),
        Some(crate::conn::CdpSessionRoute::PageTarget {
            browser_context_id: "BID-shared".to_owned(),
            target_id: "TID-page".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        }),
        "shared worker Runtime.discardConsoleEntries must not consume or rewrite the page session route"
    );
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.consoleAPICalled")
                || message.get("sessionId").is_some()
        }),
        "shared worker Runtime console output must not replay on the active page session: {:?}",
        ctx.sent
    );
}
