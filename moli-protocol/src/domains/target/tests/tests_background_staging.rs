use super::*;
use axum::extract::State;
use axum::http::Uri;

fn is_runtime_context_event(message: &serde_json::Value) -> bool {
    matches!(
        message["method"].as_str(),
        Some("Runtime.executionContextsCleared") | Some("Runtime.executionContextCreated")
    )
}

fn take_staged_about_blank_runtime_context(
    ctx: &mut TestContext,
    session_id: &str,
    target_id: &str,
) {
    let created = ctx.take_first_matching(
        "staged background Runtime.executionContextCreated",
        |message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
        },
    );
    assert_eq!(
        created["params"]["context"]["name"],
        json!("about:blank#second")
    );
    assert_eq!(
        created["params"]["context"]["auxData"]["frameId"],
        json!(target_id)
    );
    assert!(
        created["params"]["context"]["uniqueId"].as_str().is_some(),
        "staged about:blank context should come from V8 native Runtime.enable replay: {created:?}"
    );
}

async fn wait_for_session_main_document_loading_finished(
    ctx: &mut TestContext,
    session_id: &str,
    request_url: &str,
    description: &str,
) {
    let request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(session_id)
                && message["params"]["request"]["url"] == json!(request_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .unwrap_or_else(|| {
            panic!(
                "main-document request should precede its completion: {:?}",
                ctx.sent
            )
        })
        .to_owned();
    crate::testing::wait_until_messages(ctx, Some(session_id), description, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["sessionId"] == json!(session_id)
                && message["params"]["requestId"] == json!(request_id)
        })
    })
    .await;
}

async fn loaded_page_html_for_test(ctx: &mut TestContext) -> String {
    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|bc| bc.active_page_state_mut().runtime_slot.loaded_page_mut())
        .expect("loaded page");
    page.serialize_html_async()
        .await
        .expect("loaded page should serialize HTML")
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_pre_document_state_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE",
        "TID-000000000PA",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 104185,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104185, json!({}), None);
    ctx.process_async(json!({
        "id": 104186,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104186, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104188,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBPreDocumentBinding" }
    }))
    .await;
    ctx.expect_result(104188, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 104189,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.targetBPreload = 'from-target-b'; if (typeof globalThis.targetBPreDocumentBinding === 'function') globalThis.targetBPreDocumentBinding('from-preload');"
            }
        })).await;
    let add_script = take_response_by_id(&mut ctx, 104189);
    let script_id = add_script["result"]["identifier"]
        .as_str()
        .expect("script identifier")
        .to_owned();

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_ne!(active.active_target_id(), Some(second_target_id.as_str()));
        let staged = active
            .background_target(&second_target_id)
            .expect("staged background target");
        let staged_devtools_state = active
            .parked_page_session_state(staged.target_id())
            .expect("staged page session state")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .as_slice();
        assert_eq!(staged_devtools_state.len(), 1);
        assert_eq!(staged_devtools_state[0].name, "targetBPreDocumentBinding");
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts
                .len(),
            1
        );
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts[0]
                .0,
            script_id
        );
        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_bindings
                .is_empty(),
            "active target DevTools session must not inherit staged target binding"
        );
        assert!(
            active
                .active_page_state()
                .owner_state
                .document_start_scripts
                .is_empty()
        );
    }

    ctx.process_async(json!({
            "id": 104190,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBPreDocumentBinding, preload: globalThis.targetBPreload ?? 'absent' })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 104190);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["binding"], json!("undefined"));
    assert_eq!(active_payload["preload"], json!("absent"));

    ctx.process_async(json!({
        "id": 104191,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104191, json!({}), None);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
    }

    ctx.process_async(json!({
        "id": 104192,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>promoted</title><div id='ok'>promoted target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104192);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));

    let binding_called = ctx
        .take_all()
        .into_iter()
        .find(|message| message["method"] == json!("Runtime.bindingCalled"))
        .expect("binding call from pre-document script");
    assert_eq!(binding_called["sessionId"], json!(second_session_id));
    assert_eq!(
        binding_called["params"]["name"],
        json!("targetBPreDocumentBinding")
    );
    let payload = binding_called["params"]["payload"]
        .as_str()
        .expect("binding payload should be string");
    assert_eq!(payload, "from-preload");

    ctx.process_async(json!({
            "id": 104193,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBPreDocumentBinding, preload: globalThis.targetBPreload, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104193);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["binding"], json!("function"));
    assert_eq!(promoted_payload["preload"], json!("from-target-b"));
    assert_eq!(promoted_payload["text"], json!("promoted target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_utility_pre_document_state_before_promotion()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-UTILITY",
        "TID-000000000PU",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194120,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194120, json!({}), None);

    ctx.process_async(json!({
        "id": 104194121,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-UTILITY", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194121, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194122,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "targetBUtilityPreDocumentBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104194122, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 104194123,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.targetBUtilityPreload = 'from-target-b-utility'; if (typeof globalThis.targetBUtilityPreDocumentBinding === 'function') globalThis.targetBUtilityPreDocumentBinding('from-utility-preload');",
                "worldName": "utility"
            }
        })).await;
    let add_script = take_response_by_id(&mut ctx, 104194123);
    let script_id = add_script["result"]["identifier"]
        .as_str()
        .expect("script identifier")
        .to_owned();

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        let staged = active
            .background_target(&second_target_id)
            .expect("staged background target");
        let staged_devtools_state = active
            .parked_page_session_state(staged.target_id())
            .expect("staged page session state")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .as_slice();
        assert_eq!(staged_devtools_state.len(), 1);
        assert_eq!(
            staged_devtools_state[0].execution_context_name.as_deref(),
            Some("utility")
        );
        assert_eq!(
            staged_devtools_state[0].name,
            "targetBUtilityPreDocumentBinding"
        );
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts
                .len(),
            1
        );
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts[0]
                .0,
            script_id
        );
        assert!(
            active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_bindings
                .is_empty(),
            "active target DevTools session must not inherit staged target binding"
        );
        assert!(
            active
                .active_page_state()
                .owner_state
                .document_start_scripts
                .is_empty()
        );
    }

    ctx.process_async(json!({
        "id": 104194124,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-active",
        "params": {
            "frameId": "TID-000000000PU",
            "worldName": "utility"
        }
    }))
    .await;
    let active_utility_context =
        take_response_by_id(&mut ctx, 104194124)["result"]["executionContextId"]
            .as_i64()
            .expect("active utility context id");
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "active target utility world should not materialize staged target B utility state: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 104194125,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "contextId": active_utility_context,
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBUtilityPreDocumentBinding, preload: globalThis.targetBUtilityPreload ?? 'absent', text: document.getElementById('ok').textContent })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 104194125);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["binding"], json!("undefined"));
    assert_eq!(active_payload["preload"], json!("absent"));
    assert_eq!(active_payload["text"], json!("active target"));

    ctx.process_async(json!({
        "id": 104194126,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PU" }
    }))
    .await;
    ctx.expect_result(104194126, json!({}), None);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194127,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>promoted</title><div id='ok'>promoted utility target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194127);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    let binding_called_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityPreDocumentBinding")
        })
        .cloned();
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194128,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context = take_response_by_id(&mut ctx, 104194128)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    let binding_called = binding_called_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetBUtilityPreDocumentBinding")
                })
                .cloned()
        })
        .expect("promoted target utility world should materialize its staged binding/preload");
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context)
    );
    assert_eq!(
        binding_called["params"]["payload"],
        json!("from-utility-preload")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 104194129,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "contextId": utility_context,
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBUtilityPreDocumentBinding, preload: globalThis.targetBUtilityPreload, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104194129);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["binding"], json!("function"));
    assert_eq!(promoted_payload["preload"], json!("from-target-b-utility"));
    assert_eq!(promoted_payload["text"], json!("promoted utility target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_binding_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE",
        "TID-000000000PRM",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194100,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194100, json!({}), None);

    ctx.process_async(json!({
        "id": 104194101,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-REMOVE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194101, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194102,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBRemovedBinding" }
    }))
    .await;
    ctx.expect_result(104194102, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 104194103,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.targetBRemovedBindingPreload = typeof globalThis.targetBRemovedBinding;"
            }
        })).await;
    let add_script = take_response_by_id(&mut ctx, 104194103);
    let script_id = add_script["result"]["identifier"]
        .as_str()
        .expect("script identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 104194104,
        "method": "Runtime.removeBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBRemovedBinding" }
    }))
    .await;
    ctx.expect_result(104194104, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        let staged = active
            .background_target(&second_target_id)
            .expect("staged background target");
        let staged_bindings_empty = active
            .parked_page_session_state(staged.target_id())
            .is_none_or(|state| {
                state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_bindings
                    .is_empty()
            });
        assert!(
            staged_bindings_empty,
            "removed binding should be cleared from parked DevTools session"
        );
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts
                .len(),
            1
        );
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts[0]
                .0,
            script_id
        );
    }

    ctx.process_async(json!({
        "id": 104194105,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRM" }
    }))
    .await;
    ctx.expect_result(104194105, json!({}), None);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194106,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>promoted</title><div id='ok'>promoted target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194106);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));

    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed binding should not replay into first promoted navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
            "id": 104194107,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBRemovedBinding, preload: globalThis.targetBRemovedBindingPreload, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104194107);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["binding"], json!("undefined"));
    assert_eq!(promoted_payload["preload"], json!("undefined"));
    assert_eq!(promoted_payload["text"], json!("promoted target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_preload_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE-SCRIPT",
        "TID-000000000PRS",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194110,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194110, json!({}), None);

    ctx.process_async(json!({
        "id": 104194111,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-REMOVE-SCRIPT", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194111, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194112,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBRemainingBinding" }
    }))
    .await;
    ctx.expect_result(104194112, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 104194113,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.targetBRemovedPreload = 'from-target-b'; globalThis.targetBRemainingBinding('from-preload');"
            }
        })).await;
    let add_script = take_response_by_id(&mut ctx, 104194113);
    let script_id = add_script["result"]["identifier"]
        .as_str()
        .expect("script identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 104194114,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "identifier": script_id }
    }))
    .await;
    ctx.expect_result(104194114, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        let staged = active
            .background_target(&second_target_id)
            .expect("staged background target");
        let staged_bindings = &active
            .parked_page_session_state(staged.target_id())
            .expect("staged page session state")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings;
        assert_eq!(staged_bindings.len(), 1);
        assert!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts
                .is_empty()
        );
    }

    ctx.process_async(json!({
        "id": 104194115,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRS" }
    }))
    .await;
    ctx.expect_result(104194115, json!({}), None);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194116,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>promoted</title><div id='ok'>promoted target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194116);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));

    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed preload should not trigger binding call during first promoted navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
            "id": 104194117,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBRemainingBinding, preload: globalThis.targetBRemovedPreload ?? 'absent', text: document.getElementById('ok').textContent })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104194117);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["binding"], json!("function"));
    assert_eq!(promoted_payload["preload"], json!("absent"));
    assert_eq!(promoted_payload["text"], json!("promoted target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_utility_binding_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE-UTILITY-BINDING",
        "TID-000000000PRU",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194130,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194130, json!({}), None);

    ctx.process_async(json!({
            "id": 104194131,
            "method": "Target.createTarget",
            "params": {
            "background": true, "browserContextId": "BID-9-PRE-REMOVE-UTILITY-BINDING", "url": "about:blank#second"}
        })).await;
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194131, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194132,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "targetBRemovedUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104194132, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 104194133,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.targetBRemovedUtilityBindingType = typeof globalThis.targetBRemovedUtilityBinding;",
                "worldName": "utility"
            }
        })).await;
    let add_script = take_response_by_id(&mut ctx, 104194133);
    let script_id = add_script["result"]["identifier"]
        .as_str()
        .expect("script identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 104194134,
        "method": "Runtime.removeBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBRemovedUtilityBinding" }
    }))
    .await;
    ctx.expect_result(104194134, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        let staged = active
            .background_target(&second_target_id)
            .expect("staged background target");
        let staged_bindings_empty = active
            .parked_page_session_state(staged.target_id())
            .is_none_or(|state| {
                state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_bindings
                    .is_empty()
            });
        assert!(
            staged_bindings_empty,
            "removed utility binding should be cleared from parked DevTools session"
        );
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts
                .len(),
            1
        );
        assert_eq!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts[0]
                .0,
            script_id
        );
    }

    ctx.process_async(json!({
        "id": 104194135,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRU" }
    }))
    .await;
    ctx.expect_result(104194135, json!({}), None);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194136,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>promoted</title><div id='ok'>promoted utility target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194136);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed utility binding should not replay into first promoted utility world: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194137,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context = take_response_by_id(&mut ctx, 104194137)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed utility binding should stay removed when utility world materializes: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 104194138,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "contextId": utility_context,
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBRemovedUtilityBinding, preload: globalThis.targetBRemovedUtilityBindingType, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104194138);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["binding"], json!("undefined"));
    assert_eq!(promoted_payload["preload"], json!("undefined"));
    assert_eq!(promoted_payload["text"], json!("promoted utility target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_utility_preload_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE-UTILITY-SCRIPT",
        "TID-000000000PRV",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194140,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194140, json!({}), None);

    ctx.process_async(json!({
            "id": 104194141,
            "method": "Target.createTarget",
            "params": {
            "background": true, "browserContextId": "BID-9-PRE-REMOVE-UTILITY-SCRIPT", "url": "about:blank#second"}
        })).await;
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194141, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194142,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "targetBRemainingUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(104194142, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 104194143,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.targetBRemovedUtilityPreload = 'from-target-b-utility'; globalThis.targetBRemainingUtilityBinding('from-utility-preload');",
                "worldName": "utility"
            }
        })).await;
    let add_script = take_response_by_id(&mut ctx, 104194143);
    let script_id = add_script["result"]["identifier"]
        .as_str()
        .expect("script identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 104194144,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": second_session_id,
        "params": { "identifier": script_id }
    }))
    .await;
    ctx.expect_result(104194144, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        let staged = active
            .background_target(&second_target_id)
            .expect("staged background target");
        let staged_bindings = &active
            .parked_page_session_state(staged.target_id())
            .expect("staged page session state")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings;
        assert_eq!(staged_bindings.len(), 1);
        assert_eq!(
            staged_bindings[0].execution_context_name.as_deref(),
            Some("utility")
        );
        assert!(
            active
                .parked_target_owner_state_or_default(staged.target_id())
                .document_start_scripts
                .is_empty()
        );
    }

    ctx.process_async(json!({
        "id": 104194145,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRV" }
    }))
    .await;
    ctx.expect_result(104194145, json!({}), None);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194146,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>promoted</title><div id='ok'>promoted utility target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194146);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed utility preload should not trigger binding call during first promoted navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194147,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context = take_response_by_id(&mut ctx, 104194147)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed utility preload should not trigger when utility world materializes: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 104194148,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "contextId": utility_context,
                "expression": "JSON.stringify({ binding: typeof globalThis.targetBRemainingUtilityBinding, preload: globalThis.targetBRemovedUtilityPreload ?? 'absent', text: document.getElementById('ok').textContent })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104194148);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["binding"], json!("function"));
    assert_eq!(promoted_payload["preload"], json!("absent"));
    assert_eq!(promoted_payload["text"], json!("promoted utility target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_emulated_media_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-MEDIA",
        "TID-000000000PM",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 1041940,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041940, json!({}), None);

    ctx.process_async(json!({
        "id": 1041941,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": "SID-active",
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(1041941, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 1041942,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-MEDIA", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041942, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041943,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "expression": "String(matchMedia('(prefers-color-scheme: dark)').matches)"
        }
    }))
    .await;
    let active_before = take_response_by_id(&mut ctx, 1041943);
    assert_eq!(active_before["result"]["result"]["value"], json!("true"));

    ctx.process_async(json!({
        "id": 1041944,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": second_session_id,
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "light" }
            ]
        }
    }))
    .await;
    ctx.expect_result(1041944, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PM"));
        assert_eq!(
            active
                .active_page_state()
                .emulated_media
                .color_scheme
                .as_deref(),
            Some("dark")
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert_eq!(staged.emulated_media.color_scheme.as_deref(), Some("light"));
    }

    ctx.process_async(json!({
        "id": 1041945,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "expression": "String(matchMedia('(prefers-color-scheme: dark)').matches)"
        }
    }))
    .await;
    let active_after = take_response_by_id(&mut ctx, 1041945);
    assert_eq!(active_after["result"]["result"]["value"], json!("true"));

    ctx.process_async(json!({
        "id": 1041946,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PM"}
    }))
    .await;
    ctx.expect_result(1041946, json!({ "success": true }), None);

    ctx.process_async(json!({
            "id": 1041947,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = [String(matchMedia('(prefers-color-scheme: dark)').matches), String(matchMedia('(prefers-color-scheme: light)').matches)].join('|');</script></body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041947);
    ctx.take_all();

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains(">false|true<"), "got {html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_emulated_media_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-MEDIA-CLEAR",
        "TID-000000000PMC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 10419441,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(10419441, json!({}), None);

    ctx.process_async(json!({
            "id": 10419442,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify([String(matchMedia('(prefers-color-scheme: dark)').matches), String(matchMedia('(prefers-color-scheme: light)').matches)])"
            }
        })).await;
    let default_surface = take_response_by_id(&mut ctx, 10419442);
    let default_surface = default_surface["result"]["result"]["value"]
        .as_str()
        .expect("default surface should be string")
        .to_owned();
    let default_surface: serde_json::Value =
        serde_json::from_str(&default_surface).expect("default surface should be valid json");

    ctx.process_async(json!({
        "id": 10419443,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-MEDIA-CLEAR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(10419443, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10419444,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": second_session_id,
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(10419444, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10419445,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": second_session_id,
        "params": {}
    }))
    .await;
    ctx.expect_result(10419445, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PMC"));
        assert!(
            active
                .active_page_state()
                .emulated_media
                .color_scheme
                .is_none(),
            "active target should keep its default emulated media",
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "clearing staged emulated media back to default should fold away the parked state entry",
        );
    }

    ctx.process_async(json!({
        "id": 10419447,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PMC"}
    }))
    .await;
    ctx.expect_result(10419447, json!({ "success": true }), None);

    ctx.process_async(json!({
            "id": 10419448,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = [String(matchMedia('(prefers-color-scheme: dark)').matches), String(matchMedia('(prefers-color-scheme: light)').matches)].join('|');</script></body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 10419448);
    ctx.take_all();

    let html = loaded_page_html_for_test(&mut ctx).await;
    let promoted_surface = html
        .split("<body>")
        .nth(1)
        .and_then(|tail| tail.split("</body>").next())
        .expect("promoted payload should be embedded in body");
    let promoted_surface = serde_json::json!(
        promoted_surface
            .split('|')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        promoted_surface, default_surface,
        "promoted target should observe default emulated media after clearing its staged override; got {html}"
    );
    assert_ne!(
        promoted_surface,
        serde_json::json!(["true", "false"]),
        "promoted target should not retain the staged dark override"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_network_conditions_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET",
        "TID-000000000PN",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 10419450,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(10419450, json!({}), None);

    ctx.process_async(json!({
        "id": 10419451,
        "method": "Network.emulateNetworkConditions",
        "sessionId": "SID-active",
        "params": {
            "offline": false,
            "latency": 10,
            "downloadThroughput": 4096,
            "uploadThroughput": 2048,
            "connectionType": "wifi"
        }
    }))
    .await;
    ctx.expect_result(10419451, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419452,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(10419452, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10419453,
        "method": "Network.emulateNetworkConditions",
        "sessionId": second_session_id,
        "params": {
            "offline": true,
            "latency": 25,
            "downloadThroughput": 1024,
            "uploadThroughput": 256,
            "connectionType": "cellular3g"
        }
    }))
    .await;
    ctx.expect_result(10419453, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PN"));
        assert!(!active.active_page_state().network_policy.network_offline());
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .emulated_network_latency(),
            10.0
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.network_policy.network_offline());
        assert_eq!(staged.network_policy.emulated_network_latency(), 25.0);
        assert_eq!(staged.network_policy.emulated_download_throughput(), 1024.0);
        assert_eq!(staged.network_policy.emulated_upload_throughput(), 256.0);
        assert_eq!(
            staged.network_policy.emulated_connection_type(),
            Some("cellular3g")
        );
    }

    ctx.process_async(json!({
            "id": 10419454,
            "method": "Page.navigate",
            "sessionId": "SID-active",
            "params": {
                "url": "data:text/html,<title>active-still-online</title><div id='ok'>active target still online</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let active_navigation = take_response_by_id(&mut ctx, 10419454);
    assert_eq!(
        active_navigation["result"]["frameId"],
        json!("TID-000000000PN")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10419455,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PN"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419455);
    ctx.take_all();

    {
        let promoted = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert_eq!(promoted.active_target_id(), Some(second_target_id.as_str()));
        assert_eq!(
            promoted.active_session_id(),
            Some(second_session_id.as_str())
        );
        assert!(
            promoted
                .active_page_state()
                .network_policy
                .network_offline()
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_network_latency(),
            25.0
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_download_throughput(),
            1024.0
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_upload_throughput(),
            256.0
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_connection_type(),
            Some("cellular3g")
        );
    }

    ctx.process_async(json!({
        "id": 10419456,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": "http://example.test/offline-promoted" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let promoted_navigation = take_response_by_id(&mut ctx, 10419456);
    assert_eq!(
        promoted_navigation["error"]["message"],
        json!("Network emulation offline")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_blocked_urls_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-BLOCK",
        "TID-000000000PB",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 10419457,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(10419457, json!({}), None);

    ctx.process_async(json!({
        "id": 10419458,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-BLOCK", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(10419458, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10419459,
        "method": "Network.setBlockedURLs",
        "sessionId": second_session_id,
        "params": { "urls": ["http://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(10419459, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active
                .active_page_state()
                .network_policy
                .blocked_url_patterns()
                .is_empty(),
            "active target should keep its own block list"
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(
            staged.network_policy.blocked_url_patterns().is_empty(),
            "a disabled Network handler must not contribute to effective target policy"
        );
        assert_eq!(
            staged
                .devtools_sessions
                .primary()
                .network_session_state
                .blocked_url_patterns,
            ["http://example.test/blocked/*".to_owned()],
            "the disabled handler must retain its staged contribution until enable"
        );
    }

    ctx.process_async(json!({
        "id": 10419460,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PB"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419460);
    ctx.take_all();

    {
        let promoted = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert_eq!(promoted.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            promoted
                .active_page_state()
                .network_policy
                .blocked_url_patterns()
                .is_empty(),
            "promotion must not activate a disabled Network handler"
        );
    }

    ctx.process_async(json!({
        "id": 10419461,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(10419461, json!({}), Some(&second_session_id));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("promoted browser context")
            .active_page_state()
            .network_policy
            .blocked_url_patterns(),
        ["http://example.test/blocked/*".to_owned()],
        "Network.enable must activate the staged background-session contribution"
    );

    ctx.process_async(json!({
        "id": 10419462,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": "http://example.test/blocked/page" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let _ = ctx.take_one();
    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");
    let promoted_navigation = take_response_by_id(&mut ctx, 10419462);
    assert_eq!(
        promoted_navigation["error"]["message"],
        json!("net::ERR_BLOCKED_BY_CLIENT")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_reset_its_own_network_conditions_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-RESET",
        "TID-000000000PR",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 104194501,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194501, json!({}), None);

    ctx.process_async(json!({
        "id": 104194502,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-RESET", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194502, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194503,
        "method": "Network.emulateNetworkConditions",
        "sessionId": second_session_id,
        "params": {
            "offline": true,
            "latency": 250,
            "downloadThroughput": 1024,
            "uploadThroughput": 512,
            "connectionType": "cellular3g"
        }
    }))
    .await;
    ctx.expect_result(104194503, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194504,
        "method": "Network.emulateNetworkConditions",
        "sessionId": second_session_id,
        "params": {
            "offline": false,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1,
            "connectionType": "none"
        }
    }))
    .await;
    ctx.expect_result(104194504, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PR"));
        assert!(
            !active.active_page_state().network_policy.network_offline(),
            "active target should keep its default online state",
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(!staged.network_policy.network_offline());
        assert_eq!(staged.network_policy.emulated_network_latency(), 0.0);
        assert_eq!(staged.network_policy.emulated_download_throughput(), -1.0);
        assert_eq!(staged.network_policy.emulated_upload_throughput(), -1.0);
        assert_eq!(
            staged.network_policy.emulated_connection_type(),
            Some("none")
        );
    }

    ctx.process_async(json!({
        "id": 104194505,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PR"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194505);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194506,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": { "url": "data:text/html,<title>promoted-online</title><div id='ok'>promoted online</div>" }
        })).await;
    let promoted_navigation = take_response_by_id(&mut ctx, 104194506);
    assert_eq!(
        promoted_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    {
        let promoted = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(promoted.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            !promoted
                .active_page_state()
                .network_policy
                .network_offline()
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_network_latency(),
            0.0
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_download_throughput(),
            -1.0
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_upload_throughput(),
            -1.0
        );
        assert_eq!(
            promoted
                .active_page_state()
                .network_policy
                .emulated_connection_type(),
            Some("none")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_extra_headers_before_promotion() {
    async fn handler(
        State(seen): State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
        headers: HeaderMap,
        uri: Uri,
    ) -> impl IntoResponse {
        seen.lock().push((
            uri.path().to_owned(),
            headers
                .get("x-target")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-HEADERS",
        "TID-000000000PH",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 10419460,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(10419460, json!({}), None);

    ctx.process_async(json!({
        "id": 104194601,
        "method": "Network.enable",
        "sessionId": "SID-active"
    }))
    .await;
    ctx.expect_result(104194601, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419461,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-active",
        "params": {
            "headers": {
                "X-Target": "A"
            }
        }
    }))
    .await;
    ctx.expect_result(10419461, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419462,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-HEADERS", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(10419462, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194602,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194602, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10419463,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": second_session_id,
        "params": {
            "headers": {
                "X-Target": "B"
            }
        }
    }))
    .await;
    ctx.expect_result(10419463, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(
            active.active_page_state().network_policy.extra_headers(),
            vec![("X-Target".into(), "A".into())]
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert_eq!(
            staged.network_policy.extra_headers(),
            vec![("X-Target".into(), "B".into())]
        );
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 10419464,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419464);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10419465,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PH"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419465);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 10419466,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419466);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    let seen = seen.lock().clone();
    assert_eq!(
        seen,
        vec![
            ("/page-a".to_owned(), Some("A".to_owned())),
            ("/page-b".to_owned(), Some("B".to_owned()))
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_extra_headers_before_promotion() {
    async fn handler(
        State(seen): State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
        headers: HeaderMap,
        uri: Uri,
    ) -> impl IntoResponse {
        seen.lock().push((
            uri.path().to_owned(),
            headers
                .get("x-target")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-HEADERS-CLEAR",
        "TID-000000000PC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194661,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194661, json!({}), None);

    ctx.process_async(json!({
        "id": 1041946611,
        "method": "Network.enable",
        "sessionId": "SID-active"
    }))
    .await;
    ctx.expect_result(1041946611, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194662,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-active",
        "params": {
            "headers": {
                "X-Target": "A"
            }
        }
    }))
    .await;
    ctx.expect_result(104194662, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194663,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-HEADERS-CLEAR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194663, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041946631,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041946631, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194664,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": second_session_id,
        "params": {
            "headers": {
                "X-Target": "B"
            }
        }
    }))
    .await;
    ctx.expect_result(104194664, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194665,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": second_session_id,
        "params": { "headers": {} }
    }))
    .await;
    ctx.expect_result(104194665, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(
            active.active_page_state().network_policy.extra_headers(),
            vec![("X-Target".into(), "A".into())]
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("enabled background session should retain its target-owned state");
        assert!(staged.network_policy.extra_headers().is_empty());
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 104194666,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194666);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194667,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194667);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 104194668,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194668);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    let seen = seen.lock().clone();
    assert_eq!(
        seen,
        vec![
            ("/page-a".to_owned(), Some("A".to_owned())),
            ("/page-b".to_owned(), None)
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_user_agent_before_promotion() {
    async fn handler(
        State(seen): State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
        headers: HeaderMap,
        uri: Uri,
    ) -> impl IntoResponse {
        seen.lock().push((
            uri.path().to_owned(),
            headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-UA",
        "TID-000000000PU",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 10419470,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(10419470, json!({}), None);

    ctx.process_async(json!({
        "id": 10419471,
        "method": "Network.setUserAgentOverride",
        "sessionId": "SID-active",
        "params": { "userAgent": "Moli/Stage-A" }
    }))
    .await;
    ctx.expect_result(10419471, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419472,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-UA", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(10419472, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10419473,
        "method": "Network.setUserAgentOverride",
        "sessionId": second_session_id,
        "params": { "userAgent": "Moli/Stage-B" }
    }))
    .await;
    ctx.expect_result(10419473, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(
            active
                .active_page_state()
                .network_policy
                .user_agent_override(),
            Some("Moli/Stage-A")
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert_eq!(
            staged.network_policy.user_agent_override(),
            Some("Moli/Stage-B")
        );
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 10419474,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419474);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10419475,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PU"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419475);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 10419476,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419476);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 10419477,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": { "expression": "navigator.userAgent" }
    }))
    .await;
    let promoted_eval = take_response_by_id(&mut ctx, 10419477);
    assert_eq!(
        promoted_eval["result"]["result"]["value"],
        json!("Moli/Stage-B")
    );

    let seen = seen.lock().clone();
    assert_eq!(
        seen,
        vec![
            ("/page-a".to_owned(), Some("Moli/Stage-A".to_owned())),
            ("/page-b".to_owned(), Some("Moli/Stage-B".to_owned()))
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_user_agent_before_promotion() {
    async fn handler(
        State(seen): State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
        headers: HeaderMap,
        uri: Uri,
    ) -> impl IntoResponse {
        seen.lock().push((
            uri.path().to_owned(),
            headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-UA-CLEAR",
        "TID-000000000PUC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194701,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194701, json!({}), None);

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 104194702,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194702);
    ctx.take_all();

    let default_ua = seen
        .lock()
        .last()
        .and_then(|(_, ua)| ua.clone())
        .expect("default active navigation should carry a user agent");

    ctx.process_async(json!({
        "id": 104194703,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-UA-CLEAR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194703, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194704,
        "method": "Network.setUserAgentOverride",
        "sessionId": second_session_id,
        "params": { "userAgent": "Moli/Staged-B" }
    }))
    .await;
    ctx.expect_result(104194704, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194705,
        "method": "Network.setUserAgentOverride",
        "sessionId": second_session_id,
        "params": { "userAgent": default_ua }
    }))
    .await;
    ctx.expect_result(104194705, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active
                .active_page_state()
                .network_policy
                .user_agent_override()
                .is_none(),
            "active target should keep its default user agent override",
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert_eq!(
            staged.network_policy.user_agent_override(),
            Some(default_ua.as_str())
        );
    }

    ctx.process_async(json!({
        "id": 104194706,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PUC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194706);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 104194707,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194707);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    let seen = seen.lock().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "/page-a");
    assert_eq!(seen[0].1.as_deref(), Some(default_ua.as_str()));
    assert_eq!(seen[1].0, "/page-b");
    assert_eq!(seen[1].1.as_deref(), Some(default_ua.as_str()));
    assert_ne!(
        seen[1].1.as_deref(),
        Some("Moli/Staged-B"),
        "promoted target should not retain the staged user agent override",
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_stages_locale_without_changing_request_language() {
    async fn handler(
        State(seen): State<Arc<Mutex<Vec<(String, Option<String>)>>>>,
        headers: HeaderMap,
        uri: Uri,
    ) -> impl IntoResponse {
        seen.lock().push((
            uri.path().to_owned(),
            headers
                .get(axum::http::header::ACCEPT_LANGUAGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LOCALE-TZ",
        "TID-000000000PL",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 10419480,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(10419480, json!({}), None);

    ctx.process_async(json!({
        "id": 10419481,
        "method": "Emulation.setLocaleOverride",
        "sessionId": "SID-active",
        "params": { "locale": "en-GB" }
    }))
    .await;
    ctx.expect_result(10419481, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419482,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": "SID-active",
        "params": { "timezoneId": "UTC" }
    }))
    .await;
    ctx.expect_result(10419482, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 10419483,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LOCALE-TZ", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(10419483, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10419484,
        "method": "Emulation.setLocaleOverride",
        "sessionId": second_session_id,
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(10419484, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 10419485,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": second_session_id,
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    ctx.expect_result(10419485, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(
            active.active_page_state().locale_override.as_deref(),
            Some("en-GB")
        );
        assert_eq!(
            active.active_page_state().timezone_override.as_deref(),
            Some("UTC")
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert_eq!(staged.locale_override.as_deref(), Some("fr-FR"));
        assert_eq!(staged.timezone_override.as_deref(), Some("Asia/Shanghai"));
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 10419486,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419486);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 10419487,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ lang: navigator.language, locale: Intl.DateTimeFormat().resolvedOptions().locale, tz: Intl.DateTimeFormat().resolvedOptions().timeZone })"
            }
        }))
    .await;
    let active_eval = take_response_by_id(&mut ctx, 10419487);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["lang"], json!("en-US"));
    assert_eq!(active_payload["locale"], json!("en-GB"));
    assert_eq!(active_payload["tz"], json!("UTC"));

    ctx.process_async(json!({
        "id": 10419488,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PL"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419488);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 10419489,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10419489);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
            "id": 10419490,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ lang: navigator.language, locale: Intl.DateTimeFormat().resolvedOptions().locale, tz: Intl.DateTimeFormat().resolvedOptions().timeZone })"
            }
        }))
    .await;
    let promoted_eval = take_response_by_id(&mut ctx, 10419490);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["lang"], json!("en-US"));
    assert_eq!(promoted_payload["locale"], json!("fr-FR"));
    assert_eq!(promoted_payload["tz"], json!("Asia/Shanghai"));

    let seen = seen.lock().clone();
    assert_eq!(
        seen,
        vec![
            ("/page-a".to_owned(), Some("en-US,en;q=0.9".to_owned())),
            ("/page-b".to_owned(), Some("en-US,en;q=0.9".to_owned()))
        ]
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_locale_before_promotion() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let accept_language = headers
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        format!(
            "<!doctype html><html><body data-accept-language=\"{accept_language}\"><script>document.body.textContent = [navigator.language, document.body.dataset.acceptLanguage].join('|');</script></body></html>"
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LOCALE-CLEAR",
        "TID-000000000PLC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194801,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194801, json!({}), None);

    ctx.process_async(json!({
        "id": 104194802,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LOCALE-CLEAR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194802, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194803,
        "method": "Emulation.setLocaleOverride",
        "sessionId": second_session_id,
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(104194803, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194804,
        "method": "Emulation.setLocaleOverride",
        "sessionId": second_session_id,
        "params": {}
    }))
    .await;
    ctx.expect_result(104194804, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active.active_page_state().locale_override.is_none(),
            "active target should keep its default locale override",
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "clearing staged locale back to default should fold away the parked state entry",
        );
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 104194805,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194805);
    ctx.take_all();

    let active_html = loaded_page_html_for_test(&mut ctx).await;
    let active_surface = active_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("active payload should be embedded in body")
        .to_owned();
    assert!(
        active_surface.starts_with("en-US|en-US,en;q=0.9"),
        "active target should retain the default navigator and request languages: {active_surface}"
    );

    ctx.process_async(json!({
        "id": 104194806,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PLC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194806);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 104194807,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194807);
    ctx.take_all();

    let promoted_html = loaded_page_html_for_test(&mut ctx).await;
    let promoted_surface = promoted_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("promoted payload should be embedded in body")
        .to_owned();

    assert_eq!(
        promoted_surface, active_surface,
        "promoted target should observe default locale surface after clearing its staged override"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_timezone_before_promotion() {
    async fn handler() -> impl IntoResponse {
        "<!doctype html><html><body><script>document.body.textContent = Intl.DateTimeFormat().resolvedOptions().timeZone;</script></body></html>"
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page-a", get(handler))
                .route("/page-b", get(handler)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-TIMEZONE-CLEAR",
        "TID-000000000PTC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194808,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194808, json!({}), None);

    ctx.process_async(json!({
        "id": 104194809,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-TIMEZONE-CLEAR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194809, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194810,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": second_session_id,
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    ctx.expect_result(104194810, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194811,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": second_session_id,
        "params": { "timezoneId": "" }
    }))
    .await;
    ctx.expect_result(104194811, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active.active_page_state().timezone_override.is_none(),
            "active target should keep its default timezone override",
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "clearing staged timezone back to default should fold away the parked state entry",
        );
    }

    let url_a = format!("http://{addr}/page-a");
    ctx.process_async(json!({
        "id": 104194812,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": url_a }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194812);
    ctx.take_all();

    let active_html = loaded_page_html_for_test(&mut ctx).await;
    let active_surface = active_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("active payload should be embedded in body")
        .to_owned();
    assert_ne!(active_surface, "Asia/Shanghai");

    ctx.process_async(json!({
        "id": 104194813,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PTC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194813);
    ctx.take_all();

    let url_b = format!("http://{addr}/page-b");
    ctx.process_async(json!({
        "id": 104194814,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": url_b }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194814);
    ctx.take_all();

    let promoted_html = loaded_page_html_for_test(&mut ctx).await;
    let promoted_surface = promoted_html
        .split("<body")
        .nth(1)
        .and_then(|tail| tail.split('>').nth(1))
        .and_then(|tail| tail.split("</body>").next())
        .expect("promoted payload should be embedded in body")
        .to_owned();

    assert_eq!(
        promoted_surface, active_surface,
        "promoted target should observe default timezone surface after clearing its staged override"
    );
    assert_ne!(
        promoted_surface, "Asia/Shanghai",
        "promoted target should not retain the staged timezone override"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_emulation_overrides_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-EMU",
        "TID-000000000PE",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194901,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194901, json!({}), None);

    ctx.process_async(json!({
        "id": 104194902,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": "SID-active",
        "params": {
            "width": 1280,
            "height": 720,
            "deviceScaleFactor": 2,
            "screenWidth": 1440,
            "screenHeight": 900,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(104194902, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194903,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": "SID-active",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194903, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194904,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": "SID-active",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194904, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 104194905,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-EMU", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194905, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194906,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": second_session_id,
        "params": {
            "width": 640,
            "height": 360,
            "deviceScaleFactor": 1,
            "screenWidth": 800,
            "screenHeight": 600,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(104194906, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194907,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(104194907, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194908,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(104194908, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(
            active
                .active_page_state()
                .emulated_device_metrics
                .as_ref()
                .map(|metrics| (
                    metrics.width,
                    metrics.height,
                    metrics.device_scale_factor,
                    metrics.screen_width,
                    metrics.screen_height
                )),
            Some((1280, 720, 2.0, 1440, 900))
        );
        assert!(active.active_page_state().touch_emulation_enabled);
        assert!(active.active_page_state().focus_emulation_enabled);

        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert_eq!(
            staged.emulated_device_metrics.as_ref().map(|metrics| (
                metrics.width,
                metrics.height,
                metrics.device_scale_factor,
                metrics.screen_width,
                metrics.screen_height
            )),
            Some((640, 360, 1.0, 800, 600))
        );
        assert!(!staged.touch_emulation_enabled);
        assert!(!staged.focus_emulation_enabled);
    }

    ctx.process_async(json!({
        "id": 104194909,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194909);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194910,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height, maxTouchPoints: navigator.maxTouchPoints, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 104194910);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["innerWidth"], json!(1280));
    assert_eq!(active_payload["innerHeight"], json!(720));
    assert_eq!(active_payload["dpr"], json!(2));
    assert_eq!(active_payload["screenWidth"], json!(1440));
    assert_eq!(active_payload["screenHeight"], json!(900));
    assert_eq!(active_payload["maxTouchPoints"], json!(1));
    assert_eq!(active_payload["hasFocus"], json!(true));
    assert_eq!(active_payload["hidden"], json!(false));
    assert_eq!(active_payload["visibilityState"], json!("visible"));

    ctx.process_async(json!({
        "id": 104194911,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PE"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194911);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194912,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194912);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194913,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height, maxTouchPoints: navigator.maxTouchPoints, hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104194913);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["innerWidth"], json!(640));
    assert_eq!(promoted_payload["innerHeight"], json!(360));
    assert_eq!(promoted_payload["dpr"], json!(1));
    assert_eq!(promoted_payload["screenWidth"], json!(800));
    assert_eq!(promoted_payload["screenHeight"], json!(600));
    assert_eq!(promoted_payload["maxTouchPoints"], json!(0));
    assert_eq!(promoted_payload["hasFocus"], json!(true));
    assert_eq!(promoted_payload["hidden"], json!(false));
    assert_eq!(promoted_payload["visibilityState"], json!("visible"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_page_settings_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-PAGE",
        "TID-000000000PP",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194914,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194914, json!({}), None);

    ctx.process_async(json!({
        "id": 104194915,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-PAGE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194915, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194916,
        "method": "Page.setBypassCSP",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194916, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194917,
        "method": "Page.setFontFamilies",
        "sessionId": second_session_id,
        "params": {
            "standard": "Georgia",
            "fixed": "Fira Code"
        }
    }))
    .await;
    ctx.expect_result(104194917, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194918,
        "method": "Page.setInterceptFileChooserDialog",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194918, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("staged page settings for background target");
        assert!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_bypass_csp_enabled
        );
        assert_eq!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_font_families
                .get("standard"),
            Some(&json!("Georgia"))
        );
        assert_eq!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_font_families
                .get("fixed"),
            Some(&json!("Fira Code"))
        );
        assert!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled
        );
    }

    ctx.process_async(json!({
        "id": 104194919,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104194919, json!({}), None);

    let active = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("promoted browser context");
    assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
    assert!(
        active.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_bypass_csp_enabled
    );
    assert_eq!(
        active.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_font_families
            .get("standard"),
        Some(&json!("Georgia"))
    );
    assert_eq!(
        active.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_font_families
            .get("fixed"),
        Some(&json!("Fira Code"))
    );
    assert!(
        active.active_page_state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_intercept_file_chooser_dialog_enabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_device_metrics_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-EMU-CLEAR",
        "TID-000000000PM",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949131,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949131, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949132,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-EMU-CLEAR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949132, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949133,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": second_session_id,
        "params": {
            "width": 640,
            "height": 360,
            "deviceScaleFactor": 1,
            "screenWidth": 800,
            "screenHeight": 600,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(1041949133, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949134,
        "method": "Emulation.clearDeviceMetricsOverride",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949134, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active.active_page_state().emulated_device_metrics.is_none(),
            "active target should keep its default device metrics",
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "clearing staged device metrics back to default should fold away the parked state entry",
        );
    }

    ctx.process_async(json!({
        "id": 1041949135,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949135);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949136,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 1041949136);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");

    ctx.process_async(json!({
        "id": 1041949137,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PM"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949137);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949138,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949138);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949139,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ innerWidth: window.innerWidth, innerHeight: window.innerHeight, dpr: window.devicePixelRatio, screenWidth: screen.width, screenHeight: screen.height })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 1041949139);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");

    assert_eq!(
        promoted_payload, active_payload,
        "promoted target should observe default metrics after clearing its staged override"
    );
    assert_ne!(promoted_payload["innerWidth"], json!(640));
    assert_ne!(promoted_payload["innerHeight"], json!(360));
    assert_ne!(promoted_payload["screenWidth"], json!(800));
    assert_ne!(promoted_payload["screenHeight"], json!(600));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_clear_its_own_touch_and_focus_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-EMU-CLEAR-TF",
        "TID-000000000PT",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949141,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949141, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949142,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-EMU-CLEAR-TF", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949142, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949143,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(1041949143, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949144,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(1041949144, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949145,
        "method": "Emulation.setTouchEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(1041949145, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949146,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(1041949146, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_state().touch_emulation_enabled,
            "active target should keep default touch emulation"
        );
        assert!(
            !active.active_page_state().focus_emulation_enabled,
            "active target should keep default focus emulation"
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "clearing staged touch/focus back to defaults should fold away the parked state entry",
        );
    }

    ctx.process_async(json!({
        "id": 1041949147,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949147);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949148,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ maxTouchPoints: navigator.maxTouchPoints, hasFocusType: typeof document.hasFocus, hasFocusValue: typeof document.hasFocus === 'function' ? document.hasFocus() : null, hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 1041949148);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");

    ctx.process_async(json!({
        "id": 1041949149,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PT"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949149);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949150,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949150);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949151,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ maxTouchPoints: navigator.maxTouchPoints, hasFocusType: typeof document.hasFocus, hasFocusValue: typeof document.hasFocus === 'function' ? document.hasFocus() : null, hidden: document.hidden, visibilityState: document.visibilityState })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 1041949151);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");

    assert_eq!(
        promoted_payload, active_payload,
        "promoted target should observe default touch/focus surfaces after clearing staged overrides"
    );
    assert_ne!(promoted_payload["maxTouchPoints"], json!(1));
    assert_eq!(active_payload["hasFocusType"], json!("function"));
    assert_eq!(active_payload["hasFocusValue"], json!(true));
    assert_eq!(active_payload["hidden"], json!(false));
    assert_eq!(active_payload["visibilityState"], json!("visible"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_script_execution_disabled_before_promotion()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-SCRIPT-DISABLED",
        "TID-000000000PS",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194920,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194920, json!({}), None);

    ctx.process_async(json!({
        "id": 104194921,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-SCRIPT-DISABLED", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194921, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194922,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": second_session_id,
        "params": { "value": true }
    }))
    .await;
    ctx.expect_result(104194922, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_state().script_execution_disabled);
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.script_execution_disabled);
    }

    ctx.process_async(json!({
            "id": 104194923,
            "method": "Page.navigate",
            "sessionId": "SID-active",
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>active</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 104194923);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194924,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 104194924);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["inlineRan"], json!(true));
    assert_eq!(active_payload["dataset"], json!("yes"));

    ctx.process_async(json!({
        "id": 104194925,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PS"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194925);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 104194926,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>promoted</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 104194926);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    {
        let promoted = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert!(promoted.active_page_state().script_execution_disabled);
    }

    ctx.process_async(json!({
            "id": 104194927,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null, runtimeEvalStillWorks: 1 + 1 })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 104194927);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["inlineRan"], json!(false));
    assert_eq!(promoted_payload["dataset"], serde_json::Value::Null);
    assert_eq!(promoted_payload["runtimeEvalStillWorks"], json!(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_reenable_its_own_script_execution_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-SCRIPT-REENABLE",
        "TID-000000000PSE",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949270,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949270, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949271,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-SCRIPT-REENABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949271, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949272,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": second_session_id,
        "params": { "value": true }
    }))
    .await;
    ctx.expect_result(1041949272, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949273,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": second_session_id,
        "params": { "value": false }
    }))
    .await;
    ctx.expect_result(1041949273, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_state().script_execution_disabled);
        // A completed renderer call may retain its monotonic correlation
        // allocator in the parked session; only the effective setting must
        // collapse back to the default.
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none_or(|state| !state.script_execution_disabled),
            "script execution re-enable should clear the staged parked setting: {:#?}",
            active.parked_page_session_state(&second_target_id)
        );
    }

    ctx.process_async(json!({
            "id": 1041949274,
            "method": "Page.navigate",
            "sessionId": "SID-active",
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>active</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949274);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949275,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null })"
            }
        })).await;
    let active_eval = take_response_by_id(&mut ctx, 1041949275);
    let active_payload = active_eval["result"]["result"]["value"]
        .as_str()
        .expect("active payload should be string");
    let active_payload: serde_json::Value =
        serde_json::from_str(active_payload).expect("active payload should be valid json");
    assert_eq!(active_payload["inlineRan"], json!(true));
    assert_eq!(active_payload["dataset"], json!("yes"));

    ctx.process_async(json!({
        "id": 1041949276,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PSE"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949276);
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1041949277,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.inlineRan='yes'; globalThis.__inlineRan = true;</script>promoted</body>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949277);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    {
        let promoted = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert!(!promoted.active_page_state().script_execution_disabled);
    }

    ctx.process_async(json!({
            "id": 1041949278,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ inlineRan: !!globalThis.__inlineRan, dataset: document.body.dataset.inlineRan || null, runtimeEvalStillWorks: 1 + 1 })"
            }
        })).await;
    let promoted_eval = take_response_by_id(&mut ctx, 1041949278);
    let promoted_payload = promoted_eval["result"]["result"]["value"]
        .as_str()
        .expect("promoted payload should be string");
    let promoted_payload: serde_json::Value =
        serde_json::from_str(promoted_payload).expect("promoted payload should be valid json");
    assert_eq!(promoted_payload["inlineRan"], json!(true));
    assert_eq!(promoted_payload["dataset"], json!("yes"));
    assert_eq!(promoted_payload["runtimeEvalStillWorks"], json!(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_lifecycle_events_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LIFECYCLE",
        "TID-000000000PY",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194930,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194930, json!({}), None);

    ctx.process_async(json!({
        "id": 104194931,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LIFECYCLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194931, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194932,
        "method": "Page.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194932, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194933,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(104194933, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events
        );
    }

    ctx.process_async(json!({
        "id": 104194934,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194934);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!("SID-active")),
        "active target should not emit lifecycle events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194935,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PY"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194935);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194936,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194936);
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "promoted target networkIdle lifecycle event",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["frameId"] == json!(second_target_id)
                && message["params"]["name"] == json!("networkIdle")
        },
    )
    .await;
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    let lifecycle_events = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["frameId"] == json!(second_target_id)
        })
        .map(|message| {
            message["params"]["name"]
                .as_str()
                .expect("lifecycle name should be string")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle_events,
        vec![
            "init".to_owned(),
            "DOMContentLoaded".to_owned(),
            "load".to_owned(),
            "networkAlmostIdle".to_owned(),
            "networkIdle".to_owned(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_lifecycle_events_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-LIFECYCLE-DISABLE",
        "TID-000000000PYD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949320,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949320, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949321,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-LIFECYCLE-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949321, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949322,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(1041949322, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949323,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": second_session_id,
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(1041949323, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "lifecycle disable should collapse staged parked state back to default"
        );
    }

    ctx.process_async(json!({
        "id": 1041949324,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949324);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!("SID-active")),
        "active target should not emit lifecycle events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949325,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PYD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949325);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949326,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949326);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["frameId"] == json!(second_target_id)
        }),
        "disabled lifecycle events should not emit on first promoted navigation: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_runtime_enable_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-RUNTIME",
        "TID-000000000PR",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194936,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194936, json!({}), None);

    ctx.process_async(json!({
        "id": 104194937,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-RUNTIME", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194937, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194938,
        "method": "Runtime.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194938, json!({}), Some(&second_session_id));
    take_staged_about_blank_runtime_context(&mut ctx, &second_session_id, &second_target_id);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
    }

    ctx.process_async(json!({
        "id": 104194939,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194939);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!("SID-active") && is_runtime_context_event(message)
        }),
        "active target should not emit runtime context events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949391,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PR"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949391);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949392,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949392);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    let runtime_events = vec![
        ctx.wait_for_scheduler_message("promoted Runtime context reset", |message| {
            message["sessionId"] == json!(second_session_id)
                && message["method"] == json!("Runtime.executionContextsCleared")
        })
        .await,
        ctx.wait_for_scheduler_message("promoted Runtime default context", |message| {
            message["sessionId"] == json!(second_session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
        })
        .await,
    ];
    assert_eq!(
        runtime_events.len(),
        2,
        "staged Runtime.enable should use the owner-safe native path after promotion: {runtime_events:?}"
    );
    assert_eq!(
        runtime_events[0]["method"],
        "Runtime.executionContextsCleared"
    );
    assert_eq!(
        runtime_events[1]["method"],
        "Runtime.executionContextCreated"
    );
    assert_eq!(
        runtime_events[1]["params"]["context"]["name"],
        json!("data:text/html,<title>page-b</title><div id='ok'>page b</div>")
    );
    assert_eq!(
        runtime_events[1]["params"]["context"]["auxData"]["frameId"],
        json!(second_target_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_enable_replays_context_without_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-RUNTIME-DIRECT",
        "TID-000000000RDA",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949310,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949310, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949311,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-RUNTIME-DIRECT", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949311, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949312,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(1041949312, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949313,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949313);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949314,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000RDA" }
    }))
    .await;
    ctx.expect_result(1041949314, json!({}), None);
    ctx.take_all();

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000RDA"));
        assert!(
            bc.background_target(&second_target_id)
                .is_some_and(|target| target.has_loaded_page()),
            "second target should be parked with a loaded page before Runtime.enable"
        );
    }

    ctx.process_async(json!({
        "id": 1041949315,
        "method": "Runtime.enable",
        "sessionId": second_session_id
    }))
    .await;
    let messages = ctx.take_all();
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(1041949315)
                && message["result"] == json!({})
                && message["sessionId"] == json!(second_session_id)
        }),
        "Runtime.enable should return a session-scoped success response: {messages:?}"
    );
    let context_events = messages
        .iter()
        .filter(|message| {
            message["sessionId"] == json!(second_session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        context_events.len(),
        1,
        "loaded background Runtime.enable should replay exactly one default context event: {messages:?}"
    );
    assert_eq!(
        context_events[0]["params"]["context"]["auxData"]["frameId"],
        json!(second_target_id)
    );

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            bc.active_target_id(),
            Some("TID-000000000RDA"),
            "direct Runtime.enable should not promote the loaded background target"
        );
        assert!(
            bc.parked_page_session_state(&second_target_id)
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .runtime_frontend_enabled),
            "Runtime.enable should be staged on the background target owner"
        );
        assert!(
            bc.background_target(&second_target_id)
                .is_some_and(|target| target.has_loaded_page()),
            "direct Runtime.enable should leave the loaded page parked"
        );
    }
}

struct LoadedBackgroundRuntimeOwner {
    target_id: String,
    session_id: String,
}

async fn load_same_context_loaded_background_runtime_owner_async(
    ctx: &mut TestContext,
    browser_context_id: &str,
    active_target_id: &str,
    active_html: &str,
    background_url: &str,
    command_id_base: u64,
) -> LoadedBackgroundRuntimeOwner {
    load_bc_with_titled_page_async(ctx, browser_context_id, active_target_id, active_html).await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": command_id_base,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(command_id_base, json!({}), None);

    ctx.process_async(json!({
        "id": command_id_base + 1,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": browser_context_id, "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(
        command_id_base + 1,
        json!({ "targetId": second_target_id }),
        None,
    );

    ctx.process_async(json!({
        "id": command_id_base + 2,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(command_id_base + 2, json!({}), None);

    ctx.process_async(json!({
        "id": command_id_base + 3,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": background_url }
    }))
    .await;
    let _ = take_response_by_id(ctx, command_id_base + 3);
    ctx.take_all();

    ctx.process_async(json!({
        "id": command_id_base + 4,
        "method": "Target.activateTarget",
        "params": { "targetId": active_target_id }
    }))
    .await;
    ctx.expect_result(command_id_base + 4, json!({}), None);
    ctx.take_all();

    ctx.process_async(json!({
        "id": command_id_base + 5,
        "method": "Runtime.enable",
        "sessionId": second_session_id
    }))
    .await;
    let _ = take_response_by_id(ctx, command_id_base + 5);
    ctx.take_all();

    LoadedBackgroundRuntimeOwner {
        target_id: second_target_id,
        session_id: second_session_id,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_evaluate_reads_owner_page_without_promotion()
 {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-EVAL",
        "TID-000000000REA",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949410,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949416,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949416);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(response["result"]["result"]["value"], json!("second"));

    ctx.process_async(json!({
        "id": 1041949417,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('#ok')"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949417);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(response["result"]["result"]["subtype"], json!("node"));

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000REA"),
        "direct Runtime.evaluate should not promote the loaded background target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "direct Runtime.evaluate should leave the owner page parked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_window_open_self_navigates_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-POPUP-SELF",
        "TID-000000000PSA",
        "<title>active</title><main>active target</main>",
        "data:text/html,<title>background</title><main>background target</main>",
        1041949430,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949436,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "window.open('data:text/html,<title>self</title><main>self target</main>', '_self') !== null"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949436);
    assert_eq!(response["result"]["result"]["type"], json!("boolean"));
    assert_eq!(response["result"]["result"]["value"], json!(true));
    // Chromium returns the Runtime.evaluate response before the `_self`
    // navigation starts. The renderer-owned navigation is a later task, so
    // wait for its exact owner action instead of strengthening the command
    // response into an implicit navigation barrier.
    ctx.wait_until_scheduler_state("background _self navigation owner action", |conn| {
        conn.browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.background_target(&owner.target_id))
            .is_some_and(|target| {
                target.target_url() == "data:text/html,<title>self</title><main>self target</main>"
            })
    })
    .await;
    let emitted = ctx.take_all();
    assert!(
        !emitted
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "_self popup must navigate the owner target instead of creating a popup target: {emitted:?}"
    );

    {
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            browser_context.active_target_id(),
            Some("TID-000000000PSA"),
            "background _self navigation must not promote the background target"
        );
        let background_target = browser_context
            .background_target(&owner.target_id)
            .expect("background target should remain parked");
        assert_eq!(
            background_target.target_url(),
            "data:text/html,<title>self</title><main>self target</main>"
        );
        assert!(
            background_target.has_loaded_page(),
            "background _self navigation should replace the owner loaded page"
        );
    }

    ctx.process_async(json!({
        "id": 1041949437,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('main')?.textContent"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949437);
    assert_eq!(response["result"]["result"]["value"], json!("self target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_named_popup_reuse_navigates_and_promotes_loaded_owner() {
    let mut ctx = TestContext::new();
    tokio::task::LocalSet::new()
        .run_until(async {
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-NAMED-POPUP",
        "TID-000000000NPA",
        "<title>active</title><main>active target</main>",
        "data:text/html,<title>background</title><main>background target</main>",
        1041949440,
    )
    .await;
    ctx.enable_background_navigation_scheduler_for_test();
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .remember_target_window_name("reportWindow", &owner.target_id);

    ctx.process_async(json!({
        "id": 1041949446,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('data:text/html,<title>named</title><main>named target</main>', 'reportWindow') !== null"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949446);
    assert_eq!(response["result"]["result"]["type"], json!("boolean"));
    assert_eq!(response["result"]["result"]["value"], json!(true));
    ctx.wait_until_scheduler_state(
        "named popup navigation commit and foreground activation",
        |conn| {
            conn.browser_context_by_id("BID-9-NAMED-POPUP")
                .is_some_and(|browser_context| {
                    browser_context.active_target_id() == Some(owner.target_id.as_str())
                        && loaded_page_for_target(browser_context, &owner.target_id).is_some_and(
                            |page| {
                                page.final_url().as_str()
                                    == "data:text/html,<title>named</title><main>named target</main>"
                            },
                        )
                })
        },
    )
    .await;
    let emitted = ctx.take_all();
    assert!(
        !emitted
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "reusing a loaded named target must not create a new popup target: {emitted:?}"
    );
    let changed = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(owner.target_id)
                && message["params"]["targetInfo"]["url"]
                    == json!("data:text/html,<title>named</title><main>named target</main>")
        })
        .unwrap_or_else(|| {
            panic!("loaded named target reuse should report targetInfoChanged: {emitted:?}")
        });
    assert_eq!(
        changed["params"]["targetInfo"]["targetId"],
        json!(owner.target_id)
    );
    assert_eq!(
        changed["params"]["targetInfo"]["url"],
        json!("data:text/html,<title>named</title><main>named target</main>")
    );

    {
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            browser_context.active_target_id(),
            Some(owner.target_id.as_str()),
            "ordinary window.open should promote its reused named target"
        );
        assert_eq!(
            browser_context.target_url(),
            "data:text/html,<title>named</title><main>named target</main>"
        );
        assert!(
            browser_context.has_loaded_page(),
            "named popup reuse should replace the existing owner loaded page"
        );
        assert!(
            browser_context
                .background_target("TID-000000000NPA")
                .is_some(),
            "foreground named-target reuse should demote the previous active target"
        );
    }

    ctx.process_async(json!({
        "id": 1041949447,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('main')?.textContent"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949447);
    assert_eq!(response["result"]["result"]["value"], json!("named target"));
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_call_function_on_uses_owner_object_without_promotion()
 {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-CALL",
        "TID-000000000RCA",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949510,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949516,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('#ok')"
        }
    }))
    .await;
    let handle_response = take_response_by_id(&mut ctx, 1041949516);
    let object_id = handle_response["result"]["result"]["objectId"]
        .as_str()
        .expect("background node object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 1041949517,
        "method": "Runtime.callFunctionOn",
        "sessionId": owner.session_id,
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.textContent; }",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949517);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("second target")
    );

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000RCA"),
        "direct Runtime.callFunctionOn should not promote the loaded background target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "direct Runtime.callFunctionOn should leave the owner page parked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_await_promise_uses_owner_without_promotion()
{
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-AWAIT",
        "TID-000000000RWA",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949610,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949616,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "Promise.resolve(document.title)",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949616);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(response["result"]["result"]["value"], json!("second"));

    ctx.process_async(json!({
        "id": 1041949617,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('#ok')"
        }
    }))
    .await;
    let handle_response = take_response_by_id(&mut ctx, 1041949617);
    let object_id = handle_response["result"]["result"]["objectId"]
        .as_str()
        .expect("background node object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 1041949618,
        "method": "Runtime.callFunctionOn",
        "sessionId": owner.session_id,
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return Promise.resolve(this.textContent); }",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949618);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("second target")
    );

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000RWA"),
        "direct Runtime awaitPromise should not promote the loaded background target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "direct Runtime awaitPromise should leave the owner page parked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_pending_await_survives_active_target_switch() {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-AWAIT-SWITCH",
        "TID-000000000RAS",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949650,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949656,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": r#"new Promise(resolve => {
  globalThis.__lmResolvePendingAwaitAfterSwitch = () => {
  globalThis.__lmPendingAwaitOwnerMarker = document.title;
  resolve(document.title + ':' + globalThis.__lmPendingAwaitOwnerMarker);
  return 'resolved:' + document.title;
  };
})"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(1041949656)),
        "timer-backed background awaitPromise should still be pending before active target switch: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&owner.session_id)),
        "background awaitPromise should register against the background target session"
    );

    ctx.process_async(json!({
        "id": 1041949657,
        "method": "Target.activateTarget",
        "params": { "targetId": owner.target_id }
    }))
    .await;
    ctx.expect_result(1041949657, json!({}), None);
    let first_switch_messages = ctx.take_all();
    assert!(
        !first_switch_messages
            .iter()
            .any(|message| message["id"] == json!(1041949656)),
        "background awaitPromise must remain pending until the test resolves it explicitly: {:?}",
        first_switch_messages
    );

    ctx.process_async(json!({
        "id": 1041949658,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000RAS" }
    }))
    .await;
    ctx.expect_result(1041949658, json!({}), None);
    let second_switch_messages = ctx.take_all();
    assert!(
        !second_switch_messages
            .iter()
            .any(|message| message["id"] == json!(1041949656)),
        "background awaitPromise must remain pending after active target switch: {:?}",
        second_switch_messages
    );

    ctx.process_async(json!({
        "id": 1041949660,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "globalThis.__lmResolvePendingAwaitAfterSwitch()",
            "returnByValue": true
        }
    }))
    .await;
    let resolve_response = take_response_by_id(&mut ctx, 1041949660);
    assert_eq!(resolve_response["sessionId"], json!(owner.session_id));
    assert_eq!(
        resolve_response["result"]["result"]["value"],
        json!("resolved:second")
    );

    crate::testing::wait_until_message(
        &mut ctx,
        Some(owner.session_id.as_str()),
        "background awaitPromise after active target switch",
        |message| message["id"] == json!(1041949656),
    )
    .await;
    let response = take_response_by_id(&mut ctx, 1041949656);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("second:second")
    );
    assert!(
        !ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&owner.session_id)),
        "settled background awaitPromise should clear the background target pending-await entry"
    );

    ctx.process_async(json!({
        "id": 1041949659,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "typeof globalThis.__lmPendingAwaitOwnerMarker",
            "returnByValue": true
        }
    }))
    .await;
    let active_response = take_response_by_id(&mut ctx, 1041949659);
    assert_eq!(
        active_response["result"]["result"]["value"],
        json!("undefined")
    );

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000RAS"),
        "pending background awaitPromise completion must not promote the owner target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "pending background awaitPromise completion should leave the owner page parked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_runtime_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-RUNTIME-DISABLE",
        "TID-000000000PRD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949360,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949360, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949370,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-RUNTIME-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949370, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949380,
        "method": "Runtime.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949380, json!({}), Some(&second_session_id));
    take_staged_about_blank_runtime_context(&mut ctx, &second_session_id, &second_target_id);

    ctx.process_async(json!({
        "id": 1041949381,
        "method": "Runtime.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949381, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
    }

    ctx.process_async(json!({
        "id": 1041949390,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949390);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!("SID-active") && is_runtime_context_event(message)
        }),
        "active target should not emit runtime context events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949393,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PRD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949393);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949394,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949394);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!(second_session_id)
                && matches!(
                    message["method"].as_str(),
                    Some("Runtime.executionContextsCleared")
                        | Some("Runtime.executionContextCreated")
                )
        }),
        "disabled runtime should not emit execution context events on first promoted navigation: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_inspector_enable_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-INSPECTOR",
        "TID-000000000PI",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 1041949395,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949395, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949396,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-INSPECTOR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949396, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949397,
        "method": "Inspector.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949397, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active
                .active_page_state()
                .owner_state
                .target_crash_state
                .is_crashed()
        );
        assert!(
            !active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
    }
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Inspector.targetCrashed")
                && message["sessionId"] == json!(second_session_id)
        }),
        "background inspector enable should not replay active target crash state"
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949398,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PI"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949398);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949390,
        "method": "Page.crash",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949390, json!({}), Some(&second_session_id));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetCrashed")
            && message["sessionId"] == json!(second_session_id)
    }));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949399,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949399);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Inspector.targetReloadedAfterCrash")
                && message["sessionId"] == json!(second_session_id)
        }),
        "promoted target should emit crash-reload event when staged inspector is enabled"
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
async fn same_context_background_session_can_disable_its_own_inspector_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-INSPECTOR-DISABLE",
        "TID-000000000PID",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949400,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949400, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949401,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-INSPECTOR-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949401, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949402,
        "method": "Inspector.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949402, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949403,
        "method": "Inspector.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949403, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "inspector disable should collapse staged parked state back to default"
        );
    }

    ctx.process_async(json!({
        "id": 1041949404,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PID"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949404);
    ctx.take_all();

    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 1041949405,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949405);
    assert!(
        !ctx.sent.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Inspector.targetReloadedAfterCrash") | Some("Inspector.targetCrashed")
            ) && message["sessionId"] == json!(second_session_id)
        }),
        "disabled inspector should not emit crash-related events on first promoted navigation"
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed(),
        "navigation should still clear crash state even when inspector is disabled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_css_enable_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-CSS",
        "TID-000000000PC",
        "<title>active</title><style>body{color:red}</style><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949406,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949406, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949407,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-CSS", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949407, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949408,
        "method": "CSS.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949408, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_state().css_enabled);
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.css_enabled);
    }

    ctx.process_async(json!({
        "id": 1041949409,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949409);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        assert!(active.active_page_state().css_enabled);
    }

    ctx.process_async(json!({
            "id": 1041949410,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>page-b</title><style>body{color:blue}</style><div id='ok'>page b</div>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949410);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .css_enabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_css_before_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-CSS-DISABLE",
        "TID-000000000PCD",
        "<title>active</title><style>body{color:red}</style><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949411,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949411, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949412,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-CSS-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949412, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949413,
        "method": "CSS.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949413, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949414,
        "method": "CSS.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949414, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_state().css_enabled);
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "css disable should collapse staged parked state back to default"
        );
    }

    ctx.process_async(json!({
        "id": 1041949415,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PCD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949415);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        assert!(!active.active_page_state().css_enabled);
    }

    ctx.process_async(json!({
            "id": 1041949416,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>page-b</title><style>body{color:blue}</style><div id='ok'>page b</div>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949416);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .css_enabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_fetch_enable_before_promotion() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH",
        "TID-000000000PF",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194940,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194940, json!({}), None);

    ctx.process_async(json!({
        "id": 104194941,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-FETCH", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194941, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194942,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": {
            "patterns": [
                {
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(104194942, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_state().fetch_owner.is_enabled());
        assert!(
            active
                .active_page_state()
                .fetch_owner
                .config_snapshot()
                .patterns()
                .is_empty()
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.fetch_owner.config_snapshot().is_enabled());
        assert_eq!(staged.fetch_owner.config_snapshot().patterns().len(), 1);
        assert_eq!(
            staged.fetch_owner.config_snapshot().patterns()[0].url_pattern,
            "*"
        );
        assert_eq!(
            staged.fetch_owner.config_snapshot().patterns()[0].resource_type_filter,
            Some(crate::conn::FetchResourceTypeFilter::Document)
        );
        assert_eq!(
            staged.fetch_owner.config_snapshot().patterns()[0].request_stage,
            crate::conn::FetchRequestStage::Request
        );
    }

    ctx.process_async(json!({
        "id": 104194943,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194943);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "active target should not be intercepted by background-staged fetch config: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194944,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PF"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194944);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194945,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], json!(second_session_id));
    assert_eq!(paused["params"]["frameId"], json!(second_target_id));

    ctx.process_async(json!({
        "id": 104194946,
        "method": "Fetch.continueRequest",
        "sessionId": second_session_id,
        "params": {
            "requestId": paused["params"]["requestId"]
        }
    }))
    .await;
    ctx.expect_result(104194946, json!({}), Some(&second_session_id));

    let _ = take_response_by_id(&mut ctx, 104194945);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_fetch_continue_request_keeps_target_parked() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-continue-parked</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH-CONTINUE",
        "TID-000000000PFC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194962,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194962, json!({}), None);

    ctx.process_async(json!({
        "id": 104194963,
        "method": "Target.createTarget",
        "params": {
            "background": true,
            "browserContextId": "BID-9-PRE-FETCH-CONTINUE",
            "url": "about:blank#second"
        }
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194963, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194964,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": {
            "patterns": [
                {
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(104194964, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194965,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], json!(second_session_id));
    assert_eq!(paused["params"]["frameId"], json!(second_target_id));
    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PFC"));
        assert!(
            active
                .parked_fetch_state(&second_target_id)
                .is_some_and(|state| state.has_pending_fetch_navigation()),
            "background fetch pause should stay parked before continueRequest"
        );
    }

    ctx.process_async(json!({
        "id": 104194966,
        "method": "Fetch.continueRequest",
        "sessionId": second_session_id,
        "params": {
            "requestId": paused["params"]["requestId"]
        }
    }))
    .await;
    ctx.expect_result(104194966, json!({}), Some(&second_session_id));

    let navigation = take_response_by_id(&mut ctx, 104194965);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PFC"));
        assert!(
            active
                .background_target(&second_target_id)
                .is_some_and(|target| target.has_loaded_page()),
            "continued background navigation should commit to parked owner"
        );
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_fetch_auth_handling_before_promotion() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-auth-stage</body></html>",
        )
    }

    async fn protected(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let expected = "Basic dXNlcjpwYXNz";
        match headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body>secret</body></html>",
            )
                .into_response(),
            _ => (
                axum::http::StatusCode::UNAUTHORIZED,
                [(
                    axum::http::header::WWW_AUTHENTICATE.as_str(),
                    "Basic realm=\"stage-area\"",
                )],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .route("/protected", axum::routing::any(protected)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH-AUTH",
        "TID-000000000PFA",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194954,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194954, json!({}), None);

    ctx.process_async(json!({
        "id": 104194955,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-FETCH-AUTH", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194955, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194956,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(104194956, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_state().fetch_owner.is_enabled());
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.fetch_owner.config_snapshot().is_enabled());
        assert!(staged.fetch_owner.config_snapshot().handle_auth_requests());
    }

    ctx.process_async(json!({
        "id": 104194957,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194957);
    assert!(
        !ctx.sent.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Fetch.requestPaused") | Some("Fetch.authRequired")
            )
        }),
        "active target should not see auth interception from background-staged fetch config: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194958,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PFA"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194958);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194959,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/protected") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], json!(second_session_id));
    assert_eq!(paused["params"]["frameId"], json!(second_target_id));
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("paused request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 104194960,
        "method": "Fetch.continueRequest",
        "sessionId": second_session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(104194960, json!({}), Some(&second_session_id));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["sessionId"], json!(second_session_id));
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "basic");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "stage-area"
    );

    ctx.process_async(json!({
        "id": 104194961,
        "method": "Fetch.continueWithAuth",
        "sessionId": second_session_id,
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "user",
                "password": "pass"
            }
        }
    }))
    .await;
    ctx.expect_result(104194961, json!({}), Some(&second_session_id));

    let navigation = take_response_by_id(&mut ctx, 104194959);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted auth navigation: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_fetch_before_promotion() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-disable-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH-DISABLE",
        "TID-000000000PFD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194947,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194947, json!({}), None);

    ctx.process_async(json!({
        "id": 104194948,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-FETCH-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194948, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194949,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": {
            "patterns": [
                {
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(104194949, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194950,
        "method": "Fetch.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194950, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_state().fetch_owner.is_enabled());
        assert!(
            active
                .active_page_state()
                .fetch_owner
                .config_snapshot()
                .patterns()
                .is_empty()
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "fetch disable should collapse staged parked state back to default"
        );
    }

    ctx.process_async(json!({
        "id": 104194951,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194951);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "active target should not be intercepted after background-staged fetch disable: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194952,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PFD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194952);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194953,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194953);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "disabled fetch should not pause first promoted navigation: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_network_enable_before_promotion() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET",
        "TID-000000000PN",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194950,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194950, json!({}), None);

    ctx.process_async(json!({
        "id": 104194951,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194951, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194952,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194952, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.runtime_slot.primary_network_events_enabled());
    }

    ctx.process_async(json!({
        "id": 104194953,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194953);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Network.requestWillBeSent")),
        "active target should not emit network events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194954,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PN"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194954);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194955,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194955);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    wait_for_session_main_document_loading_finished(
        &mut ctx,
        &second_session_id,
        &page_url,
        "promoted session main-document network completion",
    )
    .await;
    let emitted = ctx.take_all();
    let request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .cloned()
        .expect("promoted target should emit requestWillBeSent");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    assert_eq!(request["params"]["frameId"], json!(second_target_id));

    let response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("promoted target should emit responseReceived");
    assert_eq!(response["params"]["response"]["url"], json!(page_url));
    assert_eq!(response["params"]["response"]["status"], json!(200));

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_network_before_promotion() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-disable-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-DISABLE",
        "TID-000000000PND",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194958,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194958, json!({}), None);

    ctx.process_async(json!({
        "id": 104194959,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194959, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194960,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194960, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194961,
        "method": "Network.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194961, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(
            active
                .parked_page_session_state(&second_target_id)
                .is_none(),
            "network disable should collapse staged parked state back to default"
        );
        assert!(
            active
                .background_target(&second_target_id)
                .expect("background target")
                .runtime_slot
                .network_artifacts_are_default_for_test(),
            "network disable should clear staged parked network artifacts"
        );
    }

    ctx.process_async(json!({
        "id": 104194962,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url.clone() }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194962);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Network.requestWillBeSent")
                    | Some("Network.responseReceived")
                    | Some("Network.loadingFinished")
            )
        }),
        "active target should not emit network events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194963,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PND"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194963);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194964,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194964);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!(second_session_id)
                && matches!(
                    message["method"].as_str(),
                    Some("Network.requestWillBeSent")
                        | Some("Network.responseReceived")
                        | Some("Network.loadingFinished")
                )
        }),
        "disabled network should not emit first-navigation network events after promotion: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_cache_and_service_worker_policy_before_promotion()
 {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-policy-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-POLICY",
        "TID-000000000NP",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194956,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194956, json!({}), None);

    ctx.process_async(json!({
        "id": 104194957,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-POLICY", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_first_matching(
        "Target.targetCreated for staged network-policy target",
        |message| message["method"] == json!("Target.targetCreated"),
    );
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_first_matching(
        "Target.attachedToTarget for staged network-policy target",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(second_target_id)
        },
    );
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194957, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194958,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194958, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194959,
        "method": "Network.setCacheDisabled",
        "sessionId": second_session_id,
        "params": { "cacheDisabled": true }
    }))
    .await;
    ctx.expect_result(104194959, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194960,
        "method": "Network.setBypassServiceWorker",
        "sessionId": second_session_id,
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_result(104194960, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(!active.active_page_state().network_policy.cache_disabled());
        assert!(
            !active
                .active_page_state()
                .network_policy
                .bypass_service_worker()
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.runtime_slot.primary_network_events_enabled());
        assert!(staged.network_policy.cache_disabled());
        assert!(staged.network_policy.bypass_service_worker());
    }

    ctx.process_async(json!({
        "id": 104194961,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194961);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Network.requestWillBeSent")),
        "active target should not emit network events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194962,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000NP"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194962);
    ctx.take_all();

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
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
    }

    ctx.process_async(json!({
        "id": 104194963,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194963);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    wait_for_session_main_document_loading_finished(
        &mut ctx,
        &second_session_id,
        &page_url,
        "promoted session main-document completion with staged network policy",
    )
    .await;
    let emitted = ctx.take_all();
    let request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .cloned()
        .expect("promoted target should emit requestWillBeSent");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));
    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_cache_and_service_worker_policy_before_promotion()
 {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-policy-disable-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-POLICY-DISABLE",
        "TID-000000000NPD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194965,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194965, json!({}), None);

    ctx.process_async(json!({
        "id": 104194966,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-POLICY-DISABLE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_first_matching(
        "Target.targetCreated for staged disabled network-policy target",
        |message| message["method"] == json!("Target.targetCreated"),
    );
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_first_matching(
        "Target.attachedToTarget for staged disabled network-policy target",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(second_target_id)
        },
    );
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194966, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194967,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194967, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194968,
        "method": "Network.setCacheDisabled",
        "sessionId": second_session_id,
        "params": { "cacheDisabled": true }
    }))
    .await;
    ctx.expect_result(104194968, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194969,
        "method": "Network.setBypassServiceWorker",
        "sessionId": second_session_id,
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_result(104194969, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194970,
        "method": "Network.setCacheDisabled",
        "sessionId": second_session_id,
        "params": { "cacheDisabled": false }
    }))
    .await;
    ctx.expect_result(104194970, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194971,
        "method": "Network.setBypassServiceWorker",
        "sessionId": second_session_id,
        "params": { "bypass": false }
    }))
    .await;
    ctx.expect_result(104194971, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(!active.active_page_state().network_policy.cache_disabled());
        assert!(
            !active
                .active_page_state()
                .network_policy
                .bypass_service_worker()
        );
        let staged = active
            .parked_page_session_state(&second_target_id)
            .expect("second target should have staged parked page session state");
        assert!(staged.runtime_slot.primary_network_events_enabled());
        assert!(!staged.network_policy.cache_disabled());
        assert!(!staged.network_policy.bypass_service_worker());
    }

    ctx.process_async(json!({
        "id": 104194972,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url.clone() }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194972);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Network.requestWillBeSent")),
        "active target should not emit network events before promotion: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194973,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000NPD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194973);
    ctx.take_all();

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("promoted browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            bc.active_page_state()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(!bc.active_page_state().network_policy.cache_disabled());
        assert!(
            !bc.active_page_state()
                .network_policy
                .bypass_service_worker()
        );
    }

    ctx.process_async(json!({
        "id": 104194974,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194974);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during promoted navigation: {:?}",
        ctx.sent
    );

    wait_for_session_main_document_loading_finished(
        &mut ctx,
        &second_session_id,
        &page_url,
        "promoted session main-document completion after staged policy reset",
    )
    .await;
    let emitted = ctx.take_all();
    let request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .cloned()
        .expect("promoted target should still emit requestWillBeSent");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));
    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));

    server.abort();
}
