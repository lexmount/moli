use super::*;
use crate::testing::wait_until_renderer_document_load;

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_script_execution_disabled_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9A",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104170,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": "SID-active",
        "params": { "value": true }
    }))
    .await;
    ctx.expect_result(104170, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104171,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9A", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104171, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104172,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104172)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104173,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104173, json!({}), None);

    ctx.process_async(json!({
        "id": 104174,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><body><script>globalThis.__inlineRan = true;</script><div id='ok'>second</div></body>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    ctx.expect_result(
        104174,
        json!({ "frameId": second_target_id, "loaderId": "LID-0000000001" }),
        Some(&second_session_id),
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104175,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan })"
        }
    }))
    .await;
    let second_eval = take_response_by_id(&mut ctx, 104175);
    let second_payload = second_eval["result"]["result"]["value"]
        .as_str()
        .expect("second script payload");
    let second_payload: serde_json::Value =
        serde_json::from_str(second_payload).expect("second script payload should be valid json");
    assert_eq!(second_payload["inlineRan"], json!(true));

    ctx.process_async(json!({
        "id": 104176,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(104176, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("active browser context")
            .active_page_state()
            .script_execution_disabled
    );

    ctx.process_async(json!({
        "id": 104177,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>first-restored</title><body><script>globalThis.__inlineRan = true;</script><div id='ok'>first</div></body>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    ctx.expect_result(
        104177,
        json!({ "frameId": "TID-000000000A", "loaderId": "LID-0000000002" }),
        Some("SID-active"),
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104178,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan })"
        }
    }))
    .await;
    let first_eval = take_response_by_id(&mut ctx, 104178);
    let first_payload = first_eval["result"]["result"]["value"]
        .as_str()
        .expect("first script payload");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first script payload should be valid json");
    assert_eq!(first_payload["inlineRan"], json!(false));

    ctx.process_async(json!({
        "id": 104179,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104179, json!({}), None);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("active browser context")
            .active_page_state()
            .script_execution_disabled
    );

    ctx.process_async(json!({
        "id": 104180,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second-restored</title><body><script>globalThis.__inlineRan = true;</script><div id='ok'>second restored</div></body>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    ctx.expect_result(
        104180,
        json!({ "frameId": second_target_id, "loaderId": "LID-0000000003" }),
        Some(&second_session_id),
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104181,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": {
            "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan })"
        }
    }))
    .await;
    let second_restored_eval = take_response_by_id(&mut ctx, 104181);
    let second_restored_payload = second_restored_eval["result"]["result"]["value"]
        .as_str()
        .expect("second restored script payload");
    let second_restored_payload: serde_json::Value = serde_json::from_str(second_restored_payload)
        .expect("second restored script payload should be valid json");
    assert_eq!(second_restored_payload["inlineRan"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_search_results_after_switching() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9B");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .set_active_target_id("TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104182,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": "data:text/html,<p>a1</p><p>a2</p>" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    ctx.expect_result(
        104182,
        json!({ "frameId": "TID-000000000A", "loaderId": "LID-0000000001" }),
        Some("SID-active"),
    );
    wait_until_renderer_document_load(
        &mut ctx,
        Some("SID-active"),
        "TID-000000000A",
        "LID-0000000001",
    )
    .await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104183,
        "method": "DOM.performSearch",
        "sessionId": "SID-active",
        "params": { "query": "p" }
    }))
    .await;
    ctx.expect_result(
        104183,
        json!({ "searchId": "0", "resultCount": 2 }),
        Some("SID-active"),
    );

    ctx.process_async(json!({
        "id": 104184,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9B", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 104184);

    ctx.process_async(json!({
        "id": 104185,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104185)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104186,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104186, json!({}), None);

    ctx.process_async(json!({
        "id": 104187,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": "data:text/html,<span>b1</span>" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    ctx.expect_result(
        104187,
        json!({ "frameId": second_target_id, "loaderId": "LID-0000000002" }),
        Some(&second_session_id),
    );
    wait_until_renderer_document_load(
        &mut ctx,
        Some(&second_session_id),
        &second_target_id,
        "LID-0000000002",
    )
    .await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104188,
        "method": "DOM.performSearch",
        "sessionId": second_session_id,
        "params": { "query": "span" }
    }))
    .await;
    ctx.expect_result(
        104188,
        json!({ "searchId": "0", "resultCount": 1 }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 104189,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(104189, json!({}), None);

    ctx.process_async(json!({
        "id": 104190,
        "method": "DOM.getSearchResults",
        "sessionId": "SID-active",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 2 }
    }))
    .await;
    let first_results = take_response_by_id(&mut ctx, 104190);
    assert_eq!(
        first_results["result"]["nodeIds"]
            .as_array()
            .map(|ids| ids.len()),
        Some(2)
    );

    ctx.process_async(json!({
        "id": 104191,
        "method": "DOM.performSearch",
        "sessionId": "SID-active",
        "params": { "query": "p" }
    }))
    .await;
    ctx.expect_result(
        104191,
        json!({ "searchId": "1", "resultCount": 2 }),
        Some("SID-active"),
    );

    ctx.process_async(json!({
        "id": 104192,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104192, json!({}), None);

    ctx.process_async(json!({
        "id": 104193,
        "method": "DOM.getSearchResults",
        "sessionId": second_session_id,
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    let second_results = take_response_by_id(&mut ctx, 104193);
    assert_eq!(
        second_results["result"]["nodeIds"]
            .as_array()
            .map(|ids| ids.len()),
        Some(1)
    );

    ctx.process_async(json!({
        "id": 104194,
        "method": "DOM.performSearch",
        "sessionId": second_session_id,
        "params": { "query": "span" }
    }))
    .await;
    ctx.expect_result(
        104194,
        json!({ "searchId": "1", "resultCount": 1 }),
        Some(&second_session_id),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_crash_state_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9C",
        "TID-000000000A",
        "<title>first-before-crash</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104195,
        "method": "Inspector.enable",
        "sessionId": "SID-active"
    }))
    .await;
    ctx.expect_result(104195, json!({}), Some("SID-active"));

    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();

    ctx.process_async(json!({
        "id": 104196,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9C", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104196, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104197,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 104197)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104198,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104198, json!({}), None);

    ctx.process_async(json!({
        "id": 104199,
        "method": "Inspector.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104199, json!({}), Some(&second_session_id));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Inspector.targetCrashed")
                && message["sessionId"] == json!(second_session_id)
        }),
        "second target should not inherit first target crash state"
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104200,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second-after-switch</title><div id='ok'>second target</div>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 104200);
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Inspector.targetReloadedAfterCrash")
                && message["sessionId"] == json!(second_session_id)
        }),
        "second target navigation should not look like a crash recovery"
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104201,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(104201, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("active browser context")
            .active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );

    ctx.process_async(json!({
        "id": 104202,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>first-after-recovery</title><div id='ok'>first target recovered</div>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 104202);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Inspector.targetReloadedAfterCrash")
                && message["sessionId"] == json!("SID-active")
        }),
        "first target should still remember its own crash state"
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_domain_enablement_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({"id": 104170, "method": "Page.setLifecycleEventsEnabled", "sessionId": "SID-active", "params": { "enabled": true }})).await;
    ctx.expect_result(104170, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 104171, "method": "Runtime.enable", "sessionId": "SID-active"}))
        .await;
    ctx.expect_result(104171, json!({}), Some("SID-active"));
    ctx.process_async(
        json!({"id": 104172, "method": "Inspector.enable", "sessionId": "SID-active"}),
    )
    .await;
    ctx.expect_result(104172, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 104173, "method": "Network.enable", "sessionId": "SID-active"}))
        .await;
    ctx.expect_result(104173, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 104174, "method": "Network.setCacheDisabled", "sessionId": "SID-active", "params": { "cacheDisabled": true }})).await;
    ctx.expect_result(104174, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 104175, "method": "Network.setBypassServiceWorker", "sessionId": "SID-active", "params": { "bypass": true }})).await;
    ctx.expect_result(104175, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 104176, "method": "CSS.enable", "sessionId": "SID-active"}))
        .await;
    ctx.expect_result(104176, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104177,
        "method": "Fetch.enable",
        "sessionId": "SID-active",
        "params": {
            "patterns": [
                {
                    "urlPattern": "*target-a*",
                    "resourceType": "Fetch",
                    "requestStage": "Response"
                }
            ],
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(104177, json!({}), Some("SID-active"));

    ctx.process_async(json!({"id": 104178, "method": "Target.createTarget", "params": {"browserContextId": "BID-9", "url": "about:blank#second"}})).await;
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
    ctx.expect_result(104178, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({"id": 104179, "method": "Target.attachToTarget", "params": { "targetId": second_target_id }})).await;
    let second_session_id = take_response_by_id(&mut ctx, 104179)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({"id": 104180, "method": "Page.navigate", "sessionId": second_session_id, "params": { "url": "data:text/html,<title>second</title><div id='ok'>second target</div>" }})).await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104180);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({"id": 104181, "method": "Target.activateTarget", "params": { "targetId": "TID-000000000A" }})).await;
    ctx.expect_result(104181, json!({}), None);

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
        assert!(
            bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events
        );
        assert!(
            bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        assert!(
            bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert!(
            bc.active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(bc.active_page_state().network_policy.cache_disabled());
        assert!(
            bc.active_page_state()
                .network_policy
                .bypass_service_worker()
        );
        assert!(bc.active_page_state().css_enabled);
        assert!(bc.active_page_state().fetch_owner.is_enabled());
        assert!(bc.active_page_state().fetch_owner.handle_auth_requests());
        let fetch_config = bc.active_page_state().fetch_owner.config_snapshot();
        assert_eq!(fetch_config.patterns().len(), 1);
        assert_eq!(fetch_config.patterns()[0].url_pattern, "*target-a*");
        assert_eq!(
            fetch_config.patterns()[0].resource_type_filter,
            Some(crate::conn::FetchResourceTypeFilter::Fetch)
        );
        assert_eq!(
            fetch_config.patterns()[0].request_stage,
            crate::conn::FetchRequestStage::Response
        );
    }

    ctx.process_async(json!({"id": 104182, "method": "Target.activateTarget", "params": { "targetId": second_target_id }})).await;
    ctx.expect_result(104182, json!({}), None);

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            !bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events
        );
        assert!(
            !bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        assert!(
            !bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert!(
            !bc.active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(!bc.active_page_state().network_policy.cache_disabled());
        assert!(
            !bc.active_page_state()
                .network_policy
                .bypass_service_worker()
        );
        assert!(!bc.active_page_state().css_enabled);
        assert!(!bc.active_page_state().fetch_owner.is_enabled());
        assert!(!bc.active_page_state().fetch_owner.handle_auth_requests());
        assert!(
            bc.active_page_state()
                .fetch_owner
                .config_snapshot()
                .patterns()
                .is_empty()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_network_artifacts_after_switching() {
    target_8mb_stack("same-context-network-artifacts", || async {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(
        json!({"id": 1041821, "method": "Network.enable", "sessionId": "SID-active"}),
    )
    .await;
    ctx.expect_result(1041821, json!({}), Some("SID-active"));
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.record_captured_response_body(
            "REQ-A".into(),
            "body-a".into(),
            [Some("SID-active".into())],
        );
        bc.set_next_io_stream_sequence_for_test(7);
        bc.insert_io_stream("STREAM-A".into(), b"stream-a".to_vec(), 0);
    }

    ctx.process_async(json!({"id": 104183, "method": "Target.createTarget", "params": {"browserContextId": "BID-9", "url": "about:blank#second"}})).await;
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
    ctx.expect_result(104183, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({"id": 104184, "method": "Target.attachToTarget", "params": { "targetId": second_target_id }})).await;
    let second_session_id = take_response_by_id(&mut ctx, 104184)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({"id": 104185, "method": "Target.activateTarget", "params": { "targetId": second_target_id }})).await;
    ctx.expect_result(104185, json!({}), None);

    ctx.process_async(json!({"id": 104186, "method": "Page.navigate", "sessionId": second_session_id, "params": { "url": "data:text/html,<title>second</title><div id='ok'>second target</div>" }})).await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104186);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();
    ctx.process_async(
        json!({"id": 1041861, "method": "Network.enable", "sessionId": second_session_id}),
    )
    .await;
    ctx.expect_result(1041861, json!({}), Some(&second_session_id));

    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.record_captured_response_body(
            "REQ-B".into(),
            "body-b".into(),
            [Some(second_session_id.clone())],
        );
        bc.set_next_io_stream_sequence_for_test(3);
        bc.insert_io_stream("STREAM-B".into(), b"stream-b".to_vec(), 0);
    }

    ctx.process_async(json!({"id": 104187, "method": "Target.activateTarget", "params": { "targetId": "TID-000000000A" }})).await;
    ctx.expect_result(104187, json!({}), None);
    ctx.process_async(json!({"id": 104188, "method": "Network.getResponseBody", "sessionId": "SID-active", "params": { "requestId": "REQ-A" }})).await;
    ctx.expect_result(
        104188,
        json!({ "body": "body-a", "base64Encoded": false }),
        Some("SID-active"),
    );
    ctx.process_async(json!({"id": 104189, "method": "IO.read", "sessionId": "SID-active", "params": { "handle": "STREAM-A" }})).await;
    ctx.expect_result(
        104189,
        json!({ "base64Encoded": false, "data": "stream-a", "eof": true }),
        Some("SID-active"),
    );
    ctx.process_async(json!({"id": 104190, "method": "Network.getResponseBody", "sessionId": "SID-active", "params": { "requestId": "REQ-B" }})).await;
    ctx.expect_error(104190, -32000, "No resource with given identifier found");
    ctx.process_async(json!({"id": 104191, "method": "IO.read", "sessionId": "SID-active", "params": { "handle": "STREAM-B" }})).await;
    ctx.expect_error(104191, -32000, "StreamHandleNotFound");

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.next_io_stream_sequence_for_test(), 7);
    }

    ctx.process_async(json!({"id": 104192, "method": "Target.activateTarget", "params": { "targetId": second_target_id }})).await;
    ctx.expect_result(104192, json!({}), None);
    ctx.process_async(json!({"id": 104193, "method": "Network.getResponseBody", "sessionId": second_session_id, "params": { "requestId": "REQ-B" }})).await;
    ctx.expect_result(
        104193,
        json!({ "body": "body-b", "base64Encoded": false }),
        Some(&second_session_id),
    );
    ctx.process_async(json!({"id": 104194, "method": "IO.read", "sessionId": second_session_id, "params": { "handle": "STREAM-B" }})).await;
    ctx.expect_result(
        104194,
        json!({ "base64Encoded": false, "data": "stream-b", "eof": true }),
        Some(&second_session_id),
    );
    ctx.process_async(json!({"id": 104195, "method": "Network.getResponseBody", "sessionId": second_session_id, "params": { "requestId": "REQ-A" }})).await;
    ctx.expect_error(104195, -32000, "No resource with given identifier found");
    ctx.process_async(json!({"id": 104196, "method": "IO.read", "sessionId": second_session_id, "params": { "handle": "STREAM-A" }})).await;
    ctx.expect_error(104196, -32000, "StreamHandleNotFound");

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.next_io_stream_sequence_for_test(), 3);
    }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_page_attachment_id_and_request_counters_after_switching()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.active_page_state_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(11);
        bc.set_next_network_request_sequence_for_test(41);
        bc.set_subresource_network_emitted_record_count_for_test(12);
        bc.active_page_state_mut()
            .runtime_slot
            .set_network_request_counters_for_test(4, 5);
    }

    ctx.process_async(json!({"id": 1041961, "method": "Target.createTarget", "params": {"browserContextId": "BID-9", "url": "about:blank#second"}})).await;
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
    ctx.expect_result(1041961, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({"id": 1041962, "method": "Target.attachToTarget", "params": { "targetId": second_target_id }})).await;
    let second_session_id = take_response_by_id(&mut ctx, 1041962)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({"id": 1041963, "method": "Target.activateTarget", "params": { "targetId": second_target_id }})).await;
    ctx.expect_result(1041963, json!({}), None);

    ctx.process_async(json!({"id": 1041964, "method": "Page.navigate", "sessionId": second_session_id, "params": { "url": "data:text/html,<title>second</title><div id='ok'>second target</div>" }})).await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041964);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.active_page_state_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(23);
        bc.set_next_network_request_sequence_for_test(71);
        bc.set_subresource_network_emitted_record_count_for_test(8);
        bc.active_page_state_mut()
            .runtime_slot
            .set_network_request_counters_for_test(9, 10);
    }

    ctx.process_async(json!({"id": 1041965, "method": "Target.activateTarget", "params": { "targetId": "TID-000000000A" }})).await;
    ctx.expect_result(1041965, json!({}), None);

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
        assert_eq!(
            bc.active_page_state()
                .runtime_slot
                .page_attachment_id()
                .map(crate::conn::TargetPageAttachmentId::get),
            Some(11)
        );
        assert_eq!(bc.next_network_request_sequence_for_test(), 41);
        assert_eq!(bc.subresource_network_emitted_record_count_for_test(), 12);
        assert_eq!(
            bc.active_page_state()
                .runtime_slot
                .next_fetch_request_id_for_test(),
            4
        );
        assert_eq!(
            bc.active_page_state()
                .runtime_slot
                .next_subresource_fetch_request_id_for_test(),
            5
        );
    }

    ctx.process_async(json!({"id": 1041966, "method": "Target.activateTarget", "params": { "targetId": second_target_id }})).await;
    ctx.expect_result(1041966, json!({}), None);

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert_eq!(
            bc.active_page_state()
                .runtime_slot
                .page_attachment_id()
                .map(crate::conn::TargetPageAttachmentId::get),
            Some(23)
        );
        assert_eq!(bc.next_network_request_sequence_for_test(), 71);
        assert_eq!(bc.subresource_network_emitted_record_count_for_test(), 8);
        assert_eq!(
            bc.active_page_state()
                .runtime_slot
                .next_fetch_request_id_for_test(),
            9
        );
        assert_eq!(
            bc.active_page_state()
                .runtime_slot
                .next_subresource_fetch_request_id_for_test(),
            10
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_security_identity_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.set_target_security_origin("https://a.example".into());
        bc.set_target_secure_context_type("Secure".into());
    }

    ctx.process_async(json!({"id": 1041967, "method": "Target.createTarget", "params": {"browserContextId": "BID-9", "url": "about:blank#second"}})).await;
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
    ctx.expect_result(1041967, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({"id": 1041968, "method": "Target.attachToTarget", "params": { "targetId": second_target_id }})).await;
    let second_session_id = take_response_by_id(&mut ctx, 1041968)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({"id": 1041969, "method": "Target.activateTarget", "params": { "targetId": second_target_id }})).await;
    ctx.expect_result(1041969, json!({}), None);

    ctx.process_async(json!({"id": 1041970, "method": "Page.navigate", "sessionId": second_session_id, "params": { "url": "data:text/html,<title>second</title><div id='ok'>second target</div>" }})).await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041970);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.set_target_security_origin("null".into());
        bc.set_target_secure_context_type("InsecureScheme".into());
    }

    ctx.process_async(json!({"id": 1041971, "method": "Target.activateTarget", "params": { "targetId": "TID-000000000A" }})).await;
    ctx.expect_result(1041971, json!({}), None);

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
        assert_eq!(bc.target_security_origin(), "https://a.example");
        assert_eq!(bc.target_secure_context_type(), "Secure");
    }

    ctx.process_async(json!({"id": 1041972, "method": "Page.getFrameTree"}))
        .await;
    let first_frame_tree = take_response_by_id(&mut ctx, 1041972);
    assert_eq!(
        first_frame_tree["result"]["frameTree"]["frame"]["securityOrigin"],
        json!("https://a.example")
    );
    assert_eq!(
        first_frame_tree["result"]["frameTree"]["frame"]["secureContextType"],
        json!("Secure")
    );

    ctx.process_async(json!({"id": 1041973, "method": "Target.activateTarget", "params": { "targetId": second_target_id }})).await;
    ctx.expect_result(1041973, json!({}), None);

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert_eq!(bc.target_security_origin(), "null");
        assert_eq!(bc.target_secure_context_type(), "InsecureScheme");
    }

    ctx.process_async(json!({"id": 1041974, "method": "Page.getFrameTree"}))
        .await;
    let second_frame_tree = take_response_by_id(&mut ctx, 1041974);
    assert_eq!(
        second_frame_tree["result"]["frameTree"]["frame"]["securityOrigin"],
        json!("null")
    );
    assert_eq!(
        second_frame_tree["result"]["frameTree"]["frame"]["secureContextType"],
        json!("InsecureScheme")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_search_results_after_session_scoped_owner_activity()
{
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9B-SESSION");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .set_active_target_id("TID-000000000AS");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041941,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": "data:text/html,<p>a1</p><p>a2</p>" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    ctx.expect_result(
        1041941,
        json!({ "frameId": "TID-000000000AS", "loaderId": "LID-0000000001" }),
        Some("SID-active"),
    );
    wait_until_renderer_document_load(
        &mut ctx,
        Some("SID-active"),
        "TID-000000000AS",
        "LID-0000000001",
    )
    .await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041942,
        "method": "DOM.performSearch",
        "sessionId": "SID-active",
        "params": { "query": "p" }
    }))
    .await;
    ctx.expect_result(
        1041942,
        json!({ "searchId": "0", "resultCount": 2 }),
        Some("SID-active"),
    );

    ctx.process_async(json!({
        "id": 1041943,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9B-SESSION", "url": "about:blank#second"}
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
    ctx.expect_result(1041943, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041944,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1041944)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1041945,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": "data:text/html,<span>b1</span>" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    ctx.expect_result(
        1041945,
        json!({ "frameId": second_target_id, "loaderId": "LID-0000000002" }),
        Some(&second_session_id),
    );
    wait_until_renderer_document_load(
        &mut ctx,
        Some(&second_session_id),
        &second_target_id,
        "LID-0000000002",
    )
    .await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041946,
        "method": "DOM.performSearch",
        "sessionId": second_session_id,
        "params": { "query": "span" }
    }))
    .await;
    ctx.expect_result(
        1041946,
        json!({ "searchId": "0", "resultCount": 1 }),
        Some(&second_session_id),
    );

    ctx.process_async(json!({
        "id": 1041947,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000AS" }
    }))
    .await;
    ctx.expect_result(1041947, json!({}), None);

    ctx.process_async(json!({
        "id": 1041948,
        "method": "DOM.getSearchResults",
        "sessionId": second_session_id,
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 1 }
    }))
    .await;
    let second_results = take_response_by_id(&mut ctx, 1041948);
    assert_eq!(
        second_results["result"]["nodeIds"]
            .as_array()
            .map(|ids| ids.len()),
        Some(1)
    );

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some("TID-000000000AS")
    );

    ctx.process_async(json!({
        "id": 1041949,
        "method": "DOM.getSearchResults",
        "sessionId": "SID-active",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 2 }
    }))
    .await;
    let first_results = take_response_by_id(&mut ctx, 1041949);
    assert_eq!(
        first_results["result"]["nodeIds"]
            .as_array()
            .map(|ids| ids.len()),
        Some(2)
    );

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some("TID-000000000AS")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_crash_state_after_session_scoped_owner_activity() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9C-SESSION",
        "TID-000000000CS",
        "<title>first-before-crash</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1042021,
        "method": "Inspector.enable",
        "sessionId": "SID-active"
    }))
    .await;
    ctx.expect_result(1042021, json!({}), Some("SID-active"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();

    ctx.process_async(json!({
        "id": 1042022,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9C-SESSION", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(1042022, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1042023,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1042023)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1042024,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1042024);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1042025,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000CS" }
    }))
    .await;
    ctx.expect_result(1042025, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("active browser context")
            .active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );

    ctx.process_async(json!({
        "id": 1042026,
        "method": "Inspector.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1042026, json!({}), Some(&second_session_id));
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetCrashed")
            && message["sessionId"] == json!(second_session_id)
    }));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1042027,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second-after-session-promotion</title><div id='ok'>second target</div>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 1042027);
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!(second_session_id)
    }));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1042028,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": { "expression": "document.title" }
    }))
    .await;
    let first_eval = take_response_by_id(&mut ctx, 1042028);
    assert_eq!(
        first_eval["result"]["result"]["value"],
        json!("first-before-crash")
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );

    ctx.process_async(json!({
        "id": 1042029,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>first-after-recovery</title><div id='ok'>first target recovered</div>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 1042029);
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!("SID-active")
    }));
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_restore_their_own_domain_enablement_after_session_scoped_owner_activity()
 {
    target_8mb_stack("same-context-domain-session-owner-activity", || async {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-DOMAIN-SESSION",
        "TID-000000000DS",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({"id": 1041821, "method": "Page.setLifecycleEventsEnabled", "sessionId": "SID-active", "params": { "enabled": true }})).await;
    ctx.expect_result(1041821, json!({}), Some("SID-active"));
    ctx.process_async(
        json!({"id": 1041822, "method": "Runtime.enable", "sessionId": "SID-active"}),
    )
    .await;
    ctx.expect_result(1041822, json!({}), Some("SID-active"));
    ctx.process_async(
        json!({"id": 1041823, "method": "Inspector.enable", "sessionId": "SID-active"}),
    )
    .await;
    ctx.expect_result(1041823, json!({}), Some("SID-active"));
    ctx.process_async(
        json!({"id": 1041824, "method": "Network.enable", "sessionId": "SID-active"}),
    )
    .await;
    ctx.expect_result(1041824, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 1041825, "method": "Network.setCacheDisabled", "sessionId": "SID-active", "params": { "cacheDisabled": true }})).await;
    ctx.expect_result(1041825, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 1041826, "method": "Network.setBypassServiceWorker", "sessionId": "SID-active", "params": { "bypass": true }})).await;
    ctx.expect_result(1041826, json!({}), Some("SID-active"));
    ctx.process_async(json!({"id": 1041827, "method": "CSS.enable", "sessionId": "SID-active"}))
        .await;
    ctx.expect_result(1041827, json!({}), Some("SID-active"));
    ctx.process_async(json!({
        "id": 1041828,
        "method": "Fetch.enable",
        "sessionId": "SID-active",
        "params": {
            "patterns": [{ "urlPattern": "*target-a*", "resourceType": "Fetch", "requestStage": "Response" }],
            "handleAuthRequests": true
        }
    })).await;
    ctx.expect_result(1041828, json!({}), Some("SID-active"));

    ctx.process_async(json!({"id": 1041829, "method": "Target.createTarget", "params": {"browserContextId": "BID-9-DOMAIN-SESSION", "url": "about:blank#second"}})).await;
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
    ctx.expect_result(1041829, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({"id": 1041830, "method": "Target.attachToTarget", "params": { "targetId": second_target_id }})).await;
    let second_session_id = take_response_by_id(&mut ctx, 1041830)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({"id": 1041831, "method": "Page.navigate", "sessionId": second_session_id, "params": { "url": "data:text/html,<title>second</title><div id='ok'>second target</div>" }})).await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1041831);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(
        json!({"id": 1041832, "method": "Runtime.enable", "sessionId": second_session_id}),
    )
    .await;
    ctx.expect_result(1041832, json!({}), Some(&second_session_id));
    ctx.process_async(
        json!({"id": 1041833, "method": "Network.enable", "sessionId": second_session_id}),
    )
    .await;
    ctx.expect_result(1041833, json!({}), Some(&second_session_id));
    ctx.process_async(json!({
        "id": 1041834,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": {
            "patterns": [{ "urlPattern": "*target-b*", "resourceType": "XHR", "requestStage": "Request" }],
            "handleAuthRequests": false
        }
    })).await;
    ctx.expect_result(1041834, json!({}), Some(&second_session_id));

    ctx.process_async(json!({"id": 1041835, "method": "Target.activateTarget", "params": { "targetId": "TID-000000000DS" }})).await;
    ctx.expect_result(1041835, json!({}), None);
    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000DS"));
        assert!(bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].page_session_state.page_lifecycle_events);
        assert!(bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].runtime_session_state.runtime_frontend_enabled);
        assert!(bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].runtime_session_state.inspector_enabled);
        assert!(
            bc.active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(bc.active_page_state().network_policy.cache_disabled());
        assert!(bc.active_page_state().network_policy.bypass_service_worker());
        assert!(bc.active_page_state().css_enabled);
        assert!(bc.active_page_state().fetch_owner.is_enabled());
        assert!(bc.active_page_state().fetch_owner.handle_auth_requests());
        let fetch_config = bc.active_page_state().fetch_owner.config_snapshot();
        assert_eq!(fetch_config.patterns().len(), 1);
        assert_eq!(fetch_config.patterns()[0].url_pattern, "*target-a*");
        assert_eq!(
            fetch_config.patterns()[0].resource_type_filter,
            Some(crate::conn::FetchResourceTypeFilter::Fetch)
        );
        assert_eq!(
            fetch_config.patterns()[0].request_stage,
            crate::conn::FetchRequestStage::Response
        );
    }

    ctx.process_async(
        json!({"id": 1041836, "method": "Page.bringToFront", "sessionId": second_session_id}),
    )
    .await;
    let _ = take_response_by_id(&mut ctx, 1041836);
    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context after session-scoped promotion");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert!(!bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].page_session_state.page_lifecycle_events);
        assert!(bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].runtime_session_state.runtime_frontend_enabled);
        assert!(!bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].runtime_session_state.inspector_enabled);
        assert!(
            bc.active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(!bc.active_page_state().network_policy.cache_disabled());
        assert!(!bc.active_page_state().network_policy.bypass_service_worker());
        assert!(!bc.active_page_state().css_enabled);
        assert!(bc.active_page_state().fetch_owner.is_enabled());
        assert!(!bc.active_page_state().fetch_owner.handle_auth_requests());
        let fetch_config = bc.active_page_state().fetch_owner.config_snapshot();
        assert_eq!(fetch_config.patterns().len(), 1);
        assert_eq!(fetch_config.patterns()[0].url_pattern, "*target-b*");
        assert_eq!(
            fetch_config.patterns()[0].resource_type_filter,
            Some(crate::conn::FetchResourceTypeFilter::Xhr)
        );
        assert_eq!(
            fetch_config.patterns()[0].request_stage,
            crate::conn::FetchRequestStage::Request
        );
    }

    ctx.process_async(
        json!({"id": 1041837, "method": "Page.bringToFront", "sessionId": "SID-active"}),
    )
    .await;
    let _ = take_response_by_id(&mut ctx, 1041837);
    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context after restoring first target");
        assert_eq!(bc.active_target_id(), Some("TID-000000000DS"));
        assert!(bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].page_session_state.page_lifecycle_events);
        assert!(bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].runtime_session_state.runtime_frontend_enabled);
        assert!(bc.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary].runtime_session_state.inspector_enabled);
        assert!(
            bc.active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(bc.active_page_state().network_policy.cache_disabled());
        assert!(bc.active_page_state().network_policy.bypass_service_worker());
        assert!(bc.active_page_state().css_enabled);
        assert!(bc.active_page_state().fetch_owner.is_enabled());
        assert!(bc.active_page_state().fetch_owner.handle_auth_requests());
        let fetch_config = bc.active_page_state().fetch_owner.config_snapshot();
        assert_eq!(fetch_config.patterns().len(), 1);
        assert_eq!(fetch_config.patterns()[0].url_pattern, "*target-a*");
        assert_eq!(
            fetch_config.patterns()[0].resource_type_filter,
            Some(crate::conn::FetchResourceTypeFilter::Fetch)
        );
        assert_eq!(
            fetch_config.patterns()[0].request_stage,
            crate::conn::FetchRequestStage::Response
        );
    }
    })
    .await;
}
