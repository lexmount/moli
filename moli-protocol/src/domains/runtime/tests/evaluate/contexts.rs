// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn enable_succeeds() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 1, "method": "Runtime.enable"}))
        .await;
    ctx.expect_result(1, json!({}), None);
}

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
async fn runtime_enable_publishes_ordered_replay_without_polling_command_completions() {
    let mut ctx = TestContext::new();
    with_loaded_document_for_active_target_async(
        &mut ctx,
        "<!doctype html><script>console.log('buffered-enable-replay')</script>",
        "SID-enable-order",
        "TID-enable-order",
    )
    .await;
    ctx.sent.clear();

    // Hold all adapter completions. Only the renderer journal is allowed to
    // deliver the replies, including Runtime.enable's replay and state changes.
    let mut pending = Vec::new();
    for (id, method) in [
        (11_021, "DOM.getDocument"),
        (11_022, "Runtime.enable"),
        (11_023, "DOM.getDocument"),
    ] {
        let step = ctx.conn.start_command_dispatch(
            &json!({"id": id, "method": method, "sessionId": "SID-enable-order"}).to_string(),
        );
        assert!(matches!(&step, CdpCommandTaskStep::Pending(_)));
        pending.push(step);
    }
    let following = ctx
        .wait_for_scheduler_message("terminal after Runtime.enable replay", |message| {
            message["id"] == 11_023
        })
        .await;
    assert!(following.get("result").is_some(), "{following:?}");

    let preceding = ctx
        .sent
        .iter()
        .position(|message| message["id"] == 11_021)
        .expect("the earlier renderer terminal must already be delivered");
    let context = ctx
        .sent
        .iter()
        .position(|message| message["method"] == "Runtime.executionContextCreated")
        .expect("Runtime.enable must deliver its frozen context replay");
    let console = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == "Runtime.consoleAPICalled"
                && message["params"]["args"][0]["value"] == "buffered-enable-replay"
        })
        .expect("Runtime.enable must deliver buffered console replay");
    let terminal = ctx
        .sent
        .iter()
        .position(|message| message["id"] == 11_022)
        .expect("Runtime.enable must publish without its completion being polled");
    assert!(preceding < context && context < terminal, "{:?}", ctx.sent);
    assert!(preceding < console && console < terminal, "{:?}", ctx.sent);
    assert_eq!(ctx.sent[terminal]["result"], json!({}));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["sessionId"] == "SID-enable-order")
    );
    let owner = crate::conn::CommandOwnerScope::for_session("SID-enable-order");
    let state = ctx
        .conn
        .target_runtime_session_state_for_owner(&owner)
        .unwrap();
    assert!(state.runtime_frontend_enabled && state.runtime_contexts_reported_to_frontend);

    // Cancel adapters after publication, then observe another concrete output.
    // Cancellation cannot retract replay or produce a second terminal.
    drop(pending);
    ctx.process_async(json!({
        "id": 11_024, "method": "DOM.getDocument", "sessionId": "SID-enable-order"
    }))
    .await;
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| message["id"] == 11_022)
            .count(),
        1
    );
}

#[tokio::test]
async fn runtime_enable_retired_owner_error_keeps_journal_order_and_single_terminal() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<p>retired Runtime.enable replay</p>").await;
    // Admit the live stream's owner before retiring it with output in flight.
    ctx.process_async(json!({"id": 11_025, "method": "DOM.getDocument"}))
        .await;
    ctx.sent.clear();
    let preceding = ctx
        .conn
        .start_command_dispatch(&json!({"id": 11_026, "method": "DOM.getDocument"}).to_string());
    let CdpCommandTaskStep::Pending(enable) = ctx
        .conn
        .start_command_dispatch(&json!({"id": 11_027, "method": "Runtime.enable"}).to_string())
    else {
        panic!("Runtime.enable must enter the renderer");
    };
    // This receipt proves the journal commit. Leave protocol ingress unpolled
    // until the original Browser owner has disappeared.
    let completed = enable.wait().await;
    let retired = ctx.conn.browser_context.take().unwrap();
    let error = ctx
        .wait_for_scheduler_message("retired Runtime.enable terminal", |message| {
            message["id"] == 11_027
        })
        .await;
    assert_eq!(error["error"]["code"], -32000);
    assert!(
        ctx.sent
            .iter()
            .any(|message| { message["id"] == 11_026 && message.get("result").is_some() }),
        "the retired error cannot overtake the earlier terminal: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == "Runtime.executionContextCreated")
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "replay cannot recreate a retired owner"
    );

    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("published native terminal must complete its adapter");
    };
    let (messages, scheduler_events, predecessor) = outcome.into_protocol_event_parts();
    assert!(
        messages.is_empty(),
        "adapter completion cannot emit a second terminal: {messages:?}"
    );
    assert!(scheduler_events.is_empty());
    let sent_start = ctx.sent.len();
    ctx.route_direct_command_renderer_predecessor_for_test(
        predecessor.expect("the adapter must acknowledge its published terminal"),
    )
    .await;
    assert!(
        !ctx.sent[sent_start..]
            .iter()
            .any(|message| message["id"] == 11_027)
    );
    drop(preceding);
    drop(retired);
}

#[tokio::test]
async fn runtime_disable_retired_owner_keeps_committed_success_and_single_terminal() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<p>retired Runtime.disable</p>").await;
    ctx.process_async(json!({"id": 11_034, "method": "Runtime.enable"}))
        .await;
    ctx.sent.clear();
    let preceding = ctx
        .conn
        .start_command_dispatch(&json!({"id": 11_035, "method": "DOM.getDocument"}).to_string());
    let CdpCommandTaskStep::Pending(disable) = ctx
        .conn
        .start_command_dispatch(&json!({"id": 11_036, "method": "Runtime.disable"}).to_string())
    else {
        panic!("Runtime.disable must enter the renderer");
    };
    let completed = disable.wait().await;
    // Retire the Browser owner after renderer commit, before ordered ingress.
    let retired = ctx.conn.browser_context.take().unwrap();
    let response = ctx
        .wait_for_scheduler_message("retired Runtime.disable terminal", |message| {
            message["id"] == 11_036
        })
        .await;
    assert_eq!(response["result"], json!({}));
    assert!(
        ctx.sent
            .iter()
            .any(|message| { message["id"] == 11_035 && message.get("result").is_some() }),
        "the committed disable cannot overtake an earlier terminal: {:?}",
        ctx.sent
    );
    assert!(ctx.conn.browser_context.is_none());
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == "Runtime.executionContextCreated")
    );

    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("the published disable must complete its adapter");
    };
    let (messages, scheduler_events, predecessor) = outcome.into_protocol_event_parts();
    assert!(messages.is_empty() && scheduler_events.is_empty());
    ctx.route_direct_command_renderer_predecessor_for_test(
        predecessor.expect("the adapter must acknowledge the committed disable"),
    )
    .await;
    // wait_for_scheduler_message removed the original response above.
    assert!(!ctx.sent.iter().any(|message| message["id"] == 11_036));
    assert!(
        ctx.conn.browser_context.is_none(),
        "late completion cannot recreate the retired owner"
    );
    drop(preceding);
    drop(retired);
}

#[tokio::test]
async fn runtime_enable_ready_terminal_does_not_wait_for_an_unresolved_expression() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<p>pending expression and ready enable</p>").await;
    enable_runtime_and_take_execution_context_id_async(&mut ctx, 11_028).await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 11_029, "method": "Runtime.evaluate",
        "params": {
            "expression": "new Promise(resolve => { globalThis.__resolveEnablePending = resolve; console.log('enable-pending-started'); })",
            "awaitPromise": true, "returnByValue": true
        }
    })).await;
    ctx.wait_for_scheduler_message("console output from unresolved expression", |message| {
        message["method"] == "Runtime.consoleAPICalled"
            && message["params"]["args"][0]["value"] == "enable-pending-started"
    })
    .await;

    ctx.process_async(json!({"id": 11_030, "method": "Runtime.enable"}))
        .await;
    assert_eq!(take_response_by_id(&mut ctx, 11_030)["result"], json!({}));
    assert!(ctx.conn.has_pending_inspector_awaits());
    assert!(!ctx.sent.iter().any(|message| message["id"] == 11_029));

    ctx.process_async(json!({
        "id": 11_031, "method": "Runtime.evaluate",
        "params": {"expression": "globalThis.__resolveEnablePending('settled-explicitly')"}
    }))
    .await;
    let settled = wait_for_response_by_id_async(&mut ctx, None, 11_029).await;
    assert_eq!(settled["result"]["result"]["value"], "settled-explicitly");
}

#[tokio::test]
async fn runtime_enable_then_disable_keeps_state_in_delivery_order() {
    for (first, second, expected_enabled) in [
        ("Runtime.enable", "Runtime.disable", false),
        ("Runtime.disable", "Runtime.enable", true),
    ] {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<p>ordered Runtime state</p>").await;
        let first = ctx
            .conn
            .start_command_dispatch(&json!({"id": 11_032, "method": first}).to_string());
        let second = ctx
            .conn
            .start_command_dispatch(&json!({"id": 11_033, "method": second}).to_string());
        let (messages, _) = ctx
            .complete_command_task_step_with_events_for_test(second)
            .await;
        assert!(
            messages
                .iter()
                .any(|message| message["id"] == 11_033 && message["result"] == json!({}))
        );
        assert_eq!(
            ctx.conn
                .target_runtime_session_state_for_session(None)
                .unwrap()
                .runtime_frontend_enabled,
            expected_enabled,
            "subscription state must follow journal delivery: {messages:?}"
        );
        let (late, _) = ctx
            .complete_command_task_step_with_events_for_test(first)
            .await;
        assert!(
            !late.iter().any(|message| message["id"] == 11_032),
            "late adapter completion cannot repeat a committed terminal: {late:?}"
        );
        assert_eq!(
            ctx.conn
                .target_runtime_session_state_for_session(None)
                .unwrap()
                .runtime_frontend_enabled,
            expected_enabled,
            "late completion cannot reverse a delivered subscription change"
        );
    }
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
    let (messages, scheduler_events) = ctx
        .complete_command_task_step_with_events_for_test(step)
        .await;

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
            .active_page_target()
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
            .active_page_target()
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
    let v8_state_before_navigation = browser_context.active_page_target().devtools_sessions
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
        .active_page_target()
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
        browser_context.active_page_target().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .inspector_session_state
            .v8_state
            .is_some(),
        "successful Runtime.disable must persist the disabled V8 agent cookie"
    );
    browser_context.active_page_target_mut().devtools_sessions
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
            .active_page_target()
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
            .active_page_target()
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
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled
    );
}
