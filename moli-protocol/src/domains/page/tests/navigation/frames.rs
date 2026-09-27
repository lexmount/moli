use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn navigate_emits_frame_and_load_events_in_order() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_and_wait_for_response_async(json!({
        "id": 20,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

    let started_navigating = ctx.take_one();
    assert_eq!(started_navigating["method"], "Page.frameStartedNavigating");
    assert_eq!(started_navigating["sessionId"], "SID-1");
    assert_eq!(started_navigating["params"]["frameId"], "TID-1");
    assert_eq!(started_navigating["params"]["loaderId"], LOADER_ID);
    assert_eq!(
        started_navigating["params"]["url"],
        "data:text/html,<body>hi</body>"
    );
    assert_eq!(
        started_navigating["params"]["navigationType"],
        "differentDocument"
    );

    let started_loading = ctx.take_one();
    assert_eq!(started_loading["method"], "Page.frameStartedLoading");
    assert_eq!(started_loading["sessionId"], "SID-1");
    assert_eq!(started_loading["params"]["frameId"], "TID-1");

    let result = ctx.take_one();
    assert_eq!(result["id"], 20);
    assert_eq!(result["sessionId"], "SID-1");
    assert_eq!(
        result["result"],
        json!({
            "frameId": "TID-1",
            "loaderId": LOADER_ID,
        })
    );

    let frame_navigated = ctx.take_one();
    assert_eq!(frame_navigated["method"], "Page.frameNavigated");
    assert_eq!(frame_navigated["sessionId"], "SID-1");
    assert_eq!(frame_navigated["params"]["type"], "Navigation");
    assert_eq!(frame_navigated["params"]["frame"]["id"], "TID-1");
    assert_eq!(frame_navigated["params"]["frame"]["loaderId"], LOADER_ID);
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        "data:text/html,<body>hi</body>"
    );

    let document_updated = ctx.take_one();
    assert_eq!(document_updated["method"], "DOM.documentUpdated");
    assert_eq!(document_updated["sessionId"], "SID-1");

    let parse_completed_document_updated = ctx.take_one();
    assert_eq!(
        parse_completed_document_updated["method"],
        "DOM.documentUpdated"
    );
    assert_eq!(parse_completed_document_updated["sessionId"], "SID-1");

    let dom_content_loaded = ctx.take_one();
    assert_eq!(dom_content_loaded["method"], "Page.domContentEventFired");
    assert_eq!(dom_content_loaded["sessionId"], "SID-1");
    assert!(dom_content_loaded["params"]["timestamp"].as_f64().is_some());

    let load_event = ctx.take_one();
    assert_eq!(load_event["method"], "Page.loadEventFired");
    assert_eq!(load_event["sessionId"], "SID-1");
    assert!(load_event["params"]["timestamp"].as_f64().is_some());

    let stopped_loading = ctx.take_one();
    assert_eq!(stopped_loading["method"], "Page.frameStoppedLoading");
    assert_eq!(stopped_loading["sessionId"], "SID-1");
    assert_eq!(stopped_loading["params"]["frameId"], "TID-1");
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_child_iframe_emits_child_frame_navigation_and_lifecycle_events() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;

    ctx.process_async(json!({
        "id": 521,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    })).await;

    let _ = take_response_by_id(&mut ctx, 521);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame navigation after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("child-frame")
        },
    )
    .await;
    let child_navigated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("child-frame")
        })
        .cloned()
        .expect("child frame should emit Page.frameNavigated");
    let child_frame_id = child_navigated["params"]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();
    assert_ne!(child_frame_id, "TID-1");
    assert_eq!(
        child_navigated["params"]["frame"]["parentId"],
        json!("TID-1")
    );
    assert_child_frame_attached(&ctx, &child_frame_id, "TID-1");
    assert_child_frame_navigation_completion(&mut ctx, &child_frame_id, Some("child-frame"), None)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_nested_child_frame_reports_outer_parent_frame_id() {
    // This regression targets Page.frameAttached parentFrameId for nested
    // frames discovered through the current frame tree. Dynamic insertion and
    // removal are covered separately because they exercise the activity pump.
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    ctx.process_async(json!({
        "id": 5241,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='outer' name='outer-frame' srcdoc=\"<iframe name='inner-frame' srcdoc='<body>inner</body>'></iframe>\"></iframe>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 5241);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child frame navigations after Page.navigate response",
        |messages| {
            ["outer-frame", "inner-frame"].iter().all(|name| {
                messages.iter().any(|message| {
                    message["method"] == json!("Page.frameNavigated")
                        && message["params"]["frame"]["name"] == json!(name)
                })
            })
        },
    )
    .await;
    let outer_navigated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("outer-frame")
        })
        .cloned()
        .expect("outer frame should emit Page.frameNavigated");
    let outer_frame_id = outer_navigated["params"]["frame"]["id"]
        .as_str()
        .expect("outer frame id")
        .to_owned();
    assert_eq!(
        outer_navigated["params"]["frame"]["parentId"],
        json!("TID-1")
    );
    assert_child_frame_attached(&ctx, &outer_frame_id, "TID-1");
    let inner_navigated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("inner-frame")
        })
        .cloned()
        .expect("inner frame should emit Page.frameNavigated");
    let inner_frame_id = inner_navigated["params"]["frame"]["id"]
        .as_str()
        .expect("inner frame id")
        .to_owned();
    assert_eq!(
        inner_navigated["params"]["frame"]["parentId"],
        json!(outer_frame_id)
    );
    let inner_attached = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(inner_frame_id)
                && message["params"]["parentFrameId"] == json!(outer_frame_id)
        })
        .cloned()
        .expect("inner frame should emit Page.frameAttached with outer parent");
    assert_eq!(inner_attached["params"]["frameId"], json!(inner_frame_id));
    assert_ne!(inner_frame_id, outer_frame_id);
    assert_ne!(inner_frame_id, "TID-1");
    assert!(
        inner_frame_id.starts_with("child-browsing-context-"),
        "live child frame ids should use the renderer browsing-context id; got {inner_frame_id}"
    );
    assert_child_frame_attached(&ctx, &inner_frame_id, &outer_frame_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_child_named_world_reports_context_when_document_start_script_throws() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 237,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 237);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 238,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "if (window.top !== window) { throw new Error('child utility bootstrap failed'); }",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(238, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 239,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe srcdoc=\"<body>nav-child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 239);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "throwing preload child frame attachment after Page.navigate response",
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
        .unwrap_or_else(|| panic!("child frame should emit Page.frameAttached: {:?}", ctx.sent));
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "throwing preload named child Runtime context",
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
        .expect("throwing document-start script must not orphan the child named world");
    assert!(
        named_world_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child named-world context should still come from V8 native Runtime event: {named_world_created:?}"
    );
    let child_context_id = named_world_created["params"]["context"]["id"]
        .as_i64()
        .expect("child named-world execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 240,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "document.body.textContent.trim()"
        }
    }))
    .await;
    let child_state = take_response_by_id(&mut ctx, 240);
    assert_eq!(child_state["result"]["result"]["value"], json!("nav-child"));
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_child_iframe_delays_main_load_until_child_frame_and_contexts_complete() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 236,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 236);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 237,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_child_world_before_load = true;",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(237, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 238,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 238);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "main load after child frame completion",
        |message| message["method"] == json!("Page.loadEventFired"),
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
    let child_default_context_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit default execution context before main load");
    let child_named_world_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit named isolated world before main load");
    let child_stopped_loading_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameStoppedLoading");
    let main_load_event_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.loadEventFired"))
        .expect("main frame should emit Page.loadEventFired");

    assert!(
        child_default_context_index < main_load_event_index,
        "child default execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_named_world_index < main_load_event_index,
        "child named-world execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_stopped_loading_index < main_load_event_index,
        "child frame must finish loading before main loadEventFired; sent={:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_network_child_iframe_delays_main_load_until_child_frame_and_contexts_complete()
 {
    async fn parent() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe name=\"child-frame\" src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-network-load-boundary</body></html>",
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
    // This assertion depends on native V8 Runtime events. Exercise the public
    // command so the Inspector frontend is retained across the async network
    // navigation; toggling only the protocol projection does not establish
    // that renderer-side lifetime.
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 23_09,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 23_09);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 239,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_network_child_world_before_load = true;",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(239, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23_10,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": format!("http://{addr}/parent")
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 23_10);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "network child frameAttached",
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
        .expect("network child frame should emit Page.frameAttached");
    crate::testing::wait_until_messages(
        &mut ctx,
        "SID-1",
        "network child frame completion and main load",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["name"] == json!("utility-child")
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Page.frameStoppedLoading")
                    && message["params"]["frameId"] == json!(child_frame_id)
            }) && messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;
    let child_default_context_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("network child frame should expose its default execution context before main load");
    let child_named_world_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| {
            panic!(
                "network child frame should emit named isolated world before main load; sent={:?}",
                ctx.sent
            )
        });
    let child_stopped_loading_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("network child frame should emit Page.frameStoppedLoading");
    let main_load_event_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.loadEventFired"))
        .expect("main frame should emit Page.loadEventFired");
    let child_default_context_id = ctx.sent[child_default_context_index]["params"]["context"]["id"]
        .as_i64()
        .expect("network child default execution context id");
    let child_named_world_context_id = ctx.sent[child_named_world_index]["params"]["context"]["id"]
        .as_i64()
        .expect("network child named-world execution context id");

    assert!(
        child_default_context_index < main_load_event_index,
        "network child default execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_named_world_index < main_load_event_index,
        "network child named-world execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_stopped_loading_index < main_load_event_index,
        "network child frame must finish loading before main loadEventFired; sent={:?}",
        ctx.sent
    );

    // The first eligible same-origin commit reuses the initial-empty
    // LocalWindow and its V8 contexts. The context-created event can therefore
    // retain its about:blank name; the stable context must project the newly
    // committed Document before main load completes.
    ctx.process_async(json!({
        "id": 23_12,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "JSON.stringify([location.pathname, document.body.textContent.trim()])"
        }
    }))
    .await;
    let default_world_state = take_response_by_id(&mut ctx, 23_12);
    assert_eq!(
        default_world_state["result"]["result"]["value"],
        json!("[\"/child\",\"child-network-load-boundary\"]")
    );

    ctx.process_async(json!({
        "id": 23_13,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_named_world_context_id,
            "expression": "JSON.stringify([globalThis.__lm_network_child_world_before_load, location.pathname, document.body.textContent.trim()])"
        }
    }))
    .await;
    let named_world_state = take_response_by_id(&mut ctx, 23_13);
    assert_eq!(
        named_world_state["result"]["result"]["value"],
        json!("[true,\"/child\",\"child-network-load-boundary\"]")
    );

    ctx.process_async(json!({
        "id": 23_11,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 23_11);
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree childFrames array");
    assert_eq!(
        child_frames.len(),
        1,
        "network child frame should be visible in frame tree immediately after navigate response"
    );
    assert_eq!(
        child_frames[0]["frame"]["id"],
        json!(child_frame_id),
        "frame tree child should match attached child frame id"
    );

    server.abort();
}
