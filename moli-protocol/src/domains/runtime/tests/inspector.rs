use super::*;

#[tokio::test]
async fn isolated_inspector_context_lookup_does_not_issue_internal_runtime_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    let utility_context_id = create_isolated_world_async(&mut ctx, 14, "utility").await;
    ctx.sent.clear();

    let inspector_context_id = ctx
        .conn
        .inspector_execution_context_id_for_isolated_context_async(utility_context_id)
        .await
        .expect("inspector context lookup should not fail")
        .expect("isolated context should expose inspector context id");
    assert!(inspector_context_id > 0);

    ctx.complete_one_ready_scheduler_input_for_test().await;
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["id"] == json!(900_199)
        }),
        "isolated inspector context lookup must not issue internal Runtime.enable: {:?}",
        ctx.sent
    );
}
#[tokio::test]
async fn isolated_inspector_context_lookup_does_not_collide_with_pending_inspector_await() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    let utility_context_id = create_isolated_world_async(&mut ctx, 15, "utility").await;
    ctx.conn
        .register_pending_inspector_await(900_199, Some("SID-1"));
    ctx.sent.clear();

    let inspector_context_id = ctx
        .conn
        .inspector_execution_context_id_for_isolated_context_async(utility_context_id)
        .await
        .expect("inspector context lookup should not fail");
    assert!(
        inspector_context_id.is_some(),
        "isolated context ids should be known without issuing internal Runtime.enable"
    );
    assert!(
        ctx.conn.has_pending_inspector_awaits(),
        "isolated context lookup must not consume pending inspector awaits"
    );
}
#[tokio::test]
async fn runtime_evaluate_materializes_default_context_via_inspector_hook() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><script>globalThis.__defaultProbe = 41;</script></body></html>",
    )
    .await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.sent.clear();

    let before = ctx
        .conn
        .runtime_default_execution_context_id_for_session_owner_async(Some("SID-1"))
        .await
        .expect("default context lookup should not fail before evaluate");
    assert_eq!(
        before, None,
        "test setup should start with CDP Runtime enabled but no inspector default context"
    );

    ctx.process_async(json!({
        "id": 16,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__defaultProbe + 1",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 16);
    assert_eq!(response["result"]["result"]["value"], json!(42));

    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["id"] == json!(900_199)
        }),
        "inspector default-context materialization must not issue internal Runtime.enable: {:?}",
        ctx.sent
    );
}
#[tokio::test]
async fn pending_inspector_await_does_not_block_default_context_evaluate() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><script>globalThis.__defaultProbe = 40;</script></body></html>",
    )
    .await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.conn
        .register_pending_inspector_await(900_199, Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 18,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__defaultProbe + 2",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 18);
    assert_eq!(response["result"]["result"]["value"], json!(42));
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some("SID-1")),
        "default-context inspector hook must not consume unrelated pending awaits"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(900_199)),
        "default-context inspector hook must not use the old internal Runtime.enable id: {:?}",
        ctx.sent
    );
}
#[tokio::test]
async fn unrelated_pending_inspector_await_does_not_block_background_owner_context_lookup() {
    let mut ctx = TestContext::new();
    with_loaded_runtime_frontend_enabled_background_target_async(
        &mut ctx,
        "TID-active",
        "SID-active",
        "TID-background",
        "SID-background",
        "<script>globalThis.__backgroundDefaultProbe = 41;</script>",
    )
    .await;
    ctx.conn
        .register_pending_inspector_await(900_199, Some("SID-active"));
    ctx.sent.clear();

    let before = ctx
        .conn
        .runtime_default_execution_context_id_for_session_owner_async(Some("SID-background"))
        .await
        .expect("background default context lookup should not fail before evaluate");
    assert!(
        before.is_some_and(|context_id| context_id > 0),
        "a background target whose Runtime agent was enabled before navigation must receive the concrete default-context creation"
    );

    ctx.process_async(json!({
        "id": 17,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "globalThis.__backgroundDefaultProbe + 1",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 17);
    assert_eq!(response["result"]["result"]["value"], json!(42));

    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some("SID-active")),
        "the active owner's pending await must remain untouched by background materialization"
    );
    assert!(
        !ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some("SID-background")),
        "background default-context lookup should not create a pending await entry"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["id"] == json!(900_199)
        }),
        "background context lookup must not issue internal Runtime.enable: {:?}",
        ctx.sent
    );
}
