use super::*;

#[tokio::test]
async fn enable_with_background_event_sender_defers_initial_document_page_build() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.set_target_url("about:blank".into());
    ctx.conn.browser_context = Some(bc);

    let (background_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let (completion_tx, _) =
        tokio::sync::mpsc::unbounded_channel::<BackgroundNavigationCompletion>();
    ctx.conn.set_background_event_sender(background_tx);
    ctx.conn
        .set_background_navigation_completion_sender(completion_tx);

    ctx.process_async(json!({
        "id": 21,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(21, json!({}), Some("SID-1"));
    assert!(ctx.sent.is_empty());
}
#[tokio::test]
async fn enable_uses_fresh_initial_document_without_adapter() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 21_000,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    ctx.expect_event("Target.targetCreated", None);
    let create_response = ctx.take_response_by_id(21_000);
    let target_id = create_response["result"]["targetId"]
        .as_str()
        .unwrap_or_else(|| panic!("Target.createTarget should return target id: {create_response}"))
        .to_owned();
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "Target.createTarget should install the initial about:blank page before Runtime.enable"
    );

    let raw = json!({
        "id": 21_001,
        "method": "Runtime.enable"
    })
    .to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    let (messages, scheduler_events) = complete_command_task_step_for_test(&mut ctx, step).await;

    assert!(
        scheduler_events.is_empty(),
        "Runtime.enable about:blank should not enqueue scheduler work: {scheduler_events:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| { message["id"] == json!(21_001) && message["result"] == json!({}) }),
        "Runtime.enable should emit command success: {messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("about:blank")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(target_id)
        }),
        "Runtime.enable should replay the existing about:blank default context: {messages:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .runtime_slot
            .has_loaded_page(),
        "Runtime.enable should observe the already-installed about:blank page"
    );
}

#[tokio::test]
async fn enable_reports_no_document_without_legacy_materialization_adapter() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.set_target_url("about:blank".into());
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 21_002,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;

    ctx.expect_error(21_002, -32000, "NoDocumentLoaded");
    assert!(
        !ctx.conn
            .target_runtime_session_state_for_session(Some("SID-1"))
            .expect("target runtime session state")
            .runtime_frontend_enabled,
        "failed Runtime.enable must not mark the frontend as enabled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn evaluate_hash_navigation_emits_navigated_within_document() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><body>hash target</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(page_url.clone());

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 12).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "location.hash = 'puppeteer-hash'; location.href",
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 13);
    assert_eq!(
        response["result"]["result"]["value"],
        json!(format!("{page_url}#puppeteer-hash"))
    );
    ctx.expect_event(
        "Page.navigatedWithinDocument",
        Some(&json!({
            "frameId": "TID-1",
            "url": format!("{page_url}#puppeteer-hash"),
            "navigationType": "fragment",
        })),
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Page.frameNavigated")),
        "same-document navigation must not be reported as a full frame navigation: {:?}",
        ctx.sent
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .target_url(),
        format!("{page_url}#puppeteer-hash")
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn call_function_on_hash_navigation_emits_navigated_within_document() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><body>hash target</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(page_url.clone());

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 14).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 15,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis",
        }
    }))
    .await;
    let global_object_id = take_response_by_id(&mut ctx, 15)["result"]["result"]["objectId"]
        .as_str()
        .expect("globalThis should produce an object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 16,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": global_object_id,
            "functionDeclaration": "function() { location.hash = 'puppeteer-call-function'; return location.href; }",
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 16);
    assert_eq!(
        response["result"]["result"]["value"],
        json!(format!("{page_url}#puppeteer-call-function"))
    );
    ctx.expect_event(
        "Page.navigatedWithinDocument",
        Some(&json!({
            "frameId": "TID-1",
            "url": format!("{page_url}#puppeteer-call-function"),
            "navigationType": "fragment",
        })),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn call_function_on_default_context_hash_navigation_emits_navigated_within_document() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><body>hash target</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(page_url.clone());

    let default_context_id = enable_runtime_and_take_execution_context_id_async(&mut ctx, 30).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 31,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "executionContextId": default_context_id,
            "functionDeclaration": "function() { location.hash = 'puppeteer-default-context'; }",
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 31);
    assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    ctx.expect_event(
        "Page.navigatedWithinDocument",
        Some(&json!({
            "frameId": "TID-1",
            "url": format!("{page_url}#puppeteer-default-context"),
            "navigationType": "fragment",
        })),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn evaluate_history_api_navigation_emits_navigated_within_document() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><title>history-api</title><body>history target</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let pushed_url = format!("{page_url}?state=push");
    let replaced_url = format!("{page_url}?state=replace");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(page_url.clone());

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 32).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 33,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!("window.__historyRealmMarker = 37; history.pushState({{step: 'push'}}, '', '{}'); location.href", pushed_url),
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 33);
    assert_eq!(response["result"]["result"]["value"], json!(pushed_url));
    ctx.expect_event(
        "Page.navigatedWithinDocument",
        Some(&json!({
            "frameId": "TID-1",
            "url": pushed_url,
            "navigationType": "historyApi",
        })),
    );

    ctx.process_async(json!({
        "id": 34,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 34);
    assert_eq!(history["result"]["currentIndex"], json!(1));
    assert_eq!(history["result"]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(history["result"]["entries"][1]["url"], json!(pushed_url));
    let initial_entry_id = history["result"]["entries"][0]["id"]
        .as_i64()
        .expect("initial navigation history entry id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!("history.replaceState({{step: 'replace'}}, '', '{}'); location.href", replaced_url),
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 35);
    assert_eq!(response["result"]["result"]["value"], json!(replaced_url));
    ctx.expect_event(
        "Page.navigatedWithinDocument",
        Some(&json!({
            "frameId": "TID-1",
            "url": replaced_url,
            "navigationType": "historyApi",
        })),
    );

    ctx.process_async(json!({
        "id": 36,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 36);
    assert_eq!(history["result"]["currentIndex"], json!(1));
    assert_eq!(history["result"]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(history["result"]["entries"][1]["url"], json!(replaced_url));

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .target_url(),
        replaced_url
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37,
        "method": "Page.navigateToHistoryEntry",
        "sessionId": "SID-1",
        "params": { "entryId": initial_entry_id }
    }))
    .await;
    take_response_by_id(&mut ctx, 37);
    // Chromium acknowledges Page.navigateToHistoryEntry before the history
    // traversal task emits Page.navigatedWithinDocument. Observe the real
    // renderer publication instead of making the command helper drain a later
    // Page turn synchronously.
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "history traversal Page.navigatedWithinDocument",
        |message| {
            message["method"] == json!("Page.navigatedWithinDocument")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["url"] == json!(page_url)
                && message["params"]["navigationType"] == json!("fragment")
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 38,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({href: location.href, marker: window.__historyRealmMarker})",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 38);
    assert_eq!(
        response["result"]["result"]["value"],
        json!({
            "href": page_url,
            "marker": 37,
        }),
        "Page.navigateToHistoryEntry must traverse a same-document entry without replacing the realm"
    );

    ctx.process_async(json!({
        "id": 39,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 39);
    assert_eq!(history["result"]["currentIndex"], json!(0));
    assert_eq!(history["result"]["entries"].as_array().unwrap().len(), 2);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "history.forward(); 'queued'",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 40);
    assert_eq!(response["result"]["result"]["value"], json!("queued"));
    // Chromium likewise returns from Runtime.evaluate(history.forward())
    // before the queued traversal emits its same-document notification.
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "history.forward Page.navigatedWithinDocument",
        |message| {
            message["method"] == json!("Page.navigatedWithinDocument")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["url"] == json!(replaced_url)
                && message["params"]["navigationType"] == json!("fragment")
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 41,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 41);
    assert_eq!(history["result"]["currentIndex"], json!(1));
    assert_eq!(history["result"]["entries"].as_array().unwrap().len(), 2);

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn isolated_context_hash_navigation_emits_navigated_within_document() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><body>hash target</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(page_url.clone());

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 17).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 18, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 19,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": utility_context_id,
            "expression": "location.hash = 'puppeteer-isolated'; location.href",
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 19);
    assert_eq!(
        response["result"]["result"]["value"],
        json!(format!("{page_url}#puppeteer-isolated"))
    );
    ctx.expect_event(
        "Page.navigatedWithinDocument",
        Some(&json!({
            "frameId": "TID-1",
            "url": format!("{page_url}#puppeteer-isolated"),
            "navigationType": "fragment",
        })),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn isolated_object_call_hash_navigation_emits_navigated_within_document() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><body>hash target</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(page_url.clone());

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 21, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 22,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis"
        }
    }))
    .await;
    let global_object_id = take_response_by_id(&mut ctx, 22)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated globalThis should produce an object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-1",
        "params": {
            "objectId": global_object_id,
            "functionDeclaration": "function() { location.hash = 'puppeteer-object'; return location.href; }",
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 23);
    assert_eq!(
        response["result"]["result"]["value"],
        json!(format!("{page_url}#puppeteer-object"))
    );
    ctx.expect_event(
        "Page.navigatedWithinDocument",
        Some(&json!({
            "frameId": "TID-1",
            "url": format!("{page_url}#puppeteer-object"),
            "navigationType": "fragment",
        })),
    );

    server.abort();
}
#[tokio::test]
async fn enable_replays_fresh_initial_about_blank_default_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 11,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    ctx.expect_event("Target.targetCreated", None);
    let create_response = ctx.take_response_by_id(11);
    let target_id = create_response["result"]["targetId"]
        .as_str()
        .unwrap_or_else(|| panic!("Target.createTarget should return target id: {create_response}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.enable"
    }))
    .await;

    ctx.expect_result(12, json!({}), None);
    ctx.expect_event(
        "Runtime.executionContextCreated",
        Some(&json!({
            "context": {
                "name": "about:blank",
                "auxData": {
                    "isDefault": true,
                    "frameId": target_id
                }
            }
        })),
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.URL",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 13);
    assert_eq!(response["result"]["result"]["value"], json!("about:blank"));
}
#[tokio::test(flavor = "multi_thread")]
async fn runtime_timer_publication_emits_history_api_navigation_without_followup_command() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html; charset=utf-8")],
            "<!doctype html><html><body>timer history target</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let timer_url = format!("{page_url}?timer=history");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context should exist")
        .set_target_url(page_url.clone());
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_687).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20_688,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!("setTimeout(() => history.pushState({{timer: true}}, '', '{}'), 20)", timer_url)
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 20_688);
    assert_eq!(response["result"]["result"]["type"], json!("number"));

    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "timer Page.navigatedWithinDocument",
        |message| {
            message["method"] == json!("Page.navigatedWithinDocument")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["url"] == json!(timer_url)
                && message["params"]["navigationType"] == json!("historyApi")
        },
    )
    .await;

    server.abort();
}
