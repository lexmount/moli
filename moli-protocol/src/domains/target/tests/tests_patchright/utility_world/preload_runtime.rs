use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_utility_world_binding_and_preload_stay_isolated_per_browser_context() {
    let mut ctx = TestContext::new();
    let first =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 225, 226, 227).await;

    ctx.process_async(json!({
        "id": 228,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": first.session_id,
        "params": {
            "source": "globalThis.__lm_isolated_context_marker = 'first';",
            "worldName": "utility"
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 228);
    assert_eq!(preload["sessionId"], json!(first.session_id));
    assert!(preload["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 229,
        "method": "Runtime.addBinding",
        "sessionId": first.session_id,
        "params": {
            "name": "isolatedUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 229);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "Patchright-style setup should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    for (id, session_id, label) in [(230_u64, first.session_id.as_str(), "first-page")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": format!("data:text/html,<body>{label}</body>")
            }
        }))
        .await;
        let navigation = take_response_by_id(&mut ctx, id);
        assert_eq!(navigation["sessionId"], json!(session_id));
        assert!(
            ctx.sent.iter().all(|message| {
                message.get("error").is_none()
                    && message["method"] != json!("Runtime.executionContextCreated")
                    && message["method"] != json!("Runtime.executionContextsCleared")
            }),
            "unexpected protocol/runtime event during navigation: {:?}",
            ctx.sent
        );
        ctx.take_all();
    }

    ctx.process_async(json!({
        "id": 231,
        "method": "Page.createIsolatedWorld",
        "sessionId": first.session_id,
        "params": {
            "frameId": first.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let first_utility_context = take_response_by_id(&mut ctx, 231)["result"]["executionContextId"]
        .as_i64()
        .expect("first utility context id");
    ctx.take_all();

    ctx.process_async(json!({
            "id": 232,
            "method": "Runtime.evaluate",
            "sessionId": first.session_id,
            "params": {
                "contextId": first_utility_context,
                "expression": "isolatedUtilityBinding('payload-first'); JSON.stringify([typeof globalThis.isolatedUtilityBinding, globalThis.__lm_isolated_context_marker])"
            }
        })).await;
    let first_eval = take_response_by_id(&mut ctx, 232);
    assert_eq!(
        first_eval["result"]["result"]["value"],
        json!("[\"function\",\"first\"]")
    );
    let first_binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("isolatedUtilityBinding")
        })
        .cloned()
        .expect("first browser context should emit bindingCalled");
    assert_eq!(
        first_binding_called["params"]["executionContextId"],
        json!(first_utility_context)
    );
    assert_eq!(
        first_binding_called["params"]["payload"],
        json!("payload-first")
    );
    ctx.sent.clear();

    let second =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 233, 234, 235).await;

    ctx.process_async(json!({
        "id": 236,
        "method": "Page.navigate",
        "sessionId": second.session_id,
        "params": {
            "url": "data:text/html,<body>second-page</body>"
        }
    }))
    .await;
    let second_navigation = take_response_by_id(&mut ctx, 236);
    assert_eq!(second_navigation["sessionId"], json!(second.session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "unexpected protocol/runtime event during second navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 237,
        "method": "Page.createIsolatedWorld",
        "sessionId": second.session_id,
        "params": {
            "frameId": second.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let second_utility_context = take_response_by_id(&mut ctx, 237)["result"]["executionContextId"]
        .as_i64()
        .expect("second utility context id");
    ctx.take_all();

    ctx.process_async(json!({
            "id": 238,
            "method": "Runtime.evaluate",
            "sessionId": second.session_id,
            "params": {
                "contextId": second_utility_context,
                "expression": "JSON.stringify([typeof globalThis.isolatedUtilityBinding, globalThis.__lm_isolated_context_marker ?? null])"
            }
        })).await;
    let second_eval = take_response_by_id(&mut ctx, 238);
    assert_eq!(
        second_eval["result"]["result"]["value"],
        json!("[\"undefined\",null]")
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("isolatedUtilityBinding")
        }),
        "utility-world binding/preload should not leak into a different browser context"
    );

    let active = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(active.id, second.browser_context_id);
    assert_eq!(active.active_target_id(), Some(second.target_id.as_str()));
    assert_eq!(active.active_session_id(), Some(second.session_id.as_str()));

    let first_context = ctx
        .conn
        .browser_contexts()
        .find(|bc| bc.id == first.browser_context_id)
        .expect("first browser context should still exist");
    assert_eq!(
        first_context
            .active_page_state()
            .active_target
            .owner_state
            .document_start_scripts
            .len(),
        1
    );
    assert!(
        first_context.active_page_state().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .any(|binding| {
                binding.name == "isolatedUtilityBinding"
                    && binding.execution_context_name.as_deref() == Some("utility")
            }),
        "first browser context should retain its utility-world binding definition"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_console_enable_stays_silent_while_console_api_is_usable() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 230,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 230)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 231,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 231)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 232,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id should exist");
    ctx.expect_result(232, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );
    let attached_changed = ctx.take_first_matching("attached targetInfoChanged", |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    });
    assert_eq!(
        attached_changed["params"]["targetInfo"]["attached"],
        json!(true)
    );

    ctx.process_async(json!({
        "id": 233,
        "method": "Console.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(233, json!({}), Some(&session_id));
    assert!(
        ctx.sent.is_empty(),
        "Console.enable should not emit setup events: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 234,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body>console</body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 234);
    assert_eq!(navigation["sessionId"], json!(session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 235,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 235)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "utility world creation should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 236,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "(() => { console.log('patchright-console'); globalThis.__lm_patchright_console_smoke = 'ok'; })()"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 236);
    assert!(
        evaluation.get("error").is_none(),
        "console smoke evaluation should succeed: {evaluation:?}"
    );

    ctx.process_async(json!({
        "id": 237,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis.__lm_patchright_console_smoke"
        }
    }))
    .await;
    let console_state = take_response_by_id(&mut ctx, 237);
    assert_eq!(console_state["result"]["result"]["value"], json!("ok"));
    assert!(
        ctx.sent
            .iter()
            .all(|message| { message["method"] != json!("Runtime.consoleAPICalled") }),
        "Patchright-style Console-only setup should not enable Runtime console events: {:?}",
        ctx.sent
    );
    assert!(
        ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Console.messageAdded")),
        "Console.enable should still surface Console.messageAdded for evaluated console API calls: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_pre_document_binding_registration_runs_in_main_world_on_navigation() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 238,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 238)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 239,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 239)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 240,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id should exist");
    ctx.expect_result(240, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 241,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": { "name": "preDocumentBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 241);
    assert_eq!(add_binding["result"], json!({}));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "pre-document binding registration should stay off Runtime.enable surfaces"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 242,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<body><script>globalThis.__lm_pre_document_binding_kind = typeof globalThis.preDocumentBinding; if (typeof globalThis.preDocumentBinding === 'function') globalThis.preDocumentBinding('payload-main-world');</script></body>"
            }
        })).await;
    let navigation = take_response_by_id(&mut ctx, 242);
    assert_eq!(navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "navigation without Runtime.enable should stay silent: {:?}",
        ctx.sent
    );
    let binding_called = ctx
        .wait_for_scheduler_message("pre-document main-world binding invocation", |message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("preDocumentBinding")
        })
        .await;
    assert_eq!(
        binding_called["params"]["payload"],
        json!("payload-main-world")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_existing_utility_world_binding_install_by_execution_context_id() {
    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 244, 245, 246, 247, 248).await;

    ctx.process_async(json!({
        "id": 249,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body>existing-utility-world</body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 249);
    assert_eq!(navigation["sessionId"], json!(attached.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 250,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": {
            "frameId": attached.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 250)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 251,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "utilityBindingById",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 251);
    assert_eq!(
        add_binding["result"],
        json!({}),
        "executionContextId-scoped addBinding should succeed: {add_binding:?}"
    );

    ctx.process_async(json!({
            "id": 252,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "utilityBindingById('payload-existing-by-id'); typeof globalThis.utilityBindingById"
            }
        })).await;
    let utility_eval = take_response_by_id(&mut ctx, 252);
    assert_eq!(utility_eval["result"]["result"]["value"], json!("function"));

    ctx.process_async(json!({
        "id": 253,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "typeof globalThis.utilityBindingById"
        }
    }))
    .await;
    let main_world = take_response_by_id(&mut ctx, 253);
    assert_eq!(main_world["result"]["result"]["value"], json!("undefined"));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("utilityBindingById")
        })
        .cloned()
        .expect("executionContextId binding should be callable in the existing utility world");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("payload-existing-by-id")
    );
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "utilityBindingById"),
        "executionContextId-scoped binding should stay session-local, not persisted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_existing_utility_world_remove_binding_clears_current_world() {
    let mut ctx = TestContext::new();
    let attached = create_attached_page_session_async(&mut ctx, 2530, 2531, 2532, 2533, 2534).await;

    ctx.process_async(json!({
        "id": 2535,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": {
            "url": "data:text/html,<body>existing-utility-world</body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 2535);
    assert_eq!(navigation["sessionId"], json!(attached.session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2536,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": {
            "frameId": attached.target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_id = take_response_by_id(&mut ctx, 2536)["result"]["executionContextId"]
        .as_i64()
        .expect("utility context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2537,
        "method": "Runtime.addBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "utilityBindingRemovedById",
            "executionContextId": utility_context_id
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 2537);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 2538,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "typeof globalThis.utilityBindingRemovedById"
        }
    }))
    .await;
    let installed = take_response_by_id(&mut ctx, 2538);
    assert_eq!(installed["result"]["result"]["value"], json!("function"));

    ctx.process_async(json!({
        "id": 2539,
        "method": "Runtime.removeBinding",
        "sessionId": attached.session_id,
        "params": {
            "name": "utilityBindingRemovedById"
        }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 2539);
    assert_eq!(remove_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 2540,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "contextId": utility_context_id,
            "expression": "typeof globalThis.utilityBindingRemovedById"
        }
    }))
    .await;
    let removed = take_response_by_id(&mut ctx, 2540);
    assert_eq!(removed["result"]["result"]["value"], json!("function"));

    ctx.sent.clear();
    ctx.process_async(json!({
            "id": 2541,
            "method": "Runtime.evaluate",
            "sessionId": attached.session_id,
            "params": {
                "contextId": utility_context_id,
                "expression": "globalThis.utilityBindingRemovedById('unexpected'); typeof globalThis.utilityBindingRemovedById"
            }
        })).await;
    let guarded = take_response_by_id(&mut ctx, 2541);
    assert_eq!(guarded["result"]["result"]["value"], json!("function"));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("utilityBindingRemovedById")
        }),
        "removed executionContextId binding should remain inert in the current utility world"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_pre_document_remove_binding_prevents_first_navigation_binding_replay()
{
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 254,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 254)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 255,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 255)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 256,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id should exist");
    ctx.expect_result(256, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 257,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": { "name": "temporaryPreDocumentBinding" }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 257);
    assert_eq!(add_binding["result"], json!({}));

    ctx.process_async(json!({
        "id": 258,
        "method": "Runtime.removeBinding",
        "sessionId": session_id,
        "params": { "name": "temporaryPreDocumentBinding" }
    }))
    .await;
    let remove_binding = take_response_by_id(&mut ctx, 258);
    assert_eq!(remove_binding["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_bindings
            .iter()
            .all(|binding| binding.name != "temporaryPreDocumentBinding"),
        "pre-document removeBinding should clear browser-context persistence"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "pre-document add/remove should stay off Runtime.enable surfaces"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 259,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<body><script>globalThis.__lm_removed_pre_document_binding_kind = typeof globalThis.temporaryPreDocumentBinding; if (typeof globalThis.temporaryPreDocumentBinding === 'function') globalThis.temporaryPreDocumentBinding('unexpected');</script></body>"
            }
        })).await;
    let navigation = take_response_by_id(&mut ctx, 259);
    assert_eq!(navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "navigation without Runtime.enable should stay silent: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("temporaryPreDocumentBinding")
        }),
        "removed pre-document binding should not fire during first navigation"
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 260,
        "method": "Runtime.enable",
        "sessionId": session_id
    }))
    .await;
    let enable = take_response_by_id(&mut ctx, 260);
    assert_eq!(enable["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 261,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "globalThis.__lm_removed_pre_document_binding_kind"
        }
    }))
    .await;
    let kind = take_response_by_id(&mut ctx, 261);
    assert_eq!(
        kind["result"]["result"]["value"],
        json!("undefined"),
        "removed pre-document binding surface should evaluate cleanly: {kind:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_pre_document_remove_utility_world_preload_prevents_first_navigation_injection()
 {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 262,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 262)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 263,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 263)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 264,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id should exist");
    ctx.expect_result(264, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 265,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "globalThis.__lm_removed_utility_preload = 'ready';",
            "worldName": "utility"
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 265);
    let identifier = add_script["result"]["identifier"]
        .as_str()
        .expect("preload identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 266,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "identifier": identifier
        }
    }))
    .await;
    let remove_script = take_response_by_id(&mut ctx, 266);
    assert_eq!(remove_script["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .active_target
            .owner_state
            .document_start_scripts
            .is_empty(),
        "removed utility preload should not remain persisted on the browser context"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "pre-document utility preload add/remove should stay off Runtime.enable surfaces"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 267,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<body><script>globalThis.__lm_removed_utility_preload_kind = typeof globalThis.__lm_removed_utility_preload;</script></body>"
            }
        })).await;
    let navigation = take_response_by_id(&mut ctx, 267);
    assert_eq!(navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "navigation without Runtime.enable should stay silent: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 268,
        "method": "Runtime.enable",
        "sessionId": session_id
    }))
    .await;
    let enable = take_response_by_id(&mut ctx, 268);
    assert_eq!(enable["result"], json!({}));
    let utility_context = ctx.sent.iter().find(|message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["name"] == json!("utility")
    });
    assert!(
        utility_context.is_none(),
        "removed pre-document utility preload should not materialize a utility world on first navigation"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 269,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "globalThis.__lm_removed_utility_preload_kind"
        }
    }))
    .await;
    let kind = take_response_by_id(&mut ctx, 269);
    assert_eq!(kind["result"]["result"]["value"], json!("undefined"));
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_existing_utility_world_preload_run_immediately_persists_without_runtime_enable()
 {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 270,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 270)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 271,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 271)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 272,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id should exist");
    ctx.expect_result(272, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 273,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body>existing-utility-world</body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 273);
    assert_eq!(navigation["sessionId"], json!(session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 274,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let initial_utility_context_id =
        take_response_by_id(&mut ctx, 274)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 275,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "globalThis.__lm_existing_utility_preload = 'ready-now';",
            "worldName": "utility",
            "runImmediately": true
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 275);
    assert!(add_script["result"]["identifier"].is_string());
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                || message["method"] == json!("Runtime.executionContextsCleared")
        }),
        "existing utility-world preload install should stay off Runtime.enable surfaces: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 276,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": initial_utility_context_id,
            "expression": "globalThis.__lm_existing_utility_preload"
        }
    }))
    .await;
    let initial_eval = take_response_by_id(&mut ctx, 276);
    assert_eq!(
        initial_eval["result"]["result"]["value"],
        json!("ready-now")
    );

    ctx.process_async(json!({
        "id": 277,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body>after-nav</body>"
        }
    }))
    .await;
    let second_navigation = take_response_by_id(&mut ctx, 277);
    assert_eq!(second_navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "navigation without Runtime.enable should stay silent: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 278,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let replayed_utility_context_id =
        take_response_by_id(&mut ctx, 278)["result"]["executionContextId"]
            .as_i64()
            .expect("replayed utility context id");

    ctx.process_async(json!({
        "id": 279,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": replayed_utility_context_id,
            "expression": "globalThis.__lm_existing_utility_preload"
        }
    }))
    .await;
    let replayed_eval = take_response_by_id(&mut ctx, 279);
    assert_eq!(
        replayed_eval["result"]["result"]["value"],
        json!("ready-now")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_over_cdp_existing_utility_world_preload_remove_keeps_current_world_but_blocks_future_replay()
 {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 280,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 280)["result"]["browserContextId"]
        .as_str()
        .expect("browser context id")
        .to_owned();

    ctx.process_async(json!({
        "id": 281,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = take_response_by_id(&mut ctx, 281)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event(
        "Target.targetCreated",
        Some(&json!({
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 282,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id should exist");
    ctx.expect_result(282, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 283,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body>existing-utility-world</body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 283);
    assert_eq!(navigation["sessionId"], json!(session_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 284,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let initial_utility_context_id =
        take_response_by_id(&mut ctx, 284)["result"]["executionContextId"]
            .as_i64()
            .expect("utility context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 285,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "globalThis.__lm_existing_utility_removed_later = 'ready-now';",
            "worldName": "utility",
            "runImmediately": true
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 285);
    let identifier = add_script["result"]["identifier"]
        .as_str()
        .expect("preload identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 286,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": initial_utility_context_id,
            "expression": "globalThis.__lm_existing_utility_removed_later"
        }
    }))
    .await;
    let initial_eval = take_response_by_id(&mut ctx, 286);
    assert_eq!(
        initial_eval["result"]["result"]["value"],
        json!("ready-now")
    );

    ctx.process_async(json!({
        "id": 287,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "identifier": identifier
        }
    }))
    .await;
    let remove_script = take_response_by_id(&mut ctx, 287);
    assert_eq!(remove_script["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_state()
            .active_target
            .owner_state
            .document_start_scripts
            .is_empty(),
        "removed runImmediately utility preload should not remain persisted"
    );

    ctx.process_async(json!({
        "id": 288,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": initial_utility_context_id,
            "expression": "globalThis.__lm_existing_utility_removed_later"
        }
    }))
    .await;
    let still_present = take_response_by_id(&mut ctx, 288);
    assert_eq!(
        still_present["result"]["result"]["value"],
        json!("ready-now")
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 289,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body>after-nav</body>"
        }
    }))
    .await;
    let second_navigation = take_response_by_id(&mut ctx, 289);
    assert_eq!(second_navigation["sessionId"], json!(session_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message.get("error").is_none()
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "navigation without Runtime.enable should stay silent: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 290,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let replayed_utility_context_id =
        take_response_by_id(&mut ctx, 290)["result"]["executionContextId"]
            .as_i64()
            .expect("replayed utility context id");

    ctx.process_async(json!({
        "id": 291,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "contextId": replayed_utility_context_id,
            "expression": "typeof globalThis.__lm_existing_utility_removed_later"
        }
    }))
    .await;
    let replayed_eval = take_response_by_id(&mut ctx, 291);
    assert_eq!(
        replayed_eval["result"]["result"]["value"],
        json!("undefined")
    );
}
