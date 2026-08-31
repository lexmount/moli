use super::*;

/// Runtime.enable is a no-op that succeeds.
#[tokio::test]
async fn enable_succeeds() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 1, "method": "Runtime.enable"}))
        .await;
    ctx.expect_result(1, json!({}), None);
}
/// Runtime.enable on a loaded page emits executionContextCreated.
#[tokio::test]
async fn enable_with_page_emits_execution_context_created() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({"id": 11, "method": "Runtime.enable"}))
        .await;

    ctx.expect_result(11, json!({}), None);
    ctx.expect_event(
        "Runtime.executionContextCreated",
        Some(&json!({
            "context": {
                "name": "data:text/html,<html><title>ok</title><body></body></html>",
                "origin": "://"
            }
        })),
    );
}

#[tokio::test]
async fn devtools_console_runtime_commands_accept_emitted_unique_context_id() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;

    ctx.process_async(json!({"id": 12, "method": "Runtime.enable"}))
        .await;
    let response = take_response_by_id(&mut ctx, 12);
    assert_eq!(response["result"], json!({}));
    let unique_context_id = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .and_then(|message| message["params"]["context"]["uniqueId"].as_str())
        .map(str::to_owned)
        .expect("Runtime.enable should emit an execution context uniqueId");
    assert!(
        unique_context_id.starts_with("TID-1:"),
        "the browser-global realm id should be qualified with its target owner: {unique_context_id}"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "21 * 2",
            "objectGroup": "console",
            "includeCommandLineAPI": true,
            "silent": false,
            "returnByValue": false,
            "generatePreview": true,
            "userGesture": true,
            "awaitPromise": false,
            "replMode": true,
            "allowUnsafeEvalBlockedByCSP": true,
            "uniqueContextId": unique_context_id.clone()
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 13);
    assert_eq!(response["result"]["result"]["type"], json!("number"));
    assert_eq!(response["result"]["result"]["value"], json!(42));

    ctx.process_async(json!({
        "id": 14,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "function(a, b) { return a * b; }",
            "arguments": [{"value": 6}, {"value": 7}],
            "returnByValue": true,
            "uniqueContextId": unique_context_id.clone()
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 14);
    assert_eq!(
        response["result"]["result"]["type"],
        json!("number"),
        "Runtime.callFunctionOn should accept the emitted unique context id: {response:?}"
    );
    assert_eq!(response["result"]["result"]["value"], json!(42));

    let native_realm_id = unique_context_id
        .strip_prefix("TID-1:")
        .expect("test realm id should carry its target owner");
    ctx.process_async(json!({
        "id": 15,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__wrongRealmWasEvaluated = true",
            "uniqueContextId": format!("TID-other:{native_realm_id}")
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 15);
    assert_eq!(response["error"]["code"], json!(-32602));
    assert_eq!(
        response["error"]["message"],
        json!("invalid uniqueContextId")
    );
}

#[tokio::test]
async fn emitted_isolated_world_unique_context_id_selects_that_realm() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let isolated_context_id = create_isolated_world_async(&mut ctx, 16, "console-utility").await;

    ctx.process_async(json!({"id": 17, "method": "Runtime.enable"}))
        .await;
    let response = take_response_by_id(&mut ctx, 17);
    assert_eq!(response["result"], json!({}));
    let unique_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["id"] == json!(isolated_context_id)
        })
        .and_then(|message| message["params"]["context"]["uniqueId"].as_str())
        .map(str::to_owned)
        .expect("Runtime.enable should replay the isolated world uniqueId");

    ctx.process_async(json!({
        "id": 18,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__uniqueRealmMarker = 'isolated'",
            "returnByValue": true,
            "uniqueContextId": unique_context_id
        }
    }))
    .await;
    let isolated = take_response_by_id(&mut ctx, 18);
    assert_eq!(isolated["result"]["result"]["value"], json!("isolated"));

    ctx.process_async(json!({
        "id": 19,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "typeof globalThis.__uniqueRealmMarker",
            "returnByValue": true
        }
    }))
    .await;
    let default_world = take_response_by_id(&mut ctx, 19);
    assert_eq!(
        default_world["result"]["result"]["value"],
        json!("undefined"),
        "uniqueContextId must not fall back to the default world"
    );
}

#[tokio::test]
async fn emitted_child_frame_unique_context_id_selects_that_realm() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        r#"<html><body>parent<iframe srcdoc="<body>child realm</body>"></iframe></body></html>"#,
    )
    .await;

    ctx.process_async(json!({"id": 20, "method": "Page.getFrameTree"}))
        .await;
    let child_frame_id = take_response_by_id(&mut ctx, 20)["result"]["frameTree"]["childFrames"][0]
        ["frame"]["id"]
        .as_str()
        .map(str::to_owned)
        .expect("loaded iframe should appear in Page.getFrameTree");
    ctx.process_async(json!({
        "id": 21,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": child_frame_id,
            "worldName": "child-materialization-barrier"
        }
    }))
    .await;
    let materialized = take_response_by_id(&mut ctx, 21);
    assert!(
        materialized["result"]["executionContextId"]
            .as_i64()
            .is_some(),
        "the child realm materialization barrier should complete: {materialized:?}"
    );

    ctx.process_async(json!({"id": 22, "method": "Runtime.enable"}))
        .await;
    let response = take_response_by_id(&mut ctx, 22);
    assert_eq!(response["result"], json!({}));
    let expected_child_frame_id = child_frame_id.clone();
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "child default execution context after Runtime.enable",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;
    let unique_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["uniqueId"].as_str())
        .map(str::to_owned)
        .expect("Runtime.enable should publish the child frame uniqueId");

    ctx.process_async(json!({
        "id": 23,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body.textContent.trim()",
            "objectGroup": "console",
            "includeCommandLineAPI": true,
            "generatePreview": true,
            "uniqueContextId": unique_context_id
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 23);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("child realm"),
        "DevTools must be able to evaluate in every execution context it is given"
    );
}

#[tokio::test]
async fn enable_with_http_page_emits_serialized_security_origin() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>runtime origin</title>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });
    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;

    ctx.process_async(json!({
        "id": 11_100,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 11_100);
    assert_eq!(response["result"], json!({}));
    let created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .expect("Runtime.enable should report the existing HTTP execution context");
    assert_eq!(
        created["params"]["context"]["origin"],
        json!(format!("http://{addr}"))
    );
    assert_eq!(created["params"]["context"]["name"], json!(page_url));

    server.abort();
}
#[tokio::test]
async fn enable_with_page_can_complete_through_pending_command_task() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({"id": 11_001, "method": "Runtime.enable"}).to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;

    assert!(
        scheduler_events.is_empty(),
        "Runtime.enable should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(11_001) && message["result"] == json!({})),
        "pending Runtime.enable should emit command success: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["method"] == json!("Runtime.executionContextCreated")),
        "pending Runtime.enable should replay existing context events: {messages:?}"
    );
}

#[tokio::test]
async fn loaded_page_runtime_enable_projection_waits_for_v8_success() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({"id": 11_002, "method": "Runtime.enable"}).to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "loaded-page Runtime.enable should dispatch through V8 inspector first"
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_state()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled,
        "protocol Runtime.enabled projection must not flip before V8 Runtime.enable succeeds"
    );

    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(
        scheduler_events.is_empty(),
        "Runtime.enable should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(11_002) && message["result"] == json!({})),
        "pending Runtime.enable should emit command success: {messages:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_state()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled,
        "protocol Runtime.enabled projection should flip after V8 Runtime.enable succeeds"
    );
}

#[tokio::test]
async fn loaded_page_run_if_waiting_for_debugger_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({"id": 11_003, "method": "Runtime.runIfWaitingForDebugger"}).to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "loaded-page Runtime.runIfWaitingForDebugger should dispatch through V8 Runtime agent"
    );

    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(
        scheduler_events.is_empty(),
        "Runtime.runIfWaitingForDebugger should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(11_003) && message["result"] == json!({})),
        "pending Runtime.runIfWaitingForDebugger should emit V8 inspector success: {messages:?}"
    );
}

#[tokio::test]
async fn loaded_page_runtime_agent_state_commands_require_runtime_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    for (id, method, params) in [
        (
            11_004_u64,
            "Runtime.setCustomObjectFormatterEnabled",
            json!({ "enabled": true }),
        ),
        (
            11_005_u64,
            "Runtime.setMaxCallStackSizeToCapture",
            json!({ "size": 8 }),
        ),
    ] {
        let raw = json!({
            "id": id,
            "method": method,
            "params": params
        })
        .to_string();
        let step = ctx.conn.start_command_dispatch(&raw);
        assert!(
            matches!(&step, CdpCommandTaskStep::Pending(_)),
            "{method} should dispatch to V8 Runtime agent instead of failing as UnknownMethod"
        );

        let (messages, scheduler_events) =
            complete_command_task_step_for_test(&mut ctx, step).await;
        assert!(
            scheduler_events.is_empty(),
            "{method} should not enqueue scheduler work: {scheduler_events:?}"
        );
        assert!(
            messages.iter().any(|message| {
                message["id"] == json!(id)
                    && message["error"]["message"] == json!("Runtime agent is not enabled")
            }),
            "{method} should return V8 Runtime agent disabled error: {messages:?}"
        );
    }
}

#[tokio::test]
async fn loaded_page_set_async_call_stack_depth_requires_runtime_or_debugger_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({
        "id": 11_014,
        "method": "Runtime.setAsyncCallStackDepth",
        "params": {
            "maxDepth": 8
        }
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "Runtime.setAsyncCallStackDepth should dispatch to V8 Debugger agent instead of failing as UnknownMethod"
    );

    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(
        scheduler_events.is_empty(),
        "Runtime.setAsyncCallStackDepth should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(11_014)
                && message["error"]["message"] == json!("Debugger agent is not enabled")
        }),
        "Runtime.setAsyncCallStackDepth should return V8 Debugger disabled error: {messages:?}"
    );
}

#[tokio::test]
async fn loaded_page_runtime_agent_state_commands_dispatch_after_runtime_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;
    enable_runtime_and_take_execution_context_id_async(&mut ctx, 11_006).await;

    for (id, method, params) in [
        (
            11_007_u64,
            "Runtime.setCustomObjectFormatterEnabled",
            json!({ "enabled": true }),
        ),
        (
            11_008_u64,
            "Runtime.setMaxCallStackSizeToCapture",
            json!({ "size": 8 }),
        ),
        (
            11_015_u64,
            "Runtime.setAsyncCallStackDepth",
            json!({ "maxDepth": 8 }),
        ),
    ] {
        let raw = json!({
            "id": id,
            "method": method,
            "params": params
        })
        .to_string();
        let step = ctx.conn.start_command_dispatch(&raw);
        assert!(
            matches!(&step, CdpCommandTaskStep::Pending(_)),
            "{method} should dispatch through V8 Runtime agent"
        );

        let (messages, scheduler_events) =
            complete_command_task_step_for_test(&mut ctx, step).await;
        assert!(
            scheduler_events.is_empty(),
            "{method} should not enqueue scheduler work: {scheduler_events:?}"
        );
        assert!(
            messages
                .iter()
                .any(|message| message["id"] == json!(id) && message["result"] == json!({})),
            "{method} should return V8 Runtime agent success: {messages:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_agent_configuration_is_restored_on_replacement_page_isolate() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;
    enable_runtime_and_take_execution_context_id_async(&mut ctx, 11_016).await;

    for (id, method, params) in [
        (
            11_017_u64,
            "Runtime.setAsyncCallStackDepth",
            json!({ "maxDepth": 7 }),
        ),
        (
            11_018_u64,
            "Runtime.setCustomObjectFormatterEnabled",
            json!({ "enabled": true }),
        ),
        (
            11_019_u64,
            "Runtime.setMaxCallStackSizeToCapture",
            json!({ "size": 23 }),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    let browser_context = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist");
    let v8_state_before_navigation = browser_context.active_page_state().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .inspector_session_state
        .v8_state
        .clone()
        .expect("successful Runtime agent commands must persist an opaque V8 state cookie");
    assert!(
        v8_state_before_navigation
            .as_bytes()
            .windows(b"customObjectFormatterEnabled".len())
            .any(|window| window == b"customObjectFormatterEnabled"),
        "successful Runtime agent commands must persist an opaque V8 state cookie"
    );

    ctx.process_async(json!({
        "id": 11_020,
        "method": "Page.navigate",
        "params": { "url": "data:text/html,<!doctype html><body>after</body>" }
    }))
    .await;
    let navigate = take_response_by_id(&mut ctx, 11_020);
    assert!(
        navigate["result"]["frameId"].is_string(),
        "navigation should rebuild the Inspector backend with Runtime configuration: {navigate:?}"
    );
    let v8_state_after_navigation = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist")
        .active_page_state()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .inspector_session_state
        .v8_state
        .as_ref()
        .expect("replacement Inspector session should publish its restored state");
    assert_eq!(
        v8_state_after_navigation.as_bytes(),
        v8_state_before_navigation.as_bytes(),
        "replacement V8 session should preserve the opaque Runtime configuration cookie"
    );

    ctx.process_async(json!({
        "id": 11_022,
        "method": "Runtime.evaluate",
        "params": {
            "generatePreview": true,
            "expression": r#"(() => {
  globalThis.devtoolsFormatters = [{
    header(value) {
      return value && value.cookieRestored
        ? ["span", {}, "opaque-runtime-cookie"]
        : null;
    },
    hasBody() { return false; },
    body() { return null; }
  }];
  return {cookieRestored: true};
})()"#
        }
    }))
    .await;
    let formatted = take_response_by_id(&mut ctx, 11_022);
    assert!(
        formatted["result"]["result"]["customPreview"]["header"]
            .as_str()
            .is_some_and(|header| header.contains("opaque-runtime-cookie")),
        "Runtime custom formatter state should restore from the opaque cookie when its typed projection is empty: {formatted:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn opaque_reattach_state_wins_over_conflicting_runtime_listener_configuration() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;

    ctx.process_async(json!({"id": 11_023, "method": "Runtime.enable"}))
        .await;
    ctx.expect_result(11_023, json!({}), None);
    ctx.process_async(json!({"id": 11_024, "method": "Runtime.disable"}))
        .await;
    ctx.expect_result(11_024, json!({}), None);

    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    assert!(
        browser_context.active_page_state().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .inspector_session_state
            .v8_state
            .is_some(),
        "successful Runtime.disable must persist the disabled V8 agent cookie"
    );
    browser_context.active_page_state_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 11_025,
        "method": "Page.navigate",
        "params": { "url": "data:text/html,<!doctype html><body>after</body>" }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 11_025)["result"]["frameId"].is_string(),
        "navigation should attach the replacement V8 session"
    );

    ctx.process_async(json!({
        "id": 11_026,
        "method": "Runtime.setCustomObjectFormatterEnabled",
        "params": { "enabled": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 11_026);
    assert_eq!(
        response["error"]["message"],
        json!("Runtime agent is not enabled"),
        "a reattach cookie must take precedence over conflicting protocol listener configuration: {response:?}"
    );
}

#[tokio::test]
async fn loaded_page_terminate_execution_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({
        "id": 11_009,
        "method": "Runtime.terminateExecution"
    })
    .to_string();
    let response_start = ctx.sent.len();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "Runtime.terminateExecution should dispatch to V8 Runtime agent instead of failing as UnknownMethod"
    );

    let (mut messages, scheduler_events) =
        complete_command_task_step_for_test(&mut ctx, step).await;
    if !messages
        .iter()
        .any(|message| message["id"] == json!(11_009))
    {
        ctx.wait_for_test_command_response(11_009, response_start)
            .await;
        messages.push(ctx.take_response_by_id(11_009));
    }
    assert!(
        scheduler_events.is_empty(),
        "Runtime.terminateExecution should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(11_009) && message["result"] == json!({})),
        "Runtime.terminateExecution should return V8 Runtime agent success: {messages:?}"
    );
}

#[tokio::test]
async fn loaded_page_get_isolate_id_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({
        "id": 11_010,
        "method": "Runtime.getIsolateId"
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "Runtime.getIsolateId should dispatch to V8 Runtime agent instead of failing as UnknownMethod"
    );

    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(
        scheduler_events.is_empty(),
        "Runtime.getIsolateId should not enqueue scheduler work: {scheduler_events:?}"
    );
    let isolate_id = messages
        .iter()
        .find(|message| message["id"] == json!(11_010))
        .and_then(|message| message["result"]["id"].as_str())
        .expect("Runtime.getIsolateId should return result.id");
    assert!(
        !isolate_id.is_empty() && isolate_id.chars().all(|ch| ch.is_ascii_hexdigit()),
        "Runtime.getIsolateId should return V8 isolate id as hex: {messages:?}"
    );
}

#[tokio::test]
async fn loaded_page_get_exception_details_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;
    enable_runtime_and_take_execution_context_id_async(&mut ctx, 11_011).await;

    ctx.process_async(json!({
        "id": 11_012,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "new Error('moli exception details')"
        }
    }))
    .await;
    let evaluated = take_response_by_id(&mut ctx, 11_012);
    let error_object_id = evaluated["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.evaluate should return an Error object handle: {evaluated:?}")
        })
        .to_owned();

    let raw = json!({
        "id": 11_013,
        "method": "Runtime.getExceptionDetails",
        "params": {
            "errorObjectId": error_object_id
        }
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "Runtime.getExceptionDetails should dispatch to V8 Runtime agent instead of failing as UnknownMethod"
    );

    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(
        scheduler_events.is_empty(),
        "Runtime.getExceptionDetails should not enqueue scheduler work: {scheduler_events:?}"
    );
    let details_text = messages
        .iter()
        .find(|message| message["id"] == json!(11_013))
        .and_then(|message| message["result"]["exceptionDetails"]["text"].as_str())
        .expect("Runtime.getExceptionDetails should return result.exceptionDetails.text");
    assert!(
        details_text.contains("moli exception details"),
        "Runtime.getExceptionDetails should return V8 exception details for the Error object: {messages:?}"
    );
}

#[tokio::test]
async fn get_exception_details_rejects_error_object_id_known_to_different_target_owner() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>owner-a</body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 11_014).await;

    ctx.process_async(json!({
        "id": 11_015,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "new Error('owner-a exception')"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 11_015);
    let error_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return an Error handle: {response:?}"))
        .to_owned();

    push_loaded_runtime_frontend_enabled_background_context_async(
        &mut ctx,
        "BID-2",
        "TID-2",
        "SID-2",
        "<html><body>owner-b</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 11_016,
        "method": "Runtime.getExceptionDetails",
        "sessionId": "SID-2",
        "params": {
            "errorObjectId": error_object_id
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 11_016);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );
}

#[tokio::test]
async fn loaded_page_global_lexical_scope_names_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        r#"<html><body><script>
let __lmLexicalLet = 1;
const __lmLexicalConst = 2;
class __LmLexicalClass {}
</script></body></html>"#,
    )
    .await;

    let raw = json!({
        "id": 11_010,
        "method": "Runtime.globalLexicalScopeNames"
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "Runtime.globalLexicalScopeNames should dispatch to V8 Runtime agent instead of failing as UnknownMethod"
    );

    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;
    assert!(
        scheduler_events.is_empty(),
        "Runtime.globalLexicalScopeNames should not enqueue scheduler work: {scheduler_events:?}"
    );
    let names = messages
        .iter()
        .find(|message| message["id"] == json!(11_010))
        .and_then(|message| message["result"]["names"].as_array())
        .expect("Runtime.globalLexicalScopeNames should return result.names");
    for expected in ["__lmLexicalLet", "__lmLexicalConst", "__LmLexicalClass"] {
        assert!(
            names.iter().any(|name| name.as_str() == Some(expected)),
            "Runtime.globalLexicalScopeNames should include {expected}: {messages:?}"
        );
    }
}

#[tokio::test]
async fn loaded_page_query_objects_dispatches_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        r#"<html><body><script>
class __LmQueryObjectsThing {
  constructor(value) {
    this.value = value;
  }
}
globalThis.__lmQueryObjectsThings = [
  new __LmQueryObjectsThing(1),
  new __LmQueryObjectsThing(2),
];
</script></body></html>"#,
    )
    .await;

    ctx.process_async(json!({
        "id": 11_011,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "__LmQueryObjectsThing.prototype",
            "objectGroup": "query-prototype"
        }
    }))
    .await;
    let prototype_response = take_response_by_id(&mut ctx, 11_011);
    let prototype_object_id = prototype_response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.evaluate should return a prototype handle: {prototype_response:?}")
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 11_012,
        "method": "Runtime.queryObjects",
        "params": {
            "prototypeObjectId": prototype_object_id,
            "objectGroup": "query-results"
        }
    }))
    .await;
    let query_response = take_response_by_id(&mut ctx, 11_012);
    let objects_id = query_response["result"]["objects"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.queryObjects should return an array handle: {query_response:?}")
        })
        .to_owned();
    let devtools_session_state = ctx
        .conn
        .target_devtools_session_state_for_session(None)
        .expect("default DevTools session state should exist");
    assert!(
        devtools_session_state.has_runtime_remote_object_id(&objects_id),
        "Runtime.queryObjects result handle should be tracked: {query_response:?}"
    );
    assert_eq!(
        devtools_session_state.runtime_remote_object_group(&objects_id),
        Some("query-results"),
        "Runtime.queryObjects should register its result in the explicit objectGroup"
    );

    ctx.process_async(json!({
        "id": 11_013,
        "method": "Runtime.releaseObjectGroup",
        "params": {
            "objectGroup": "query-results"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 11_013)["result"], json!({}));
    let devtools_session_state = ctx
        .conn
        .target_devtools_session_state_for_session(None)
        .expect("default DevTools session state should exist");
    assert!(
        !devtools_session_state.has_runtime_remote_object_id(&objects_id),
        "Runtime.releaseObjectGroup should clear the queryObjects result handle"
    );
    assert_eq!(
        devtools_session_state.runtime_remote_object_group(&objects_id),
        None,
        "Runtime.releaseObjectGroup should clear the queryObjects result group"
    );
}

#[tokio::test]
async fn loaded_page_compile_and_run_script_dispatch_through_v8_runtime_agent() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    enable_runtime_and_take_execution_context_id_async(&mut ctx, 11_014).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11_015,
        "method": "Runtime.compileScript",
        "params": {
            "expression": "({ compiled: 42 })",
            "sourceURL": "moli://runtime-compile-script/page.js",
            "persistScript": true
        }
    }))
    .await;
    let compile_response = take_response_by_id(&mut ctx, 11_015);
    let script_id = compile_response["result"]["scriptId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.compileScript should return a V8 scriptId: {compile_response:?}")
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 11_016,
        "method": "Runtime.runScript",
        "params": {
            "scriptId": script_id,
            "objectGroup": "compiled-script-results"
        }
    }))
    .await;
    let run_response = take_response_by_id(&mut ctx, 11_016);
    let object_id = run_response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.runScript should return an object handle: {run_response:?}")
        })
        .to_owned();
    let devtools_session_state = ctx
        .conn
        .target_devtools_session_state_for_session(None)
        .expect("default DevTools session state should exist");
    assert!(
        devtools_session_state.has_runtime_remote_object_id(&object_id),
        "Runtime.runScript result handle should be tracked: {run_response:?}"
    );
    assert_eq!(
        devtools_session_state.runtime_remote_object_group(&object_id),
        Some("compiled-script-results"),
        "Runtime.runScript should register its result in the explicit objectGroup"
    );

    ctx.process_async(json!({
        "id": 11_017,
        "method": "Runtime.releaseObjectGroup",
        "params": {
            "objectGroup": "compiled-script-results"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 11_017)["result"], json!({}));
    let devtools_session_state = ctx
        .conn
        .target_devtools_session_state_for_session(None)
        .expect("default DevTools session state should exist");
    assert!(
        !devtools_session_state.has_runtime_remote_object_id(&object_id),
        "Runtime.releaseObjectGroup should clear the runScript result handle"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_emits_buffered_console_api_called() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><script>console.warn('boot warning')</script>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_681).await;

    ctx.expect_event(
        "Runtime.consoleAPICalled",
        Some(&json!({
            "type": "warning",
            "args": [
                {
                    "type": "string",
                    "value": "boot warning"
                }
            ],
            "executionContextId": execution_context_id,
        })),
    );
}
#[tokio::test]
async fn runtime_enable_replays_existing_isolated_world_contexts() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    let _ = create_isolated_world_async(&mut ctx, 14, "utility").await;

    ctx.process_async(json!({"id": 15, "method": "Runtime.enable", "sessionId": "SID-1"}))
        .await;
    let response = take_response_by_id(&mut ctx, 15);
    assert_eq!(response["result"], json!({}));

    let created = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(created.len(), 2);
    assert!(
        created
            .iter()
            .any(|message| { message["params"]["context"]["auxData"]["isDefault"] == json!(true) })
    );
    assert!(created.iter().any(|message| {
        message["params"]["context"]["name"] == json!("utility")
            && message["params"]["context"]["auxData"]["isDefault"] == json!(false)
            && message["params"]["context"]["auxData"]["type"] == json!("isolated")
    }));
}
#[tokio::test]
async fn runtime_enable_replays_multiple_isolated_worlds_in_registration_order() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    ctx.process_async(json!({
        "id": 140,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-a"
        }
    }))
    .await;
    let world_a_id = take_response_by_id(&mut ctx, 140)["result"]["executionContextId"]
        .as_i64()
        .expect("first isolated world id");

    ctx.process_async(json!({
        "id": 141,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-b",
            "grantUniversalAccess": true
        }
    }))
    .await;
    let world_b_id = take_response_by_id(&mut ctx, 141)["result"]["executionContextId"]
        .as_i64()
        .expect("second isolated world id");
    ctx.sent.clear();

    ctx.process_async(json!({"id": 142, "method": "Runtime.enable", "sessionId": "SID-1"}))
        .await;
    let response = take_response_by_id(&mut ctx, 142);
    assert_eq!(response["result"], json!({}));

    let created = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(created.len(), 3);

    assert!(
        created
            .iter()
            .all(|message| message["sessionId"] == "SID-1"),
        "Runtime.enable replay should keep events scoped to the attached session: {created:?}"
    );
    assert!(
        created
            .iter()
            .any(|message| message["params"]["context"]["auxData"]["isDefault"] == json!(true)),
        "Runtime.enable replay should include the default context: {created:?}"
    );

    let world_a = created
        .iter()
        .find(|message| message["params"]["context"]["id"] == json!(world_a_id))
        .expect("Runtime.enable replay should include utility-a");
    assert_eq!(world_a["params"]["context"]["name"], "utility-a");
    assert_eq!(world_a["params"]["context"]["auxData"]["isDefault"], false);
    assert_eq!(
        world_a["params"]["context"]["auxData"]["grantUniversalAccess"],
        false
    );
    assert!(
        world_a["params"]["context"]["uniqueId"].as_str().is_some(),
        "isolated utility-a context should come from V8 RuntimeAgent native replay: {world_a:?}"
    );

    let world_b = created
        .iter()
        .find(|message| message["params"]["context"]["id"] == json!(world_b_id))
        .expect("Runtime.enable replay should include utility-b");
    assert_eq!(world_b["params"]["context"]["name"], "utility-b");
    assert_eq!(world_b["params"]["context"]["auxData"]["isDefault"], false);
    assert_eq!(
        world_b["params"]["context"]["auxData"]["grantUniversalAccess"],
        true
    );
    assert!(
        world_b["params"]["context"]["uniqueId"].as_str().is_some(),
        "isolated utility-b context should come from V8 RuntimeAgent native replay: {world_b:?}"
    );
}
#[tokio::test]
async fn isolated_world_keeps_specialized_input_wrapper_surface() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><input id='chooser' type='file' multiple></body></html>",
    )
    .await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");

    ctx.process_async(json!({
        "id": 148,
        "method": "Runtime.evaluate",
        "params": {
            "returnByValue": true,
            "expression": "(() => document.querySelector('#chooser').type)()"
        }
    }))
    .await;
    let main_world_response = take_response_by_id(&mut ctx, 148);
    assert_eq!(
        main_world_response["result"]["result"]["value"],
        json!("file")
    );

    let utility_context_id = create_isolated_world_async(&mut ctx, 149, "utility").await;

    ctx.process_async(json!({
        "id": 150,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": utility_context_id,
            "returnByValue": true,
            "expression": "(() => { const input = document.querySelector('#chooser'); return [input instanceof HTMLInputElement, input.constructor && input.constructor.name, typeof input.type, input.type, input.multiple].join('|'); })()"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 150);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("true|HTMLInputElement|string|file|true")
    );
}
#[tokio::test]
async fn document_replacement_does_not_reuse_stale_body_wrapper_for_new_input_handle() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    let default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 150).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 1_501, "utility").await;

    ctx.process_async(json!({
        "id": 151,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body"
        }
    }))
    .await;
    let body_object_id = take_response_by_id(&mut ctx, 151)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("document.body should return an objectId");

    ctx.process_async(json!({
        "id": 152,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": body_object_id,
            "returnByValue": true,
            "functionDeclaration": "function() { return this.constructor && this.constructor.name; }"
        }
    }))
    .await;
    let before = take_response_by_id(&mut ctx, 152);
    assert_eq!(
        before["result"]["result"]["value"],
        json!("HTMLBodyElement")
    );

    ctx.process_async(json!({
        "id": 153,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open(); document.write(\"<input id='chooser' type='file' multiple>\"); document.close(); document.querySelector('#chooser')"
        }
    }))
    .await;
    let input_object_id = take_response_by_id(&mut ctx, 153)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("replacement input should return an objectId");

    ctx.process_async(json!({
        "id": 154,
        "method": "DOM.describeNode",
        "params": {
            "objectId": input_object_id.clone()
        }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 154);
    assert_eq!(described["result"]["node"]["nodeName"], json!("INPUT"));
    assert_eq!(
        described["result"]["node"]["attributes"],
        json!(["id", "chooser", "type", "file", "multiple", ""])
    );
    let backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("describeNode should return backendNodeId");
    let backend_node_id_u32 =
        u32::try_from(backend_node_id).expect("backendNodeId should fit CDP u32 range");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id_u32),
        "live object describeNode should return renderer-owned backendNodeId"
    );

    ctx.process_async(json!({
        "id": 155,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": input_object_id,
            "returnByValue": true,
            "functionDeclaration": "function() { return [this instanceof HTMLInputElement, this.constructor && this.constructor.name, typeof this.type, this.type, this.multiple].join('|'); }"
        }
    }))
    .await;
    let after = take_response_by_id(&mut ctx, 155);
    assert_eq!(
        after["result"]["result"]["value"],
        json!("true|HTMLInputElement|string|file|true")
    );

    ctx.process_async(json!({
        "id": 156,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": backend_node_id,
            "executionContextId": default_context_id
        }
    }))
    .await;
    let resolved_object_id = take_response_by_id(&mut ctx, 156)["result"]["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("DOM.resolveNode should return an objectId");

    ctx.process_async(json!({
        "id": 157,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": resolved_object_id,
            "returnByValue": true,
            "functionDeclaration": "function() { return [this instanceof HTMLInputElement, this.constructor && this.constructor.name, typeof this.type, this.type, this.multiple].join('|'); }"
        }
    }))
    .await;
    let resolved_after = take_response_by_id(&mut ctx, 157);
    assert_eq!(
        resolved_after["result"]["result"]["value"],
        json!("true|HTMLInputElement|string|file|true")
    );

    ctx.process_async(json!({
        "id": 158,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": utility_context_id,
            "returnByValue": true,
            "expression": r#"(() => {
                const input = document.querySelector('#chooser');
                const rawNodeId = 1;
                return [
                    typeof __moliHostResolveNodeById,
                    typeof __moliHostResolveNodeIdForObject,
                    __moliHostResolveBackendNodeIdForObject(rawNodeId) === null,
                    __moliHostQuerySelector(rawNodeId, '#chooser') === null,
                    __moliHostQuerySelectorAll(rawNodeId, '*').length,
                    __moliHostMatches(rawNodeId, 'html'),
                    __moliHostClosest(rawNodeId, 'html') === null,
                    input instanceof HTMLInputElement,
                    input && input.constructor && input.constructor.name,
                    typeof input?.type,
                    input?.type,
                    input?.multiple
                ].join('|');
            })()"#
        }
    }))
    .await;
    let utility_surface = take_response_by_id(&mut ctx, 158);
    assert_eq!(
        utility_surface["result"]["result"]["value"],
        json!("undefined|undefined|true|true|0|false|true|true|HTMLInputElement|string|file|true")
    );

    ctx.process_async(json!({
        "id": 160,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": backend_node_id,
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let utility_resolved_object_id =
        take_response_by_id(&mut ctx, 160)["result"]["object"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("DOM.resolveNode should resolve replacement input in utility world");

    ctx.process_async(json!({
        "id": 161,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": utility_resolved_object_id,
            "returnByValue": true,
            "functionDeclaration": "function() { return [this instanceof HTMLInputElement, this.constructor && this.constructor.name, typeof this.type, this.type, this.multiple, this.isConnected, this.ownerDocument === document].join('|'); }"
        }
    }))
    .await;
    let utility_resolved_after = take_response_by_id(&mut ctx, 161);
    assert_eq!(
        utility_resolved_after["result"]["result"]["value"],
        json!("true|HTMLInputElement|string|file|true|true|true")
    );

    ctx.process_async(json!({
        "id": 162,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": utility_resolved_object_id,
            "functionDeclaration": "function() { return this.ownerDocument && this.ownerDocument.documentElement; }"
        }
    }))
    .await;
    let document_element_object_id =
        take_response_by_id(&mut ctx, 162)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("owner document element should carry an objectId");

    ctx.process_async(json!({
        "id": 163,
        "method": "DOM.describeNode",
        "params": {
            "objectId": document_element_object_id
        }
    }))
    .await;
    let described_document_element = take_response_by_id(&mut ctx, 163);
    assert_eq!(
        described_document_element["result"]["node"]["nodeName"],
        json!("HTML")
    );
    assert_eq!(
        described_document_element["result"]["node"]["frameId"],
        json!("TID-1")
    );
}
#[tokio::test]
async fn runtime_enable_after_navigation_replays_registered_named_world_contexts() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    let old_context_id = create_isolated_world_async(&mut ctx, 143, "utility").await;
    ctx.process_async(json!({
        "id": 146,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__afterNavMarker = 7; globalThis.__afterNavMarker",
            "contextId": old_context_id
        }
    }))
    .await;
    let old_marker = take_response_by_id(&mut ctx, 146);
    assert_eq!(old_marker["result"]["result"]["value"], 7);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1431,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "",
            "worldName": "utility"
        }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 1431)["result"]["identifier"]
            .as_str()
            .is_some()
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 144,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 144);
    ctx.sent.clear();

    ctx.process_async(json!({"id": 145, "method": "Runtime.enable", "sessionId": "SID-1"}))
        .await;
    let response = take_response_by_id(&mut ctx, 145);
    assert_eq!(response["result"], json!({}));

    let created = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(created.len(), 2);
    assert!(
        created
            .iter()
            .any(|message| message["params"]["context"]["auxData"]["isDefault"] == json!(true)),
        "Runtime.enable after navigation should include the replacement default context: {created:?}"
    );
    let isolated_context = created
        .iter()
        .find(|message| message["params"]["context"]["name"] == "utility")
        .expect("Runtime.enable after navigation should replay the utility isolated context");
    assert_eq!(
        isolated_context["params"]["context"]["auxData"]["isDefault"],
        false
    );
    assert!(
        isolated_context["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "replayed isolated context should come from V8 RuntimeAgent native replay: {isolated_context:?}"
    );

    let replayed_context_id = isolated_context["params"]["context"]["id"]
        .as_i64()
        .expect("replayed isolated context id");

    ctx.process_async(json!({
        "id": 147,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "typeof globalThis.__afterNavMarker",
            "contextId": replayed_context_id
        }
    }))
    .await;
    let replayed_marker = take_response_by_id(&mut ctx, 147);
    assert_eq!(replayed_marker["result"]["result"]["type"], json!("string"));
    assert_eq!(
        replayed_marker["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 148,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__afterNavMarker = 11; globalThis.__afterNavMarker",
            "contextId": replayed_context_id
        }
    }))
    .await;
    let replayed_assignment = take_response_by_id(&mut ctx, 148);
    assert_eq!(replayed_assignment["result"]["result"]["value"], 11);

    ctx.process_async(json!({
        "id": 149,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "typeof globalThis.__afterNavMarker"
        }
    }))
    .await;
    let default_context = take_response_by_id(&mut ctx, 149);
    assert_eq!(default_context["result"]["result"]["type"], json!("string"));
    assert_eq!(
        default_context["result"]["result"]["value"],
        json!("undefined")
    );
}

#[tokio::test]
async fn document_navigation_clears_old_runtime_remote_object_tracking() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 170).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 171,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({ answer: 42 })",
            "objectGroup": "before-navigation"
        }
    }))
    .await;
    let object_response = take_response_by_id(&mut ctx, 171);
    let object_id = object_response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.evaluate should return an object handle: {object_response:?}")
        })
        .to_owned();

    assert!(
        ctx.conn
            .target_devtools_session_state_for_session(Some("SID-1"))
            .expect("DevTools session state should exist")
            .has_runtime_remote_object_id(&object_id),
        "pre-navigation object handle should be tracked by the DevTools session"
    );

    ctx.process_async(json!({
        "id": 172,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 172);

    let devtools_session_state = ctx
        .conn
        .target_devtools_session_state_for_session(Some("SID-1"))
        .expect("DevTools session state should exist");
    assert!(
        !devtools_session_state.has_runtime_remote_object_id(&object_id),
        "committed document navigation must forget old document Runtime object handles"
    );
    assert!(
        devtools_session_state
            .runtime_remote_object_group(&object_id)
            .is_none(),
        "committed document navigation must forget old document Runtime object groups"
    );

    ctx.process_async(json!({
        "id": 173,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { globalThis.__staleObjectMutation = true; return this.answer; }",
            "returnByValue": true
        }
    }))
    .await;
    let stale_object_response = take_response_by_id(&mut ctx, 173);
    assert!(
        stale_object_response.get("error").is_some()
            || stale_object_response["result"]["exceptionDetails"].is_object(),
        "old document object handle must fail closed after navigation: {stale_object_response:?}"
    );

    ctx.process_async(json!({
        "id": 174,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.__staleObjectMutation",
            "returnByValue": true
        }
    }))
    .await;
    let marker = take_response_by_id(&mut ctx, 174);
    assert_eq!(marker["result"]["result"]["value"], json!("undefined"));
}

#[tokio::test]
async fn enable_and_disable_update_browser_context_runtime_flag() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_state()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled
    );

    ctx.process_async(json!({"id": 12, "method": "Runtime.enable"}))
        .await;
    ctx.expect_result(12, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_state()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled
    );

    ctx.process_async(json!({"id": 13, "method": "Runtime.disable"}))
        .await;
    ctx.expect_result(13, json!({}), None);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_state()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled
    );
}
/// Runtime.evaluate without a loaded page fails.
#[tokio::test]
async fn evaluate_without_page_errors() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 2, "method": "Runtime.evaluate",
                             "params": {"expression": "1 + 1"}}))
        .await;
    ctx.expect_error(2, -32000, "NoDocumentLoaded");
}
#[tokio::test]
async fn evaluate_rejects_default_context_while_main_document_navigation_is_pending() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context.set_target_url("data:text/html,previous".to_owned());
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;

    ctx.expect_error(3, -32000, "Navigation is changing the document");
}

#[tokio::test]
async fn evaluate_started_before_pending_navigation_can_complete() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context.set_target_url("data:text/html,previous".to_owned());
    ctx.sent.clear();

    let step = ctx.conn.start_command_dispatch(
        &json!({
            "id": 4,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "21 + 21",
                "returnByValue": true
            }
        })
        .to_string(),
    );
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");

    let (messages, _scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;

    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(4) && message["result"]["result"]["value"] == json!(42)
        }),
        "started Runtime.evaluate should complete against its original page: {messages:?}"
    );
}

#[tokio::test]
async fn evaluate_rejects_auxiliary_session_while_main_document_navigation_is_pending() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    assert!(
        browser_context.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()),
        "auxiliary session should attach to the active target"
    );
    browser_context.set_target_url("data:text/html,previous".to_owned());
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    assert!(
        ctx.conn
            .has_pending_document_navigation_for_session_owner(Some("SID-aux")),
        "auxiliary session should inherit the active target document gate"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "sessionId": "SID-aux",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;

    ctx.expect_error(4, -32000, "Navigation is changing the document");
}
#[tokio::test]
async fn dom_get_document_rejects_while_main_document_navigation_is_pending() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<!doctype html><html><head><title>previous</title></head><body>old</body></html>",
    )
    .await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context.set_target_url("data:text/html,previous".to_owned());
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getDocument",
        "sessionId": "SID-1"
    }))
    .await;

    ctx.expect_error(5, -32000, "Navigation is changing the document");
}
#[tokio::test]
async fn document_navigation_gate_is_scoped_to_background_target_owner() {
    let mut ctx = TestContext::new();
    let background_target = crate::conn::PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-active");
    browser_context.attach_active_session("SID-active");
    browser_context.active_page_state_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    browser_context.insert_page_target_host(background_target);
    ctx.conn.browser_context = Some(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><title>active</title></html>",
        Some("SID-active"),
    )
    .await;
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><title>background</title></html>",
        Some("SID-background"),
    )
    .await;
    let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
    browser_context.mutate_parked_page_session_state("TID-background", |state| {
        state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
    });
    browser_context
        .start_document_navigation_for_target(
            "TID-background",
            "PENDING-BACKGROUND-LOADER".to_owned(),
        )
        .expect("background document navigation should start");
    assert!(
        ctx.conn
            .has_pending_document_navigation_for_session_owner(Some("SID-background")),
        "background session should see its own pending document navigation"
    );
    assert!(
        !ctx.conn
            .has_pending_document_navigation_for_session_owner(Some("SID-active")),
        "active session should not inherit a background target document gate"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_error(6, -32000, "Navigation is changing the document");

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 7);
    assert_eq!(response["result"]["result"]["value"], json!("active"));
}
#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_commits_http_error_response_document() {
    async fn first() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>first</title></head><body>first page</body></html>",
        )
    }

    async fn server_error() -> impl IntoResponse {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>server-error</title></head><body><main>error document</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/first", get(first))
                .route("/error", get(server_error)),
        )
        .await
        .unwrap();
    });

    let first_url = format!("http://{addr}/first");
    let error_url = format!("http://{addr}/error");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &first_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(first_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": error_url }
    }))
    .await;
    let navigate = take_response_by_id(&mut ctx, 4);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert!(
        navigate["result"].get("errorText").is_none(),
        "HTTP error responses should commit as documents, not Page.navigate failures: {navigate:?}"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "`${document.title}:${document.querySelector('main')?.textContent}`",
            "returnByValue": true
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 5);
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("server-error:error document")
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .target_url(),
        error_url
    );

    server.abort();
}

async fn assert_empty_http_error_navigation(ctx: &mut TestContext, error_url: &str) {
    ctx.process_and_wait_for_response_async(json!({
        "id": 42,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": error_url }
    }))
    .await;

    let navigate = take_response_by_id(ctx, 42);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert!(navigate["result"]["loaderId"].is_string());
    assert_eq!(navigate["result"]["isDownload"], json!(false));
    assert_eq!(
        navigate["result"]["errorText"],
        json!("net::ERR_HTTP_RESPONSE_CODE_FAILURE")
    );
    wait_until_message(
        ctx,
        Some("SID-1"),
        "empty HTTP error Document stop loading",
        |message| message["method"] == json!("Page.frameStoppedLoading"),
    )
    .await;
    assert!(
        ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Page.loadEventFired")),
        "browser-owned error Document should publish load before frameStoppedLoading"
    );

    let response_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Document")
                && message["params"]["response"]["status"] == json!(429)
                && message["params"]["response"]["url"] == json!(error_url)
        })
        .unwrap_or_else(|| panic!("missing original HTTP 429 response: {:?}", ctx.sent));
    let failure_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["type"] == json!("Document")
                && message["params"]["errorText"] == json!("net::ERR_HTTP_RESPONSE_CODE_FAILURE")
        })
        .unwrap_or_else(|| panic!("missing HTTP response-code failure: {:?}", ctx.sent));
    assert!(
        response_index < failure_index,
        "Chromium publishes the real HTTP response before failing the empty response body"
    );

    let frame_navigated = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Page.frameNavigated"))
        .unwrap_or_else(|| panic!("missing browser error Document commit: {:?}", ctx.sent));
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_navigated["params"]["frame"]["unreachableUrl"],
        error_url
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 43,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({href: location.href, title: document.title, ready: document.readyState, text: document.body?.innerText || ''})",
            "returnByValue": true
        }
    }))
    .await;
    let state = take_response_by_id(ctx, 43)["result"]["result"]["value"].clone();
    assert_eq!(state["href"], NETWORK_ERROR_PAGE_URL);
    assert_eq!(state["title"], "127.0.0.1");
    assert_eq!(state["ready"], "complete");
    assert!(
        state["text"]
            .as_str()
            .is_some_and(|text| text.contains("HTTP ERROR 429")),
        "browser-owned error Document should be usable by text automation: {state:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .target_url(),
        error_url
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_empty_http_error_commits_browser_error_document() {
    async fn first() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>first</title></head><body>first page</body></html>",
        )
    }

    async fn empty_rate_limit() -> impl IntoResponse {
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            [
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
                (axum::http::header::CONTENT_LENGTH.as_str(), "0"),
            ],
            "",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/first", get(first))
                .route("/empty-429", get(empty_rate_limit)),
        )
        .await
        .unwrap();
    });

    let first_url = format!("http://{addr}/first");
    let error_url = format!("http://{addr}/empty-429");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &first_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(first_url);
    ctx.enable_background_navigation_scheduler_for_test();
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({
        "id": 40,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 40);
    ctx.process_async(json!({
        "id": 41,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 41);
    ctx.sent.clear();

    tokio::task::LocalSet::new()
        .run_until(assert_empty_http_error_navigation(&mut ctx, &error_url))
        .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_network_failure_commits_error_document() {
    async fn first() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><head><title>first</title></head><body>first page</body></html>",
        )
    }

    let (failing_addr, failing_server) = spawn_connection_drop_server().await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/first", get(first)))
            .await
            .unwrap();
    });

    let first_url = format!("http://{addr}/first");
    let failing_url = format!("http://{failing_addr}/failed");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &first_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(first_url);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5);
    ctx.process_async(json!({
        "id": 5_1,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5_1);
    ctx.process_async(json!({
        "id": 5_2,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__beforeNetworkError = 'old realm'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 5_2)["result"]["result"]["value"],
        json!("old realm")
    );
    let old_document_token = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .start_document_navigation_for_active_target("LOADER-before-network-error".to_owned())
        .expect("loaded document token");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .commit_document_navigation_if_matches(&old_document_token);
    let before_target_page = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("loaded target Page residence");
    let before_renderer_page = ctx
        .conn
        .renderer_page_residence_identity_for_session_owner(Some("SID-1"))
        .expect("loaded renderer Page residence");
    let before_renderer_attachment = ctx
        .conn
        .current_renderer_agent_attachment_id_for_session_owner(Some("SID-1"))
        .expect("loaded renderer attachment");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": failing_url }
    }))
    .await;

    let navigate = take_response_by_id(&mut ctx, 6);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert!(navigate["result"]["loaderId"].is_string());
    assert_eq!(navigate["result"]["isDownload"], json!(false));
    assert!(
        navigate["result"]["errorText"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "failed navigation should return the browser network error: {navigate:?}"
    );
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "network error Document load",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["type"] == json!("Document")
        }),
        "failed navigation should emit a document loadingFailed event: {:?}",
        ctx.sent
    );
    let frame_navigated = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Page.frameNavigated"))
        .unwrap_or_else(|| panic!("missing error Document frame commit: {:?}", ctx.sent));
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_navigated["params"]["frame"]["unreachableUrl"],
        failing_url
    );
    assert_eq!(frame_navigated["params"]["frame"]["securityOrigin"], "://");
    assert_eq!(
        frame_navigated["params"]["frame"]["secureContextType"],
        "InsecureScheme"
    );
    assert!(
        ctx.sent
            .iter()
            .any(|message| { message["method"] == json!("Runtime.executionContextsCleared") })
    );
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["name"] == json!("DOMContentLoaded")
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["name"] == json!("load")
    }));

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify([location.href, location.origin, typeof globalThis.__beforeNetworkError, document.title, document.readyState])",
            "returnByValue": true
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 7);
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(format!(
            "[\"{NETWORK_ERROR_PAGE_URL}\",\"null\",\"undefined\",\"127.0.0.1\",\"complete\"]"
        ))
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .target_url(),
        failing_url
    );
    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(BrowserContext::loaded_page)
        .expect("network error Document should remain loaded");
    assert_eq!(page.final_url().as_str(), NETWORK_ERROR_PAGE_URL);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .accepts_document_body_completion_event(&old_document_token),
        "the retired document generation must not accept late completion"
    );
    let error_document_loader_id = ctx
        .conn
        .target_session_owner_frame_tree_loader_id(Some("SID-1"))
        .expect("error Document loader id");
    let stale_request_id = "REQ-before-network-error";
    let stale_body_completion = BackgroundNavigationCompletion::main_document_body(
        old_document_token.clone(),
        crate::conn::NavigationDispatchState {
            navigate_id: None,
            navigate_session_id: Some("SID-1".to_owned()),
            result_projection: crate::conn::NavigationResultProjection::Cdp(json!({})),
            frame_id: "TID-1".to_owned(),
            session_id: Some("SID-1".to_owned()),
            request_id: Some(stale_request_id.to_owned()),
            loader_id: old_document_token.loader_id.clone(),
            request_announced: true,
            requested_url: url::Url::parse("https://stale.example.test/old-body").unwrap(),
            request_method: "GET".to_owned(),
            request_body: None,
            request_body_bytes: None,
            request_headers: Vec::new(),
            request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: 0.0,
            source_document_security: crate::conn::NavigationSourceDocumentSecurityContext::new(
                "http://127.0.0.1".to_owned(),
                "InsecureScheme".to_owned(),
            ),
        },
        None,
        Ok(crate::conn::CapturedBody::from_string(
            "stale body".to_owned(),
        )),
        false,
        crate::domains::network::MainDocumentBodyProgressSource::default(),
        url::Url::parse("https://stale.example.test/old-body").unwrap(),
        vec![("content-type".to_owned(), "text/plain".to_owned())],
        false,
    );
    let (stale_completion_messages, stale_completion_scheduler_events) = ctx
        .conn
        .drain_background_navigation_completion_turn_async(stale_body_completion)
        .await
        .into_parts();
    assert!(stale_completion_messages.is_empty());
    assert!(stale_completion_scheduler_events.is_empty());
    assert_eq!(
        ctx.conn
            .target_session_owner_frame_tree_loader_id(Some("SID-1"))
            .as_deref(),
        Some(error_document_loader_id.as_str()),
        "stale body completion must not replace the error Document loader"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::loaded_page)
            .expect("error Document should survive stale body completion")
            .final_url()
            .as_str(),
        NETWORK_ERROR_PAGE_URL
    );
    ctx.process_async(json!({
        "id": 91,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": stale_request_id }
    }))
    .await;
    ctx.expect_error(91, -32000, "No resource with given identifier found");
    assert_ne!(
        ctx.conn
            .target_page_residence_identity_for_session(Some("SID-1")),
        Some(before_target_page),
        "current master installs navigation Documents as a new target Page attachment"
    );
    assert_ne!(
        ctx.conn
            .renderer_page_residence_identity_for_session_owner(Some("SID-1")),
        Some(before_renderer_page),
        "the error Document must not reuse the retired renderer Page"
    );
    assert_ne!(
        ctx.conn
            .current_renderer_agent_attachment_id_for_session_owner(Some("SID-1")),
        Some(before_renderer_attachment),
        "the error Document must own a replacement realm/Inspector attachment"
    );

    ctx.process_async(json!({
        "id": 8,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 8);
    assert_eq!(
        frame_tree["result"]["frameTree"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_tree["result"]["frameTree"]["frame"]["unreachableUrl"],
        failing_url
    );

    ctx.process_async(json!({
        "id": 9,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 9);
    let current_index = history["result"]["currentIndex"]
        .as_u64()
        .expect("current history index") as usize;
    assert_eq!(
        history["result"]["entries"][current_index]["url"], failing_url,
        "history must expose the failed request URL, not the internal error URL"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 10,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "resourceType": "Document" }]
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10);

    let secure_url = "https://secure.example.test/after-error";
    ctx.process_async(json!({
        "id": 11,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": secure_url }
    }))
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
                && message["params"]["request"]["url"] == json!(secure_url)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing HTTPS document request pause: {:?}", ctx.sent));
    let paused_request_id = paused["params"]["requestId"]
        .as_str()
        .expect("paused HTTPS request id")
        .to_owned();
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 12,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": paused_request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "PCFkb2N0eXBlIGh0bWw+PHRpdGxlPnNlY3VyZS1hZnRlci1lcnJvcjwvdGl0bGU+"
        }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "successful HTTPS navigation after error Document",
        |message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!("SID-1")
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let secure_frame_tree = take_response_by_id(&mut ctx, 13);
    let secure_frame = &secure_frame_tree["result"]["frameTree"]["frame"];
    assert_eq!(secure_frame["url"], secure_url);
    assert_eq!(
        secure_frame["securityOrigin"],
        "https://secure.example.test"
    );
    assert_eq!(secure_frame["secureContextType"], "Secure");
    assert!(secure_frame.get("unreachableUrl").is_none());

    ctx.process_async(json!({
        "id": 14,
        "method": "Fetch.disable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 14);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 15,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "about:blank" }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "about:blank navigation inheriting the secure source Document",
        |message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!("SID-1")
        },
    )
    .await;
    ctx.process_async(json!({
        "id": 16,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let inherited_frame_tree = take_response_by_id(&mut ctx, 16);
    let inherited_frame = &inherited_frame_tree["result"]["frameTree"]["frame"];
    assert_eq!(inherited_frame["url"], "about:blank");
    assert_eq!(
        inherited_frame["securityOrigin"],
        "https://secure.example.test"
    );
    assert_eq!(inherited_frame["secureContextType"], "Secure");

    failing_server.abort();
    server.abort();
}
/// Runtime.callFunctionOn without a loaded page fails.
#[tokio::test]
async fn call_function_on_without_page_errors() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 2_1,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "() => 1"
        }
    }))
    .await;
    ctx.expect_error(2_1, -32000, "NoDocumentLoaded");
}
/// Runtime.evaluate executes real JS in the page context.
#[tokio::test]
async fn evaluate_returns_number_value() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 30).await;

    ctx.process_async(json!({"id": 3, "method": "Runtime.evaluate",
                             "params": {"expression": "1 + 1"}}))
        .await;

    let msg = take_response_by_id(&mut ctx, 3);
    assert_eq!(msg["id"], json!(3));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(2));
}
#[tokio::test]
async fn evaluate_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({
        "id": 3_01,
        "method": "Runtime.evaluate",
        "params": {"expression": "2 + 3"}
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("simple Runtime.evaluate should start as a pending command");
    let (mut messages, scheduler_events) =
        super::complete_pending_command_task_for_test(&mut ctx, pending).await;

    let msg = messages
        .pop()
        .expect("pending Runtime.evaluate should produce a response");
    assert_eq!(msg["id"], json!(3_01));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(5));
    assert!(
        scheduler_events.is_empty(),
        "a Runtime command without an owner action must not publish scheduler work: {scheduler_events:?}"
    );
}
#[tokio::test]
async fn evaluate_await_promise_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({
        "id": 3_02,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "Promise.resolve(7)",
            "awaitPromise": true,
            "returnByValue": true
        }
    })
    .to_string();
    let pending = ctx.conn.try_start_pending_command_dispatch(&raw).expect(
        "Runtime.evaluate awaitPromise without contextId should start as a pending command",
    );
    let (mut messages, _scheduler_events) =
        super::complete_pending_command_task_for_test(&mut ctx, pending).await;

    let msg = messages
        .pop()
        .expect("pending Runtime.evaluate awaitPromise should produce a response");
    assert_eq!(msg["id"], json!(3_02));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(7));
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "settled pending Runtime.evaluate awaitPromise should drain its pending inspector entry"
    );
}

#[tokio::test]
async fn evaluate_await_promise_connected_style_events_advance_on_owner_turns() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><head></head><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_020,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
Promise.all(Array.from({ length: 129 }, (_, index) => new Promise((resolve, reject) => {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = `data:text/css,:root{--runtime-owner-turn-${index}:${index}}`;
  link.addEventListener("load", () => resolve(index));
  link.addEventListener("error", () => reject(new Error(`stylesheet ${index} failed`)));
  document.head.appendChild(link);
}))).then(values => values.length)
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;

    let response = wait_for_response_by_id_async(&mut ctx, None, 3_020).await;
    assert_eq!(
        response["result"]["result"]["value"],
        json!(129),
        "connected stylesheet events beyond the old protocol drain cap should settle through owner turns: {response:?}"
    );
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "owner-turn stylesheet delivery should retire the pending inspector await"
    );
}

#[tokio::test]
async fn evaluate_await_promise_scheduler_routed_reply_clears_pending_await_registry() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_021,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "Promise.resolve(17)",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 3_021);
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(17));
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "scheduler-routed Runtime.evaluate awaitPromise completion must consume the pending inspector await registry"
    );
}

#[tokio::test]
async fn runtime_await_promise_pending_registers_renderer_response_receiver() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_022,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "new Promise(() => {})"
        }
    }))
    .await;
    let promise_object_id = take_response_by_id(&mut ctx, 3_022)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a promise object handle")
        .to_owned();

    let raw = json!({
        "id": 3_023,
        "method": "Runtime.awaitPromise",
        "params": {
            "promiseObjectId": promise_object_id,
            "returnByValue": true
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Runtime.awaitPromise should start as a pending command");
    let (mut pending, scheduler_events) = match ctx
        .conn
        .complete_pending_command_dispatch(pending.wait().await)
        .await
    {
        CdpCommandTaskStep::Pending(pending) => (*pending, ctx.conn.take_scheduler_events()),
        CdpCommandTaskStep::Complete(outcome) => {
            panic!(
                "never-settling Runtime.awaitPromise should stay pending after initial inspector dispatch: {:?}",
                outcome.into_parts().0
            );
        }
    };
    assert!(
        scheduler_events.is_empty(),
        "pending Runtime.awaitPromise must wait on its inspector response receiver, not scheduler follow-up work: {scheduler_events:?}"
    );

    assert!(
        pending.waits_for_scheduler_deferred_inspector_reply(),
        "Runtime.awaitPromise should enter scheduler deferred-reply state"
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(None),
        "scheduler-deferred Runtime.awaitPromise should remain visible as pending command-owned work"
    );
    assert!(
        !ctx.conn
            .has_unclaimed_pending_inspector_awaits_for_session_owner(None),
        "scheduler-deferred Runtime.awaitPromise should claim the pending await out of the document registry"
    );
    assert!(
        ctx.conn
            .has_claimed_pending_inspector_awaits_for_session_owner(None),
        "scheduler-deferred Runtime.awaitPromise should be tracked by the command-owned await index"
    );
    assert!(
        pending
            .take_scheduler_deferred_inspector_reply_receiver()
            .is_some(),
        "pending Runtime.awaitPromise should own the renderer response receiver"
    );
    pending.forget_scheduler_deferred_inspector_reply(&mut ctx.conn);
}

#[tokio::test]
async fn runtime_await_promise_armed_timer_reply_arrives_through_renderer_receiver() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
            "id": 3_024,
            "method": "Runtime.evaluate",
            "params": {
            "expression": "new Promise(resolve => { globalThis.__armAwaitTimer = () => setTimeout(() => resolve('await-timer'), 0); })"
            }
    }))
    .await;
    let promise_object_id = take_response_by_id(&mut ctx, 3_024)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a timer-backed promise object handle")
        .to_owned();

    ctx.process_command_only_async(json!({
        "id": 3_025,
        "method": "Runtime.awaitPromise",
        "params": {
            "promiseObjectId": promise_object_id,
            "returnByValue": true
        }
    }))
    .await;

    assert!(
        !ctx.sent.iter().any(|message| message["id"] == json!(3_025)),
        "timer-backed Runtime.awaitPromise should defer until the renderer callback response arrives: {:?}",
        ctx.sent
    );
    ctx.process_async(json!({
        "id": 3_026,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__armAwaitTimer(); 'armed'"
        }
    }))
    .await;
    let arm_response = take_response_by_id(&mut ctx, 3_026);
    assert_eq!(arm_response["result"]["result"]["value"], json!("armed"));

    let response = wait_for_response_by_id_async(&mut ctx, None, 3_025).await;
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("await-timer"));
    assert!(
        !ctx.conn.has_pending_inspector_awaits(),
        "renderer callback completion must consume Runtime.awaitPromise pending inspector state"
    );
}

#[tokio::test]
async fn isolated_evaluate_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_active_target_id("TID-1");
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_021, "utility").await;

    let raw = json!({
        "id": 3_022,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": utility_context_id,
            "returnByValue": true,
            "expression": "globalThis.__pendingIsolated = 11"
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Runtime.evaluate with isolated contextId should start as a pending command");
    let (messages, _) = super::complete_pending_command_task_for_test(&mut ctx, pending).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(3_022))
        .expect("pending isolated Runtime.evaluate should produce a response");
    assert_eq!(response["result"]["result"]["value"], json!(11));
}
#[tokio::test]
async fn call_function_context_resolution_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><title>default-title</title><body></body></html>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_active_target_id("TID-1");
    let _default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 3_020).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_023, "utility").await;

    ctx.process_async(json!({
        "id": 3_024,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis.__pendingIsolated = 17",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3_024);

    let isolated_raw = json!({
        "id": 3_025,
        "method": "Runtime.callFunctionOn",
        "params": {
            "executionContextId": utility_context_id,
            "functionDeclaration": "function() { return globalThis.__pendingIsolated + 1; }",
            "returnByValue": true
        }
    })
    .to_string();
    let isolated_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&isolated_raw)
        .expect("Runtime.callFunctionOn with isolated executionContextId should start pending");
    let (isolated_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, isolated_pending).await;
    let isolated_response = isolated_messages
        .iter()
        .find(|message| message["id"] == json!(3_025))
        .expect("pending isolated Runtime.callFunctionOn should produce a response");
    assert_eq!(isolated_response["result"]["result"]["value"], json!(18));

    let default_raw = json!({
        "id": 3_026,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "function() { return document.title; }",
            "returnByValue": true
        }
    })
    .to_string();
    let default_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&default_raw)
        .expect("Runtime.callFunctionOn without objectId should start pending");
    let (default_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, default_pending).await;
    let default_response = default_messages
        .iter()
        .find(|message| message["id"] == json!(3_026))
        .expect("pending default Runtime.callFunctionOn should produce a response");
    assert_eq!(
        default_response["result"]["result"]["value"],
        json!("default-title")
    );
}
#[tokio::test]
async fn object_runtime_commands_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_03,
        "method": "Runtime.evaluate",
        "params": {"expression": "({ answer: 41, label: 'ok', promise: Promise.resolve('done') })"}
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 3_03)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate should return an objectId");

    let call_raw = json!({
        "id": 3_04,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.answer + 1; }",
            "returnByValue": true
        }
    })
    .to_string();
    let call_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&call_raw)
        .expect("Runtime.callFunctionOn with objectId should start as a pending command");
    let (call_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, call_pending).await;
    let call_response = call_messages
        .iter()
        .find(|message| message["id"] == json!(3_04))
        .expect("pending Runtime.callFunctionOn should produce a response");
    assert_eq!(call_response["result"]["result"]["value"], json!(42));

    let properties_raw = json!({
        "id": 3_05,
        "method": "Runtime.getProperties",
        "params": {"objectId": object_id, "ownProperties": true}
    })
    .to_string();
    let properties_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&properties_raw)
        .expect("Runtime.getProperties with objectId should start as a pending command");
    let (properties_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, properties_pending).await;
    let properties_response = properties_messages
        .iter()
        .find(|message| message["id"] == json!(3_05))
        .expect("pending Runtime.getProperties should produce a response");
    let properties = properties_response["result"]["result"]
        .as_array()
        .expect("Runtime.getProperties should return a property array");
    let promise_object_id = properties
        .iter()
        .find(|property| property["name"] == json!("promise"))
        .and_then(|property| property["value"]["objectId"].as_str())
        .map(str::to_owned)
        .expect("Runtime.getProperties should expose the promise objectId");

    let await_raw = json!({
        "id": 3_06,
        "method": "Runtime.awaitPromise",
        "params": {
            "promiseObjectId": promise_object_id,
            "returnByValue": true
        }
    })
    .to_string();
    let await_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&await_raw)
        .expect("Runtime.awaitPromise with promiseObjectId should start as a pending command");
    let (await_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, await_pending).await;
    let await_response = await_messages
        .iter()
        .find(|message| message["id"] == json!(3_06))
        .expect("pending Runtime.awaitPromise should produce a response");
    assert_eq!(await_response["result"]["result"]["value"], json!("done"));

    let release_raw = json!({
        "id": 3_07,
        "method": "Runtime.releaseObject",
        "params": {"objectId": object_id}
    })
    .to_string();
    let release_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&release_raw)
        .expect("Runtime.releaseObject should start as a pending command");
    let (release_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, release_pending).await;
    let release_response = release_messages
        .iter()
        .find(|message| message["id"] == json!(3_07))
        .expect("pending Runtime.releaseObject should produce a response");
    assert_eq!(release_response["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(None, &[object_id])
            .is_ok(),
        "pending Runtime.releaseObject should unregister released handles"
    );
}
#[tokio::test]
async fn heap_usage_and_release_group_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let heap_raw = json!({
        "id": 3_08,
        "method": "Runtime.getHeapUsage"
    })
    .to_string();
    let heap_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&heap_raw)
        .expect("Runtime.getHeapUsage should start as a pending command");
    let (heap_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, heap_pending).await;
    let heap_response = heap_messages
        .iter()
        .find(|message| message["id"] == json!(3_08))
        .expect("pending Runtime.getHeapUsage should produce a response");
    assert!(
        heap_response["result"]["usedSize"].as_u64().is_some(),
        "pending Runtime.getHeapUsage should return usedSize: {heap_response:?}"
    );

    ctx.process_async(json!({
        "id": 3_09,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ grouped: true })",
            "objectGroup": "pending-release-group"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 3_09)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate should return a grouped objectId");

    let release_group_raw = json!({
        "id": 3_10,
        "method": "Runtime.releaseObjectGroup",
        "params": {"objectGroup": "pending-release-group"}
    })
    .to_string();
    let release_group_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&release_group_raw)
        .expect("Runtime.releaseObjectGroup should start as a pending command");
    let (release_group_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, release_group_pending).await;
    let release_group_response = release_group_messages
        .iter()
        .find(|message| message["id"] == json!(3_10))
        .expect("pending Runtime.releaseObjectGroup should produce a response");
    assert_eq!(release_group_response["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(None, &[object_id])
            .is_ok(),
        "pending Runtime.releaseObjectGroup should unregister grouped handles"
    );
}
#[tokio::test]
async fn evaluate_without_runtime_enable_uses_inspector_default_context_silently() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><script>globalThis.__runtimelessProbe = 41;</script></body></html>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3_1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__runtimelessProbe + 1",
            "returnByValue": true
        }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 3_1);
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(42));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.executionContextCreated")),
        "runtimeless evaluate must not open Runtime event surface: {:?}",
        ctx.sent
    );
}
/// After Step A/B of the event-loop refactor, `setTimeout(0)` no longer fires
/// inline as part of the previous evaluate's CDP reply. Instead it fires from
/// the owner loop's idle tick branch, which the renderer's `render_runtime`
/// thread runs in the background. A subsequent `Runtime.evaluate` should still
/// observe the side effect because the owner loop has had time to tick before
/// the next command is dispatched.
#[tokio::test(flavor = "multi_thread")]
async fn process_message_async_observes_background_timer_between_evaluates() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_100,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_async_drain_marker = "pending";
  setTimeout(() => { globalThis.__lm_async_drain_marker = "drained"; }, 0);
  return "scheduled";
})()"#
        }
    }))
    .await;
    let scheduled = take_response_by_id(&mut ctx, 3_100);
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    // Yield long enough for the owner loop to fire the due timer in its
    // background tick branch before the next evaluate is dispatched.
    for _ in 0..16 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        ctx.process_async(json!({
            "id": 3_101,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lm_async_drain_marker"
            }
        }))
        .await;
        let observed = take_response_by_id(&mut ctx, 3_101);
        if observed["result"]["result"]["value"] == json!("drained") {
            return;
        }
    }
    panic!("background timer did not fire within retry budget");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_document_replacement_clears_timer_mutated_inline_style_state() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 3_120,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  document.body.innerHTML = "<div id='before' style='display:none'>before</div>";
  return new Promise(resolve => setTimeout(() => {
    document.getElementById('before').style.display = 'block';
    resolve("mutated");
  }, 0));
})()"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let mutated = take_response_by_id(&mut ctx, 3_120);
    assert_eq!(mutated["result"]["result"]["value"], json!("mutated"));

    ctx.process_async(json!({
        "id": 3_121,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const before = document.getElementById('before');
  return `${before.style.display}:${getComputedStyle(before).display}:${before.getClientRects().length}`;
})()"#
        }
    }))
    .await;
    let warmed = take_response_by_id(&mut ctx, 3_121);
    assert_eq!(warmed["result"]["result"]["value"], json!("block:block:1"));

    ctx.process_async(json!({
        "id": 3_122,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
  return "replaced";
})()"#
        }
    }))
    .await;
    let replaced = take_response_by_id(&mut ctx, 3_122);
    assert_eq!(replaced["result"]["result"]["value"], json!("replaced"));

    ctx.process_async(json!({
        "id": 3_123,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const after = document.getElementById('after');
  return `${after.getAttribute('style')}:${after.style.display}:${getComputedStyle(after).display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()"#
        }
    }))
    .await;
    let after = take_response_by_id(&mut ctx, 3_123);
    assert_eq!(
        after["result"]["result"]["value"],
        json!("display:none:none:none:0:0")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_call_function_document_replacement_clears_default_world_style_state() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_active_target_id("TID-1");
    let _default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 3_130).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_131, "utility").await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 3_132,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  document.body.innerHTML = "<div id='before' style='display:none'>before</div>";
  return new Promise(resolve => setTimeout(() => {
    document.getElementById('before').style.display = 'block';
    resolve("mutated");
  }, 0));
})()"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let mutated = take_response_by_id(&mut ctx, 3_132);
    assert_eq!(mutated["result"]["result"]["value"], json!("mutated"));

    ctx.process_async(json!({
        "id": 3_133,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const before = document.getElementById('before');
  return `${before.style.display}:${getComputedStyle(before).display}:${before.getClientRects().length}`;
})()"#
        }
    }))
    .await;
    let warmed = take_response_by_id(&mut ctx, 3_133);
    assert_eq!(warmed["result"]["result"]["value"], json!("block:block:1"));

    ctx.process_async(json!({
        "id": 3_134,
        "method": "Runtime.callFunctionOn",
        "params": {
            "executionContextId": utility_context_id,
            "functionDeclaration": r#"function() {
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
  return "replaced";
}"#,
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let replaced = take_response_by_id(&mut ctx, 3_134);
    assert_eq!(replaced["result"]["result"]["value"], json!("replaced"));

    ctx.process_async(json!({
        "id": 3_135,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const after = document.getElementById('after');
  return `${after.getAttribute('style')}:${after.style.display}:${getComputedStyle(after).display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()"#
        }
    }))
    .await;
    let after = take_response_by_id(&mut ctx, 3_135);
    assert_eq!(
        after["result"]["result"]["value"],
        json!("display:none:none:none:0:0")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_utility_document_replacement_clears_default_world_style_state() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_active_target_id("TID-1");
    let default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 3_140).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_141, "utility").await;

    for (id, context_id) in [(3_142, default_context_id), (3_143, utility_context_id)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": context_id,
                "expression": r#"(() => ({
  evaluate(expression) {
    return globalThis.eval(expression);
  }
}))()"#
            }
        }))
        .await;
    }
    let default_utility = take_response_by_id(&mut ctx, 3_142)["result"]["result"]["objectId"]
        .as_str()
        .expect("default utility object id")
        .to_owned();
    let isolated_utility = take_response_by_id(&mut ctx, 3_143)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated utility object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 3_144,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": isolated_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": isolated_utility },
                { "value": r#"(() => {
  document.open();
  document.write("<!doctype html><html><body><div id='before' style='display:none'>before</div><script>globalThis.__lm_timer_style_mutation = new Promise(resolve => { setTimeout(() => { document.getElementById('before').style.display = 'block'; resolve('mutated'); }, 0); });</script></body></html>");
  document.close();
})()"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3_144);

    ctx.process_async(json!({
        "id": 3_145,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": default_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": default_utility },
                { "value": r#"globalThis.__lm_timer_style_mutation.then(() => {
  const before = document.getElementById('before');
  return `${before.style.display}:${getComputedStyle(before).display}:${before.getClientRects().length}`;
})"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let warmed = take_response_by_id(&mut ctx, 3_145);
    assert_eq!(warmed["result"]["result"]["value"], json!("block:block:1"));

    ctx.process_async(json!({
        "id": 3_146,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": isolated_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": isolated_utility },
                { "value": r#"(() => {
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
})()"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3_146);

    ctx.process_async(json!({
        "id": 3_147,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": default_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": default_utility },
                { "value": r#"(() => {
  const after = document.getElementById('after');
  return `${after.getAttribute('style')}:${after.style.display}:${getComputedStyle(after).display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let after = take_response_by_id(&mut ctx, 3_147);
    assert_eq!(
        after["result"]["result"]["value"],
        json!("display:none:none:none:0:0")
    );
}
/// Runtime.evaluate now comes from inspector and can return objectId.
#[tokio::test]
async fn evaluate_returns_remote_object_id_for_objects() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 31).await;

    ctx.process_async(json!({"id": 6, "method": "Runtime.evaluate",
                             "params": {"expression": "({ answer: 42 })"}}))
        .await;

    let msg = take_response_by_id(&mut ctx, 6);
    assert_eq!(msg["id"], json!(6));
    assert_eq!(
        msg["result"]["result"]["type"],
        json!("object"),
        "unexpected isolated Runtime.evaluate response: {msg:?}"
    );
    assert!(msg["result"]["result"]["objectId"].as_str().is_some());
}
#[tokio::test]
async fn scoped_binding_persists_across_navigation_for_registered_named_world() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 331).await;
    let _ = create_isolated_world_async(&mut ctx, 332, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 333,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "persistedUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 333);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3331,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "",
            "worldName": "utility"
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 3331);
    assert!(add_script["result"]["identifier"].as_str().is_some());
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 334,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let navigation_messages = ctx.take_all();
    let replayed_context_id = navigation_messages
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("registered named world should be recreated while Runtime is enabled");

    ctx.process_async(json!({
        "id": 336,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.persistedUtilityBinding"
        }
    }))
    .await;
    let default_world = take_response_by_id(&mut ctx, 336);
    assert_eq!(default_world["result"]["result"]["type"], json!("string"));
    assert_eq!(
        default_world["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 337,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.persistedUtilityBinding('after-nav'); 11",
            "contextId": replayed_context_id
        }
    }))
    .await;
    let call = take_response_by_id(&mut ctx, 337);
    assert_eq!(call["result"]["result"]["type"], json!("number"));
    assert_eq!(call["result"]["result"]["value"], json!(11));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("persistedUtilityBinding")
        })
        .cloned()
        .expect("scoped binding should survive navigation");
    assert_eq!(binding_called["params"]["payload"], json!("after-nav"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(replayed_context_id)
    );
}
#[tokio::test]
async fn scoped_binding_applies_to_matching_isolated_world_created_after_registration() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 338).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 339,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "lateUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 339);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    let utility_context_id = create_isolated_world_async(&mut ctx, 340, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 341,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.lateUtilityBinding('created-late'); 13",
            "contextId": utility_context_id
        }
    }))
    .await;
    let call = take_response_by_id(&mut ctx, 341);
    assert_eq!(call["result"]["result"]["type"], json!("number"));
    assert_eq!(call["result"]["result"]["value"], json!(13));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("lateUtilityBinding")
        })
        .cloned()
        .expect("late-created matching world should receive binding");
    assert_eq!(binding_called["params"]["payload"], json!("created-late"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
}
#[tokio::test]
async fn scoped_binding_does_not_apply_to_non_matching_isolated_world() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 342).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 343,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "utilityOnlyBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 343);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    let other_context_id = create_isolated_world_async(&mut ctx, 344, "other").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 345,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.utilityOnlyBinding",
            "contextId": other_context_id
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 345);
    assert_eq!(result["result"]["result"]["type"], json!("string"));
    assert_eq!(result["result"]["result"]["value"], json!("undefined"));
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("utilityOnlyBinding")
    }));
}
#[tokio::test]
async fn evaluate_in_isolated_world_uses_separate_global_context() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 33).await;

    let isolated_context_id = create_isolated_world_async(&mut ctx, 34, "utility").await;

    ctx.process_async(json!({
        "id": 35,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__lmIso = (globalThis.__lmIso || 0) + 1; globalThis.__lmIso",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let isolated = take_response_by_id(&mut ctx, 35);
    assert_eq!(isolated["result"]["result"]["type"], json!("number"));
    assert_eq!(isolated["result"]["result"]["value"], json!(1));

    ctx.process_async(json!({
        "id": 36,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "typeof globalThis.__lmIso"
        }
    }))
    .await;
    let default_context = take_response_by_id(&mut ctx, 36);
    assert_eq!(default_context["result"]["result"]["type"], json!("string"));
    assert_eq!(
        default_context["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 37,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__lmIso",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let isolated_again = take_response_by_id(&mut ctx, 37);
    assert_eq!(isolated_again["result"]["result"]["type"], json!("number"));
    assert_eq!(isolated_again["result"]["result"]["value"], json!(1));
}
#[tokio::test]
async fn evaluate_in_isolated_world_does_not_require_runtime_frontend_enabled() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    let isolated_context_id = create_isolated_world_async(&mut ctx, 38, "utility").await;

    ctx.process_async(json!({
        "id": 39,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ answer: 42 })",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let msg = take_response_by_id(&mut ctx, 39);
    assert_eq!(
        msg["result"]["result"]["type"],
        json!("object"),
        "unexpected isolated Runtime.evaluate response: {msg:?}"
    );
    assert!(
        msg["result"]["result"]["objectId"].as_str().is_some(),
        "isolated Runtime.evaluate without Runtime.enable should use inspector handles"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.executionContextCreated")),
        "silent isolated inspector materialization must not open Runtime event surface"
    );
}
#[tokio::test]
async fn isolated_world_file_assignment_updates_main_world_input_files_surface() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><input id='upload' type='file'></body></html>",
    )
    .await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 381).await;
    let isolated_context_id = create_isolated_world_async(&mut ctx, 382, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 383,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(() => {
  const input = document.getElementById('upload');
  globalThis.__mainWorldUploadEvents = [];
  input.addEventListener('input', () => {
    globalThis.__mainWorldUploadEvents.push(`input:${input.files.length}:${input.value}`);
  });
  input.addEventListener('change', () => {
    globalThis.__mainWorldUploadEvents.push(`change:${input.files[0].name}:${input.files[0].type}`);
  });
  return 'ready';
})()
"#
        }
    }))
    .await;
    let setup = take_response_by_id(&mut ctx, 383);
    assert_eq!(setup["result"]["result"]["value"], json!("ready"));

    ctx.process_async(json!({
        "id": 384,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(() => {
  const input = document.getElementById('upload');
  const dt = new DataTransfer();
  dt.items.add(new File([new Uint8Array([1, 2, 3])], 'note.txt', {
    type: 'text/plain',
    lastModified: 42
  }));
  input.files = dt.files;
  input.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
  return `${input.files.length}|${input.files[0].name}|${input.value}`;
})()
"#,
            "contextId": isolated_context_id
        }
    }))
    .await;
    let utility_result = take_response_by_id(&mut ctx, 384);
    assert_eq!(
        utility_result["result"]["result"]["value"],
        json!("1|note.txt|C:\\fakepath\\note.txt")
    );

    ctx.process_async(json!({
        "id": 385,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
JSON.stringify({
  count: document.getElementById('upload').files.length,
  name: document.getElementById('upload').files[0].name,
  type: document.getElementById('upload').files[0].type,
  lastModified: document.getElementById('upload').files[0].lastModified,
  value: document.getElementById('upload').value,
  events: globalThis.__mainWorldUploadEvents
})
"#
        }
    }))
    .await;
    let main_world = take_response_by_id(&mut ctx, 385);
    assert_eq!(
        main_world["result"]["result"]["value"],
        json!(
            r#"{"count":1,"name":"note.txt","type":"text/plain","lastModified":42,"value":"C:\\fakepath\\note.txt","events":["input:1:C:\\fakepath\\note.txt","change:note.txt:text/plain"]}"#
        )
    );
}
/// Runtime.callFunctionOn executes via inspector in a real execution context.
#[tokio::test]
async fn call_function_on_with_args() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 40).await;

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "(a, b) => a + b",
            "executionContextId": execution_context_id,
            "arguments": [{"value": 2}, {"value": 3}]
        }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 4);
    assert_eq!(msg["id"], json!(4));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(5));
}
/// Runtime.getProperties can read back values through inspector objectId.
#[tokio::test]
async fn get_properties_reads_object_via_object_id() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 32).await;

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "params": {"expression": "({ answer: 42, label: 'ok' })"}
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 7)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate must return an objectId for objects");

    ctx.process_async(json!({
        "id": 8,
        "method": "Runtime.getProperties",
        "params": {"objectId": object_id, "ownProperties": true}
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 8);
    assert_eq!(msg["id"], json!(8));
    let props = msg["result"]["result"]
        .as_array()
        .expect("Runtime.getProperties must return a property array");
    assert!(
        props
            .iter()
            .any(|prop| { prop["name"] == json!("answer") && prop["value"]["value"] == json!(42) })
    );
}
#[tokio::test]
async fn runtime_evaluate_node_result_reports_node_subtype() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body><input id='box'></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 320).await;

    ctx.process_async(json!({
        "id": 321,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#box')" }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 321);
    assert_eq!(msg["result"]["result"]["type"], json!("object"));
    assert_eq!(msg["result"]["result"]["subtype"], json!("node"));
    assert!(msg["result"]["result"]["objectId"].as_str().is_some());
}
#[tokio::test]
async fn runtime_get_properties_reports_node_subtype_for_node_properties() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body><input id='box'></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 322).await;

    ctx.process_async(json!({
        "id": 323,
        "method": "Runtime.evaluate",
        "params": { "expression": "({ element: document.querySelector('#box') })" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 323)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate must return an objectId for objects");

    ctx.process_async(json!({
        "id": 324,
        "method": "Runtime.getProperties",
        "params": { "objectId": object_id, "ownProperties": true }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 324);
    let props = msg["result"]["result"]
        .as_array()
        .expect("Runtime.getProperties must return a property array");
    let element = props
        .iter()
        .find(|prop| prop["name"] == json!("element"))
        .expect("element property should be present");
    assert_eq!(element["value"]["type"], json!("object"));
    assert_eq!(element["value"]["subtype"], json!("node"));
    assert!(element["value"]["objectId"].as_str().is_some());
}
/// Runtime.evaluate captures JS exceptions into exceptionDetails.
#[tokio::test]
async fn evaluate_reports_exception_details() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 33).await;

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {"expression": "(() => { throw new Error('boom'); })()"}
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 5);
    assert_eq!(msg["id"], json!(5));
    assert!(msg["result"]["exceptionDetails"].is_object());
    assert!(
        msg["result"]["exceptionDetails"]["exceptionId"]
            .as_u64()
            .is_some(),
        "Runtime.evaluate exceptionDetails should include exceptionId: {msg:?}"
    );
    let text = msg["result"]["exceptionDetails"]["exception"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(text.contains("boom"), "unexpected exception text: {text}");
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "ok"),
            ],
            "runtime fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 199).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 200,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "fetch('/api').then(r => r.text())" }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 200);
    assert_eq!(response["id"], 200);

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "ok"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 201,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        201,
        json!({
            "body": "runtime fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_call_function_on_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "ok"),
            ],
            "runtime fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 1980).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1981,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "functionDeclaration": "() => fetch('/api').then(r => r.text())",
            "executionContextId": execution_context_id
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 1981);
    assert_eq!(response["id"], 1981);

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime callFunctionOn fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_call_function_on_xhr_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "ok"),
            ],
            "runtime xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 1982).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1983,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "functionDeclaration": "() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); return 'scheduled'; }",
            "executionContextId": execution_context_id
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 1983);
    assert_eq!(response["id"], 1983);
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime callFunctionOn xhr request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_fetch_applies_extra_http_headers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api(headers: axum::http::HeaderMap) -> impl IntoResponse {
        let received = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "ok"),
            ],
            received.to_owned(),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.process_async(json!({
        "id": 489,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 489);
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 490).await;
    ctx.process_async(json!({
        "id": 491,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "runtime-fetch" } }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 491);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 492,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "fetch('/api').then(r => r.text())" }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 492);
    assert_eq!(response["id"], 492);

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime fetch request event");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "runtime-fetch"
    );
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 493,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        493,
        json!({
            "body": "runtime-fetch",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_xhr_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "ok"),
            ],
            "runtime xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 204).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 202,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); return xhr.responseText; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 202);
    assert_eq!(response["id"], 202);

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime xhr request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime xhr request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime xhr response event");
    assert_eq!(response_event["params"]["type"], "XHR");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-xhr"],
        "ok"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 203,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        203,
        json!({
            "body": "runtime xhr body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_xhr_applies_extra_http_headers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr(headers: axum::http::HeaderMap) -> impl IntoResponse {
        let received = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "ok"),
            ],
            received.to_owned(),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.process_async(json!({
        "id": 493,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 493);
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 494).await;
    ctx.process_async(json!({
        "id": 495,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "runtime-xhr" } }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 495);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 496,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); return xhr.responseText; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 496);
    assert_eq!(response["id"], 496);

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime xhr request event");
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "runtime-xhr"
    );
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 497,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        497,
        json!({
            "body": "runtime-xhr",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_set_timeout_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "timeout"),
            ],
            "runtime timeout fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 206).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 207,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { setTimeout(() => { fetch('/api').then(r => r.text()); }, 0); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 207);
    assert_eq!(response["id"], 207);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime timeout fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime timeout fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime timeout fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "timeout"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 208,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        208,
        json!({
            "body": "runtime timeout fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_set_timeout_xhr_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "timeout"),
            ],
            "runtime timeout xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 209).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 210,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { setTimeout(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); }, 0); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 210);
    assert_eq!(response["id"], 210);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime timeout xhr request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime timeout xhr request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime timeout xhr response event");
    assert_eq!(response_event["params"]["type"], "XHR");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-xhr"],
        "timeout"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 211,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        211,
        json!({
            "body": "runtime timeout xhr body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_set_interval_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "interval"),
            ],
            "runtime interval fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2110).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2111,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const id = setInterval(() => { clearInterval(id); fetch('/api').then(r => r.text()); }, 0); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 2111);
    assert_eq!(response["id"], 2111);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        1,
        "setInterval fetch should fire once after clearInterval"
    );
    let request = requests[0].clone();
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime interval fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime interval fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "interval"
    );

    ctx.process_async(json!({
        "id": 2112,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        2112,
        json!({
            "body": "runtime interval fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_set_interval_xhr_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "interval"),
            ],
            "runtime interval xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2113).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2114,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const id = setInterval(() => { clearInterval(id); const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); }, 0); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 2114);
    assert_eq!(response["id"], 2114);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        1,
        "setInterval xhr should fire once after clearInterval"
    );
    let request = requests[0].clone();
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime interval xhr request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime interval xhr response event");
    assert_eq!(response_event["params"]["type"], "XHR");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-xhr"],
        "interval"
    );

    ctx.process_async(json!({
        "id": 2115,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        2115,
        json!({
            "body": "runtime interval xhr body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_request_animation_frame_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "raf"),
            ],
            "runtime raf fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 212).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 213,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { requestAnimationFrame(() => { fetch('/api').then(r => r.text()); }); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 213);
    assert_eq!(response["id"], 213);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime raf fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime raf fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime raf fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "raf"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 214,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        214,
        json!({
            "body": "runtime raf fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_request_animation_frame_xhr_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "raf"),
            ],
            "runtime raf xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 215).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 216,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { requestAnimationFrame(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); }); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 216);
    assert_eq!(response["id"], 216);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime raf xhr request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime raf xhr request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime raf xhr response event");
    assert_eq!(response_event["params"]["type"], "XHR");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-xhr"],
        "raf"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 217,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        217,
        json!({
            "body": "runtime raf xhr body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_request_idle_callback_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "idle"),
            ],
            "runtime idle fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 218).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 219,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { requestIdleCallback(deadline => { globalThis.__lm_idle_did_timeout = deadline.didTimeout; globalThis.__lm_idle_time_remaining = deadline.timeRemaining() > 0; fetch('/api'); }); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 219);
    assert_eq!(response["id"], 219);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime idle fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime idle fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime idle fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "idle"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 220,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__lm_idle_did_timeout === false && globalThis.__lm_idle_time_remaining === true"
        }
    })).await;
    let idle_meta = take_response_by_id(&mut ctx, 220);
    assert_eq!(idle_meta["result"]["result"]["value"], true);

    ctx.process_async(json!({
        "id": 221,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        221,
        json!({
            "body": "runtime idle fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_request_idle_callback_xhr_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "idle"),
            ],
            "runtime idle xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 222).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 223,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { requestIdleCallback(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); }); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 223);
    assert_eq!(response["id"], 223);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime idle xhr request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime idle xhr request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime idle xhr response event");
    assert_eq!(response_event["params"]["type"], "XHR");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-xhr"],
        "idle"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 224,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        224,
        json!({
            "body": "runtime idle xhr body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_promise_then_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "promise"),
            ],
            "runtime promise fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 225).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 226,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { Promise.resolve().then(() => { fetch('/api').then(r => r.text()); }); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 226);
    assert_eq!(response["id"], 226);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime promise fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime promise fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime promise fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "promise"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 227,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        227,
        json!({
            "body": "runtime promise fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_promise_then_xhr_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-xhr", "promise"),
            ],
            "runtime promise xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 228).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 229,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { Promise.resolve().then(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', '/xhr'); xhr.send(); }); return 'scheduled'; })()"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 229);
    assert_eq!(response["id"], 229);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime promise xhr request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime promise xhr request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime promise xhr response event");
    assert_eq!(response_event["params"]["type"], "XHR");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-xhr"],
        "promise"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 230,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        230,
        json!({
            "body": "runtime promise xhr body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_window_post_message_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "postmessage"),
            ],
            "runtime postmessage fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 230).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 231,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  window.addEventListener('message', () => {
fetch('/api').then(r => r.text());
  }, { once: true });
  window.postMessage('go', '*');
  return 'scheduled';
})()"#
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 231);
    assert_eq!(response["id"], 231);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime postMessage fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime postMessage fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime postMessage fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "postmessage"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 232,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        232,
        json!({
            "body": "runtime postmessage fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_mutation_observer_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "mutation"),
            ],
            "runtime mutation fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 233).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 234,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  const observer = new MutationObserver(() => {
fetch('/api').then(r => r.text());
observer.disconnect();
  });
  observer.observe(document.body, { attributes: true });
  document.body.setAttribute('data-trigger', '1');
  return 'scheduled';
})()"#
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 234);
    assert_eq!(response["id"], 234);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime mutation fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime mutation fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime mutation fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "mutation"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 235,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        235,
        json!({
            "body": "runtime mutation fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_intersection_observer_fetch_emits_subresource_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><div id='target'>ok</div></body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-runtime-fetch", "intersection"),
            ],
            "runtime intersection fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 236).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 237,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  const observer = new IntersectionObserver(() => {
fetch('/api').then(r => r.text());
observer.disconnect();
  });
  observer.observe(document.getElementById('target'));
  return 'scheduled';
})()"#
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 237);
    assert_eq!(response["id"], 237);
    assert_eq!(response["result"]["result"]["value"], json!("scheduled"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime intersection fetch request event");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["documentURL"], page_url);
    assert_eq!(request["params"]["request"]["url"], api_url);
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime intersection fetch request id")
        .to_owned();

    let response_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime intersection fetch response event");
    assert_eq!(response_event["params"]["type"], "Fetch");
    assert_eq!(
        response_event["params"]["response"]["headers"]["x-runtime-fetch"],
        "intersection"
    );

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 238,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        238,
        json!({
            "body": "runtime intersection fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_fetch_failure_emits_loading_failed() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let (failing_addr, failing_server) = spawn_connection_drop_server().await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{failing_addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 202).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 203,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": format!("fetch('{api_url}').catch(() => 'failed')") }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 203);
    assert_eq!(response["id"], 203);

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network failure",
        |message| message["method"] == json!("Network.loadingFailed"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .expect("runtime fetch request event");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime fetch request id")
        .to_owned();
    assert_eq!(request["params"]["request"]["url"], api_url);

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime fetch loadingFailed event");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["canceled"], false);
    assert!(
        failed["params"]["errorText"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );

    ctx.process_async(json!({
        "id": 204,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        204,
        -32000,
        "No data found for resource with given identifier",
    );

    failing_server.abort();
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_xhr_failure_emits_loading_failed() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let (failing_addr, failing_server) = spawn_connection_drop_server().await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{failing_addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 205).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 206,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!("new Promise(resolve => {{ const xhr = new XMLHttpRequest(); xhr.onerror = () => resolve('failed'); xhr.open('GET', '{xhr_url}'); xhr.send(); }})")
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 206);
    assert_eq!(response["id"], 206);

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "runtime subresource network failure",
        |message| message["method"] == json!("Network.loadingFailed"),
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .expect("runtime xhr request event");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("runtime xhr request id")
        .to_owned();
    assert_eq!(request["params"]["request"]["url"], xhr_url);

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("runtime xhr loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["canceled"], false);
    assert!(
        failed["params"]["errorText"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );

    ctx.process_async(json!({
        "id": 207,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        207,
        -32000,
        "No data found for resource with given identifier",
    );

    failing_server.abort();
    server.abort();
}
#[tokio::test]
async fn runtime_evaluate_detached_document_xpath_returns_iterator_results() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 150).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 151,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const doc = new DOMParser().parseFromString('<div id=\"a\"></div><section><div id=\"b\"></div></section>', 'text/html'); const result = doc.evaluate('//div', doc, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE); return [result.resultType, result.iterateNext()?.id ?? null, result.iterateNext()?.id ?? null, result.iterateNext()]; })()",
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 151);
    assert_eq!(
        response["result"]["result"]["value"],
        json!([5, "a", "b", null])
    );
}
#[tokio::test]
async fn runtime_evaluate_live_document_xpath_returns_iterator_results() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><div id=\"a\"></div><section><div id=\"b\"></div></section></body></html>",
    )
    .await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 151).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 152,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const result = document.evaluate('.//div', document.body, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE); return [result.resultType, result.iterateNext()?.id ?? null, result.iterateNext()?.id ?? null, result.iterateNext()]; })()",
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 152);
    assert_eq!(
        response["result"]["result"]["value"],
        json!([5, "a", "b", null])
    );
}
#[tokio::test]
async fn runtime_evaluate_patchright_closed_shadow_root_xpath_engine_maps_back_to_live_nodes() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 152).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 153,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                const host = document.createElement('div');
                document.body.appendChild(host);
                const root = host.attachShadow({ mode: 'closed' });
                root.innerHTML = '<section><div id="a"></div><span><div id="b"></div></span></section>';

                const result = [];
                const parser = new DOMParser();
                function getAllChildElements(node) {
                    const elements = [];
                    const traverse = currentNode => {
                        if (currentNode.nodeType === Node.ELEMENT_NODE)
                            elements.push(currentNode);
                        currentNode.childNodes?.forEach(traverse);
                    };
                    if (node.nodeType === Node.DOCUMENT_FRAGMENT_NODE || node.nodeType === Node.ELEMENT_NODE)
                        traverse(node);
                    return elements;
                }

                const csrHTMLContent = root.innerHTML;
                const csrChildElements = getAllChildElements(root);
                const htmlDoc = parser.parseFromString(csrHTMLContent, 'text/html');
                const rootDiv = htmlDoc.body;
                const rootDivChildElements = getAllChildElements(rootDiv);
                const it = htmlDoc.evaluate('//div', htmlDoc, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE);
                for (let node = it.iterateNext(); node; node = it.iterateNext()) {
                    const nodeIndex = rootDivChildElements.indexOf(node) - 1;
                    if (nodeIndex >= 0) {
                        const originalNode = csrChildElements[nodeIndex];
                        if (originalNode.nodeType === Node.ELEMENT_NODE)
                            result.push(originalNode.id);
                    }
                }
                return result;
            })()"#,
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 153);
    assert_eq!(response["result"]["result"]["value"], json!(["a", "b"]));
}
#[tokio::test]
async fn runtime_evaluate_dom_parser_query_apis_reuse_existing_detached_node_identity() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 154).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 155,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                const doc = new DOMParser().parseFromString('<div id="a"></div><section><div id="b"></div></section>', 'text/html');
                const bodyChildren = Array.from(doc.body.childNodes).filter(node => node.nodeType === Node.ELEMENT_NODE);
                const first = bodyChildren[0];
                const second = bodyChildren[1].childNodes[0];
                return [
                    first === doc.querySelector('#a'),
                    second === doc.querySelector('#b'),
                    second === doc.getElementById('b'),
                    doc.getElementsByTagName('div').item(1) === second,
                ];
            })()"#,
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 155);
    assert_eq!(
        response["result"]["result"]["value"],
        json!([true, true, true, true])
    );
}
#[tokio::test]
async fn registered_named_world_object_handles_remain_callable_after_navigation() {
    let mut ctx = TestContext::new();
    let (background_tx, mut background_rx) = tokio::sync::mpsc::unbounded_channel();
    ctx.conn.set_background_event_sender(background_tx);
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    let _ = create_isolated_world_async(&mut ctx, 506, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5061,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "",
            "worldName": "utility"
        }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 5061)["result"]["identifier"]
            .as_str()
            .is_some()
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 507,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 507);
    let isolated_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("navigation should replay the isolated utility world");
    ctx.sent.clear();
    while background_rx.try_recv().is_ok() {}

    ctx.process_async(json!({
        "id": 508,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({ answer: 42 })",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 508)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated evaluation should return an object handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 509,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({ bonus: 1 })",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let argument_object_id = take_response_by_id(&mut ctx, 509)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated evaluation should return an argument object handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 510,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function(arg) { return Promise.resolve(this.answer + arg.bonus); }",
            "arguments": [{ "objectId": argument_object_id }],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let mut other_context = crate::conn::BrowserContext::new("BID-2".into());
    other_context.set_active_target_id("TID-2");
    other_context.attach_active_session("SID-2");
    ctx.conn.inactive_browser_contexts.push(other_context);
    assert!(
        ctx.conn.activate_browser_context_by_id_async("BID-2").await,
        "test setup should switch the active context away from the pending Runtime.callFunctionOn owner"
    );

    for _ in 0..64 {
        if ctx.sent.iter().any(|message| message["id"] == json!(510)) {
            break;
        }
        while let Ok(message) = background_rx.try_recv() {
            match message.take_runtime_inspector_response_ready() {
                Ok(response) => {
                    ctx.sent
                        .push(response.into_protocol_message_for_typed_runtime_route());
                }
                Err(message) => ctx.sent.push(message.into_protocol_message()),
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        ctx.sent.iter().any(|message| message["id"] == json!(510)),
        "expected deferred Runtime.callFunctionOn response; sent={:?}",
        ctx.sent
    );
    let result = take_response_by_id(&mut ctx, 510);
    assert_eq!(result["result"]["result"]["value"], json!(43));
}
#[tokio::test]
async fn call_function_on_rejects_object_id_known_to_different_target_owner() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>owner-a</body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 511).await;

    ctx.process_async(json!({
        "id": 512,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 512);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return an object handle: {response:?}"))
        .to_owned();

    let mut other_context = crate::conn::BrowserContext::new("BID-2".into());
    other_context.set_active_target_id("TID-2");
    other_context.attach_active_session("SID-2");
    ctx.conn.inactive_browser_contexts.push(other_context);

    ctx.process_async(json!({
        "id": 513,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-2",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.owner; }",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 513);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );
}
#[tokio::test]
async fn dom_resolve_node_with_execution_context_returns_handle_for_calling_session() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><div id='wait-handle'></div></body></html>",
    )
    .await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    ctx.process_async(json!({
        "id": 50_001,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let enable = take_response_by_id(&mut ctx, 50_001);
    assert_eq!(enable["result"], json!({}));
    let default_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-1")
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("Runtime.enable should report the default execution context");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 50_002,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": default_context_id,
            "expression": "({ utility: true })"
        }
    }))
    .await;
    let utility = take_response_by_id(&mut ctx, 50_002);
    let utility_object_id = utility["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return utility handle: {utility:?}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 50_003,
        "method": "DOM.getDocument",
        "sessionId": "SID-1"
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 50_003);
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or_else(|| panic!("DOM.getDocument should return root nodeId: {document:?}"));

    ctx.process_async(json!({
        "id": 50_004,
        "method": "DOM.querySelector",
        "sessionId": "SID-1",
        "params": {
            "nodeId": root_id,
            "selector": "#wait-handle"
        }
    }))
    .await;
    let selected = take_response_by_id(&mut ctx, 50_004);
    let node_id = selected["result"]["nodeId"]
        .as_u64()
        .unwrap_or_else(|| panic!("DOM.querySelector should return nodeId: {selected:?}"));

    ctx.process_async(json!({
        "id": 50_005,
        "method": "DOM.resolveNode",
        "sessionId": "SID-1",
        "params": {
            "nodeId": node_id,
            "executionContextId": default_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 50_005);
    let resolved_object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("DOM.resolveNode should return element handle: {resolved:?}"))
        .to_owned();
    assert_ne!(
        resolved_object_id, utility_object_id,
        "DOM.resolveNode must allocate in the caller inspector session, not reuse an existing Runtime handle"
    );

    ctx.process_async(json!({
        "id": 50_006,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": utility_object_id,
            "functionDeclaration": "function(element) { element.remove(); return document.querySelector('#wait-handle') === null; }",
            "arguments": [{ "objectId": resolved_object_id }],
            "returnByValue": true
        }
    }))
    .await;
    let removed = take_response_by_id(&mut ctx, 50_006);
    assert_eq!(
        removed["result"]["result"]["value"],
        json!(true),
        "resolved node handle should remain a callable DOM Element in the same session: {removed:?}"
    );
}

#[tokio::test]
async fn call_function_on_rejects_dom_resolve_node_object_id_from_different_target_owner() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><div id='owner-node'>owner-a</div></body></html>",
    )
    .await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 514).await;

    ctx.process_async(json!({
        "id": 515,
        "method": "DOM.getDocument"
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 515)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("DOM.getDocument should return root nodeId");

    ctx.process_async(json!({
        "id": 516,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#owner-node" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 516)["result"]["nodeId"]
        .as_u64()
        .expect("DOM.querySelector should return nodeId");

    ctx.process_async(json!({
        "id": 517,
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 517);
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("DOM.resolveNode should return an object handle: {resolved:?}"))
        .to_owned();

    let mut other_context = crate::conn::BrowserContext::new("BID-2".into());
    other_context.set_active_target_id("TID-2");
    other_context.attach_active_session("SID-2");
    ctx.conn.inactive_browser_contexts.push(other_context);

    ctx.process_async(json!({
        "id": 518,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-2",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.id; }",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 518);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );
}
#[tokio::test]
async fn get_properties_rejects_object_id_known_to_different_target_owner() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>owner-a</body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 519).await;

    ctx.process_async(json!({
        "id": 520,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 520);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return an object handle: {response:?}"))
        .to_owned();

    push_loaded_runtime_frontend_enabled_background_context_async(
        &mut ctx,
        "BID-2",
        "TID-2",
        "SID-2",
        "<html><body>owner-b</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 521,
        "method": "Runtime.getProperties",
        "sessionId": "SID-2",
        "params": {
            "objectId": object_id,
            "ownProperties": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 521);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );
}
#[tokio::test]
async fn call_function_on_rejects_get_properties_returned_object_id_from_different_owner() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>owner-a</body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 522).await;

    ctx.process_async(json!({
        "id": 523,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ child: { answer: 42 } })"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 523);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return an object handle: {response:?}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 524,
        "method": "Runtime.getProperties",
        "params": {
            "objectId": object_id,
            "ownProperties": true
        }
    }))
    .await;
    let properties = take_response_by_id(&mut ctx, 524);
    let child_object_id = properties["result"]["result"]
        .as_array()
        .and_then(|properties| {
            properties
                .iter()
                .find(|property| property["name"] == json!("child"))
        })
        .and_then(|property| property["value"]["objectId"].as_str())
        .unwrap_or_else(|| {
            panic!("Runtime.getProperties should return a child object handle: {properties:?}")
        })
        .to_owned();

    push_loaded_runtime_frontend_enabled_background_context_async(
        &mut ctx,
        "BID-2",
        "TID-2",
        "SID-2",
        "<html><body>owner-b</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 525,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-2",
        "params": {
            "objectId": child_object_id,
            "functionDeclaration": "function() { return this.answer; }",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 525);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );
}
#[tokio::test]
async fn release_object_group_drops_inherited_get_properties_and_call_function_handles() {
    let mut ctx = TestContext::new();
    with_loaded_runtime_frontend_enabled_background_target_async(
        &mut ctx,
        "TID-active",
        "SID-active",
        "TID-background",
        "SID-background",
        "<html><body>owner-background</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 528,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 528);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 529,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "({ child: { answer: 42 } })",
            "objectGroup": "background-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 529);
    let parent_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return an object handle: {response:?}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 530,
        "method": "Runtime.getProperties",
        "sessionId": "SID-background",
        "params": {
            "objectId": parent_object_id,
            "ownProperties": true
        }
    }))
    .await;
    let properties = take_response_by_id(&mut ctx, 530);
    let child_object_id = properties["result"]["result"]
        .as_array()
        .and_then(|properties| {
            properties
                .iter()
                .find(|property| property["name"] == json!("child"))
        })
        .and_then(|property| property["value"]["objectId"].as_str())
        .unwrap_or_else(|| {
            panic!("Runtime.getProperties should return a child object handle: {properties:?}")
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 531,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-background",
        "params": {
            "objectId": child_object_id,
            "functionDeclaration": "function() { return { nested: this.answer }; }"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 531);
    let returned_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.callFunctionOn should return an object handle: {response:?}")
        })
        .to_owned();

    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                &[child_object_id.clone(), returned_object_id.clone()],
            )
            .is_err(),
        "active owner should see inherited-group handles as belonging to the background target"
    );

    ctx.process_async(json!({
        "id": 532,
        "method": "Runtime.releaseObjectGroup",
        "sessionId": "SID-background",
        "params": {
            "objectGroup": "background-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 532);
    assert_eq!(response["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                &[child_object_id, returned_object_id],
            )
            .is_ok(),
        "releaseObjectGroup should remove handles whose group was inherited from the receiver object"
    );
}
#[tokio::test]
async fn background_runtime_evaluate_emits_runtime_observable_from_background_owner_without_promotion()
 {
    let mut ctx = TestContext::new();
    with_loaded_runtime_frontend_enabled_background_target_async(
        &mut ctx,
        "TID-active",
        "SID-active",
        "TID-background",
        "SID-background",
        "<html><body>owner-background</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 530,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 530);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 531,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "console.warn('background observable')"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 531);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    let runtime_event = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Runtime.consoleAPICalled"))
        .unwrap_or_else(|| {
            panic!(
                "background Runtime.evaluate should emit Runtime.consoleAPICalled: {:?}",
                ctx.sent
            )
        });
    assert_eq!(runtime_event["sessionId"], json!("SID-background"));
    assert_eq!(
        runtime_event["params"]["args"][0]["value"],
        json!("background observable")
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.evaluate observable drain should not promote the target"
    );
}

#[tokio::test]
async fn call_function_on_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    with_loaded_runtime_frontend_enabled_background_target_async(
        &mut ctx,
        "TID-active",
        "SID-active",
        "TID-background",
        "SID-background",
        "<html><body>owner-background</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 533,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 533);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 534,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "({ owner: 'background-call-target' })"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 534);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("background Runtime.evaluate should return an object handle: {response:?}")
        })
        .to_owned();

    ctx.process_async(json!({
        "id": 535,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-background",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "function() { globalThis.__backgroundCallCount = (globalThis.__backgroundCallCount || 0) + 1; return this.owner + ':' + globalThis.__backgroundCallCount; }",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 535);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("background-call-target:1")
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.callFunctionOn should not promote the target"
    );

    ctx.process_async(json!({
        "id": 536,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-active",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.owner; }",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 536);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );

    ctx.process_async(json!({
        "id": 537,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "globalThis.__backgroundCallCount",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 537);
    assert_eq!(response["result"]["result"]["value"], json!(1));
}

#[tokio::test]
async fn evaluate_in_isolated_world_preserves_remote_object_handles() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 379).await;
    ctx.sent.clear();
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    let isolated_context_id = create_isolated_world_async(&mut ctx, 380, "utility").await;

    ctx.process_async(json!({
        "id": 381,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ answer: 42 })",
            "contextId": isolated_context_id
        }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 381);
    assert_eq!(msg["result"]["result"]["type"], json!("object"));
    assert!(msg["result"]["result"]["objectId"].as_str().is_some());
}
#[tokio::test]
async fn isolated_world_evaluate_registers_remote_object_owner() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>owner-a</body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 382).await;
    ctx.sent.clear();
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    let isolated_context_id = create_isolated_world_async(&mut ctx, 383, "utility").await;

    ctx.process_async(json!({
        "id": 384,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ owner: 'active-isolated' })",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 384)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated world evaluate should return an object handle")
        .to_owned();

    push_loaded_runtime_frontend_enabled_background_context_async(
        &mut ctx,
        "BID-2",
        "TID-2",
        "SID-2",
        "<html><body>owner-b</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 385,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-2",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.owner; }"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 385);
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );
}
/// Runtime.callFunctionOn executes via inspector in a real execution context.

#[test]
fn runtime_evaluate_set_timeout_flushes_runtime_activity_inside_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        ctx.process_async(json!({
            "id": 2059,
            "method": "Runtime.enable"
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 2059);
        assert_eq!(response["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2060,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "setTimeout(() => { globalThis.__timerRan = 1; }, 0)"
            }
        }))
        .await;

        let response = take_response_by_id(&mut ctx, 2060);
        assert_eq!(response["id"], 2060);
        assert!(
            response.get("error").is_none(),
            "runtime evaluate should not fail: {response:?}"
        );

        // The setTimeout(0) callback now fires from the owner loop's idle tick
        // branch (Step A/B refactor) instead of inline as part of the evaluate
        // reply. Retry the observation evaluate to give the owner a chance to
        // tick before the next command arrives.
        for attempt in 0..16 {
            ctx.process_async(json!({
                "id": 2061,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "globalThis.__timerRan ?? 0"
                }
            }))
            .await;

            let response = take_response_by_id(&mut ctx, 2061);
            if response["result"]["result"]["value"] == json!(1) {
                return;
            }
            if attempt + 1 == 16 {
                panic!("background timer did not fire within retry budget: {response:?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}
#[test]
fn runtime_evaluate_await_promise_waits_for_set_timeout_settlement_inside_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2062,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => setTimeout(() => resolve(7), 0))",
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2062).await;
        assert_eq!(response["result"]["result"]["type"], json!("number"));
        assert_eq!(response["result"]["result"]["value"], json!(7));
    });
}
#[test]
fn runtime_evaluate_await_promise_waits_for_nonzero_timeout_settlement() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2063,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => setTimeout(() => resolve(11), 40))",
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2063).await;
        assert_eq!(response["result"]["result"]["type"], json!("number"));
        assert_eq!(response["result"]["result"]["value"], json!(11));
    });
}
#[test]
fn runtime_evaluate_request_idle_callback_time_remaining_expires_within_callback() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2064,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"
new Promise(resolve => requestIdleCallback(deadline => {
  const first = deadline.timeRemaining();
  const start = Date.now();
  while (Date.now() - start < 80) {}
  const second = deadline.timeRemaining();
  resolve({
    firstPositive: first > 0,
    secondExpired: second === 0,
    decreased: second < first
  });
}))
"#,
                "awaitPromise": true,
                "returnByValue": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2064).await;
        assert_eq!(
            response["result"]["result"]["value"],
            json!({
                "firstPositive": true,
                "secondExpired": true,
                "decreased": true
            })
        );
    });
}

#[test]
fn runtime_call_function_on_await_promise_settles_after_request_animation_frame() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2065,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 2065);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2066,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmAsyncProbe = { check() { return new Promise(resolve => requestAnimationFrame(() => resolve({ ready: true, source: 'raf' }))); } }; globalThis.__lmAsyncProbe"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 2066)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the async probe");

        ctx.process_async(json!({
            "id": 2067,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.check(); }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2067).await;
        assert_eq!(response["result"]["result"]["type"], json!("object"));
        assert_eq!(
            response["result"]["result"]["value"],
            json!({ "ready": true, "source": "raf" })
        );
    });
}
#[test]
fn runtime_call_function_on_await_promise_waits_for_nonzero_timeout_settlement() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 20_671,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 20_671);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 20_672,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmDelayedAsyncProbe = { check() { return new Promise(resolve => setTimeout(() => resolve({ ready: true, delayMs: 40 }), 40)); } }; globalThis.__lmDelayedAsyncProbe"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 20_672)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the delayed async probe");

        ctx.process_async(json!({
            "id": 20_673,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.check(); }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 20_673).await;
        assert_eq!(response["result"]["result"]["type"], json!("object"));
        assert_eq!(
            response["result"]["result"]["value"],
            json!({ "ready": true, "delayMs": 40 })
        );
    });
}
#[test]
fn runtime_call_function_on_await_promise_waits_for_polling_handle_result_until_settled() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 20_674,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 20_674);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 20_675,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  globalThis.__lmWaitProbeDone = false;
  setTimeout(() => { globalThis.__lmWaitProbeDone = true; }, 50);
  globalThis.__lmWaitProbe = {
    result: new Promise(resolve => {
      const next = () => {
        if (globalThis.__lmWaitProbeDone) {
          resolve("wait-ready");
          return;
        }
        requestAnimationFrame(next);
      };
      next();
    })
  };
  return globalThis.__lmWaitProbe;
})()"#
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 20_675)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the polling await probe");

        ctx.process_async(json!({
            "id": 20_676,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.result; }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 20_676).await;
        assert_eq!(response["result"]["result"]["type"], json!("string"));
        assert_eq!(response["result"]["result"]["value"], json!("wait-ready"));
    });
}
#[test]
fn runtime_evaluate_await_promise_waits_for_request_animation_frame_polling_condition() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 206_761,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  window.__done = false;
  setTimeout(() => { window.__done = true; }, 50);
  return new Promise(resolve => {
    const poll = () => {
      if (window.__done === true) {
        resolve(true);
        return;
      }
      requestAnimationFrame(poll);
    };
    poll();
  });
})()"#,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_761).await;
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected rAF polling await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}
#[test]
fn runtime_evaluate_await_promise_waits_for_request_animation_frame_polling_with_indirect_eval() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 206_762,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  window.__done = false;
  setTimeout(() => { window.__done = true; }, 50);
  return new Promise(resolve => {
    const poll = () => {
      if (globalThis.eval("window.__done === true")) {
        resolve(true);
        return;
      }
      requestAnimationFrame(poll);
    };
    poll();
  });
})()"#,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_762).await;
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected indirect-eval rAF polling await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}
#[test]
fn runtime_evaluate_await_promise_without_enable_does_not_create_legacy_global_token() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        // Keep the test promise reachable so V8's legitimate collection of an
        // otherwise unreferenced pending promise cannot race the registry
        // assertion below. The behavior under test is that moli itself
        // does not create a `__lmAwaitPromise_*` polling token.
        ctx.process_async(json!({
            "id": 206_763,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__testPendingInspectorPromise = new Promise(() => {})",
                "awaitPromise": true
            }
        }))
        .await;

        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(206_763)),
            "never-settling inspector await should defer the response: {:?}",
            ctx.sent
        );
        assert!(
            ctx.conn.has_pending_inspector_awaits(),
            "runtimeless awaitPromise should use the inspector pending-await registry"
        );

        ctx.process_async(json!({
            "id": 206_764,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "Object.keys(globalThis).filter(key => key.startsWith('__lmAwaitPromise_')).length",
                "returnByValue": true
            }
        }))
        .await;

        let probe = take_response_by_id(&mut ctx, 206_764);
        assert_eq!(
            probe["result"]["result"]["value"],
            json!(0),
            "inspector awaitPromise should not create legacy global polling tokens: {probe:?}"
        );
    });
}
#[test]
fn runtime_evaluate_await_promise_pending_is_failed_when_page_closes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        if let Some(bc) = ctx.conn.browser_context.as_mut() {
            bc.set_active_target_id("TID-1");
        }
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 9_001).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 9_002,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__pageClosePendingPromise = new Promise(() => {})",
                "awaitPromise": true
            }
        }))
        .await;

        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(9_002)),
            "awaitPromise on a never-resolving promise should defer the response, got: {:?}",
            ctx.sent
        );
        assert!(
            ctx.conn.has_pending_inspector_awaits(),
            "deferred awaitPromise must register a pending inspector entry"
        );

        ctx.process_async(json!({
            "id": 9_003,
            "method": "Page.close"
        }))
        .await;

        let failed = take_response_by_id(&mut ctx, 9_002);
        assert_eq!(failed["error"]["code"], json!(-32000));
        assert_eq!(failed["error"]["message"], json!("Page closed"));
        assert!(
            !ctx.conn.has_pending_inspector_awaits(),
            "all pending inspector awaits should be cleared after Page.close"
        );
    });
}

#[test]
fn runtime_evaluate_await_promise_pending_is_terminated_once_by_navigation_replacement() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_for_active_target_async(
            &mut ctx,
            "<html><body>old</body></html>",
            "SID-1",
            "TID-1",
        )
        .await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 9_012,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "globalThis.__navigationPendingPromise = new Promise(() => {})",
                "awaitPromise": true
            }
        }))
        .await;
        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(9_012)),
            "never-settling evaluate must remain outstanding before navigation"
        );

        ctx.process_async(json!({
            "id": 9_013,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<html><body>new</body></html>"
            }
        }))
        .await;

        wait_until_message(
            &mut ctx,
            "SID-1",
            "navigation terminal response for old Runtime.evaluate",
            |message| message["id"] == json!(9_012),
        )
        .await;
        let responses = ctx
            .sent
            .iter()
            .filter(|message| message["id"] == json!(9_012))
            .collect::<Vec<_>>();
        assert_eq!(
            responses.len(),
            1,
            "navigation replacement must complete the old evaluate exactly once: {:?}",
            ctx.sent
        );
        assert_eq!(responses[0]["error"]["code"], json!(-32000));
        assert_eq!(responses[0]["sessionId"], json!("SID-1"));
        assert_eq!(
            responses[0]["error"]["message"],
            json!("Inspected target navigated or closed")
        );
        assert!(
            ctx.conn
                .renderer_runtime_command_cause_for_frontend(Some("SID-1"), 9_012)
                .is_none(),
            "terminal replacement response must consume the renderer correlation"
        );
        assert!(
            !ctx.conn.has_pending_inspector_awaits(),
            "terminal replacement response must consume the pending command owner"
        );
    });
}

#[test]
fn replay_policy_command_rotates_lease_and_completes_on_replacement_attachment() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_for_active_target_async(
            &mut ctx,
            "<html><body>old</body></html>",
            "SID-1",
            "TID-1",
        )
        .await;
        let old_attachment = ctx
            .conn
            .current_renderer_agent_attachment_id_for_session_owner(Some("SID-1"))
            .expect("old Page attachment");
        let frontend_id = 9_021;
        let payload = json!({
            "id": frontend_id,
            "method": "Console.clearMessages",
            "sessionId": "SID-1",
            "params": {}
        })
        .to_string();
        let descriptor = crate::conn::RendererCommandDescriptor::from_synthesized_payload(payload)
            .expect("supported replay command");
        let prepared = ctx
            .conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-1"),
                frontend_id,
                Some(old_attachment),
                descriptor,
            )
            .expect("register replay command");
        let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
        let response_receiver = response_receiver
            .expect("a synthesized CommandReply call must allocate a response receiver");

        ctx.process_async(json!({
            "id": 9_022,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<html><body>new</body></html>"
            }
        }))
        .await;

        let new_attachment = ctx
            .conn
            .current_renderer_agent_attachment_id_for_session_owner(Some("SID-1"))
            .expect("replacement Page attachment");
        assert_ne!(new_attachment, old_attachment);
        assert!(
            old_sender
                .send(json!({
                    "id": old_correlation.renderer_call_id().get(),
                    "result": { "stale": true }
                }))
                .is_err(),
            "attachment commit must invalidate the old sender before Page teardown"
        );
        let completion = tokio::time::timeout(std::time::Duration::from_secs(5), response_receiver)
            .await
            .expect("replacement replay should complete")
            .expect("replacement response channel should remain open");
        assert_ne!(completion.call_id, old_correlation.renderer_call_id().get());
        assert_eq!(
            completion.renderer_agent_attachment_id(),
            Some(new_attachment)
        );
        assert_eq!(
            completion
                .output
                .protocol_response(completion.call_id)
                .expect("Console.clearMessages replay response")["result"],
            json!({})
        );

        let resolved = ctx
            .conn
            .resolve_runtime_inspector_response_ready(
                crate::conn::RuntimeInspectorResponseReady::new(
                    frontend_id,
                    Some("SID-1"),
                    Ok(completion),
                ),
            )
            .expect("current replay completion must consume its frontend correlation");
        assert_eq!(resolved.command_id(), frontend_id);
    });
}

#[test]
fn runtime_evaluate_await_promise_timer_reply_ignores_unrelated_output_stream_control() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 9_101).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 9_102,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => setTimeout(() => resolve('timer-only'), 500))",
                "awaitPromise": true,
                "returnByValue": true
            }
        }))
        .await;

        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(9_102)),
            "timer-backed awaitPromise should defer until the timer settles: {:?}",
            ctx.sent
        );
        let unrelated_output = ctx
            .route_renderer_publication_for_test(
                RendererOutputStreamControl::Opened {
                    stream: RendererOutputStreamIdentity::new_page_for_protocol_test(
                        PageId::new_for_testing(9_102),
                    ),
                }
                .into(),
            )
            .await;
        assert!(
            !unrelated_output
                .iter()
                .any(|message| message["id"] == json!(9_102)),
            "an unrelated stream control must not synthesize the pending awaitPromise response: {unrelated_output:?}"
        );

        let response = wait_for_response_by_id_async(&mut ctx, None, 9_102).await;
        assert_eq!(
            response["result"]["result"]["value"],
            json!("timer-only"),
            "timer-backed awaitPromise should still complete through the renderer receiver after a stale runtime wake: {response:?}"
        );
    });
}
#[test]
fn runtime_call_function_on_playwright_style_handle_result_awaits_polling_promise_until_settled() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_677).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 20_679,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  function parseEvaluationResultValue(value, handles = [], refs = new Map()) {
    if (Object.is(value, undefined))
      return undefined;
    if (typeof value === 'object' && value) {
      if ('ref' in value)
        return refs.get(value.ref);
      if ('v' in value) {
        if (value.v === 'undefined')
          return undefined;
        if (value.v === 'null')
          return null;
        return undefined;
      }
      if ('a' in value) {
        const result = [];
        refs.set(value.id, result);
        for (const item of value.a)
          result.push(parseEvaluationResultValue(item, handles, refs));
        return result;
      }
      if ('o' in value) {
        const result = {};
        refs.set(value.id, result);
        for (const { k, v } of value.o) {
          if (k === '__proto__')
            continue;
          result[k] = parseEvaluationResultValue(v, handles, refs);
        }
        return result;
      }
      if ('h' in value)
        return handles[value.h];
    }
    return value;
  }

  class UtilityScript {
    constructor(global, isUnderTest) {
      this.global = global;
      this.isUnderTest = isUnderTest;
    }

    evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) {
      const args = argsAndHandles.slice(0, argCount);
      const handles = argsAndHandles.slice(argCount);
      const parameters = [];
      for (let i = 0; i < args.length; ++i)
        parameters[i] = parseEvaluationResultValue(args[i], handles);
      let result = this.global.eval(expression);
      if (isFunction === true) {
        result = result(...parameters);
      } else if (isFunction === false) {
        result = result;
      } else if (typeof result === 'function') {
        result = result(...parameters);
      }
      return returnByValue ? Promise.resolve(result).then(value => JSON.parse(JSON.stringify(value))) : result;
    }
  }

  globalThis.__lmPlaywrightInjected = {
    utils: {
      builtins: {
        requestAnimationFrame: globalThis.requestAnimationFrame.bind(globalThis),
        setTimeout: globalThis.setTimeout.bind(globalThis),
      }
    }
  };

  return new UtilityScript(globalThis, false);
})()"#
            }
        }))
        .await;
        let utility_object_id =
            take_response_by_id(&mut ctx, 20_679)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.evaluate should return a utilityScript object id");

        ctx.process_async(json!({
            "id": 206_710,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmPlaywrightInjected"
            }
        }))
        .await;
        let injected_object_id =
            take_response_by_id(&mut ctx, 206_710)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.evaluate should return an injected helper object id");

        ctx.process_command_only_async(json!({
            "id": 206_711,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": utility_object_id.clone(),
                "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
                "arguments": [
                    { "objectId": utility_object_id.clone() },
                    { "value": true },
                    { "value": false },
                    { "value": "(injected, { expression: expression2, isFunction: isFunction2, polling, arg: arg2 }) => {\n  let evaledExpression;\n  const predicate = () => {\n    let result2 = evaledExpression ?? globalThis.eval(expression2);\n    if (isFunction2 === true) {\n      evaledExpression = result2;\n      result2 = result2(arg2);\n    } else if (isFunction2 === false) {\n      result2 = result2;\n    } else if (typeof result2 === 'function') {\n      evaledExpression = result2;\n      result2 = result2(arg2);\n    }\n    return result2;\n  };\n  let fulfill;\n  let reject;\n  let aborted = false;\n  const result = new Promise((f, r) => {\n    fulfill = f;\n    reject = r;\n  });\n  const next = () => {\n    if (aborted)\n      return;\n    try {\n      const success = predicate();\n      if (success) {\n        fulfill(success);\n        return;\n      }\n      if (typeof polling !== 'number')\n        injected.utils.builtins.requestAnimationFrame(next);\n      else\n        injected.utils.builtins.setTimeout(next, polling);\n    } catch (e) {\n      reject(e);\n    }\n  };\n  next();\n  globalThis.__lmPlaywrightWaitHandle = { result, abort: () => aborted = true };\n  return globalThis.__lmPlaywrightWaitHandle;\n}" },
                    { "value": 2 },
                    { "value": { "h": 0 } },
                    { "value": { "o": [
                      { "k": "expression", "v": "window.__done === true" },
                      { "k": "isFunction", "v": { "v": "undefined" } },
                      { "k": "polling", "v": { "v": "undefined" } },
                      { "k": "arg", "v": { "v": "null" } }
                    ], "id": 1 } },
                    { "objectId": injected_object_id }
                ],
                "returnByValue": false,
                "awaitPromise": true
            }
        }))
        .await;
        let handle_object_id =
            take_response_by_id(&mut ctx, 206_711)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.callFunctionOn should return a handle object id");

        ctx.process_async(json!({
            "id": 206_712,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__done = false; setTimeout(() => { globalThis.__done = true; }, 50); 'armed'"
            }
        }))
        .await;
        let armed = take_response_by_id(&mut ctx, 206_712);
        assert_eq!(armed["result"]["result"]["value"], json!("armed"));

        ctx.process_async(json!({
            "id": 206_713,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": utility_object_id.clone(),
                "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
                "arguments": [
                    { "objectId": utility_object_id },
                    { "value": true },
                    { "value": false },
                    { "value": "(h) => h.result" },
                    { "value": 2 },
                    { "value": { "h": 0 } },
                    { "value": { "v": "undefined" } },
                    { "objectId": handle_object_id }
                ],
                "returnByValue": false,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_713).await;
        if response.get("error").is_some() {
            ctx.process_async(json!({
                "id": 206_714,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "window.__done === true"
                }
            }))
            .await;
            let done_state = take_response_by_id(&mut ctx, 206_714);

            ctx.process_async(json!({
                "id": 206_715,
                "method": "Runtime.callFunctionOn",
                "params": {
                    "objectId": handle_object_id.clone(),
                    "functionDeclaration": "function() { return [typeof this.result, typeof this.result?.then]; }",
                    "returnByValue": true,
                    "awaitPromise": false
                }
            }))
            .await;
            let handle_state = take_response_by_id(&mut ctx, 206_715);

            ctx.process_async(json!({
                "id": 206_716,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "globalThis.__lmPlaywrightWaitHandle.result",
                    "awaitPromise": true
                }
            }))
            .await;
            let direct_wait = take_response_by_id(&mut ctx, 206_716);

            panic!(
                "unexpected Playwright-style await response: {response:?}; done_state={done_state:?}; handle_state={handle_state:?}; direct_wait={direct_wait:?}"
            );
        }
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected Playwright-style await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}
#[test]
fn runtime_call_function_on_playwright_style_utility_promise_awaits_polling_condition() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_720).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 206_721,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  class UtilityScript {
    constructor(global) {
      this.global = global;
    }

    evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) {
      const args = argsAndHandles.slice(0, argCount);
      let result = this.global.eval(expression);
      if (isFunction === true)
        result = result(...args);
      else if (isFunction === false)
        result = result;
      else if (typeof result === 'function')
        result = result(...args);
      return returnByValue ? Promise.resolve(result).then(value => JSON.parse(JSON.stringify(value))) : result;
    }
  }

  return new UtilityScript(globalThis);
})()"#
            }
        }))
        .await;
        let utility_object_id =
            take_response_by_id(&mut ctx, 206_721)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.evaluate should return a utilityScript object id");

        ctx.process_async(json!({
            "id": 206_722,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": utility_object_id.clone(),
                "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
                "arguments": [
                    { "objectId": utility_object_id },
                    { "value": true },
                    { "value": true },
                    { "value": r#"() => {
  window.__done = false;
  setTimeout(() => { window.__done = true; }, 50);
  return new Promise(resolve => {
    const poll = () => {
      if (window.__done === true) {
        resolve(true);
        return;
      }
      requestAnimationFrame(poll);
    };
    poll();
  });
}"# },
                    { "value": 0 }
                ],
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_722).await;
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected utility promise await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_await_promise_in_isolated_world_waits_for_fetch_driven_dom_change() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body>waiting<script>
setTimeout(() => {
  fetch('/api').then(r => r.text()).then(text => {
    document.body.dataset.ready = text;
  });
}, 0);
</script></body></html>"#,
        )
    }

    async fn api() -> impl IntoResponse {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        ([(CONTENT_TYPE.as_str(), "text/plain")], "fetch-ready")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_701).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 20_702, "utility").await;
    ctx.sent.clear();

    // Timer polling lets the test harness return from the command turn; rAF
    // would continuously enqueue immediate runtime work before the fetch settles.
    ctx.process_async(json!({
        "id": 20_703,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": utility_context_id,
            "expression": r#"new Promise(resolve => {
  const poll = () => {
    const ready = document.body.dataset.ready;
    if (ready) {
      resolve(ready);
      return;
    }
    setTimeout(poll, 5);
  };
  poll();
})"#,
            "awaitPromise": true
        }
    }))
    .await;

    let response = wait_for_response_by_id_async(&mut ctx, "SID-1", 20_703).await;
    assert_eq!(
        response["result"]["result"]["type"],
        json!("string"),
        "unexpected Runtime.evaluate awaitPromise response: {response:?}"
    );
    assert_eq!(response["result"]["result"]["value"], json!("fetch-ready"));

    server.abort();
}
#[test]
fn runtime_call_function_on_await_promise_handles_sync_undefined_result() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2068,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 2068);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2069,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmSyncStopProbe = { stop() {} }; globalThis.__lmSyncStopProbe"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 2069)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the sync stop probe");

        ctx.process_async(json!({
            "id": 2070,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.stop(); }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = take_response_by_id(&mut ctx, 2070);
        assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    });
}
