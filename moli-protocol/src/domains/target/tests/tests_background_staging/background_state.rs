use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_pre_document_state_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE",
        "TID-000000000PA",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });
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
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            assert_ne!(active.active_target_id(), Some(second_target_id.as_str()));
            let staged = active
                .background_target(&second_target_id)
                .expect("staged background target");
            let staged_devtools_state = active
                .background_target(staged.target_id())
                .filter(|target| target.has_non_default_session_state())
                .expect("staged page session state")
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_bindings
                .as_slice();
            assert_eq!(staged_devtools_state.len(), 1);
            assert_eq!(staged_devtools_state[0].name, "targetBPreDocumentBinding");
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
                script_id
            );
            assert!(
                active.active_page_target().devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_bindings
                    .is_empty(),
                "active target DevTools session must not inherit staged target binding"
            );
            assert!(
                active
                    .active_page_target()
                    .owner_state
                    .document_start_scripts
                    .is_empty()
            );
        });
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
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        });
    }

    ctx.navigate_with_page_events(json!({
        "id": 104192,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>activated</title><div id='ok'>activated target</div>"
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
    let activated_eval = take_response_by_id(&mut ctx, 104193);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["binding"], json!("function"));
    assert_eq!(activated_payload["preload"], json!("from-target-b"));
    assert_eq!(activated_payload["text"], json!("activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_utility_pre_document_state_before_activation()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-UTILITY",
        "TID-000000000PU",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            let staged = active
                .background_target(&second_target_id)
                .expect("staged background target");
            let staged_devtools_state = active
                .background_target(staged.target_id())
                .filter(|target| target.has_non_default_session_state())
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
                script_id
            );
            assert!(
                active.active_page_target().devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_bindings
                    .is_empty(),
                "active target DevTools session must not inherit staged target binding"
            );
            assert!(
                active
                    .active_page_target()
                    .owner_state
                    .document_start_scripts
                    .is_empty()
            );
        });
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

    ctx.navigate_with_page_events(json!({
            "id": 104194127,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>activated</title><div id='ok'>activated utility target</div>"
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
        .expect("activated target utility world should materialize its staged binding/preload");
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
    let activated_eval = take_response_by_id(&mut ctx, 104194129);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["binding"], json!("function"));
    assert_eq!(activated_payload["preload"], json!("from-target-b-utility"));
    assert_eq!(activated_payload["text"], json!("activated utility target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_binding_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE",
        "TID-000000000PRM",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            let staged = active
                .background_target(&second_target_id)
                .expect("staged background target");
            let staged_bindings_empty = active
                .background_target(staged.target_id())
                .filter(|target| target.has_non_default_session_state())
                .is_none_or(|state| {
                    state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                        .runtime_bindings
                        .is_empty()
                });
            assert!(
                staged_bindings_empty,
                "removed binding should be cleared from background DevTools session"
            );
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
                script_id
            );
        });
    }

    ctx.process_async(json!({
        "id": 104194105,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRM" }
    }))
    .await;
    ctx.expect_result(104194105, json!({}), None);
    ctx.take_all();

    ctx.navigate_with_page_events(json!({
        "id": 104194106,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>activated</title><div id='ok'>activated target</div>"
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
        "removed binding should not replay into first activated navigation: {:?}",
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
    let activated_eval = take_response_by_id(&mut ctx, 104194107);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["binding"], json!("undefined"));
    assert_eq!(activated_payload["preload"], json!("undefined"));
    assert_eq!(activated_payload["text"], json!("activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_preload_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE-SCRIPT",
        "TID-000000000PRS",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            let staged = active
                .background_target(&second_target_id)
                .expect("staged background target");
            let staged_bindings = &active
                .background_target(staged.target_id())
                .filter(|target| target.has_non_default_session_state())
                .expect("staged page session state")
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_bindings;
            assert_eq!(staged_bindings.len(), 1);
            assert!(
                active
                    .background_target(staged.target_id())
                    .expect("background target must exist")
                    .owner_state
                    .document_start_scripts
                    .is_empty()
            );
        });
    }

    ctx.process_async(json!({
        "id": 104194115,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRS" }
    }))
    .await;
    ctx.expect_result(104194115, json!({}), None);
    ctx.take_all();

    ctx.navigate_with_page_events(json!({
        "id": 104194116,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>activated</title><div id='ok'>activated target</div>"
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
        "removed preload should not trigger binding call during first activated navigation: {:?}",
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
    let activated_eval = take_response_by_id(&mut ctx, 104194117);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["binding"], json!("function"));
    assert_eq!(activated_payload["preload"], json!("absent"));
    assert_eq!(activated_payload["text"], json!("activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_utility_binding_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE-UTILITY-BINDING",
        "TID-000000000PRU",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            let staged = active
                .background_target(&second_target_id)
                .expect("staged background target");
            let staged_bindings_empty = active
                .background_target(staged.target_id())
                .filter(|target| target.has_non_default_session_state())
                .is_none_or(|state| {
                    state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                        .runtime_bindings
                        .is_empty()
                });
            assert!(
                staged_bindings_empty,
                "removed utility binding should be cleared from background DevTools session"
            );
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
                script_id
            );
        });
    }

    ctx.process_async(json!({
        "id": 104194135,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRU" }
    }))
    .await;
    ctx.expect_result(104194135, json!({}), None);
    ctx.take_all();

    ctx.navigate_with_page_events(json!({
            "id": 104194136,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>activated</title><div id='ok'>activated utility target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194136);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed utility binding should not replay into first activated utility world: {:?}",
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
    let activated_eval = take_response_by_id(&mut ctx, 104194138);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["binding"], json!("undefined"));
    assert_eq!(activated_payload["preload"], json!("undefined"));
    assert_eq!(activated_payload["text"], json!("activated utility target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_remove_its_own_utility_preload_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-REMOVE-UTILITY-SCRIPT",
        "TID-000000000PRV",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let active = {
                conn.browser_context
                    .as_ref()
                    .expect("active browser context")
            };
            let staged = active
                .background_target(&second_target_id)
                .expect("staged background target");
            let staged_bindings = &active
                .background_target(staged.target_id())
                .filter(|target| target.has_non_default_session_state())
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
                    .background_target(staged.target_id())
                    .expect("background target must exist")
                    .owner_state
                    .document_start_scripts
                    .is_empty()
            );
        });
    }

    ctx.process_async(json!({
        "id": 104194145,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000PRV" }
    }))
    .await;
    ctx.expect_result(104194145, json!({}), None);
    ctx.take_all();

    ctx.navigate_with_page_events(json!({
            "id": 104194146,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>activated</title><div id='ok'>activated utility target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194146);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "removed utility preload should not trigger binding call during first activated navigation: {:?}",
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
    let activated_eval = take_response_by_id(&mut ctx, 104194148);
    let activated_payload = activated_eval["result"]["result"]["value"]
        .as_str()
        .expect("activated payload should be string");
    let activated_payload: serde_json::Value =
        serde_json::from_str(activated_payload).expect("activated payload should be valid json");
    assert_eq!(activated_payload["binding"], json!("function"));
    assert_eq!(activated_payload["preload"], json!("absent"));
    assert_eq!(activated_payload["text"], json!("activated utility target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_emulated_media_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-MEDIA",
        "TID-000000000PM",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });
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
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(active.active_target_id(), Some("TID-000000000PM"));
                assert_eq!(
                    active
                        .active_page_target()
                        .effective_emulation_state
                        .emulated_media
                        .color_scheme
                        .as_deref(),
                    Some("dark")
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert_eq!(
                staged
                    .effective_emulation_state
                    .emulated_media
                    .color_scheme
                    .as_deref(),
                Some("light")
            );
        });
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
async fn same_context_background_session_can_clear_its_own_emulated_media_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-MEDIA-CLEAR",
        "TID-000000000PMC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });
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
        ctx.with_fixture(|conn| {
let active = {
            conn.browser_context
                .as_ref()
                .expect("active browser context")
        };
        assert_eq!(active.active_target_id(), Some("TID-000000000PMC"));
        assert!(
            active
                .active_page_target()
                .effective_emulation_state
                .emulated_media
                .color_scheme
                .is_none(),
            "active target should keep its default emulated media",
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "clearing staged emulated media back to default should fold away the background state entry",
        );
});
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
    let activated_surface = html
        .split("<body>")
        .nth(1)
        .and_then(|tail| tail.split("</body>").next())
        .expect("activated payload should be embedded in body");
    let activated_surface = serde_json::json!(
        activated_surface
            .split('|')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        activated_surface, default_surface,
        "activated target should observe default emulated media after clearing its staged override; got {html}"
    );
    assert_ne!(
        activated_surface,
        serde_json::json!(["true", "false"]),
        "activated target should not retain the staged dark override"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_network_conditions_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET",
        "TID-000000000PN",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });
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
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1,
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
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1,
        }
    }))
    .await;
    ctx.expect_result(10419453, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(active.active_target_id(), Some("TID-000000000PN"));
                assert!(!active.active_page_target().network_policy.network_offline());

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert!(staged.network_policy.network_offline());
        });
    }

    ctx.navigate_with_page_events(json!({
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
        ctx.with_fixture(|conn| {
            let activated = {
                conn.browser_context
                    .as_ref()
                    .expect("activated browser context")
            };
            assert_eq!(
                activated.active_target_id(),
                Some(second_target_id.as_str())
            );
            assert_eq!(
                activated.active_session_id(),
                Some(second_session_id.as_str())
            );
            assert!(
                activated
                    .active_page_target()
                    .network_policy
                    .network_offline()
            );
        });
    }

    ctx.navigate_with_page_events(json!({
        "id": 10419456,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": "http://example.test/offline-activated" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let activated_navigation = take_response_by_id(&mut ctx, 10419456);
    assert!(
        activated_navigation.get("error").is_none(),
        "offline navigation must not return a protocol error; got {activated_navigation}"
    );
    assert_eq!(
        activated_navigation["result"]["errorText"],
        json!("net::ERR_INTERNET_DISCONNECTED")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_blocked_urls_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-BLOCK",
        "TID-000000000PB",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });
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
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert!(
                    active
                        .active_page_target()
                        .effective_policy()
                        .blocked_url_patterns()
                        .is_empty(),
                    "active target should keep its own block list"
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert!(
                staged.effective_policy().blocked_url_patterns().is_empty(),
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
        });
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
        ctx.with_fixture(|conn| {
            let activated = {
                conn.browser_context
                    .as_ref()
                    .expect("activated browser context")
            };
            assert_eq!(
                activated.active_target_id(),
                Some(second_target_id.as_str())
            );
            assert!(
                activated
                    .active_page_target()
                    .effective_policy()
                    .blocked_url_patterns()
                    .is_empty(),
                "activation must not activate a disabled Network handler"
            );
        });
    }

    ctx.process_async(json!({
        "id": 10419461,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(10419461, json!({}), Some(&second_session_id));
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.browser_context
                .as_ref()
                .expect("activated browser context")
                .active_page_target()
                .effective_policy()
                .blocked_url_patterns(),
            ["http://example.test/blocked/*".to_owned()],
            "Network.enable must activate the staged background-session contribution"
        )
    });

    ctx.navigate_with_page_events(json!({
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
    let activated_navigation = take_response_by_id(&mut ctx, 10419462);
    assert_eq!(
        activated_navigation["error"]["message"],
        json!("net::ERR_BLOCKED_BY_CLIENT")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_reset_its_own_network_conditions_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-RESET",
        "TID-000000000PR",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });
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
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1,
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
        }
    }))
    .await;
    ctx.expect_result(104194504, json!({}), Some(&second_session_id));

    {
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(active.active_target_id(), Some("TID-000000000PR"));
                assert!(
                    !active.active_page_target().network_policy.network_offline(),
                    "active target should keep its default online state",
                );

                active
                    .background_target(&second_target_id)
                    .expect("second target should have staged background page session state")
            };
            assert!(!staged.network_policy.network_offline());
            assert!(
                !staged.has_non_default_session_state(),
                "offline reset should return to the default policy"
            );
        });
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
            "params": { "url": "data:text/html,<title>activated-online</title><div id='ok'>activated online</div>" }
        })).await;
    let activated_navigation = take_response_by_id(&mut ctx, 104194506);
    assert_eq!(
        activated_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    {
        ctx.with_fixture(|conn| {
            let activated = conn.browser_context.as_ref().expect("browser context");
            assert_eq!(
                activated.active_target_id(),
                Some(second_target_id.as_str())
            );
            assert!(
                !activated
                    .active_page_target()
                    .network_policy
                    .network_offline()
            );
        });
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_extra_headers_before_activation() {
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
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(
                    active
                        .active_page_target()
                        .effective_policy()
                        .extra_headers()
                        .to_byte_strings(),
                    vec![("X-Target".into(), "A".into())]
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert_eq!(
                staged.effective_policy().extra_headers().to_byte_strings(),
                vec![("X-Target".into(), "B".into())]
            );
        });
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
        "unexpected protocol error during activated navigation: {:?}",
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
async fn same_context_background_session_can_clear_its_own_extra_headers_before_activation() {
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
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(
                    active
                        .active_page_target()
                        .effective_policy()
                        .extra_headers()
                        .to_byte_strings(),
                    vec![("X-Target".into(), "A".into())]
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("enabled background session should retain its target-owned state")
            };
            assert!(staged.effective_policy().extra_headers().is_empty());
        });
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
        "unexpected protocol error during activated navigation: {:?}",
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
async fn same_context_background_session_can_stage_its_own_user_agent_before_activation() {
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
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert_eq!(
                    active
                        .active_page_target()
                        .effective_policy()
                        .browser_identity_override()
                        .map(|identity| identity.user_agent()),
                    Some("Moli/Stage-A")
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert_eq!(
                staged
                    .effective_policy()
                    .browser_identity_override()
                    .map(|identity| identity.user_agent()),
                Some("Moli/Stage-B")
            );
        });
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
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 10419477,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": { "expression": "navigator.userAgent" }
    }))
    .await;
    let activated_eval = take_response_by_id(&mut ctx, 10419477);
    assert_eq!(
        activated_eval["result"]["result"]["value"],
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
async fn same_context_background_session_can_clear_its_own_user_agent_before_activation() {
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
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .unwrap()
            .attach_active_session("SID-active")
    });

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
    let navigation = take_response_by_id(&mut ctx, 104194702);
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        Some("SID-active"),
        "TID-000000000PUC",
        navigation["result"]["loaderId"].as_str().unwrap(),
    )
    .await;
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
        ctx.with_fixture(|conn| {
            let staged = {
                let active = {
                    conn.browser_context
                        .as_ref()
                        .expect("active browser context")
                };
                assert!(
                    active
                        .active_page_target()
                        .effective_policy()
                        .browser_identity_override()
                        .map(|identity| identity.user_agent())
                        .is_none(),
                    "active target should keep its default user agent override",
                );

                active
                    .background_target(&second_target_id)
                    .filter(|target| target.has_non_default_session_state())
                    .expect("second target should have staged background page session state")
            };
            assert_eq!(
                staged
                    .effective_policy()
                    .browser_identity_override()
                    .map(|identity| identity.user_agent()),
                Some(default_ua.as_str())
            );
        });
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
        "unexpected protocol error during activated navigation: {:?}",
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
        "activated target should not retain the staged user agent override",
    );

    server.abort();
}
