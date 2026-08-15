use super::*;

async fn wait_for_child_default_execution_context_id(
    ctx: &mut TestContext,
    child_frame_id: &str,
    description: &str,
) -> i64 {
    let expected_frame_id = child_frame_id.to_owned();
    wait_until_scheduler_message(ctx, description, move |message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
            && message["params"]["context"]["auxData"]["frameId"] == json!(expected_frame_id)
    })
    .await;
    ctx.sent
        .iter()
        .rev()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("scheduler wake must retain the child default execution context event")
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_replays_child_frame_isolated_world_with_child_frame_id() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-frame</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 405).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;

    ctx.process_async(json!({
        "id": 406,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-child"
        }
    }))
    .await;
    let context_id = take_response_by_id(&mut ctx, 406)["result"]["executionContextId"]
        .as_i64()
        .expect("child isolated execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 407,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 407);
    assert_eq!(response["result"], json!({}));

    let created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
        })
        .cloned()
        .expect("Runtime.enable should replay child isolated world context");
    assert_eq!(created["params"]["context"]["id"], json!(context_id));
    assert_eq!(
        created["params"]["context"]["auxData"]["frameId"],
        child_frame_id
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_replays_child_frame_default_context_with_child_frame_id() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-default-frame</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4070).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4071,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 4071);
    assert_eq!(response["result"], json!({}));

    let created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("Runtime.enable should replay child default execution context");
    let context_id = created["params"]["context"]["id"]
        .as_i64()
        .expect("child default execution context id");
    assert_eq!(
        created["params"]["context"]["auxData"]["type"],
        json!("default")
    );

    ctx.process_async(json!({
        "id": 4072,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": context_id,
            "expression": "document.body.textContent.trim()"
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 4072);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-default-frame")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn document_start_viewport_override_preserves_child_inner_viewport() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<iframe style='width:300px;height:65px' srcdoc=\"<body>child viewport</body>\"></iframe>",
            Some("SID-1"),
        )
        .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4073).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 4074,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4074);
    let child_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4075,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "[innerWidth, Function.prototype.toString.call(Object.getOwnPropertyDescriptor(globalThis, 'innerWidth').get).includes('[native code]')].join('|')",
            "returnByValue": true
        }
    }))
    .await;
    let top_result = take_response_by_id(&mut ctx, 4075);
    assert_eq!(top_result["result"]["result"]["value"], json!("1920|true"));

    ctx.process_async(json!({
        "id": 4076,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "[innerWidth, innerHeight].join('|')",
            "returnByValue": true
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 4076);

    assert_eq!(result["result"]["result"]["value"], json!("300|65"));

    ctx.process_async(json!({
        "id": 4077,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "[innerWidth, innerHeight].join('|')",
            "returnByValue": true
        }
    }))
    .await;
    let restored_top_result = take_response_by_id(&mut ctx, 4077);
    assert_eq!(
        restored_top_result["result"]["result"]["value"],
        json!("1920|1080")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_runtime_enable_does_not_replay_child_default_context_twice() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-default-frame</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 40710).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40711,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let first_response = take_response_by_id(&mut ctx, 40711);
    assert_eq!(first_response["result"], json!({}));
    let first_child_default_count = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .count();
    assert_eq!(
        first_child_default_count, 1,
        "first Runtime.enable should emit one child default context"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40712,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let second_response = take_response_by_id(&mut ctx, 40712);
    assert_eq!(second_response["result"], json!({}));
    let second_child_default_count = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .count();
    assert_eq!(
        second_child_default_count, 0,
        "repeated Runtime.enable should not replay an already-emitted child default context"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_replays_child_default_context_per_auxiliary_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>aux-child-default</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 40715).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40716,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let primary_response = take_response_by_id(&mut ctx, 40716);
    assert_eq!(primary_response["result"], json!({}));
    let primary_child_default_count = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .count();
    assert_eq!(
        primary_child_default_count, 1,
        "primary Runtime.enable should emit the child default context once"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40717,
        "method": "Runtime.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    let auxiliary_response = take_response_by_id(&mut ctx, 40717);
    assert_eq!(auxiliary_response["result"], json!({}));
    let auxiliary_child_default_count = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-aux")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .count();
    assert_eq!(
        auxiliary_child_default_count, 1,
        "auxiliary Runtime.enable should have its own child default replay cursor"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40718,
        "method": "Runtime.disable",
        "sessionId": "SID-1"
    }))
    .await;
    let primary_disable = take_response_by_id(&mut ctx, 40718);
    assert_eq!(primary_disable["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40719,
        "method": "Runtime.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    let auxiliary_enable_again = take_response_by_id(&mut ctx, 40719);
    assert_eq!(auxiliary_enable_again["result"], json!({}));
    let repeated_auxiliary_child_default_count = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-aux")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .count();
    assert_eq!(
        repeated_auxiliary_child_default_count, 0,
        "disabling primary Runtime must not clear auxiliary child default replay cursor"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_replays_live_child_default_context_after_disable() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-default-frame</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 40721).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.process_async(json!({
        "id": 40722,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 40722);
    let first_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("first child default execution context id");

    ctx.process_async(json!({
        "id": 40723,
        "method": "Runtime.disable",
        "sessionId": "SID-1"
    }))
    .await;
    let disable = take_response_by_id(&mut ctx, 40723);
    assert_eq!(disable["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40724,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let enable_again = take_response_by_id(&mut ctx, 40724);
    assert_eq!(enable_again["result"], json!({}));
    let replayed_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("Runtime.enable should replay live child default execution context");
    assert_eq!(replayed_context_id, first_context_id);
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_child_default_execution_context_returns_object_id() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-default-object</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4073).await;
    ctx.process_async(json!({
        "id": 4074,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4074);
    let child_default_context_id = wait_for_child_default_execution_context_id(
        &mut ctx,
        &child_frame_id,
        "child default context used by Runtime.evaluate",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4075,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "({ tag: document.body.textContent.trim(), sameWindow: window === globalThis })"
        }
    })).await;
    let eval_response = take_response_by_id(&mut ctx, 4075);
    let object_id = eval_response["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            panic!("child default Runtime.evaluate should return objectId: {eval_response}")
        });

    ctx.process_async(json!({
        "id": 4076,
        "method": "Runtime.getProperties",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id,
            "ownProperties": true
        }
    }))
    .await;
    let props = take_response_by_id(&mut ctx, 4076)["result"]["result"]
        .as_array()
        .cloned()
        .expect("Runtime.getProperties result");
    assert!(props.iter().any(|prop| {
        prop["name"] == json!("tag") && prop["value"]["value"] == json!("child-default-object")
    }));
    assert!(props.iter().any(|prop| {
        prop["name"] == json!("sameWindow") && prop["value"]["value"] == json!(true)
    }));
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_with_child_default_execution_context_id_installs_child_binding() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-default-binding</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4077).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.process_async(json!({
        "id": 4078,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4078);
    let child_default_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    wait_until_message(
        &mut ctx,
        "SID-1",
        "network child frame navigated before body evaluation",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
        },
    )
    .await;
    ctx.sent.clear();

    let raw = json!({
        "id": 4079,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "childDefaultBinding",
            "executionContextId": child_default_context_id
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-default Runtime.addBinding should start as a command task");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    let binding_result = messages
        .iter()
        .find(|message| message["id"] == json!(4079))
        .expect("child-default Runtime.addBinding should produce a response");
    assert_eq!(binding_result["result"], json!({}));
    assert!(
        scheduler_events.is_empty(),
        "child-default binding install should not enqueue scheduler work: {scheduler_events:?}"
    );

    ctx.process_async(json!({
        "id": 4080,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "globalThis.childDefaultBinding('child-default'); 7"
        }
    }))
    .await;
    let eval_result = take_response_by_id(&mut ctx, 4080);
    assert_eq!(eval_result["result"]["result"]["value"], json!(7));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("childDefaultBinding")
        })
        .cloned()
        .expect("child default binding should emit Runtime.bindingCalled");
    assert_eq!(binding_called["params"]["payload"], json!("child-default"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(child_default_context_id)
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_before_navigation_keeps_child_default_binding_context_id() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 40801,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "childDefaultBindingBeforeNavigate"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 40801);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 40802,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe srcdoc=\"<body>child-default-before-nav</body>\"></iframe>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 40802);
    ctx.sent.clear();

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 40803).await;
    ctx.process_async(json!({
        "id": 40804,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 40804);
    let child_default_context_id = wait_for_child_default_execution_context_id(
        &mut ctx,
        &child_frame_id,
        "child default context after Runtime.enable",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40805,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "globalThis.childDefaultBindingBeforeNavigate('child-before-nav'); 1"
        }
    }))
    .await;
    let eval = take_response_by_id(&mut ctx, 40805);
    assert_eq!(eval["result"]["result"]["value"], json!(1));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("childDefaultBindingBeforeNavigate")
        })
        .cloned()
        .expect("child default binding should emit Runtime.bindingCalled");
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(child_default_context_id)
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_call_function_on_in_child_default_execution_context_uses_child_scope() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-default-call</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4081).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.process_async(json!({
        "id": 4082,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4082);
    let child_default_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4083,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "functionDeclaration": "() => document.body.textContent.trim()",
            "executionContextId": child_default_context_id
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 4083);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-default-call")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_call_function_on_child_default_object_uses_child_window_eval_scope() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-object-eval</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4084).await;
    ctx.process_async(json!({
        "id": 4085,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4085);
    let child_default_context_id = wait_for_child_default_execution_context_id(
        &mut ctx,
        &child_frame_id,
        "child default context for object evaluation",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4086,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "({ global: globalThis, evaluate(expression) { return this.global.eval(expression); } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4086)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("child default utility object id");

    ctx.process_async(json!({
        "id": 4087,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function(expression) { return this.evaluate(expression); }",
            "arguments": [{ "value": "document.body.textContent.trim()" }]
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 4087);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-object-eval")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_call_function_on_child_default_utility_object_with_local_await_uses_child_scope() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-await-eval</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4088).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.process_async(json!({
        "id": 4089,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4089);
    let child_default_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4090,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "({ global: globalThis, evaluate(isFunction, returnByValue, expression, argCount, ...args) { let result = this.global.eval(expression); if (isFunction === true) { result = result(...args.slice(0, argCount)); } else if (isFunction !== false && typeof result === 'function') { result = result(...args.slice(0, argCount)); } return returnByValue ? result : result; } })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4090)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("child default utility object id");

    ctx.process_async(json!({
        "id": 4091,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                { "value": false },
                { "value": true },
                { "value": "document.body.textContent.trim()" },
                { "value": 0 }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 4091);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-await-eval")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_call_function_on_child_default_playwright_style_utility_script_uses_child_scope() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-playwright-eval</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4092).await;
    ctx.process_async(json!({
        "id": 4093,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4093);
    let child_default_context_id = wait_for_child_default_execution_context_id(
        &mut ctx,
        &child_frame_id,
        "child default context used by the Playwright-style utility script",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4094,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "(() => { const module = { exports: {} }; class UtilityScript { constructor(global, isUnderTest) { this.global = global; this.isUnderTest = isUnderTest; } evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? result : result; } } module.exports.UtilityScript = () => UtilityScript; return new (module.exports.UtilityScript())(globalThis, false); })()"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4094)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("playwright-style utility object id");

    ctx.process_async(json!({
        "id": 4095,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                { "value": {} },
                { "value": true },
                { "value": "document.body.textContent.trim()" },
                { "value": 1 },
                { "value": null }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 4095);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-playwright-eval")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_call_function_on_network_child_default_playwright_style_utility_script_uses_child_scope()
 {
    async fn parent() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-network-playwright-eval</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(parent))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        &format!("http://{addr}/parent"),
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4096).await;
    ctx.process_async(json!({
        "id": 4097,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 4097);
    wait_for_child_frame_navigated_url(&mut ctx, &child_frame_id, &format!("http://{addr}/child"))
        .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "network child default execution context",
        |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        },
    )
    .await;
    let child_default_context_id = ctx
        .sent
        .iter()
        .rev()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4098,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "(() => { const module = { exports: {} }; class UtilityScript { constructor(global, isUnderTest) { this.global = global; this.isUnderTest = isUnderTest; } evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? result : result; } } module.exports.UtilityScript = () => UtilityScript; return new (module.exports.UtilityScript())(globalThis, false); })()"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4098)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("playwright-style utility object id");

    ctx.process_async(json!({
        "id": 4099,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": object_id },
                { "value": {} },
                { "value": true },
                { "value": "document.body.textContent.trim()" },
                { "value": 1 },
                { "value": null }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 4099);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("child-network-playwright-eval")
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_default_world_replays_into_runtime_materialized_child_frame() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>top</body>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 421,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "childDefaultBinding"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 421);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 422,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
                const iframe = document.createElement('iframe');
                iframe.id = 'child-default-binding';
                iframe.srcdoc = '<body>binding-child</body>';
                document.body.appendChild(iframe);
                return true;
            })()"#
        }
    }))
    .await;
    let materialize = take_response_by_id(&mut ctx, 422);
    assert_eq!(materialize["result"]["result"]["value"], json!(true));
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4221).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;

    ctx.process_async(json!({
        "id": 423,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof document.getElementById('child-default-binding').contentWindow.childDefaultBinding"
        }
    })).await;
    let binding_type = take_response_by_id(&mut ctx, 423);
    assert_eq!(binding_type["result"]["result"]["value"], json!("function"));

    ctx.process_async(json!({
        "id": 424,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('child-default-binding').contentWindow.childDefaultBinding('child-default-payload'); 7"
        }
    })).await;
    let call = take_response_by_id(&mut ctx, 424);
    assert_eq!(call["result"]["result"]["value"], json!(7));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("childDefaultBinding")
        })
        .cloned()
        .expect("default-world child binding should emit Runtime.bindingCalled");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("child-default-payload")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_child_navigation_emits_child_frame_navigation_and_lifecycle_events() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .devtools_session_state
        .page_session_state
        .page_lifecycle_events = true;
    ctx.process_async(json!({
        "id": 522,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='child' name='child-frame' srcdoc=\"<body>initial</body>\"></iframe>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 522);
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 523).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 524,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('child').srcdoc = '<body>runtime-child</body>'"
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 524);
    assert!(
        response.get("error").is_none(),
        "runtime evaluate should not fail: {response:?}"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        }),
        "existing child frame should not re-emit Page.frameAttached on re-navigation; sent={:?}",
        ctx.sent
    );
    assert_child_frame_navigation_completion(&mut ctx, &child_frame_id, Some("child-frame"), None)
        .await;
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_timeout_child_navigation_emits_child_frame_navigation_and_lifecycle_events()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .devtools_session_state
        .page_session_state
        .page_lifecycle_events = true;
    ctx.process_async(json!({
        "id": 525,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='child' name='child-frame' srcdoc=\"<body>initial</body>\"></iframe>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 525);
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 526).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 527,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "setTimeout(() => { document.getElementById('child').srcdoc = '<body>timer-child</body>'; }, 0)"
        }
    })).await;

    let response = take_response_by_id(&mut ctx, 527);
    assert!(
        response.get("error").is_none(),
        "runtime evaluate should not fail: {response:?}"
    );
    assert_child_frame_navigation_completion(&mut ctx, &child_frame_id, Some("child-frame"), None)
        .await;
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_child_location_navigation_emits_child_frame_navigation_and_lifecycle_events()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .devtools_session_state
        .page_session_state
        .page_lifecycle_events = true;
    ctx.process_async(json!({
        "id": 528,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='child' name='child-frame' srcdoc=\"<body>initial</body>\"></iframe>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 528);
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 529).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;
    ctx.sent.clear();

    let child_url = "data:text/html,<body>location-child</body>";
    ctx.process_async(json!({
        "id": 530,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(
                "document.getElementById('child').contentWindow.location.href = {}",
                serde_json::to_string(child_url).expect("child url should encode")
            )
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 530);
    assert!(
        response.get("error").is_none(),
        "runtime evaluate should not fail: {response:?}"
    );
    assert_child_frame_navigation_completion(
        &mut ctx,
        &child_frame_id,
        Some("child-frame"),
        Some(child_url),
    )
    .await;
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_child_history_back_emits_child_frame_navigation_and_lifecycle_events() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/main",
                    axum::routing::get(|| async {
                        "<iframe id='child' name='child-frame' src='/initial'></iframe>"
                    }),
                )
                .route(
                    "/initial",
                    axum::routing::get(|| async { "<body>initial</body>" }),
                )
                .route(
                    "/history-a",
                    axum::routing::get(|| async { "<body>history-a</body>" }),
                )
                .route(
                    "/history-b",
                    axum::routing::get(|| async { "<body>history-b</body>" }),
                ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .devtools_session_state
        .page_session_state
        .page_lifecycle_events = true;
    ctx.process_async(json!({
        "id": 531,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": format!("http://{addr}/main")
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 531);
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 532).await;
    assert_child_frame_navigation_completion(
        &mut ctx,
        &child_frame_id,
        Some("child-frame"),
        Some(&format!("http://{addr}/initial")),
    )
    .await;

    let child_url_a = format!("http://{addr}/history-a");
    ctx.process_async(json!({
        "id": 533,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(
                "document.getElementById('child').contentWindow.location.href = {}",
                serde_json::to_string(&child_url_a).expect("child url should encode")
            )
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 533);
    assert!(
        response.get("error").is_none(),
        "runtime evaluate should not fail: {response:?}"
    );
    wait_for_child_frame_navigated_url(&mut ctx, &child_frame_id, child_url_a.as_str()).await;

    let child_url_b = format!("http://{addr}/history-b");
    ctx.process_async(json!({
        "id": 534,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(
                "document.getElementById('child').contentWindow.location.href = {}",
                serde_json::to_string(&child_url_b).expect("child url should encode")
            )
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 534);
    assert!(
        response.get("error").is_none(),
        "runtime evaluate should not fail: {response:?}"
    );
    wait_for_child_frame_navigated_url(&mut ctx, &child_frame_id, child_url_b.as_str()).await;

    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 535,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('child').contentWindow.history.back()"
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 535);
    assert!(
        response.get("error").is_none(),
        "runtime evaluate should not fail: {response:?}"
    );
    assert_child_frame_navigation_completion(
        &mut ctx,
        &child_frame_id,
        Some("child-frame"),
        Some(child_url_a.as_str()),
    )
    .await;
}
