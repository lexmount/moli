use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_allocate_document_start_script_identifiers_target_locally() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-SCRIPT-ID",
        "TID-000000000SI",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "source": "globalThis.targetAFirst = true;" }
    }))
    .await;
    ctx.expect_result(104194, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104195,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-SCRIPT-ID", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104195, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104196,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104196)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104197,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "source": "globalThis.targetBFirst = true;" }
    }))
    .await;
    ctx.expect_result(
        104197,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 104198,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000SI" }
    }))
    .await;
    let close = take_response_by_id(&mut ctx, 104198);
    assert_eq!(close["result"]["success"], json!(true));

    ctx.process_async(json!({
        "id": 104199,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "source": "globalThis.targetBSecond = true;" }
    }))
    .await;
    ctx.expect_result(
        104199,
        json!({ "identifier": "2" }),
        Some(&second_session_id),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_allocate_document_start_script_identifiers_target_locally_after_session_scoped_owner_activity()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-SCRIPT-ID-SESSION",
        "TID-000000000SX",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041941,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "source": "globalThis.targetAFirst = true;" }
    }))
    .await;
    ctx.expect_result(1041941, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 1041942,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-SCRIPT-ID-SESSION", "url": "about:blank#second"}
    }))
    .await;
    let created = loop {
        let message = ctx.take_one();
        if message["method"] == json!("Target.targetCreated") {
            break message;
        }
    };
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(1041942, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041943,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1041943)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1041944,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "source": "globalThis.targetBFirst = true;" }
    }))
    .await;
    ctx.expect_result(
        1041944,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 1041945,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041945);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041946,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000SX" }
    }))
    .await;
    ctx.expect_result(1041946, json!({}), None);

    ctx.process_async(json!({
        "id": 1041947,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": { "expression": "document.title" }
    }))
    .await;
    let promoted_second = take_response_by_id(&mut ctx, 1041947);
    assert_eq!(
        promoted_second["result"]["result"]["value"],
        json!("second")
    );

    ctx.process_async(json!({
        "id": 1041948,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "source": "globalThis.targetBSecond = true;" }
    }))
    .await;
    ctx.expect_result(
        1041948,
        json!({ "identifier": "2" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 1041949,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": { "expression": "document.title" }
    }))
    .await;
    let restored_first = take_response_by_id(&mut ctx, 1041949);
    assert_eq!(restored_first["result"]["result"]["value"], json!("active"));

    ctx.process_async(json!({
        "id": 1041950,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "source": "globalThis.targetASecond = true;" }
    }))
    .await;
    ctx.expect_result(1041950, json!({ "identifier": "2" }), Some("SID-active"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_allocate_utility_pre_document_script_identifiers_target_locally_after_session_scoped_owner_activity()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-UTILITY-SCRIPT-ID-SESSION",
        "TID-000000000UX",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041951,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.targetAUtilityFirst = true;",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(1041951, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 1041952,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-UTILITY-SCRIPT-ID-SESSION", "url": "about:blank#second"}
    })).await;
    let created = loop {
        let message = ctx.take_one();
        if message["method"] == json!("Target.targetCreated") {
            break message;
        }
    };
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(1041952, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041953,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1041953)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1041954,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.targetBUtilityFirst = true;",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(
        1041954,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 1041955,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041955);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041956,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000UX" }
    }))
    .await;
    ctx.expect_result(1041956, json!({}), None);

    ctx.process_async(json!({
        "id": 1041957,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": { "expression": "document.title" }
    }))
    .await;
    let promoted_second = take_response_by_id(&mut ctx, 1041957);
    assert_eq!(
        promoted_second["result"]["result"]["value"],
        json!("second")
    );

    ctx.process_async(json!({
        "id": 1041958,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.targetBUtilitySecond = true;",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(
        1041958,
        json!({ "identifier": "2" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 1041959,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": { "expression": "document.title" }
    }))
    .await;
    let restored_first = take_response_by_id(&mut ctx, 1041959);
    assert_eq!(restored_first["result"]["result"]["value"], json!("active"));

    ctx.process_async(json!({
        "id": 1041960,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.targetAUtilitySecond = true;",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(1041960, json!({ "identifier": "2" }), Some("SID-active"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_remove_only_their_own_pre_document_script_identifier_after_switching()
{
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-SCRIPT-REMOVE",
        "TID-000000000SR",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104200,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "source": "globalThis.targetAMarker = 'A';" }
    }))
    .await;
    ctx.expect_result(104200, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104201,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-SCRIPT-REMOVE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104201, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104202,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104202)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104203,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "source": "globalThis.targetBMarker = 'B';" }
    }))
    .await;
    ctx.expect_result(
        104203,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 104204,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000SR" }
    }))
    .await;
    ctx.expect_result(104204, json!({}), None);

    ctx.process_async(json!({
        "id": 104205,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "identifier": "1" }
    }))
    .await;
    ctx.expect_result(104205, json!({}), Some("SID-active"));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active
                .active_page_target()
                .owner_state
                .document_start_scripts
                .is_empty()
        );
        let staged = active
            .background_target(&second_target_id)
            .expect("background target should remain staged");
        assert_eq!(
            active
                .background_target(staged.target_id())
                .expect("background target must exist")
                .owner_state
                .document_start_scripts
                .len(),
            1
        );
        assert_eq!(
            active
                .background_target(staged.target_id())
                .expect("background target must exist")
                .owner_state
                .document_start_scripts[0]
                .0,
            "1"
        );
    }

    ctx.process_async(json!({
        "id": 104206,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104206, json!({}), None);

    ctx.process_async(json!({
        "id": 104207,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104207);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104208,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ a: globalThis.targetAMarker ?? 'absent', b: globalThis.targetBMarker ?? 'absent' })"
        }
    })).await;
    let eval = take_response_by_id(&mut ctx, 104208);
    let payload = eval["result"]["result"]["value"]
        .as_str()
        .expect("payload should be string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("payload should be valid json");
    assert_eq!(payload["a"], json!("absent"));
    assert_eq!(payload["b"], json!("B"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_remove_only_their_own_utility_pre_document_script_identifier_after_switching()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-UTILITY-SCRIPT-REMOVE",
        "TID-000000000UR",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104209,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.targetAUtilityMarker = 'A';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104209, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104210,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-UTILITY-SCRIPT-REMOVE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104210, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104211,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104211)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104212,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.targetBUtilityMarker = 'B';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(
        104212,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 104213,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "identifier": "1" }
    }))
    .await;
    ctx.expect_result(104213, json!({}), Some("SID-active"));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active
                .active_page_target()
                .owner_state
                .document_start_scripts
                .is_empty()
        );
        let staged = active
            .background_target(&second_target_id)
            .expect("background target should remain staged");
        assert_eq!(
            active
                .background_target(staged.target_id())
                .expect("background target must exist")
                .owner_state
                .document_start_scripts
                .len(),
            1
        );
        assert_eq!(
            active
                .background_target(staged.target_id())
                .expect("background target must exist")
                .owner_state
                .document_start_scripts[0]
                .0,
            "1"
        );
    }

    ctx.process_async(json!({
        "id": 104214,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104214, json!({}), None);

    ctx.process_async(json!({
        "id": 104215,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104215);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104216,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 104216)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");

    ctx.process_async(json!({
        "id": 104217,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "JSON.stringify({ a: globalThis.targetAUtilityMarker ?? 'absent', b: globalThis.targetBUtilityMarker ?? 'absent' })"
        }
    })).await;
    let eval = take_response_by_id(&mut ctx, 104217);
    let payload = eval["result"]["result"]["value"]
        .as_str()
        .expect("payload should be string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("payload should be valid json");
    assert_eq!(payload["a"], json!("absent"));
    assert_eq!(payload["b"], json!("B"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_remove_only_their_own_utility_binding_definition_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-UTILITY-BINDING-REMOVE",
        "TID-000000000UB",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104218,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": {
            "name": "sharedUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104218, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104219,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.targetAUtilityBindingMarker = 'A';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104219, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104220,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-UTILITY-BINDING-REMOVE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104220, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104221,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104221)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104222,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "sharedUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104222, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104223,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.targetBUtilityBindingMarker = 'B';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(
        104223,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );
    ctx.process_async(json!({
        "id": 104224,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-active",
        "params": { "name": "sharedUtilityBinding" }
    }))
    .await;
    ctx.expect_result(104224, json!({}), Some("SID-active"));
    ctx.process_async(json!({
        "id": 1042241,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(1042241, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_bindings
                .iter()
                .any(|binding| {
                    binding.name == "sharedUtilityBinding"
                        && binding.execution_context_name.as_deref() == Some("utility")
                })
        );
        let staged = active
            .background_target("TID-000000000UB")
            .expect("first target should remain staged");
        let staged_bindings = active
            .background_target(staged.target_id())
            .filter(|target| target.has_non_default_session_state())
            .map(|state| {
                state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_bindings
                    .as_slice()
            })
            .unwrap_or(&[]);
        assert!(!staged_bindings.iter().any(|binding| {
            binding.name == "sharedUtilityBinding"
                && binding.execution_context_name.as_deref() == Some("utility")
        }));
    }

    ctx.process_async(json!({
        "id": 104225,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104225, json!({}), None);

    ctx.process_async(json!({
        "id": 104226,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104226);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104227,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 104227)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");

    ctx.process_async(json!({
        "id": 104228,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "JSON.stringify({ marker: globalThis.targetBUtilityBindingMarker ?? 'absent', binding: typeof globalThis.sharedUtilityBinding })"
        }
    })).await;
    let eval = take_response_by_id(&mut ctx, 104228);
    let payload = eval["result"]["result"]["value"]
        .as_str()
        .expect("payload should be string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("payload should be valid json");
    assert_eq!(payload["marker"], json!("B"));
    assert_eq!(payload["binding"], json!("function"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_remove_only_their_own_main_world_binding_definition_after_switching()
{
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-MAIN-BINDING-REMOVE",
        "TID-000000000MB",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104229,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": { "name": "sharedMainBinding" }
    }))
    .await;
    ctx.expect_result(104229, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104230,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.targetAMainBindingMarker = 'A';"
        }
    }))
    .await;
    ctx.expect_result(104230, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104231,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-MAIN-BINDING-REMOVE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104231, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104232,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104232)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104233,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "sharedMainBinding"
        }
    }))
    .await;
    ctx.expect_result(104233, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104234,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.targetBMainBindingMarker = 'B';"
        }
    }))
    .await;
    ctx.expect_result(
        104234,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 104235,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-active",
        "params": { "name": "sharedMainBinding" }
    }))
    .await;
    ctx.expect_result(104235, json!({}), Some("SID-active"));
    ctx.process_async(json!({
        "id": 1042351,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(1042351, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_bindings
                .iter()
                .any(|binding| {
                    binding.name == "sharedMainBinding" && binding.execution_context_name.is_none()
                })
        );
        let staged = active
            .background_target("TID-000000000MB")
            .expect("first target should remain staged");
        let staged_bindings = active
            .background_target(staged.target_id())
            .filter(|target| target.has_non_default_session_state())
            .map(|state| {
                state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_bindings
                    .as_slice()
            })
            .unwrap_or(&[]);
        assert!(!staged_bindings.iter().any(|binding| {
            binding.name == "sharedMainBinding" && binding.execution_context_name.is_none()
        }));
    }

    ctx.process_async(json!({
        "id": 104236,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104236, json!({}), None);

    ctx.process_async(json!({
        "id": 104237,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104237);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104238,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ marker: globalThis.targetBMainBindingMarker ?? 'absent', binding: typeof globalThis.sharedMainBinding, other: globalThis.targetAMainBindingMarker ?? 'absent' })"
        }
    })).await;
    let eval = take_response_by_id(&mut ctx, 104238);
    let payload = eval["result"]["result"]["value"]
        .as_str()
        .expect("payload should be string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("payload should be valid json");
    assert_eq!(payload["marker"], json!("B"));
    assert_eq!(payload["binding"], json!("function"));
    assert_eq!(payload["other"], json!("absent"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_remove_only_their_own_dual_world_binding_definition_after_switching()
{
    target_8mb_stack("same-context-dual-world-binding-remove", || async {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-DUAL-BINDING-REMOVE",
        "TID-000000000DB",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104239,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": { "name": "sharedDualBinding" }
    }))
    .await;
    ctx.expect_result(104239, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104240,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": {
            "name": "sharedDualBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104240, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104241,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "source": "globalThis.targetADualMainMarker = 'A';" }
    }))
    .await;
    ctx.expect_result(104241, json!({ "identifier": "1" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104242,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.targetADualUtilityMarker = 'A';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104242, json!({ "identifier": "2" }), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104243,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-DUAL-BINDING-REMOVE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104243, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104244,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104244)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104245,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "sharedDualBinding" }
    }))
    .await;
    ctx.expect_result(104245, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104246,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "sharedDualBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104246, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104247,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "source": "globalThis.targetBDualMainMarker = 'B';" }
    }))
    .await;
    ctx.expect_result(
        104247,
        json!({ "identifier": "1" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 104248,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.targetBDualUtilityMarker = 'B';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(
        104248,
        json!({ "identifier": "2" }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 104249,
        "method": "Runtime.removeBinding",
        "sessionId": "SID-active",
        "params": { "name": "sharedDualBinding" }
    }))
    .await;
    ctx.expect_result(104249, json!({}), Some("SID-active"));
    ctx.process_async(json!({
        "id": 1042491,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(1042491, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active
                .active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]

                .runtime_bindings
                .iter()
                .any(|binding| binding.name == "sharedDualBinding")
        );
        let staged = active
            .background_target("TID-000000000DB")
            .expect("first target should remain staged");
        let staged_bindings = active
            .background_target(staged.target_id()).filter(|target| target.has_non_default_session_state())
            .map(|state| state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].runtime_bindings.as_slice())
            .unwrap_or(&[]);
        assert_eq!(
            staged_bindings
                .iter()
                .filter(|binding| binding.name == "sharedDualBinding")
                .count(),
            0
        );
    }

    ctx.process_async(json!({
        "id": 104250,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104250, json!({}), None);

    ctx.process_async(json!({
        "id": 104251,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104251);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104252,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 104252)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");

    ctx.process_async(json!({
        "id": 104253,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ mainMarker: globalThis.targetBDualMainMarker ?? 'absent', mainBinding: typeof globalThis.sharedDualBinding, other: globalThis.targetADualMainMarker ?? 'absent' })"
        }
    })).await;
    let main_eval = take_response_by_id(&mut ctx, 104253);
    let main_payload = main_eval["result"]["result"]["value"]
        .as_str()
        .expect("payload should be string");
    let main_payload: serde_json::Value =
        serde_json::from_str(main_payload).expect("payload should be valid json");
    assert_eq!(main_payload["mainMarker"], json!("B"));
    assert_eq!(main_payload["mainBinding"], json!("function"));
    assert_eq!(main_payload["other"], json!("absent"));

    ctx.process_async(json!({
        "id": 104254,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "JSON.stringify({ utilityMarker: globalThis.targetBDualUtilityMarker ?? 'absent', utilityBinding: typeof globalThis.sharedDualBinding, other: globalThis.targetADualUtilityMarker ?? 'absent' })"
        }
    })).await;
    let utility_eval = take_response_by_id(&mut ctx, 104254);
    let utility_payload = utility_eval["result"]["result"]["value"]
        .as_str()
        .expect("payload should be string");
    let utility_payload: serde_json::Value =
        serde_json::from_str(utility_payload).expect("payload should be valid json");
    assert_eq!(utility_payload["utilityMarker"], json!("B"));
    assert_eq!(utility_payload["utilityBinding"], json!("function"));
    assert_eq!(utility_payload["other"], json!("absent"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_replay_only_their_own_pre_document_binding_and_preload_after_session_scoped_owner_activity()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 10416,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": { "name": "targetABinding" }
    }))
    .await;
    ctx.expect_result(10416, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10417,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.__lm_target_marker = 'A'; if (typeof globalThis.targetABinding === 'function') globalThis.targetABinding('payload-A');"
        }
    })).await;
    let add_a = take_response_by_id(&mut ctx, 10417);
    assert!(add_a["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10418,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10418, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10419,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10419, json!({}), None);

    ctx.process_async(json!({
        "id": 10420,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBBinding" }
    }))
    .await;
    ctx.expect_result(10420, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10421,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.__lm_target_marker = 'B'; if (typeof globalThis.targetBBinding === 'function') globalThis.targetBBinding('payload-B');"
        }
    })).await;
    let add_b = take_response_by_id(&mut ctx, 10421);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10422,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-initial</title><div id='ok'>B initial page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let first_b_navigation = take_response_by_id(&mut ctx, 10422);
    assert_eq!(
        first_b_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
                && message["params"]["payload"] == json!("payload-B")
        }),
        "target B should replay its own binding on first navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
        }),
        "target A binding should not leak into target B first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10423,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10423, json!({}), None);

    ctx.process_async(json!({
        "id": 10424,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>target-a-initial</title><div id='ok'>A initial page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let first_a_navigation = take_response_by_id(&mut ctx, 10424);
    assert_eq!(
        first_a_navigation["result"]["frameId"],
        json!("TID-000000000A")
    );
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
                && message["params"]["payload"] == json!("payload-A")
        }),
        "target A should replay its own binding on first navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
        }),
        "target B binding should not leak into target A first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10425,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ title: document.title, marker: globalThis.__lm_target_marker, hasA: typeof globalThis.targetABinding, hasB: typeof globalThis.targetBBinding, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let parked_eval = take_response_by_id(&mut ctx, 10425);
    let parked_payload = parked_eval["result"]["result"]["value"]
        .as_str()
        .expect("parked target payload should be string");
    let parked_payload: serde_json::Value =
        serde_json::from_str(parked_payload).expect("parked target payload should be valid json");
    assert_eq!(parked_payload["title"], json!("target-b-initial"));
    assert_eq!(parked_payload["marker"], json!("B"));
    assert_eq!(parked_payload["hasA"], json!("undefined"));
    assert_eq!(parked_payload["hasB"], json!("function"));
    assert_eq!(parked_payload["text"], json!("B initial page"));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10426,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-promoted</title><div id='ok'>B promoted page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    let promoted_navigation = take_response_by_id(&mut ctx, 10426);
    assert_eq!(
        promoted_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
                && message["params"]["payload"] == json!("payload-B")
        }),
        "session-scoped promotion should replay target B binding: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
        }),
        "session-scoped promotion should not leak target A binding into target B: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10427,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ title: document.title, marker: globalThis.__lm_target_marker, hasA: typeof globalThis.targetABinding, hasB: typeof globalThis.targetBBinding, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 10427);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted target payload should be string");
    let promoted_payload: serde_json::Value = serde_json::from_str(promoted_payload)
        .expect("promoted target payload should be valid json");
    assert_eq!(promoted_payload["title"], json!("target-b-promoted"));
    assert_eq!(promoted_payload["marker"], json!("B"));
    assert_eq!(promoted_payload["hasA"], json!("undefined"));
    assert_eq!(promoted_payload["hasB"], json!("function"));
    assert_eq!(promoted_payload["text"], json!("B promoted page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_replay_only_their_own_pre_document_binding_and_preload_after_close_target_promotion()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 10470,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": { "name": "targetABinding" }
    }))
    .await;
    ctx.expect_result(10470, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10471,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.__lm_target_marker = 'A'; if (typeof globalThis.targetABinding === 'function') globalThis.targetABinding('payload-A');"
        }
    })).await;
    let add_a = take_response_by_id(&mut ctx, 10471);
    assert!(add_a["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10472,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10472, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10473,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10473, json!({}), None);

    ctx.process_async(json!({
        "id": 10474,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBBinding" }
    }))
    .await;
    ctx.expect_result(10474, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10475,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.__lm_target_marker = 'B'; if (typeof globalThis.targetBBinding === 'function') globalThis.targetBBinding('payload-B');"
        }
    })).await;
    let add_b = take_response_by_id(&mut ctx, 10475);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10476,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-initial</title><div id='ok'>B close page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10476);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
                && message["params"]["payload"] == json!("payload-B")
        }),
        "target B should replay its own binding on first navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
        }),
        "target A binding should not leak into target B first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10477,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10477, json!({}), None);

    ctx.process_async(json!({
        "id": 10478,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>target-a-initial</title><div id='ok'>A close page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10478);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
                && message["params"]["payload"] == json!("payload-A")
        }),
        "target A should replay its own binding on first navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
        }),
        "target B binding should not leak into target A first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10479,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10479, json!({ "success": true }), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000A",
            "sessionId": "SID-active",
        })),
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10480,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-promoted</title><div id='ok'>B promoted close page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    let promoted_navigation = take_response_by_id(&mut ctx, 10480);
    assert_eq!(
        promoted_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
                && message["params"]["payload"] == json!("payload-B")
        }),
        "closeTarget promotion should replay target B binding: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
        }),
        "closeTarget promotion should not leak target A binding into target B: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10481,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ title: document.title, marker: globalThis.__lm_target_marker, hasA: typeof globalThis.targetABinding, hasB: typeof globalThis.targetBBinding, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 10481);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted target payload should be string");
    let promoted_payload: serde_json::Value = serde_json::from_str(promoted_payload)
        .expect("promoted target payload should be valid json");
    assert_eq!(promoted_payload["title"], json!("target-b-promoted"));
    assert_eq!(promoted_payload["marker"], json!("B"));
    assert_eq!(promoted_payload["hasA"], json!("undefined"));
    assert_eq!(promoted_payload["hasB"], json!("function"));
    assert_eq!(promoted_payload["text"], json!("B promoted close page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_materialize_only_their_own_utility_pre_document_binding_and_preload_after_close_target_promotion()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 10428,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": {
            "name": "targetAUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10428, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10429,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.__lm_target_utility_marker = 'A'; if (typeof globalThis.targetAUtilityBinding === 'function') globalThis.targetAUtilityBinding('payload-A-utility');",
            "worldName": "utility"
        }
    })).await;
    let add_a = take_response_by_id(&mut ctx, 10429);
    assert!(add_a["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10430,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10430, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10431,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10431, json!({}), None);

    ctx.process_async(json!({
        "id": 10432,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "targetBUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10432, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10433,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.__lm_target_utility_marker = 'B'; if (typeof globalThis.targetBUtilityBinding === 'function') globalThis.targetBUtilityBinding('payload-B-utility');",
            "worldName": "utility"
        }
    })).await;
    let add_b = take_response_by_id(&mut ctx, 10433);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10434,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-utility-initial</title><div id='ok'>B utility initial page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10434);
    let initial_binding_called_b_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak into target B first navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10435,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_b_initial =
        take_response_by_id(&mut ctx, 10435)["result"]["executionContextId"]
            .as_i64()
            .expect("target B utility context id");
    let initial_binding_called_b = initial_binding_called_b_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetBUtilityBinding")
                })
                .cloned()
        })
        .expect("target B utility binding should replay on first materialization");
    assert_eq!(
        initial_binding_called_b["params"]["payload"],
        json!("payload-B-utility")
    );
    assert_eq!(
        initial_binding_called_b["params"]["executionContextId"],
        json!(utility_context_b_initial)
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak into target B first materialization: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10436,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10436, json!({}), None);

    ctx.process_async(json!({
        "id": 10437,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>target-a-utility-initial</title><div id='ok'>A utility initial page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10437);
    let binding_called_a_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        }),
        "target B utility binding should not leak into target A first navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10438,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-active",
        "params": {
            "frameId": "TID-000000000A",
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_a = take_response_by_id(&mut ctx, 10438)["result"]["executionContextId"]
        .as_i64()
        .expect("target A utility context id");
    let binding_called_a = binding_called_a_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetAUtilityBinding")
                })
                .cloned()
        })
        .expect("target A utility binding should replay on first materialization");
    assert_eq!(
        binding_called_a["params"]["payload"],
        json!("payload-A-utility")
    );
    assert_eq!(
        binding_called_a["params"]["executionContextId"],
        json!(utility_context_a)
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        }),
        "target B utility binding should not leak into target A first materialization: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10439,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10439, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000A",
            "sessionId": "SID-active",
        })),
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10440,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-utility-promoted</title><div id='ok'>B utility promoted page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    let promoted_navigation = take_response_by_id(&mut ctx, 10440);
    assert_eq!(
        promoted_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    let promoted_binding_called_b_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak into target B promoted navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10441,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_b_promoted =
        take_response_by_id(&mut ctx, 10441)["result"]["executionContextId"]
            .as_i64()
            .expect("promoted target B utility context id");
    let promoted_binding_called_b = promoted_binding_called_b_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetBUtilityBinding")
                })
                .cloned()
        })
        .expect("target B utility binding should replay after closeTarget promotion");
    assert_eq!(
        promoted_binding_called_b["params"]["payload"],
        json!("payload-B-utility")
    );
    assert_eq!(
        promoted_binding_called_b["params"]["executionContextId"],
        json!(utility_context_b_promoted)
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak after closeTarget promotion: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10442,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "contextId": utility_context_b_promoted,
            "expression": "JSON.stringify({ marker: globalThis.__lm_target_utility_marker, hasA: typeof globalThis.targetAUtilityBinding, hasB: typeof globalThis.targetBUtilityBinding, title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let eval_b = take_response_by_id(&mut ctx, 10442);
    let payload_b = eval_b["result"]["result"]["value"]
        .as_str()
        .expect("target B utility payload should be string");
    let payload_b: serde_json::Value =
        serde_json::from_str(payload_b).expect("target B utility payload should be valid json");
    assert_eq!(payload_b["marker"], json!("B"));
    assert_eq!(payload_b["hasA"], json!("undefined"));
    assert_eq!(payload_b["hasB"], json!("function"));
    assert_eq!(payload_b["title"], json!("target-b-utility-promoted"));
    assert_eq!(payload_b["text"], json!("B utility promoted page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_materialize_only_their_own_utility_pre_document_binding_and_preload_after_session_scoped_owner_activity()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 10482,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": {
            "name": "targetAUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10482, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10483,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": {
            "source": "globalThis.__lm_target_utility_marker = 'A'; if (typeof globalThis.targetAUtilityBinding === 'function') globalThis.targetAUtilityBinding('payload-A-utility');",
            "worldName": "utility"
        }
    })).await;
    let add_a = take_response_by_id(&mut ctx, 10483);
    assert!(add_a["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10484,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10484, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10485,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10485, json!({}), None);

    ctx.process_async(json!({
        "id": 10486,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "targetBUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10486, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10487,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.__lm_target_utility_marker = 'B'; if (typeof globalThis.targetBUtilityBinding === 'function') globalThis.targetBUtilityBinding('payload-B-utility');",
            "worldName": "utility"
        }
    })).await;
    let add_b = take_response_by_id(&mut ctx, 10487);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10488,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-utility-initial</title><div id='ok'>B utility parked page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10488);
    let binding_called_b_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak into target B first navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10489,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_b_initial =
        take_response_by_id(&mut ctx, 10489)["result"]["executionContextId"]
            .as_i64()
            .expect("target B utility context id");
    let binding_called_b = binding_called_b_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetBUtilityBinding")
                })
                .cloned()
        })
        .expect("target B utility binding should replay on first materialization");
    assert_eq!(
        binding_called_b["params"]["payload"],
        json!("payload-B-utility")
    );
    assert_eq!(
        binding_called_b["params"]["executionContextId"],
        json!(utility_context_b_initial)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10490,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10490, json!({}), None);

    ctx.process_async(json!({
        "id": 10491,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>target-a-utility-initial</title><div id='ok'>A utility parked page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10491);
    let binding_called_a_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        }),
        "target B utility binding should not leak into target A first navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10492,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-active",
        "params": {
            "frameId": "TID-000000000A",
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_a = take_response_by_id(&mut ctx, 10492)["result"]["executionContextId"]
        .as_i64()
        .expect("target A utility context id");
    let binding_called_a = binding_called_a_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetAUtilityBinding")
                })
                .cloned()
        })
        .expect("target A utility binding should replay on first materialization");
    assert_eq!(
        binding_called_a["params"]["payload"],
        json!("payload-A-utility")
    );
    assert_eq!(
        binding_called_a["params"]["executionContextId"],
        json!(utility_context_a)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10493,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": { "expression": "document.title" }
    }))
    .await;
    let parked_eval = take_response_by_id(&mut ctx, 10493);
    assert_eq!(
        parked_eval["result"]["result"]["value"],
        json!("target-b-utility-initial")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10494,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-utility-promoted</title><div id='ok'>B utility promoted page</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10494);
    let promoted_binding_called_b_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "session-scoped promotion should not leak target A utility binding into target B: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10495,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_b_promoted =
        take_response_by_id(&mut ctx, 10495)["result"]["executionContextId"]
            .as_i64()
            .expect("promoted target B utility context id");
    let promoted_binding_called_b = promoted_binding_called_b_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetBUtilityBinding")
                })
                .cloned()
        })
        .expect("target B utility binding should replay after session-scoped promotion");
    assert_eq!(
        promoted_binding_called_b["params"]["payload"],
        json!("payload-B-utility")
    );
    assert_eq!(
        promoted_binding_called_b["params"]["executionContextId"],
        json!(utility_context_b_promoted)
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak after session-scoped promotion: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10496,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "contextId": utility_context_b_promoted,
            "expression": "JSON.stringify({ marker: globalThis.__lm_target_utility_marker, hasA: typeof globalThis.targetAUtilityBinding, hasB: typeof globalThis.targetBUtilityBinding, title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let eval_b = take_response_by_id(&mut ctx, 10496);
    let payload_b = eval_b["result"]["result"]["value"]
        .as_str()
        .expect("target B utility payload should be string");
    let payload_b: serde_json::Value =
        serde_json::from_str(payload_b).expect("target B utility payload should be valid json");
    assert_eq!(payload_b["marker"], json!("B"));
    assert_eq!(payload_b["hasA"], json!("undefined"));
    assert_eq!(payload_b["hasB"], json!("function"));
    assert_eq!(payload_b["title"], json!("target-b-utility-promoted"));
    assert_eq!(payload_b["text"], json!("B utility promoted page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_only_their_own_pre_document_binding_and_preload_after_attach_to_target_chain()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 10443,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": { "name": "targetABinding" }
    }))
    .await;
    ctx.expect_result(10443, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10444,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-active",
        "params": { "source": "globalThis.__lm_target_marker = 'A';" }
    }))
    .await;
    let add_a = take_response_by_id(&mut ctx, 10444);
    assert!(add_a["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10445,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(10445, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10446,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 10446)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 10447,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBBinding" }
    }))
    .await;
    ctx.expect_result(10447, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10448,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "source": "globalThis.__lm_target_marker = 'B';" }
    }))
    .await;
    let add_b = take_response_by_id(&mut ctx, 10448);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10449,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-initial</title><div id='ok'>B page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10449);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10450,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    let first_session_id = take_response_by_id(&mut ctx, 10450)["result"]["sessionId"]
        .as_str()
        .expect("first target session id")
        .to_owned();
    assert_ne!(first_session_id, "SID-active");
    ctx.take_first_matching("first target reattached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(first_session_id)
    });

    ctx.process_async(json!({
        "id": 10451,
        "method": "Page.navigate",
        "sessionId": first_session_id,
        "params": {
            "url": "data:text/html,<title>target-a-initial</title><div id='ok'>A page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10451);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10452,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let second_reattach_session_id = take_response_by_id(&mut ctx, 10452)["result"]["sessionId"]
        .as_str()
        .expect("second reattach session id")
        .to_owned();
    assert_ne!(second_reattach_session_id, second_session_id);
    ctx.take_first_matching("second target reattached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(second_reattach_session_id)
    });

    ctx.process_async(json!({
        "id": 10453,
        "method": "Runtime.evaluate",
        "sessionId": second_reattach_session_id,
        "params": {
            "expression": "JSON.stringify({ title: document.title, marker: globalThis.__lm_target_marker, hasA: typeof globalThis.targetABinding, hasB: typeof globalThis.targetBBinding, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let second_eval = take_response_by_id(&mut ctx, 10453);
    let second_payload = second_eval["result"]["result"]["value"]
        .as_str()
        .expect("second payload should be string");
    let second_payload: serde_json::Value =
        serde_json::from_str(second_payload).expect("second payload should be valid json");
    assert_eq!(second_payload["title"], json!("target-b-initial"));
    assert_eq!(second_payload["marker"], json!("B"));
    assert_eq!(second_payload["hasA"], json!("undefined"));
    assert_eq!(second_payload["hasB"], json!("function"));
    assert_eq!(second_payload["text"], json!("B page"));

    ctx.process_async(json!({
        "id": 10454,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    let first_reattach_session_id = take_response_by_id(&mut ctx, 10454)["result"]["sessionId"]
        .as_str()
        .expect("first reattach session id")
        .to_owned();
    assert_ne!(first_reattach_session_id, "SID-active");
    assert_ne!(first_reattach_session_id, first_session_id);
    ctx.take_first_matching("first target attached again", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(first_reattach_session_id)
    });

    ctx.process_async(json!({
        "id": 10455,
        "method": "Runtime.evaluate",
        "sessionId": first_reattach_session_id,
        "params": {
            "expression": "JSON.stringify({ title: document.title, marker: globalThis.__lm_target_marker, hasA: typeof globalThis.targetABinding, hasB: typeof globalThis.targetBBinding, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let first_eval = take_response_by_id(&mut ctx, 10455);
    let first_payload = first_eval["result"]["result"]["value"]
        .as_str()
        .expect("first payload should be string");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first payload should be valid json");
    assert_eq!(first_payload["title"], json!("target-a-initial"));
    assert_eq!(first_payload["marker"], json!("A"));
    assert_eq!(first_payload["hasA"], json!("function"));
    assert_eq!(first_payload["hasB"], json!("undefined"));
    assert_eq!(first_payload["text"], json!("A page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_only_their_own_utility_pre_document_binding_and_preload_after_set_auto_attach_chain()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.insert_page_target_host(crate::conn::PageTargetHost::new(
        "TID-000000000F".into(),
        None,
        crate::conn::TargetIdentityState::new(
            "about:blank#second".into(),
            crate::conn::URL_BASE.into(),
            "Secure".into(),
        ),
        crate::conn::TargetPageSlot::empty_for_test_fixture(),
    ));
    bc.insert_page_target_host(crate::conn::PageTargetHost::new(
        "TID-0000000010".into(),
        None,
        crate::conn::TargetIdentityState::new(
            "about:blank#third".into(),
            crate::conn::URL_BASE.into(),
            "Secure".into(),
        ),
        crate::conn::TargetPageSlot::empty_for_test_fixture(),
    ));

    ctx.process_async(json!({
        "id": 10456,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(10456, json!({}), None);
    let events = ctx.take_all();
    let second_session_id = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-000000000F")
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second target session id")
        .to_owned();
    let third_session_id = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-0000000010")
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("third target session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 10457,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "targetBUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10457, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10458,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": {
            "source": "globalThis.__lm_target_utility_marker = 'B'; if (typeof globalThis.targetBUtilityBinding === 'function') globalThis.targetBUtilityBinding('payload-B-utility');",
            "worldName": "utility"
        }
    })).await;
    let add_b = take_response_by_id(&mut ctx, 10458);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10459,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-chain</title><div id='ok'>B chain page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10459);
    let binding_called_b_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetCUtilityBinding")
        }),
        "target C utility binding should not leak into target B first navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10460,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": "TID-000000000F",
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_b = take_response_by_id(&mut ctx, 10460)["result"]["executionContextId"]
        .as_i64()
        .expect("target B utility context id");
    let binding_called_b = binding_called_b_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetBUtilityBinding")
                })
                .cloned()
        })
        .expect("target B utility binding should materialize");
    assert_eq!(
        binding_called_b["params"]["payload"],
        json!("payload-B-utility")
    );
    assert_eq!(
        binding_called_b["params"]["executionContextId"],
        json!(utility_context_b)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10461,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-0000000010"}
    }))
    .await;
    ctx.expect_result(10461, json!({}), None);

    ctx.process_async(json!({
        "id": 10462,
        "method": "Runtime.addBinding",
        "sessionId": third_session_id,
        "params": {
            "name": "targetCUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10462, json!({}), Some(&third_session_id));

    ctx.process_async(json!({
        "id": 10463,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": third_session_id,
        "params": {
            "source": "globalThis.__lm_target_utility_marker = 'C'; if (typeof globalThis.targetCUtilityBinding === 'function') globalThis.targetCUtilityBinding('payload-C-utility');",
            "worldName": "utility"
        }
    })).await;
    let add_c = take_response_by_id(&mut ctx, 10463);
    assert!(add_c["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10464,
        "method": "Page.navigate",
        "sessionId": third_session_id,
        "params": {
            "url": "data:text/html,<title>target-c-chain</title><div id='ok'>C chain page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    take_response_by_id(&mut ctx, 10464);
    let binding_called_c_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetCUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        }),
        "target B utility binding should not leak into target C first navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10465,
        "method": "Page.createIsolatedWorld",
        "sessionId": third_session_id,
        "params": {
            "frameId": "TID-0000000010",
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_c = take_response_by_id(&mut ctx, 10465)["result"]["executionContextId"]
        .as_i64()
        .expect("target C utility context id");
    let binding_called_c = binding_called_c_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetCUtilityBinding")
                })
                .cloned()
        })
        .expect("target C utility binding should materialize");
    assert_eq!(
        binding_called_c["params"]["payload"],
        json!("payload-C-utility")
    );
    assert_eq!(
        binding_called_c["params"]["executionContextId"],
        json!(utility_context_c)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10466,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000F"}
    }))
    .await;
    ctx.expect_result(10466, json!({}), None);

    ctx.process_async(json!({
        "id": 10467,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "contextId": utility_context_b,
            "expression": "JSON.stringify({ marker: globalThis.__lm_target_utility_marker, hasB: typeof globalThis.targetBUtilityBinding, hasC: typeof globalThis.targetCUtilityBinding, title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let eval_b = take_response_by_id(&mut ctx, 10467);
    let payload_b = eval_b["result"]["result"]["value"]
        .as_str()
        .expect("target B payload should be string");
    let payload_b: serde_json::Value =
        serde_json::from_str(payload_b).expect("target B payload should be valid json");
    assert_eq!(payload_b["marker"], json!("B"));
    assert_eq!(payload_b["hasB"], json!("function"));
    assert_eq!(payload_b["hasC"], json!("undefined"));
    assert_eq!(payload_b["title"], json!("target-b-chain"));
    assert_eq!(payload_b["text"], json!("B chain page"));

    ctx.process_async(json!({
        "id": 10468,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-0000000010"}
    }))
    .await;
    ctx.expect_result(10468, json!({}), None);

    ctx.process_async(json!({
        "id": 10469,
        "method": "Runtime.evaluate",
        "sessionId": third_session_id,
        "params": {
            "contextId": utility_context_c,
            "expression": "JSON.stringify({ marker: globalThis.__lm_target_utility_marker, hasB: typeof globalThis.targetBUtilityBinding, hasC: typeof globalThis.targetCUtilityBinding, title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let eval_c = take_response_by_id(&mut ctx, 10469);
    let payload_c = eval_c["result"]["result"]["value"]
        .as_str()
        .expect("target C payload should be string");
    let payload_c: serde_json::Value =
        serde_json::from_str(payload_c).expect("target C payload should be valid json");
    assert_eq!(payload_c["marker"], json!("C"));
    assert_eq!(payload_c["hasB"], json!("undefined"));
    assert_eq!(payload_c["hasC"], json!("function"));
    assert_eq!(payload_c["title"], json!("target-c-chain"));
    assert_eq!(payload_c["text"], json!("C chain page"));
}
