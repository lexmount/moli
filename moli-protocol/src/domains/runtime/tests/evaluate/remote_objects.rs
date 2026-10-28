// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

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
    ctx.with_fixture(|conn| conn.push_inactive_browser_context_fixture_for_test(other_context));

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
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });

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
    ctx.with_fixture(|conn| conn.push_inactive_browser_context_fixture_for_test(other_context));

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

    ctx.with_fixture(|conn| {
        assert!(
            conn.validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                &[child_object_id.clone(), returned_object_id.clone()],
            )
            .is_err(),
            "active owner should see inherited-group handles as belonging to the background target"
        )
    });

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
    ctx.with_fixture(|conn| { assert!(
        conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                &[child_object_id, returned_object_id],
            )
            .is_ok(),
        "releaseObjectGroup should remove handles whose group was inherited from the receiver object"
    ) });
}

#[tokio::test]
async fn background_runtime_evaluate_emits_runtime_observable_from_background_owner_without_activation()
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
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.browser_context
                .as_ref()
                .and_then(|browser_context| browser_context.active_target_id()),
            Some("TID-active"),
            "background Runtime.evaluate observable drain should not activate the target"
        )
    });
}

#[tokio::test]
async fn call_function_on_loaded_background_owner_without_activation() {
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
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.browser_context
                .as_ref()
                .and_then(|browser_context| browser_context.active_target_id()),
            Some("TID-active"),
            "background Runtime.callFunctionOn should not activate the target"
        )
    });

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
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });

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
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });

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
