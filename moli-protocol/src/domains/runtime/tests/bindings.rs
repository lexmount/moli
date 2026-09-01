use super::*;

#[tokio::test]
async fn add_binding_dispatches_to_inspector_and_emits_binding_called() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 319).await;

    ctx.process_async(json!({
        "id": 32,
        "method": "Runtime.addBinding",
        "params": { "name": "moliBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 32);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 320,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.moliBinding('payload-1'); 7"
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 320);
    assert_eq!(evaluate["result"]["result"]["type"], json!("number"));
    assert_eq!(evaluate["result"]["result"]["value"], json!(7));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Runtime.bindingCalled"))
        .cloned()
        .expect("binding call should emit Runtime.bindingCalled");
    assert_eq!(binding_called["params"]["name"], json!("moliBinding"));
    assert_eq!(binding_called["params"]["payload"], json!("payload-1"));
    assert!(
        binding_called["params"]["executionContextId"]
            .as_i64()
            .is_some_and(|id| id > 0),
        "default-world binding calls must report a real executionContextId: {binding_called:?}"
    );
}

#[tokio::test]
async fn runtime_disable_clears_stored_binding_definitions_when_enabled() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 31_940).await;

    ctx.process_async(json!({
        "id": 31_941,
        "method": "Runtime.addBinding",
        "params": { "name": "bindingClearedOnDisable" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 31_941);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| binding.name == "bindingClearedOnDisable")
    );

    ctx.process_async(json!({
        "id": 31_942,
        "method": "Runtime.disable"
    }))
    .await;
    let disable = take_response_by_id(&mut ctx, 31_942);
    assert_eq!(disable["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .is_empty(),
        "Runtime.disable should clear stored binding definitions when the Runtime agent was enabled"
    );
}

#[tokio::test]
async fn playwright_style_binding_call_from_await_promise_command_emits_before_response() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    {
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should exist");
        browser_context.set_active_target_id("TID-1");
        browser_context.attach_active_session("SID-1");
    }
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 31_910).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 31_911,
        "sessionId": "SID-1",
        "method": "Runtime.addBinding",
        "params": { "name": "__playwright__binding__" }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 31_911)["result"], json!({}));

    ctx.process_async(json!({
        "id": 31_912,
        "sessionId": "SID-1",
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
                (() => {
                    globalThis.__computeHost = {};
                    globalThis.__computeHost.compute = (a, b) => {
                        const seq = 1;
                        const payload = { name: 'compute', seq, serializedArgs: [a, b] };
                        globalThis.__computePromise = new Promise(resolve => {
                            globalThis.__resolveCompute = resolve;
                        });
                        globalThis.__computePromise.then(value => {
                            globalThis.__computeSettledValue = value;
                        });
                        globalThis.__playwright__binding__(JSON.stringify(payload));
                        return globalThis.__computePromise;
                    };
                    return globalThis.__computeHost;
                })()
            "#
        }
    }))
    .await;
    let host_object_id = take_response_by_id(&mut ctx, 31_912)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate should return the compute host objectId");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 31_913,
        "sessionId": "SID-1",
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": host_object_id,
            "functionDeclaration": "function() { return this.compute(9, 4); }",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;

    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(31_913)),
        "Playwright-style exposed binding command must remain pending until the client delivers the binding result: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some("SID-1")),
        "Playwright-style exposed binding await should be registered on the session owner"
    );
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "Playwright-style Runtime.bindingCalled before pending awaitPromise response",
        |message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["name"] == json!("__playwright__binding__")
        },
    )
    .await;
    let binding_called = ctx.take_first_matching(
        "Playwright-style Runtime.bindingCalled before pending awaitPromise response",
        |message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["name"] == json!("__playwright__binding__")
        },
    );
    let payload: serde_json::Value =
        serde_json::from_str(binding_called["params"]["payload"].as_str().unwrap())
            .expect("binding payload should be JSON");
    assert_eq!(payload["name"], json!("compute"));
    assert_eq!(payload["seq"], json!(1));
    assert_eq!(payload["serializedArgs"], json!([9, 4]));
    assert!(
        binding_called["params"]["executionContextId"]
            .as_i64()
            .is_some_and(|id| id > 0),
        "Playwright drops bindingCalled events that do not map to a live context: {binding_called:?}"
    );

    ctx.process_async(json!({
        "id": 31_914,
        "sessionId": "SID-1",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__resolveCompute(36); 'delivered'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 31_914)["result"]["result"]["value"],
        json!("delivered")
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some("SID-1"))
            || ctx
                .sent
                .iter()
                .any(|message| message["id"] == json!(31_913)),
        "deliver command should leave the exposed binding await pending or emit its response; sent={:?}",
        ctx.sent
    );
    ctx.process_async(json!({
        "id": 31_915,
        "sessionId": "SID-1",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__computeSettledValue",
            "returnByValue": true
        }
    }))
    .await;
    let settled = take_response_by_id(&mut ctx, 31_915);
    assert_eq!(
        settled["result"]["result"]["value"],
        json!(36),
        "compute promise should settle after deliver: {settled:?}"
    );
    let resolved = wait_for_response_by_id_async(&mut ctx, Some("SID-1"), 31_913).await;
    assert_eq!(resolved["sessionId"], json!("SID-1"));
    assert_eq!(resolved["result"]["result"]["value"], json!(36));
}

#[tokio::test]
async fn add_binding_can_complete_through_multi_phase_pending_command_task() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 31_900).await;

    let raw = json!({
        "id": 31_901,
        "method": "Runtime.addBinding",
        "params": { "name": "pendingBinding" }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Runtime.addBinding without executionContextId should start as a command task");
    let (mut messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;

    let response = messages
        .pop()
        .expect("pending Runtime.addBinding should produce a response");
    assert_eq!(response["id"], json!(31_901));
    assert_eq!(response["result"], json!({}));
    assert!(
        scheduler_events.is_empty(),
        "binding install should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| binding.name == "pendingBinding")
    );

    ctx.process_async(json!({
        "id": 31_902,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.pendingBinding('pending-payload'); 11"
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 31_902);
    assert_eq!(evaluate["result"]["result"]["value"], json!(11));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("pendingBinding")
            && message["params"]["payload"] == json!("pending-payload")
    }));
}

#[tokio::test]
async fn add_binding_during_document_navigation_persists_without_touching_retiring_page() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>retiring</body></html>").await;
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    assert!(
        ctx.conn
            .renderer_document_navigation_is_suspended_for_session_owner(Some("SID-1"))
    );

    ctx.process_async(json!({
        "id": 31_903,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "navigationCommitBinding" }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 31_903)["result"], json!({}));
    assert!(
        ctx.conn
            .navigation_load_inputs_for_session_owner(Some("SID-1"))
            .runtime_bindings
            .iter()
            .any(|binding| binding.name == "navigationCommitBinding"),
        "the next document must consume the binding persisted during navigation suspend"
    );
}

#[tokio::test]
async fn add_binding_persists_across_navigation() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 321).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 322,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "persistedBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 322);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 323,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 324,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.persistedBinding('after-nav'); 9"
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 324);
    assert_eq!(evaluate["result"]["result"]["type"], json!("number"));
    assert_eq!(evaluate["result"]["result"]["value"], json!(9));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Runtime.bindingCalled"))
        .cloned()
        .expect("binding should remain callable after navigation");
    assert_eq!(binding_called["params"]["name"], json!("persistedBinding"));
    assert_eq!(binding_called["params"]["payload"], json!("after-nav"));
    assert_eq!(binding_called["sessionId"], json!("SID-1"));
}

#[tokio::test]
async fn add_binding_rejects_execution_context_id_with_context_name() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let context_id = enable_runtime_and_take_execution_context_id_async(&mut ctx, 325).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 326,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "invalidBinding",
            "executionContextId": context_id,
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 326);
    assert_eq!(add_binding["error"]["code"], json!(-32602));
    assert_eq!(
        add_binding["error"]["message"],
        json!("executionContextName is mutually exclusive with executionContextId")
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| binding.name == "invalidBinding"),
        "invalid addBinding must not persist a protocol-side binding definition"
    );
}

#[tokio::test]
async fn remove_binding_deactivates_current_page_binding_and_removes_persisted_definition() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 500).await;

    ctx.process_async(json!({
        "id": 501,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "tempBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 501);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 502,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-1",
        "params": { "name": "tempBinding" }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 502);
    assert_eq!(remove_binding["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "tempBinding")
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 503,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.tempBinding('after-remove'); typeof globalThis.tempBinding" }
    }))
    .await;
    let removed = take_response_by_id(&mut ctx, 503);
    assert_eq!(removed["result"]["result"]["type"], json!("string"));
    assert_eq!(removed["result"]["result"]["value"], json!("function"));
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("tempBinding")
    }));
}
#[tokio::test]
async fn remove_binding_can_complete_through_multi_phase_pending_command_task() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 50_100).await;

    ctx.process_async(json!({
        "id": 50_101,
        "method": "Runtime.addBinding",
        "params": { "name": "pendingRemoveBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 50_101);
    assert_eq!(add_binding["result"], json!({}));

    let raw = json!({
        "id": 50_102,
        "method": "Runtime.removeBinding",
        "params": { "name": "pendingRemoveBinding" }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Runtime.removeBinding should start as a command task");
    let (mut messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;

    let response = messages
        .pop()
        .expect("pending Runtime.removeBinding should produce a response");
    assert_eq!(response["id"], json!(50_102));
    assert_eq!(response["result"], json!({}));
    assert!(
        scheduler_events.is_empty(),
        "binding removal should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "pendingRemoveBinding")
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 50_103,
        "method": "Runtime.evaluate",
        "params": { "expression": "globalThis.pendingRemoveBinding('after-remove'); typeof globalThis.pendingRemoveBinding" }
    }))
    .await;
    let removed = take_response_by_id(&mut ctx, 50_103);
    assert_eq!(removed["result"]["result"]["value"], json!("function"));
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("pendingRemoveBinding")
    }));
}
#[tokio::test]
async fn remove_binding_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 512,
        "method": "Runtime.removeBinding",
        "params": { "name": "tempBinding" }
    }))
    .await;
    ctx.expect_error(512, -31998, "BrowserContextNotLoaded");
}
#[tokio::test]
async fn add_binding_with_execution_context_name_targets_existing_isolated_world() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 325).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 326, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 327,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "utilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 327);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 328,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.utilityBinding"
        }
    }))
    .await;
    let default_world = take_response_by_id(&mut ctx, 328);
    assert_eq!(default_world["result"]["result"]["type"], json!("string"));
    assert_eq!(
        default_world["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 329,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.utilityBinding",
            "contextId": utility_context_id
        }
    }))
    .await;
    let utility_world = take_response_by_id(&mut ctx, 329);
    assert_eq!(utility_world["result"]["result"]["type"], json!("string"));
    assert_eq!(
        utility_world["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
        "id": 330,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.utilityBinding('scoped-payload'); 5",
            "contextId": utility_context_id
        }
    }))
    .await;
    let call = take_response_by_id(&mut ctx, 330);
    assert_eq!(call["result"]["result"]["type"], json!("number"));
    assert_eq!(call["result"]["result"]["value"], json!(5));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("utilityBinding")
        })
        .cloned()
        .expect("scoped binding should emit Runtime.bindingCalled");
    assert_eq!(binding_called["params"]["payload"], json!("scoped-payload"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
}
#[tokio::test]
async fn remove_binding_prevents_scoped_binding_replay_across_navigation() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 504).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 505, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 506,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "temporaryUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 506);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 507,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-1",
        "params": { "name": "temporaryUtilityBinding" }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 507);
    assert_eq!(remove_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 5071,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "",
            "worldName": "utility"
        }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 5071)["result"]["identifier"]
            .as_str()
            .is_some()
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 508,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.temporaryUtilityBinding('after-remove'); typeof globalThis.temporaryUtilityBinding",
            "contextId": utility_context_id
        }
    }))
    .await;
    let removed_in_current_world = take_response_by_id(&mut ctx, 508);
    assert_eq!(
        removed_in_current_world["result"]["result"]["value"],
        json!("function")
    );
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("temporaryUtilityBinding")
    }));

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 509,
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
        .expect("navigation should replay utility context id while Runtime is enabled");

    ctx.process_async(json!({
        "id": 511,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.temporaryUtilityBinding",
            "contextId": replayed_context_id
        }
    }))
    .await;
    let removed_after_nav = take_response_by_id(&mut ctx, 511);
    assert_eq!(
        removed_after_nav["result"]["result"]["value"],
        json!("undefined")
    );
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("temporaryUtilityBinding")
    }));
}
#[tokio::test]
async fn add_binding_without_runtime_enable_installs_current_main_world_and_persists_definition() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    ctx.process_async(json!({
        "id": 321,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "immediateBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 321);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 322,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.immediateBinding('before-nav'); 5"
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 322);
    assert_eq!(evaluate["result"]["result"]["type"], json!("number"));
    assert_eq!(evaluate["result"]["result"]["value"], json!(5));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("immediateBinding")
        })
        .cloned()
        .expect("main world should receive no-runtime addBinding immediately");
    assert_eq!(binding_called["params"]["payload"], json!("before-nav"));
    assert_eq!(binding_called["sessionId"], json!("SID-1"));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| binding.name == "immediateBinding"
                && binding.execution_context_name.is_none())
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "no-runtime addBinding should not force Runtime.enable surfaces: {:?}",
        ctx.sent
    );
}
#[tokio::test]
async fn add_binding_with_execution_context_id_targets_existing_isolated_world() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 331).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 332, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 333,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "utilityBindingById",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 333);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 334,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.utilityBindingById"
        }
    }))
    .await;
    let default_world = take_response_by_id(&mut ctx, 334);
    assert_eq!(default_world["result"]["result"]["type"], json!("string"));
    assert_eq!(
        default_world["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 335,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.utilityBindingById",
            "contextId": utility_context_id
        }
    }))
    .await;
    let utility_world = take_response_by_id(&mut ctx, 335);
    assert_eq!(utility_world["result"]["result"]["type"], json!("string"));
    assert_eq!(
        utility_world["result"]["result"]["value"],
        json!("function")
    );

    ctx.process_async(json!({
        "id": 336,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "utilityBindingById('scoped-by-id'); 7",
            "contextId": utility_context_id
        }
    }))
    .await;
    let call = take_response_by_id(&mut ctx, 336);
    assert_eq!(call["result"]["result"]["type"], json!("number"));
    assert_eq!(call["result"]["result"]["value"], json!(7));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("utilityBindingById")
        })
        .cloned()
        .expect("context-id scoped binding should emit Runtime.bindingCalled");
    assert_eq!(binding_called["params"]["payload"], json!("scoped-by-id"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "utilityBindingById"),
        "executionContextId-scoped binding should not persist on the browser context"
    );
}
#[tokio::test]
async fn add_binding_execution_context_id_completes_through_pending_command_task() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 50_200).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 50_201, "pending-utility").await;
    ctx.sent.clear();

    let raw = json!({
        "id": 50_202,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "pendingUtilityBindingById",
            "executionContextId": utility_context_id
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("Runtime.addBinding with executionContextId should start as a command task");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    let add_binding = messages
        .iter()
        .find(|message| message["id"] == json!(50_202))
        .expect("pending Runtime.addBinding by id should produce a response");
    assert_eq!(
        add_binding["result"],
        json!({}),
        "pending addBinding messages: {messages:?}"
    );
    assert!(
        scheduler_events.is_empty(),
        "context-id binding install should not enqueue scheduler work: {scheduler_events:?}"
    );

    ctx.process_async(json!({
        "id": 50_203,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.pendingUtilityBindingById"
        }
    }))
    .await;
    let default_world = take_response_by_id(&mut ctx, 50_203);
    assert_eq!(
        default_world["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 50_204,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.pendingUtilityBindingById",
            "contextId": utility_context_id
        }
    }))
    .await;
    let utility_world = take_response_by_id(&mut ctx, 50_204);
    assert_eq!(
        utility_world["result"]["result"]["value"],
        json!("function")
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "pendingUtilityBindingById"),
        "executionContextId-scoped binding should not persist on the browser context"
    );
}
#[tokio::test]
async fn patchright_style_utility_world_init_and_binding_work_without_runtime_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 346,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_patchright_preload = 'utility-ready';",
            "worldName": "utility"
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 346);
    assert_eq!(preload["result"]["identifier"], json!("1"));

    ctx.process_async(json!({
        "id": 347,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "utilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 347);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| { message["method"] == json!("Runtime.executionContextCreated") }),
        "Patchright-style setup should not require Runtime.enable side effects"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 348,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body><script>document.body.textContent = String(globalThis.__lm_patchright_preload || typeof globalThis.utilityBinding);</script></body>"
        }
    }))
    .await;
    let _ = ctx.take_all();

    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "navigation without Runtime.enable should not emit runtime context events"
    );
    ctx.sent.clear();

    let utility_context_id = create_isolated_world_async(&mut ctx, 350, "utility").await;

    ctx.process_async(json!({
        "id": 351,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": utility_context_id,
            "expression": "utilityBinding('payload-no-runtime'); JSON.stringify([typeof globalThis.utilityBinding, globalThis.__lm_patchright_preload])"
        }
    }))
    .await;
    let utility_world = take_response_by_id(&mut ctx, 351);
    assert_eq!(
        utility_world["result"]["result"]["value"],
        json!("[\"function\",\"utility-ready\"]")
    );

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("utilityBinding")
        })
        .cloned()
        .expect("utility binding should remain callable without Runtime.enable");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("payload-no-runtime")
    );
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
}
#[tokio::test]
async fn patchright_style_existing_utility_world_binding_install_persists_without_runtime_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.sent.clear();

    let initial_utility_context_id = create_isolated_world_async(&mut ctx, 352, "utility").await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| { message["method"] == json!("Runtime.executionContextCreated") }),
        "createIsolatedWorld without Runtime.enable should stay silent"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 353,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_existing_utility_preload = 'ready-now';",
            "worldName": "utility",
            "runImmediately": true
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 353);
    assert_eq!(preload["result"]["identifier"], json!("1"));

    ctx.process_async(json!({
        "id": 354,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "existingUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 354);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "installing utility-world preload/binding should not backdoor Runtime.enable events"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 355,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": initial_utility_context_id,
            "expression": "existingUtilityBinding('payload-existing'); JSON.stringify([typeof globalThis.existingUtilityBinding, globalThis.__lm_existing_utility_preload])"
        }
    }))
    .await;
    let initial_world = take_response_by_id(&mut ctx, 355);
    assert_eq!(
        initial_world["result"]["result"]["value"],
        json!("[\"function\",\"ready-now\"]")
    );
    let initial_binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("existingUtilityBinding")
        })
        .cloned()
        .expect("binding should install into already-existing utility world");
    assert_eq!(
        initial_binding_called["params"]["executionContextId"],
        json!(initial_utility_context_id)
    );
    assert_eq!(
        initial_binding_called["params"]["payload"],
        json!("payload-existing")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 356,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let _ = ctx.take_all();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "navigation without Runtime.enable should still avoid runtime context events"
    );
    ctx.sent.clear();

    let replayed_utility_context_id = create_isolated_world_async(&mut ctx, 357, "utility").await;
    ctx.process_async(json!({
        "id": 358,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": replayed_utility_context_id,
            "expression": "existingUtilityBinding('payload-after-nav'); JSON.stringify([typeof globalThis.existingUtilityBinding, globalThis.__lm_existing_utility_preload])"
        }
    }))
    .await;
    let replayed_world = take_response_by_id(&mut ctx, 358);
    assert_eq!(
        replayed_world["result"]["result"]["value"],
        json!("[\"function\",\"ready-now\"]")
    );
    let replayed_binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("existingUtilityBinding")
        })
        .cloned()
        .expect("binding should persist into recreated utility world");
    assert_eq!(
        replayed_binding_called["params"]["executionContextId"],
        json!(replayed_utility_context_id)
    );
    assert_eq!(
        replayed_binding_called["params"]["payload"],
        json!("payload-after-nav")
    );
}
#[tokio::test]
async fn patchright_style_pre_document_add_binding_persists_until_navigation() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(crate::conn::BrowserContext::new("BID-1".into()));
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    ctx.process_async(json!({
        "id": 360,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "preDocumentBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 360);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| {
                binding.name == "preDocumentBinding" && binding.execution_context_name.is_none()
            }),
        "pre-document binding should persist on the browser context"
    );

    ctx.process_async(json!({
        "id": 361,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let _ = ctx.take_all();

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 362).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 363,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "preDocumentBinding('after-nav'); typeof globalThis.preDocumentBinding"
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 363);
    assert_eq!(evaluation["result"]["result"]["value"], json!("function"));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("preDocumentBinding")
        })
        .cloned()
        .expect("binding should remain callable after navigation");
    assert_eq!(binding_called["params"]["payload"], json!("after-nav"));
}
#[tokio::test]
async fn pre_document_add_binding_completes_through_command_task_without_live_page() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(crate::conn::BrowserContext::new("BID-1".into()));
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    let raw = json!({
        "id": 36_001,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "preDocumentTaskBinding" }
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(36_001));
    assert_eq!(messages[0]["sessionId"], json!("SID-1"));
    assert_eq!(messages[0]["result"], json!({}));
    assert!(
        scheduler_events.is_empty(),
        "pre-document binding should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| {
                binding.name == "preDocumentTaskBinding" && binding.execution_context_name.is_none()
            }),
        "pre-document command task should persist the binding"
    );
}
#[tokio::test]
async fn run_if_waiting_for_debugger_completes_through_command_task() {
    let mut ctx = TestContext::new();
    let raw = json!({
        "id": 36_050,
        "method": "Runtime.runIfWaitingForDebugger"
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;

    assert_eq!(messages, vec![json!({"id": 36_050, "result": {}})]);
    assert!(
        scheduler_events.is_empty(),
        "Runtime.runIfWaitingForDebugger should not enqueue scheduler work: {scheduler_events:?}"
    );
}
#[tokio::test]
async fn patchright_style_pre_document_remove_binding_succeeds_before_navigation() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(crate::conn::BrowserContext::new("BID-1".into()));
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    ctx.process_async(json!({
        "id": 364,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "temporaryPreDocumentBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 364);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 365,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-1",
        "params": { "name": "temporaryPreDocumentBinding" }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 365);
    assert_eq!(remove_binding["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "temporaryPreDocumentBinding"),
        "pre-document removeBinding should clear persisted browser-context state"
    );

    ctx.process_async(json!({
        "id": 366,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body><script>globalThis.__lm_removed_pre_document_binding_kind = typeof globalThis.temporaryPreDocumentBinding; if (typeof globalThis.temporaryPreDocumentBinding === 'function') globalThis.temporaryPreDocumentBinding('unexpected');</script></body>"
        }
    }))
    .await;
    let _ = ctx.take_all();

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 367).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 368,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__lm_removed_pre_document_binding_kind"
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 368);
    assert_eq!(evaluation["result"]["result"]["value"], json!("undefined"));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("temporaryPreDocumentBinding")
        }),
        "removed pre-document binding should not replay into the first navigation"
    );
}
#[tokio::test]
async fn pre_document_remove_binding_completes_through_command_task_without_live_page() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(crate::conn::BrowserContext::new("BID-1".into()));
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");

    let add_raw = json!({
        "id": 36_101,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "preDocumentRemoveTaskBinding" }
    })
    .to_string();
    let add_step = ctx.conn.start_command_dispatch(&add_raw);
    let (add_messages, _) = complete_command_task_step_for_test(&mut ctx, add_step).await;
    assert_eq!(add_messages[0]["result"], json!({}));

    let remove_raw = json!({
        "id": 36_102,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-1",
        "params": { "name": "preDocumentRemoveTaskBinding" }
    })
    .to_string();
    let remove_step = ctx.conn.start_command_dispatch(&remove_raw);
    let (remove_messages, scheduler_events) =
        complete_command_task_step_for_test(&mut ctx, remove_step).await;

    assert_eq!(remove_messages.len(), 1);
    assert_eq!(remove_messages[0]["id"], json!(36_102));
    assert_eq!(remove_messages[0]["sessionId"], json!("SID-1"));
    assert_eq!(remove_messages[0]["result"], json!({}));
    assert!(
        scheduler_events.is_empty(),
        "pre-document binding removal should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "preDocumentRemoveTaskBinding"),
        "pre-document command task should remove persisted binding state"
    );
}
