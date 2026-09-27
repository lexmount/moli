use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_network_child_playwright_style_utility_script_uses_child_scope()
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
            "<!doctype html><html><body>child-navigate-playwright-eval</body></html>",
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
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_and_wait_for_response_async(json!({
                "id": 4100,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/parent") }
            }))
            .await;
            let _ = take_response_by_id(&mut ctx, 4100);

            wait_until_message(
                &mut ctx,
                "SID-1",
                "navigate child frame attached",
                |message| {
                    message["method"] == json!("Page.frameAttached")
                        && message["params"]["parentFrameId"] == json!("TID-1")
                },
            )
            .await;
            let child_frame_id = ctx
                .sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Page.frameAttached")
                        && message["params"]["parentFrameId"] == json!("TID-1")
                })
                .and_then(|message| message["params"]["frameId"].as_str())
                .map(str::to_owned)
                .expect("child frame should emit Page.frameAttached");
            wait_until_message(
                &mut ctx,
                "SID-1",
                "navigate child default execution context",
                |message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(child_frame_id)
                },
            )
            .await;
            let child_default_context_id = ctx
                .sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(child_frame_id)
                })
                .and_then(|message| message["params"]["context"]["id"].as_i64())
                .expect("child default execution context id");

            ctx.sent.clear();

            ctx.process_async(json!({
                "id": 4101,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "contextId": child_default_context_id,
                    "expression": "(() => { const module = { exports: {} }; class UtilityScript { constructor(global, isUnderTest) { this.global = global; this.isUnderTest = isUnderTest; } evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? result : result; } } module.exports.UtilityScript = () => UtilityScript; return new (module.exports.UtilityScript())(globalThis, false); })()"
                }
            }))
            .await;
            let utility_response = take_response_by_id(&mut ctx, 4101);
            let object_id = utility_response["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    panic!("playwright-style utility object id: {utility_response:?}")
                });

            ctx.process_async(json!({
                "id": 4102,
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
            let result = take_response_by_id(&mut ctx, 4102);
            assert_eq!(
                result["result"]["result"]["value"],
                json!("child-navigate-playwright-eval")
            );
        })
        .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_removing_child_iframe_emits_frame_detached_and_forgets_owner_state() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 5242,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='removable' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5242);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "removable child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    assert!(ctx.conn.has_attached_child_frame_id(&child_frame_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5243,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('removable').remove(); true",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 5243);
    assert_eq!(response["result"]["result"]["value"], json!(true));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "removed child frameDetached",
        |message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(child_frame_id)
        },
    )
    .await;

    let detached = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        detached.len(),
        1,
        "frame detach must be emitted exactly once"
    );
    assert_eq!(detached[0]["params"]["reason"], json!("remove"));
    assert_eq!(detached[0]["sessionId"], json!("SID-1"));
    assert!(
        !ctx.conn.has_attached_child_frame_id(&child_frame_id),
        "detached frame must be removed from protocol owner state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_insert_then_remove_iframe_preserves_attach_before_detach() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5244,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const frame = document.createElement('iframe'); document.body.appendChild(frame); frame.remove(); return true; })()",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 5244);
    assert_eq!(response["result"]["result"]["value"], json!(true));
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "same-command child frameAttached followed by frameDetached",
        |messages| {
            messages
                .iter()
                .enumerate()
                .any(|(attached_index, attached)| {
                    if attached["method"] != json!("Page.frameAttached") {
                        return false;
                    }
                    messages.iter().skip(attached_index + 1).any(|detached| {
                        detached["method"] == json!("Page.frameDetached")
                            && detached["params"]["frameId"] == attached["params"]["frameId"]
                    })
                })
        },
    )
    .await;

    let attached_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameAttached"))
        .expect("same-command insertion should emit Page.frameAttached");
    let child_frame_id = ctx.sent[attached_index]["params"]["frameId"]
        .as_str()
        .expect("attached child frame id")
        .to_owned();
    let detached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("same-command removal should emit Page.frameDetached");
    assert!(attached_index < detached_index);
    assert_eq!(
        ctx.sent[detached_index]["params"]["reason"],
        json!("remove")
    );
    assert!(!ctx.conn.has_attached_child_frame_id(&child_frame_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_removing_nested_iframe_detaches_descendant_before_parent() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 5245,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='outer' srcdoc=\"<iframe srcdoc='<body>inner</body>'></iframe>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5245);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child frame attachments",
        |messages| {
            messages
                .iter()
                .find(|message| {
                    message["method"] == json!("Page.frameAttached")
                        && message["params"]["parentFrameId"] == json!("TID-1")
                })
                .and_then(|message| message["params"]["frameId"].as_str())
                .is_some_and(|outer_frame_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Page.frameAttached")
                            && message["params"]["parentFrameId"] == json!(outer_frame_id)
                    })
                })
        },
    )
    .await;
    let outer_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("outer child frame should attach");
    let inner_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(outer_frame_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("inner child frame should attach");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5246,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('outer').remove(); true",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5246);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child frameDetached events",
        |messages| {
            [inner_frame_id.as_str(), outer_frame_id.as_str()]
                .into_iter()
                .all(|frame_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Page.frameDetached")
                            && message["params"]["frameId"] == json!(frame_id)
                    })
                })
        },
    )
    .await;

    let inner_detached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(inner_frame_id)
        })
        .expect("inner frame should detach");
    let outer_detached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(outer_frame_id)
        })
        .expect("outer frame should detach");
    assert!(
        inner_detached_index < outer_detached_index,
        "Chromium detaches descendants before their parent: {:?}",
        ctx.sent
    );
    assert!(!ctx.conn.has_attached_child_frame_id(&inner_frame_id));
    assert!(!ctx.conn.has_attached_child_frame_id(&outer_frame_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_emits_nested_child_default_context() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    browser_context.active_page_target_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    browser_context.active_page_target_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 5242,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='outer' name='outer-frame' srcdoc=\"<iframe name='inner-frame' srcdoc='<body>inner</body>'></iframe>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5242);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child attachments and Runtime contexts",
        |messages| {
            let child_attachments = messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameAttached"))
                .count();
            let child_default_contexts = messages
                .iter()
                .filter(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"] != json!("TID-1")
                })
                .count();
            child_attachments >= 2 && child_default_contexts >= 2
        },
    )
    .await;

    let outer_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("outer frame should emit Page.frameAttached");
    let inner_context_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"]
                    .as_str()
                    .is_some_and(|frame_id| {
                        frame_id != "TID-1" && frame_id != outer_frame_id.as_str()
                    })
        })
        .and_then(|message| message["params"]["context"]["auxData"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("navigation should emit nested child default execution context");
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(inner_context_frame_id)
                && message["params"]["parentFrameId"] == json!(outer_frame_id)
        }),
        "nested child default context should belong to an attached inner frame; sent={:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_legacy_runtime_frontend_projection_emits_context_creation_without_synthetic_clear()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_and_wait_for_response_async(json!({
                "id": 22,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": "data:text/html,<body>hi</body>" }
            }))
            .await;
            wait_until_scheduler_message(
                &mut ctx,
                "legacy default Runtime execution context",
                |message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"] == json!("TID-1")
                },
            )
            .await;
            wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

            assert_eq!(ctx.take_one()["method"], "Page.frameStartedNavigating");
            assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");

            let result = ctx.take_one();
            assert_eq!(result["id"], 22);
            assert_eq!(result["sessionId"], "SID-1");

            assert_eq!(ctx.take_one()["method"], "Page.frameNavigated");
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");

            let created = ctx.take_one();
            assert_eq!(created["method"], "Runtime.executionContextCreated");
            assert_eq!(created["sessionId"], "SID-1");
            assert_eq!(
                created["params"]["context"]["name"],
                json!("data:text/html,<body>hi</body>")
            );
            assert!(created["params"]["context"]["id"].as_i64().is_some());
            assert_eq!(
                created["params"]["context"]["auxData"]["isDefault"],
                json!(true)
            );
            assert!(
                !ctx.sent
                    .iter()
                    .any(|message| message["method"]
                        == json!("Runtime.executionContextsCleared")),
                "legacy protocol projection must not synthesize a Runtime.executionContextsCleared event: {:?}",
                ctx.sent
            );
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
            assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");

            assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");
            assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");
            assert!(ctx.sent.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_resets_before_creating_default_context() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
        }),
        "real Runtime.enable should connect the renderer V8 Runtime agent: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    assert_runtime_navigation_context_reset(&sent, "SID-1", "TID-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_fans_out_context_reset_to_attached_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(21, "SID-1"), (22, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        assert!(
            ctx.sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(session_id)
            }),
            "Runtime.enable should connect renderer V8 Runtime agent for {session_id}: {:?}",
            ctx.sent
        );
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>multi-session</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    for session_id in ["SID-1", "SID-attached"] {
        assert_runtime_navigation_context_reset(&sent, session_id, "TID-1");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_from_attached_session_keeps_primary_and_attached_runtime_events_separate() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(21, "SID-1"), (22, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-attached",
        "params": { "url": "data:text/html,<body>aux-session-nav</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    for session_id in ["SID-1", "SID-attached"] {
        assert_runtime_navigation_context_reset(&sent, session_id, "TID-1");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_attached_runtime_disable_keeps_primary_runtime_enabled() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(21, "SID-1"), (22, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        ctx.sent.clear();
    }
    ctx.process_async(json!({
        "id": 23,
        "method": "Runtime.disable",
        "sessionId": "SID-attached"
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 23)["result"],
        json!({}),
        "attached Runtime.disable should succeed through its own V8 session"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24,
        "method": "Page.navigate",
        "sessionId": "SID-attached",
        "params": { "url": "data:text/html,<body>aux-runtime-disabled</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    assert_runtime_navigation_context_reset(&sent, "SID-1", "TID-1");
    assert!(
        sent.iter().all(|message| {
            message["sessionId"] != json!("SID-attached")
                || !matches!(
                    message["method"].as_str(),
                    Some("Runtime.executionContextsCleared")
                        | Some("Runtime.executionContextCreated")
                )
        }),
        "Runtime-disabled attached session must stay off context lifecycle surfaces after navigation: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_staged_attached_runtime_enable_emits_context_created_for_attached_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );
    ctx.conn.commit_declared_session_fixtures_for_test();
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(Some("SID-1"), |state| {
            state.runtime_session_state.runtime_frontend_enabled = true;
        });
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(Some("SID-attached"), |state| {
            state.runtime_session_state.runtime_frontend_enabled = true;
        });

    ctx.process_async(json!({
        "id": 24,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>staged-multi-session</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    for session_id in ["SID-1", "SID-attached"] {
        assert!(
            sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(session_id)
            }),
            "staged Runtime-enabled session {session_id} should receive new default context on navigation: {sent:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_emits_initial_console_before_dcl() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 25,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<script>console.warn('boot warning')</script><body>hi</body>"
        }
    }))
    .await;

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let sent = ctx.take_all();
    let context_created_index = sent
        .iter()
        .position(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .unwrap_or_else(|| panic!("navigation should emit context creation: {sent:?}"));
    let console_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["type"] == json!("warning")
                && message["params"]["args"][0]["value"] == json!("boot warning")
        })
        .unwrap_or_else(|| panic!("navigation should emit initial console output: {sent:?}"));
    let dcl_index = sent
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .unwrap_or_else(|| panic!("navigation should emit DOMContentLoaded: {sent:?}"));

    assert!(
        context_created_index < console_index,
        "initial console must follow context creation so clients can resolve the context: {sent:?}"
    );
    assert!(
        console_index < dcl_index,
        "parser-time console output should be visible before DOMContentLoaded: {sent:?}"
    );
    assert_eq!(
        sent.iter()
            .filter(|message| message["method"] == json!("Runtime.consoleAPICalled"))
            .count(),
        1,
        "initial console output should not be duplicated by post-navigation capture: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_console_enabled_emits_initial_console_without_runtime_enable() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({
        "id": 26,
        "method": "Console.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(26, json!({}), Some("SID-1"));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .inspector_session_state
            .v8_state
            .is_none(),
        "pre-document Console.enable should remain a first-attach bootstrap without inventing a V8 cookie"
    );

    ctx.process_async(json!({
        "id": 27,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<script>console.warn('console only boot warning')</script><body>hi</body>"
        }
    }))
    .await;

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let sent = ctx.take_all();
    let console_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["params"]["message"]["text"] == json!("console only boot warning")
        })
        .unwrap_or_else(|| {
            panic!("Console-only navigation should emit parser-time console output: {sent:?}")
        });
    let dcl_index = sent
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .unwrap_or_else(|| panic!("navigation should emit DOMContentLoaded: {sent:?}"));

    assert!(
        console_index < dcl_index,
        "Console-only parser-time output should be visible before DOMContentLoaded: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Runtime.consoleAPICalled")),
        "Console-only navigation should not emit Runtime.consoleAPICalled without Runtime.enable: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_console_enable_survives_navigation_without_enabling_primary_or_runtime() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    ctx.process_async(json!({
        "id": 28,
        "method": "Console.enable",
        "sessionId": "SID-attached"
    }))
    .await;
    ctx.expect_result(28, json!({}), Some("SID-attached"));

    ctx.process_async(json!({
        "id": 29,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>hi</body>"
        }
    }))
    .await;

    let navigation = ctx.take_all();
    assert!(
        navigation.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared")
                    | Some("Runtime.executionContextCreated")
                    | Some("Runtime.consoleAPICalled")
            )
        }),
        "Console-only subscription must not implicitly enable Runtime surfaces: {navigation:?}"
    );

    ctx.process_async(json!({
        "id": 30,
        "method": "Runtime.evaluate",
        "sessionId": "SID-attached",
        "params": {
            "expression": "console.warn('aux console after navigation')"
        }
    }))
    .await;
    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["sessionId"] == json!("SID-attached")
                && message["params"]["message"]["text"] == json!("aux console after navigation")
        }),
        "Console-enabled attached session should retain its own V8 Console subscription after navigation: {sent:?}"
    );
    assert!(
        sent.iter().all(|message| {
            message["sessionId"] != json!("SID-1")
                || message["method"] != json!("Console.messageAdded")
        }),
        "Console-disabled primary session must not receive the attached subscription's events: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Runtime.consoleAPICalled")),
        "Console-only evaluation must not implicitly enable Runtime console events: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_without_runtime_frontend_enabled_emits_no_runtime_context_events() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;

    let messages = ctx.take_all();
    assert!(
        !messages.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared") | Some("Runtime.executionContextCreated")
            )
        }),
        "runtime context events should not be emitted when Runtime is disabled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_emits_child_default_execution_context_created() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 231,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 231);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child default Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;

    let child_context_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should emit child default execution context created");
    assert_eq!(child_context_created["sessionId"], json!("SID-1"));
    assert!(
        child_context_created["params"]["context"]["id"]
            .as_i64()
            .is_some()
    );
    assert_eq!(
        child_context_created["params"]["context"]["auxData"]["type"],
        json!("default")
    );

    let frame_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameAttached");
    let child_context_created_index = ctx
        .sent
        .iter()
        .position(|message| message == &child_context_created)
        .expect("child default execution context created event should remain buffered");
    assert!(
        frame_attached_index < child_context_created_index,
        "child Page.frameAttached must precede child default Runtime.executionContextCreated; sent={:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_emits_native_child_default_execution_context_created() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["context"]["uniqueId"].as_str().is_some()
        }),
        "real Runtime.enable should connect the renderer V8 Runtime agent: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 231,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 231);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native child default Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;

    let child_context_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should emit child default execution context created");
    assert_eq!(child_context_created["sessionId"], json!("SID-1"));
    assert!(
        child_context_created["params"]["context"]["id"]
            .as_i64()
            .is_some()
    );
    assert!(
        child_context_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child default context should come from V8 native Runtime event with uniqueId: {child_context_created:?}"
    );
    assert_eq!(
        child_context_created["params"]["context"]["auxData"]["type"],
        json!("default")
    );

    let frame_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameAttached");
    let child_context_created_index = ctx
        .sent
        .iter()
        .position(|message| message == &child_context_created)
        .expect("child default execution context created event should remain buffered");
    assert!(
        frame_attached_index < child_context_created_index,
        "child Page.frameAttached must precede child default Runtime.executionContextCreated; sent={:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_emits_each_child_default_context_identity_once_per_runtime_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-primary", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-primary"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-primary")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (command_id, session_id) in [(25_301, "SID-primary"), (25_302, "SID-attached")] {
        ctx.process_async(json!({
            "id": command_id,
            "method": "Runtime.enable",
            "sessionId": session_id,
        }))
        .await;
        assert_eq!(
            take_response_by_id(&mut ctx, command_id)["result"],
            json!({})
        );
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 25_303,
        "method": "Page.navigate",
        "sessionId": "SID-primary",
        "params": {
            "url": "data:text/html,<iframe srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 25_303);
    wait_until_message(
        &mut ctx,
        "SID-primary",
        "child frame attachment for multi-session Runtime navigation",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should be attached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_messages(
        &mut ctx,
        "SID-primary",
        "child default Runtime context fan-out to primary and attached sessions",
        move |messages| {
            ["SID-primary", "SID-attached"]
                .into_iter()
                .all(|session_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Runtime.executionContextCreated")
                            && message["sessionId"] == json!(session_id)
                            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                            && message["params"]["context"]["auxData"]["frameId"]
                                == json!(expected_child_frame_id)
                    })
                })
        },
    )
    .await;

    let child_context_identities = |session_id: &str| {
        ctx.sent
            .iter()
            .filter(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(session_id)
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            })
            .map(|message| {
                format!(
                    "{}:{}",
                    message["params"]["context"]["id"], message["params"]["context"]["uniqueId"]
                )
            })
            .collect::<Vec<_>>()
    };
    let primary_identities = child_context_identities("SID-primary");
    let attached_identities = child_context_identities("SID-attached");
    assert!(
        !primary_identities.is_empty() && !attached_identities.is_empty(),
        "both Runtime frontends must observe a child default context: {:?}",
        ctx.sent
    );
    // Chromium can expose more than one legitimate child context generation
    // here (the initial about:blank realm followed by the srcdoc realm). The
    // invariant is one delivery of each identity per frontend, not a fixed
    // total number of child contexts.
    let mut unique_primary_identities = primary_identities.clone();
    unique_primary_identities.sort();
    unique_primary_identities.dedup();
    assert_eq!(
        primary_identities.len(),
        unique_primary_identities.len(),
        "the primary Runtime frontend must receive each child context once: {:?}",
        ctx.sent
    );
    let mut unique_attached_identities = attached_identities.clone();
    unique_attached_identities.sort();
    unique_attached_identities.dedup();
    assert_eq!(
        attached_identities.len(),
        unique_attached_identities.len(),
        "the attached Runtime frontend must receive each child context once: {:?}",
        ctx.sent
    );
    assert_eq!(
        unique_primary_identities, unique_attached_identities,
        "both Runtime frontends should observe the same child context identities"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_auto_creates_native_named_child_world_before_frame_navigated()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 231,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 231);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 232,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_nav_child_world = 'ready';",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(232, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 233,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "childNavigationBinding",
            "executionContextName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(233, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 234,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>nav-child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 234);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "named child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "named child Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;
    let named_world_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should auto-create child named world");
    assert!(
        named_world_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child named-world context should come from V8 native Runtime event with uniqueId: {named_world_created:?}"
    );
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame navigation after its native named Runtime context",
        move |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(expected_child_frame_id)
        },
    )
    .await;
    let named_world_created_index = ctx
        .sent
        .iter()
        .position(|message| message == &named_world_created)
        .expect("child named-world event index");
    let child_frame_navigated_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameNavigated");
    assert!(
        named_world_created_index < child_frame_navigated_index,
        "child named-world context should be emitted before child frameNavigated; sent={:?}",
        ctx.sent
    );
    let child_context_id = named_world_created["params"]["context"]["id"]
        .as_i64()
        .expect("child named-world execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 235,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "JSON.stringify([globalThis.__lm_nav_child_world, typeof childNavigationBinding, document.body.textContent.trim()])"
        }
    }))
    .await;
    let child_state = take_response_by_id(&mut ctx, 235);
    assert_eq!(
        child_state["result"]["result"]["value"],
        json!("[\"ready\",\"function\",\"nav-child\"]")
    );

    ctx.process_async(json!({
        "id": 236,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "childNavigationBinding('nav-child-payload'); 13"
        }
    }))
    .await;
    let binding_call = take_response_by_id(&mut ctx, 236);
    assert_eq!(binding_call["result"]["result"]["value"], json!(13));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("childNavigationBinding")
        })
        .cloned()
        .expect("navigation child named-world binding should emit Runtime.bindingCalled");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("nav-child-payload")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_child_runtime_contexts_fan_out_to_each_runtime_enabled_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(90_240, "SID-1"), (90_241, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_242,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__fanoutChildWorld = true;",
            "worldName": "fanout-child-world"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_242)["result"]["identifier"],
        json!("1")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_243,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe srcdoc=\"<body>fanout-child</body>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 90_243);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "multi-session child frame attachment",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame id");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "default and named child contexts for both Runtime sessions",
        move |messages| {
            ["SID-1", "SID-attached"].into_iter().all(|session_id| {
                let has_default = messages.iter().any(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["sessionId"] == json!(session_id)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(expected_child_frame_id)
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                });
                let has_named = messages.iter().any(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["sessionId"] == json!(session_id)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(expected_child_frame_id)
                        && message["params"]["context"]["name"] == json!("fanout-child-world")
                });
                has_default && has_named
            })
        },
    )
    .await;

    for (is_default, name) in [(true, None), (false, Some("fanout-child-world"))] {
        let contexts = ["SID-1", "SID-attached"]
            .into_iter()
            .map(|session_id| {
                ctx.sent
                    .iter()
                    .filter(|message| {
                        message["method"] == json!("Runtime.executionContextCreated")
                            && message["sessionId"] == json!(session_id)
                            && message["params"]["context"]["auxData"]["frameId"]
                                == json!(child_frame_id)
                            && message["params"]["context"]["auxData"]["isDefault"]
                                == json!(is_default)
                            && name.is_none_or(|name| {
                                message["params"]["context"]["name"] == json!(name)
                            })
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            contexts[0].len(),
            1,
            "primary context should emit once: {contexts:?}"
        );
        assert_eq!(
            contexts[1].len(),
            1,
            "attached context should emit once: {contexts:?}"
        );
        assert_eq!(
            contexts[0][0]["params"]["context"]["id"], contexts[1][0]["params"]["context"]["id"],
            "both sessions must observe the same V8 context id"
        );
        assert_eq!(
            contexts[0][0]["params"]["context"]["uniqueId"],
            contexts[1][0]["params"]["context"]["uniqueId"],
            "both sessions must observe the same V8 context unique id"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_emits_native_child_named_world_context_created() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 10231,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10231);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10232,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_native_child_world = 'ready';",
            "worldName": "utility-native-child"
        }
    }))
    .await;
    ctx.expect_result(10232, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10233,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>native-child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 10233);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native named child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native named child Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-native-child")
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;
    let named_world_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-native-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should auto-create child named world from native Runtime agent");
    assert_eq!(named_world_created["sessionId"], json!("SID-1"));
    assert!(
        named_world_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child named-world context should come from V8 native Runtime event with uniqueId: {named_world_created:?}"
    );
    assert_eq!(
        named_world_created["params"]["context"]["auxData"]["type"],
        json!("isolated")
    );

    let child_context_id = named_world_created["params"]["context"]["id"]
        .as_i64()
        .expect("child named-world execution context id");
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 10234,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "JSON.stringify([globalThis.__lm_native_child_world, document.body.textContent.trim()])"
        }
    }))
    .await;
    let child_state = take_response_by_id(&mut ctx, 10234);
    assert_eq!(
        child_state["result"]["result"]["value"],
        json!("[\"ready\",\"native-child\"]")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_failure_creates_runtime_context_and_completes_lifecycle() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;

    ctx.process_async(json!({
        "id": 232,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/missing") }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "network error Document stopped loading",
        |message| message["method"] == json!("Page.frameStoppedLoading"),
    )
    .await;

    let response = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(232))
        .expect("Page.navigate response");
    assert_eq!(response["id"], 232);
    assert_eq!(response["result"]["frameId"], "TID-1");
    assert!(response["result"]["loaderId"].is_string());
    assert_eq!(response["result"]["isDownload"], false);
    assert!(response["result"]["errorText"].is_string());

    let messages = ctx.take_all();
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "error Document should create a default runtime context: {messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
        }) && messages.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("load")
        }),
        "error Document should complete lifecycle: {messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_and_lifecycle_enabled_replays_contexts_before_load() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 2401,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2401);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
        }),
        "real Runtime.enable should connect the renderer V8 Runtime agent: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2402,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(2402, json!({}), Some("SID-1"));
    ctx.sent.clear();
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_async(json!({
                "id": 24,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": "data:text/html,<body>hi</body>" }
            }))
            .await;

            wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
            assert_eq!(ctx.take_one()["method"], "Page.frameStartedNavigating");
            assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");
            assert_eq!(ctx.take_one()["id"], 24);
            assert_eq!(ctx.take_one()["method"], "Runtime.executionContextsCleared");
            let init = ctx.take_one();
            assert_eq!(init["method"], "Page.lifecycleEvent");
            assert_eq!(init["params"]["name"], "init");
            assert_eq!(ctx.take_one()["method"], "Page.frameNavigated");
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
            assert_eq!(ctx.take_one()["method"], "Runtime.executionContextCreated");
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
            assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");
            let dcl = ctx.take_one();
            assert_eq!(dcl["method"], "Page.lifecycleEvent");
            assert_eq!(dcl["params"]["name"], "DOMContentLoaded");
            assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");
            assert_eq!(ctx.take_one()["method"], "Page.lifecycleEvent");
            assert_eq!(ctx.take_one()["method"], "Page.lifecycleEvent");
            assert_eq!(ctx.take_one()["method"], "Page.lifecycleEvent");
            assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");
            assert!(ctx.sent.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_http_navigation_after_runtime_enable_replaces_the_context_group() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>runtime context replacement</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_request_count = request_count.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/page",
                axum::routing::get(move || {
                    let request_count = server_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        page().await
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 404,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url.clone() }
    }))
    .await;
    let first_loader_id = take_response_by_id(&mut ctx, 404)["result"]["loaderId"]
        .as_str()
        .expect("first HTTP navigation should have a loader")
        .to_owned();
    // Page.navigate acknowledges the navigation before the renderer reaches
    // load. Chromium clients wait for a lifecycle event when they need a
    // fully completed document; reading readyState immediately after the
    // command response is allowed to observe "interactive".
    wait_until_renderer_document_load(&mut ctx, Some("SID-1"), "TID-1", &first_loader_id).await;
    ctx.process_async(json!({
        "id": 4041,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.readyState",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 4041)["result"]["result"]["value"],
        json!("complete"),
        "the benchmark enables Runtime only after the first document has completed"
    );
    let state_before_enable = ctx
        .conn
        .navigation_load_inputs_for_session_owner(Some("SID-1"))
        .runtime_inspector_session_restore_snapshots
        .into_iter()
        .find(|restore| restore.inspector_session_id.is_none())
        .and_then(|restore| restore.v8_attach.reattach_state().cloned())
        .expect("Runtime.evaluate should establish the primary V8 session state");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 405,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "Runtime.enable should report the first HTTP document context: {:?}",
        ctx.sent
    );
    let state_after_enable = ctx
        .conn
        .navigation_load_inputs_for_session_owner(Some("SID-1"))
        .runtime_inspector_session_restore_snapshots
        .into_iter()
        .find(|restore| restore.inspector_session_id.is_none())
        .and_then(|restore| restore.v8_attach.reattach_state().cloned())
        .expect("Runtime.enable should retain the primary V8 session state");
    assert_ne!(
        state_before_enable, state_after_enable,
        "Runtime.enable must replace the pre-enable V8 session cookie"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 406,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    let second_loader_id = take_response_by_id(&mut ctx, 406)["result"]["loaderId"]
        .as_str()
        .expect("second HTTP navigation should have a loader")
        .to_owned();

    assert_ne!(first_loader_id, second_loader_id);
    assert_eq!(
        request_count.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "same-URL Page.navigate should fetch and commit a replacement document"
    );
    assert_runtime_navigation_context_reset(&ctx.sent, "SID-1", "TID-1");

    server.abort();
}
