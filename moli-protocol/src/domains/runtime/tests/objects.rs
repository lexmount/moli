use super::*;

#[tokio::test]
async fn child_default_context_activity_stays_silent_before_runtime_enable() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><iframe srcdoc='<body>child-before-enable</body>'></iframe></body></html>",
    )
    .await;
    let bc = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35921,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.querySelector('iframe').srcdoc = '<body>child-updated-before-enable</body>'; 1"
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 35921);
    assert_eq!(result["result"]["result"]["value"], json!(1));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "child default context activity should stay silent before Runtime.enable"
    );
}
#[tokio::test]
async fn release_object_rejects_cross_owner_and_drops_owner_registry_on_success() {
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
        "id": 525,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 525);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 526,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "({ owner: 'background' })"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 526);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return an object handle: {response:?}"))
        .to_owned();
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.evaluate should not activate the target"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                std::slice::from_ref(&object_id),
            )
            .is_err(),
        "active owner should see the background handle as belonging to another target"
    );

    ctx.process_async(json!({
        "id": 527,
        "method": "Runtime.releaseObject",
        "sessionId": "SID-active",
        "params": {
            "objectId": object_id
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 527);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );

    ctx.process_async(json!({
        "id": 528,
        "method": "Runtime.releaseObject",
        "sessionId": "SID-background",
        "params": {
            "objectId": object_id
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 528);
    assert_eq!(response["result"], json!({}));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.releaseObject should not activate the target"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                std::slice::from_ref(&object_id),
            )
            .is_ok(),
        "successful owner release should remove the handle from the owner registry"
    );
}
#[tokio::test]
async fn release_object_group_drops_owner_registry_on_success() {
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
        "id": 525,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 525);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 526,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "({ owner: 'background-group' })",
            "objectGroup": "background-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 526);
    let object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return an object handle: {response:?}"))
        .to_owned();
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                std::slice::from_ref(&object_id),
            )
            .is_err(),
        "active owner should see the grouped background handle as belonging to another target"
    );

    ctx.process_async(json!({
        "id": 527,
        "method": "Runtime.releaseObjectGroup",
        "sessionId": "SID-background",
        "params": {
            "objectGroup": "background-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 527);
    assert_eq!(response["result"], json!({}));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.releaseObjectGroup should not activate the target"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                std::slice::from_ref(&object_id),
            )
            .is_ok(),
        "successful owner releaseObjectGroup should remove grouped handles from the owner registry"
    );
}
#[tokio::test]
async fn release_object_group_drops_await_promise_inherited_result_handle() {
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
            "expression": "Promise.resolve({ owner: 'background-await-group' })",
            "objectGroup": "background-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 534);
    let promise_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return a promise handle: {response:?}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 535,
        "method": "Runtime.awaitPromise",
        "sessionId": "SID-background",
        "params": {
            "promiseObjectId": promise_object_id
        }
    }))
    .await;
    let response = wait_for_response_by_id_async(&mut ctx, Some("SID-background"), 535).await;
    let resolved_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.awaitPromise should return an object handle: {response:?}")
        })
        .to_owned();
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                std::slice::from_ref(&resolved_object_id),
            )
            .is_err(),
        "active owner should see the inherited-group await result as belonging to the background target"
    );

    ctx.process_async(json!({
        "id": 536,
        "method": "Runtime.releaseObjectGroup",
        "sessionId": "SID-background",
        "params": {
            "objectGroup": "background-group"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 536);
    assert_eq!(response["result"], json!({}));
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                std::slice::from_ref(&resolved_object_id),
            )
            .is_ok(),
        "releaseObjectGroup should remove awaitPromise result handles whose group was inherited from the promise"
    );
}
#[tokio::test]
async fn await_promise_routes_to_loaded_background_owner_without_activation() {
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
        "id": 529,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 529);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 530,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "new Promise(resolve => setTimeout(() => resolve({ owner: 'background-await' }), 0))"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 530);
    let promise_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return a promise handle: {response:?}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 531,
        "method": "Runtime.awaitPromise",
        "sessionId": "SID-background",
        "params": {
            "promiseObjectId": promise_object_id
        }
    }))
    .await;
    let response = wait_for_response_by_id_async(&mut ctx, Some("SID-background"), 531).await;
    let resolved_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("Runtime.awaitPromise should return an object handle: {response:?}")
        })
        .to_owned();
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.awaitPromise should not activate the target"
    );
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                Some("SID-active"),
                std::slice::from_ref(&resolved_object_id),
            )
            .is_err(),
        "active owner should see the Runtime.awaitPromise result handle as belonging to the background target"
    );
}
#[tokio::test]
async fn await_promise_rejects_promise_object_id_known_to_different_target_owner() {
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
        "id": 532,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 532);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 533,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "Promise.resolve({ owner: 'background-promise' })"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 533);
    let promise_object_id = response["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return a promise handle: {response:?}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 534,
        "method": "Runtime.awaitPromise",
        "sessionId": "SID-active",
        "params": {
            "promiseObjectId": promise_object_id
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 534);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Cannot find object with given id")
    );
}
#[tokio::test]
async fn loaded_background_runtime_binding_routes_to_owner_without_activation() {
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
        "id": 529,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 529);
    assert_eq!(response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 530,
        "method": "Runtime.addBinding",
        "sessionId": "SID-background",
        "params": { "name": "backgroundBinding" }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 530);
    assert_eq!(response["result"], json!({}));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.addBinding should not activate the target"
    );
    assert!(
        ctx.conn
            .target_devtools_session_state_for_session(Some("SID-background"))
            .expect("background DevTools session state should be readable")
            .runtime_bindings
            .iter()
            .any(|binding| binding.name == "backgroundBinding"),
        "background addBinding should persist on the background owner"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 531,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "backgroundBinding('from-background-owner'); typeof globalThis.backgroundBinding",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 531);
    assert_eq!(response["result"]["result"]["value"], json!("function"));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("backgroundBinding")
        })
        .cloned()
        .expect("background binding should fire on the background owner page");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("from-background-owner")
    );

    ctx.process_async(json!({
        "id": 532,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-background",
        "params": { "name": "backgroundBinding" }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 532);
    assert_eq!(response["result"], json!({}));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Runtime.removeBinding should not activate the target"
    );
    assert!(
        ctx.conn
            .target_devtools_session_state_for_session(Some("SID-background"))
            .is_none_or(|state| state
                .runtime_bindings
                .iter()
                .all(|binding| binding.name != "backgroundBinding")),
        "background removeBinding should clear persisted session state"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 533,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "globalThis.backgroundBinding('after-remove'); typeof globalThis.backgroundBinding",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 533);
    assert_eq!(response["result"]["result"]["value"], json!("function"));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("backgroundBinding")
        }),
        "background removeBinding should leave the existing function inert"
    );
}
#[tokio::test]
async fn runtime_disable_advances_background_owner_observable_cursor_without_activation() {
    let mut ctx = TestContext::new();
    with_loaded_runtime_frontend_enabled_background_target_async(
        &mut ctx,
        "TID-active",
        "SID-active",
        "TID-background",
        "SID-background",
        "<script>console.log('owner-disable')</script>",
    )
    .await;

    let queue_console_entries = {
        let runtime_slot = ctx
            .conn
            .runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot should exist");
        runtime_slot.ingest_owner_page_observable_output_updates();
        runtime_slot
            .observable_output_queue_snapshot()
            .expect("background observable queue should exist")
            .observable_output_items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    moli_core::page::ScriptObservableOutputItem::ConsoleMessage(_)
                )
            })
            .count()
    };
    assert_eq!(
        queue_console_entries, 1,
        "background owner observable queue should have one console message"
    );

    ctx.process_async(json!({
        "id": 529,
        "method": "Runtime.disable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 529);
    assert_eq!(response["result"], json!({}));
    let browser_context = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist");
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Runtime.disable should not activate the target"
    );
    assert_eq!(
        browser_context
            .background_target("TID-background")
            .expect("background target must exist")
            .owner_state
            .runtime_observable_state
            .emitted_console_entries(),
        queue_console_entries,
        "background Runtime.disable should advance the background owner observable cursor"
    );
}
#[tokio::test]
async fn runtime_disable_clears_background_child_default_context_emission_cursor_without_activation()
 {
    let mut ctx = TestContext::new();
    let html = "<iframe srcdoc=\"<body>background child</body>\"></iframe>";
    let background_target = crate::conn::PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        format!("data:text/html,{html}"),
    );
    let mut browser_context = crate::conn::BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.insert_page_target_host(background_target);
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        &format!("data:text/html,{html}"),
        Some("SID-background"),
    )
    .await;

    ctx.process_async(json!({
        "id": 534,
        "method": "Page.getFrameTree",
        "sessionId": "SID-background"
    }))
    .await;
    let child_frame_id = take_response_by_id(&mut ctx, 534)["result"]["frameTree"]["childFrames"]
        [0]["frame"]["id"]
        .as_str()
        .expect("background child frame id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 535,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let first_enable = take_response_by_id(&mut ctx, 535);
    assert_eq!(first_enable["result"], json!({}));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "first background child default context",
        |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        },
    )
    .await;
    let first_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("first background child default context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 536,
        "method": "Runtime.disable",
        "sessionId": "SID-background"
    }))
    .await;
    let disable = take_response_by_id(&mut ctx, 536);
    assert_eq!(disable["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 537,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let second_enable = take_response_by_id(&mut ctx, 537);
    assert_eq!(second_enable["result"], json!({}));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "replayed background child default context after Runtime.disable",
        |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        },
    )
    .await;
    let replayed_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("Runtime.enable should replay background child default context after disable");
    assert_eq!(replayed_context_id, first_context_id);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some("TID-active"),
        "background Runtime.disable/enable must not activate the target"
    );
}
