use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_true_ensures_existing_background_initial_document_before_attached_event() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-active",
        "<!doctype html><title>active</title>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.stage_background_target(
            "TID-background-pending".to_owned(),
            None,
            "about:blank#background".to_owned(),
            None,
            None,
        );
    }
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .pending_document_page_build_count(),
        1,
        "staged background target should begin as target lifecycle pending initial document"
    );

    ctx.process_async(json!({
        "id": 10330,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(10330, json!({}), None);
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        "TID-background-pending"
    );
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            bc.pending_document_page_build_count(),
            0,
            "auto-attach must complete background initial document before emitting attachedToTarget"
        );
        assert!(
            bc.background_target("TID-background-pending")
                .expect("background target")
                .has_loaded_page(),
            "attached background target should expose a current Page immediately"
        );
    }

    ctx.process_async(json!({
        "id": 10331,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": { "expression": "document.URL" }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 10331);
    assert_eq!(
        evaluation["result"]["result"]["value"],
        json!("about:blank#background")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_true_still_reports_transient_no_page_target_like_chromium() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-active",
        "<!doctype html><title>active</title>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-background-in-transit".to_owned(),
            None,
            crate::conn::TargetIdentityState::new(
                "https://example.test/in-transit".to_owned(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
    }

    ctx.process_async(json!({
        "id": 10332,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(10332, json!({}), None);
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        "TID-background-in-transit"
    );
    assert_eq!(
        attached["params"]["targetInfo"]["url"],
        "https://example.test/in-transit"
    );
    assert_eq!(
        attached["params"]["targetInfo"]["attached"],
        json!(true),
        "auto-attach must expose the session even before a Page is materialized"
    );
    assert!(
        ctx.sent.is_empty(),
        "transient no-page auto-attach should not emit unsolicited protocol errors: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_on_auto_attached_background_target_session_routes_without_promotion_when_active_target_has_no_loaded_page()
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
        "id": 1034,
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
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(1034, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1035,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": { "name": "patchedBinding" }
    }))
    .await;
    ctx.expect_result(1035, json!({}), Some(&session_id));

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert_eq!(
        bc.background_target(&second_target_id)
            .and_then(|target| target.session_id()),
        Some(session_id.as_str())
    );
    assert!(
        ctx.conn
            .target_devtools_session_state_for_session(Some(&session_id))
            .expect("background DevTools session state")
            .runtime_bindings
            .iter()
            .any(|binding| binding.name == "patchedBinding"),
        "binding definition should persist on the background DevTools session without promotion"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_then_navigation_on_auto_attached_background_target_session_rehydrates_binding_after_initial_document()
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
        "id": 10351,
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
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10351, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10352,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": { "name": "patchedBinding" }
    }))
    .await;
    ctx.expect_result(10352, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 10353,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<title>binding-after-initial-document</title><div id='ok'>binding after initial document</div>"
        }
    })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 10353);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10354,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "globalThis.patchedBinding('after-nav'); JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 10354);
    assert_eq!(
        evaluation["result"]["result"]["type"],
        json!("string"),
        "expected string evaluation payload after background-target navigation, got: {evaluation:?}"
    );
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("binding-after-initial-document"));
    assert_eq!(payload["text"], json!("binding after initial document"));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("patchedBinding")
        })
        .cloned()
        .expect("binding should stay callable after initial document and first navigation");
    assert_eq!(binding_called["params"]["payload"], json!("after-nav"));
    assert_eq!(binding_called["sessionId"], json!(session_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_and_preload_on_auto_attached_background_target_session_run_on_first_navigation_after_initial_document()
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
        "id": 10355,
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
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10355, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10356,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": { "name": "preDocumentBinding" }
    }))
    .await;
    ctx.expect_result(10356, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 10357,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "globalThis.__lm_pre_document_binding_kind = typeof globalThis.preDocumentBinding; if (typeof globalThis.preDocumentBinding === 'function') globalThis.preDocumentBinding('payload-preload');"
        }
    })).await;
    let add_script = take_response_by_id(&mut ctx, 10357);
    assert!(
        add_script["result"]["identifier"].as_str().is_some(),
        "pre-document preload should return an identifier: {add_script:?}"
    );

    ctx.process_async(json!({
        "id": 10358,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<title>pre-document-after-initial-document</title><body><div id='ok'>pre-document initial document target</div></body>"
        }
    })).await;
    let navigation = take_response_by_id(&mut ctx, 10358);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    let binding_called = ctx
        .wait_for_scheduler_message("pre-document binding invocation", |message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("preDocumentBinding")
        })
        .await;
    assert_eq!(
        binding_called["params"]["payload"],
        json!("payload-preload")
    );
    assert_eq!(binding_called["sessionId"], json!(session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "pre-document background session should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10359,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "JSON.stringify({ kind: globalThis.__lm_pre_document_binding_kind, title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 10359);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["kind"], json!("function"));
    assert_eq!(
        payload["title"],
        json!("pre-document-after-initial-document")
    );
    assert_eq!(
        payload["text"],
        json!("pre-document initial document target")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_and_utility_preload_on_auto_attached_background_target_session_run_when_utility_world_first_materializes_after_initial_document()
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
        "id": 10369,
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
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10369, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10370,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": {
            "name": "utilityPreDocumentBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10370, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 10371,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": r#"
                globalThis.__lm_utility_binding_kind = typeof globalThis.utilityPreDocumentBinding;
                if (typeof globalThis.utilityPreDocumentBinding === 'function')
                    globalThis.utilityPreDocumentBinding('payload-utility-preload');
            "#,
            "worldName": "utility"
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 10371);
    assert!(add_script["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10372,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<title>utility-pre-document-after-initial-document</title><body><div id='ok'>utility pre-document initial document target</div></body>"
        }
    })).await;
    let navigation = take_response_by_id(&mut ctx, 10372);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    let utility_binding_called_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("utilityPreDocumentBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "utility pre-document background session should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10373,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 10373)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    let binding_called = utility_binding_called_during_navigation.or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("utilityPreDocumentBinding")
                })
                .cloned()
        })
        .expect("utility-world preload should call the scoped binding when the world first materializes");
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    assert_eq!(
        binding_called["params"]["payload"],
        json!("payload-utility-preload")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10374,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "JSON.stringify({ kind: globalThis.__lm_utility_binding_kind, title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 10374);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["kind"], json!("function"));
    assert_eq!(
        payload["title"],
        json!("utility-pre-document-after-initial-document")
    );
    assert_eq!(
        payload["text"],
        json!("utility pre-document initial document target")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_and_utility_preload_then_remove_on_auto_attached_background_target_session_prevent_first_utility_world_replay_after_initial_document()
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
        "id": 10375,
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
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10375, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10376,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": {
            "name": "temporaryUtilityPreDocumentBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10376, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 10377,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": r#"
                globalThis.__lm_removed_utility_binding_kind = typeof globalThis.temporaryUtilityPreDocumentBinding;
                globalThis.__lm_removed_utility_preload = 'ready';
                if (typeof globalThis.temporaryUtilityPreDocumentBinding === 'function')
                    globalThis.temporaryUtilityPreDocumentBinding('unexpected-utility-preload');
            "#,
            "worldName": "utility"
        }
    })).await;
    let add_script = take_response_by_id(&mut ctx, 10377);
    let identifier = add_script["result"]["identifier"]
        .as_str()
        .expect("preload identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 10378,
        "method": "Runtime.removeBinding",
        "sessionId": session_id,
        "params": { "name": "temporaryUtilityPreDocumentBinding" }
    }))
    .await;
    ctx.expect_result(10378, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 10379,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": { "identifier": identifier }
    }))
    .await;
    let remove_script = take_response_by_id(&mut ctx, 10379);
    assert_eq!(remove_script["result"], json!({}));

    ctx.process_async(json!({
        "id": 10380,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<title>removed-utility-pre-document-after-initial-document</title><body><div id='ok'>removed utility pre-document state</div></body>"
        }
    })).await;
    let navigation = take_response_by_id(&mut ctx, 10380);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.bindingCalled")
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "removed utility-world binding/preload should not replay during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10381,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 10381)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.bindingCalled")
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "removed utility-world binding/preload should stay absent when utility world first materializes: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10382,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "JSON.stringify({ bindingType: typeof globalThis.temporaryUtilityPreDocumentBinding, preload: globalThis.__lm_removed_utility_preload ?? 'absent', title: document.title, text: document.getElementById('ok').textContent })"
        }
    })).await;
    let evaluation = take_response_by_id(&mut ctx, 10382);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["bindingType"], json!("undefined"));
    assert_eq!(payload["preload"], json!("absent"));
    assert_eq!(
        payload["title"],
        json!("removed-utility-pre-document-after-initial-document")
    );
    assert_eq!(payload["text"], json!("removed utility pre-document state"));
}
