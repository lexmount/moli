use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_pauses_until_continue_request_then_resolves_promise() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-subresource", "ok"),
            ],
            format!("continued:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 360,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(360, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 361,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(361, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 401).await;

    ctx.process_async(json!({
        "id": 362,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  fetch('/api', {
    method: 'POST',
    headers: { 'x-from-runtime': '1' },
    body: 'payload'
  })
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 362);
    assert_eq!(evaluate["id"], 362);
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], api_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(
        paused["params"]["request"]["headers"]["x-from-runtime"],
        "1"
    );
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("Fetch"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    assert_eq!(request["params"]["request"]["hasPostData"], true);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 363,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(363, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource fetch network completion",
    )
    .await;

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(
        response["params"]["response"]["headers"]["x-subresource"],
        "ok"
    );
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(network_request_id)
    }));

    ctx.process_async(json!({
        "id": 364,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": network_request_id }
    }))
    .await;
    ctx.expect_result(
        364,
        json!({
            "body": "continued:payload",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 365,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 365);
    assert_eq!(resolved["result"]["result"]["value"], "continued:payload");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_continue_request_fails_when_network_offline() {
    let mut ctx = TestContext::new();
    with_loaded_http_document(
        &mut ctx,
        "data:text/html,<html><body>ready</body></html>",
        "SID-1",
        "TID-1",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 16640,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(16640, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 16641).await;

    ctx.process_async(json!({
        "id": 16642,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  fetch('http://example.test/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_result = text; })
    .catch(error => { globalThis.__lm_fetch_result = String(error); });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 16642);
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 16643,
        "method": "Network.emulateNetworkConditions",
        "params": {
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }
    }))
    .await;
    ctx.expect_result(16643, json!({}), None);

    ctx.process_async(json!({
        "id": 16644,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(16644, json!({}), Some("SID-1"));
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["errorText"], "Network emulation offline");

    ctx.process_async(json!({
        "id": 16645,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 16645);
    assert!(
        resolved["result"]["result"]["value"]
            .as_str()
            .expect("fetch result string")
            .contains("Network emulation offline")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_request_animation_frame_pauses_until_continue_request_then_resolves_promise()
 {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-subresource", "raf"),
            ],
            format!("continued-raf:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 365,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(365, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 366,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(366, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 402).await;

    ctx.process_async(json!({
        "id": 367,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  requestAnimationFrame(() => {
    fetch('/api', {
      method: 'POST',
      headers: { 'x-from-runtime': 'raf' },
      body: 'payload'
    })
      .then(response => response.text())
      .then(text => { globalThis.__lm_fetch_result = text; });
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 367);
    assert_eq!(evaluate["id"], 367);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch requestAnimationFrame requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["request"]["headers"]["x-from-runtime"] == json!("raf")
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["request"]["headers"]["x-from-runtime"] == json!("raf")
        })
        .cloned()
        .expect("subresource fetch requestAnimationFrame requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], api_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(
        paused["params"]["request"]["headers"]["x-from-runtime"],
        "raf"
    );
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("Fetch"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    assert_eq!(request["params"]["request"]["hasPostData"], true);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 368,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(368, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource fetch requestAnimationFrame network completion",
    )
    .await;

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(
        response["params"]["response"]["headers"]["x-subresource"],
        "raf"
    );

    ctx.process_async(json!({
        "id": 369,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 369);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "continued-raf:payload"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_queue_microtask_pauses_until_continue_request_then_resolves_promise()
 {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-subresource", "microtask"),
            ],
            format!("continued-microtask:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 798,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(798, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 799,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(799, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 409).await;

    ctx.process_async(json!({
        "id": 800,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  queueMicrotask(() => {
    fetch('/api', {
      method: 'POST',
      headers: { 'x-from-runtime': 'microtask' },
      body: 'payload'
    })
      .then(response => response.text())
      .then(text => { globalThis.__lm_fetch_result = text; });
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 800);
    assert_eq!(evaluate["id"], 800);
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch queueMicrotask requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], api_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(
        paused["params"]["request"]["headers"]["x-from-runtime"],
        "microtask"
    );
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("Fetch"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 801,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(801, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource fetch queueMicrotask network completion",
    )
    .await;

    ctx.process_async(json!({
        "id": 802,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 802);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "continued-microtask:payload"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_promise_then_pauses_until_continue_request_then_resolves_promise()
 {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-subresource", "promise"),
            ],
            format!("continued-promise:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 808,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(808, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 809,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(809, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 411).await;

    ctx.process_async(json!({
        "id": 810,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  Promise.resolve().then(() => {
    fetch('/api', {
      method: 'POST',
      headers: { 'x-from-runtime': 'promise' },
      body: 'payload'
    })
      .then(response => response.text())
      .then(text => { globalThis.__lm_fetch_result = text; });
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 810);
    assert_eq!(evaluate["id"], 810);
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch promise.then requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], api_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(
        paused["params"]["request"]["headers"]["x-from-runtime"],
        "promise"
    );
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("Fetch"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 811,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(811, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource fetch promise.then network completion",
    )
    .await;

    ctx.process_async(json!({
        "id": 812,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 812);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "continued-promise:payload"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_request_idle_callback_pauses_until_continue_request_then_resolves_promise()
 {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-subresource", "idle"),
            ],
            format!("continued-idle:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 382,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(382, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 383,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(383, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 406).await;

    ctx.process_async(json!({
        "id": 384,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  requestIdleCallback(deadline => {
    globalThis.__lm_idle_meta = deadline.didTimeout === false && deadline.timeRemaining() > 0;
    fetch('/api', {
      method: 'POST',
      headers: { 'x-from-runtime': 'idle' },
      body: 'payload'
    })
      .then(response => response.text())
      .then(text => { globalThis.__lm_fetch_result = text; });
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 384);
    assert_eq!(evaluate["id"], 384);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch requestIdleCallback requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["request"]["headers"]["x-from-runtime"] == json!("idle")
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["request"]["headers"]["x-from-runtime"] == json!("idle")
        })
        .cloned()
        .expect("subresource fetch requestIdleCallback requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], api_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(
        paused["params"]["request"]["headers"]["x-from-runtime"],
        "idle"
    );
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("Fetch"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 385,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(385, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource fetch requestIdleCallback network completion",
    )
    .await;

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(
        response["params"]["response"]["headers"]["x-subresource"],
        "idle"
    );

    ctx.process_async(json!({
            "id": 386,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": { "expression": "String(globalThis.__lm_idle_meta) + ':' + globalThis.__lm_fetch_result" }
        })).await;
    let resolved = take_response_by_id(&mut ctx, 386);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "true:continued-idle:payload"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_resource_type_filter_pauses_shared_xhr_interception_type() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "ok")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let fetch_url = format!("http://{addr}/api?kind=fetch");
    let xhr_url = format!("http://{addr}/api?kind=xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 606,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(606, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 607,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(607, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 608).await;

    ctx.process_async(json!({
        "id": 609,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
  fetch('{fetch_url}').catch(() => {{}});
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '{xhr_url}');
  xhr.onerror = () => {{}};
  xhr.send();
  return "scheduled";
}})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 609);

    let paused = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("XHR")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        paused.len(),
        2,
        "Fetch filter should pause both fetch and XHR: {:?}",
        ctx.sent
    );
    for expected_url in [&fetch_url, &xhr_url] {
        assert!(
            paused
                .iter()
                .any(|event| { event["params"]["request"]["url"] == json!(expected_url) })
        );
    }
    for (url, expected_type) in [(&fetch_url, "Fetch"), (&xhr_url, "XHR")] {
        assert!(ctx.sent.iter().any(|event| {
            event["method"] == json!("Network.requestWillBeSent")
                && event["params"]["request"]["url"] == json!(url)
                && event["params"]["type"] == json!(expected_type)
        }));
    }

    for (offset, event) in paused.into_iter().enumerate() {
        let request_id = event["params"]["requestId"]
            .as_str()
            .expect("fetch-like request id");
        let command_id = 610 + offset as u64;
        ctx.process_async(json!({
            "id": command_id,
            "method": "Fetch.continueRequest",
            "sessionId": "SID-1",
            "params": { "requestId": request_id }
        }))
        .await;
        ctx.expect_result(command_id, json!({}), Some("SID-1"));
    }
    for expected_url in [&fetch_url, &xhr_url] {
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|event| {
                    event["method"] == json!("Network.requestWillBeSent")
                        && event["params"]["request"]["url"] == json!(expected_url)
                })
                .count(),
            1,
            "Fetch interception must not republish requestWillBeSent after continue: {:?}",
            ctx.sent
        );
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_url_pattern_only_pauses_matching_fetch_subresources() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn hit() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "hit")
    }

    async fn miss() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "miss")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api/hit", any(hit))
                .route("/api/miss", any(miss)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let hit_url = format!("http://{addr}/api/hit");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 610,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": {
                "patterns": [{ "urlPattern": "*/api/hit", "requestStage": "Request", "resourceType": "Fetch" }]
            }
        })).await;
    ctx.expect_result(610, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 611).await;

    ctx.process_async(json!({
        "id": 612,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_pattern_result = "pending";
  Promise.all([
    fetch('/api/miss').then(r => r.text()),
    fetch('/api/hit').then(r => r.text()),
  ]).then(values => { globalThis.__lm_pattern_result = values.join(','); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 612);

    let pauses = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(pauses.len(), 1);
    assert_eq!(pauses[0]["params"]["request"]["url"], json!(hit_url));
    let request_id = pauses[0]["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 613,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(613, json!({}), Some("SID-1"));

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        36_700,
        "globalThis.__lm_pattern_result",
        &json!("miss,hit"),
        "fetch URL pattern result",
    )
    .await;
    assert_eq!(resolved["result"]["result"]["value"], "miss,hit");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn bidi_add_network_intercept_pauses_matching_fetch_subresources() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn hit() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "hit")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api/hit", any(hit)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let hit_url = format!("http://{addr}/api/hit");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit"),
                phases: vec![DevToolsNetworkInterceptPhase::BeforeRequestSent],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: hit_url.clone(),
                }],
            }),
        )
        .await;
    assert_eq!(
        result.expect("BiDi add intercept should succeed"),
        AutomationResult::AddNetworkIntercept(
            crate::automation::DevToolsAddNetworkInterceptResult {
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit")
            }
        )
    );
    ctx.sent.clear();

    let (evaluate_result, scheduler_events, protocol_events, renderer_output_predecessor) = ctx
        .conn
        .execute_automation_command(AutomationCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                realm_id: None,
                world_name: None,
                expression: r#"(() => {
  globalThis.__lm_bidi_intercept_result = "pending";
  fetch('/api/hit').then(r => r.text()).then(text => { globalThis.__lm_bidi_intercept_result = text; });
  return "scheduled";
})()"#
                .to_owned(),
                await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await
        .into_complete_parts();
    if let Some(predecessor) = renderer_output_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    evaluate_result.expect("BiDi script.evaluate should start fetch");
    let mut scheduler_output = protocol_events_into_internal_messages(protocol_events);
    drain_scheduler_events_like_scheduler_preserving_internal_fields(
        &mut ctx.conn,
        &mut scheduler_output,
        scheduler_events,
    )
    .await;
    ctx.sent.extend(scheduler_output);

    let paused = wait_for_target_fetch_request_paused(
        &mut ctx,
        None,
        &hit_url,
        None,
        "BiDi request-stage network intercept pause",
    )
    .await;
    assert_eq!(paused["params"]["request"]["url"], json!(hit_url));
    assert_eq!(paused["params"]["resourceType"], json!("XHR"));
    let before_requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(hit_url)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        before_requests.len(),
        1,
        "expected one synthetic request event for paused fetch: {:?}",
        ctx.sent
    );
    assert_eq!(
        before_requests[0]["params"]["requestId"], paused["params"]["networkId"],
        "the public Fetch networkId must correlate the pause with its Network request"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_then_bidi_network_intercept_request_stage_chain_completes() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn hit() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "mixed-hit")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api/hit", any(hit)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let hit_url = format!("http://{addr}/api/hit");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 41_010,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": hit_url.clone(), "requestStage": "Request", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(41_010, json!({}), Some("SID-1"));

    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit"),
                phases: vec![DevToolsNetworkInterceptPhase::BeforeRequestSent],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: hit_url.clone(),
                }],
            }),
        )
        .await;
    assert!(
        result.is_ok(),
        "BiDi add intercept should succeed: {result:?}"
    );
    enable_runtime_async(&mut ctx, "SID-1", 41_011).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 41_012,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_mixed_intercept_result = "pending";
  fetch('/api/hit')
    .then(response => response.text())
    .then(text => { globalThis.__lm_mixed_intercept_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 41_012);

    let cdp_pause = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["request"]["url"] == json!(hit_url)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing CDP Fetch pause: {:?}", ctx.sent));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("BIDI-SID")
        }),
        "BiDi-owned pause should wait until the CDP Fetch handler continues"
    );
    let cdp_request_id = cdp_pause["params"]["requestId"]
        .as_str()
        .expect("CDP Fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 41_013,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": cdp_request_id }
    }))
    .await;
    ctx.expect_result(41_013, json!({}), Some("SID-1"));

    let bidi_pause = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("BIDI-SID")
                && message["params"]["request"]["url"] == json!(hit_url)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing chained BiDi pause: {:?}", ctx.sent));
    let bidi_request_id = bidi_pause["params"]["requestId"]
        .as_str()
        .expect("BiDi chained request id")
        .to_owned();
    ctx.sent.clear();

    let (
        continue_result,
        continue_scheduler_events,
        _continue_protocol_events,
        continue_renderer_output_predecessor,
    ) = ctx
        .conn
        .execute_automation_command(AutomationCommand::ContinueInterceptedRequest(
            DevToolsContinueInterceptedRequestCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                request_id: DevToolsRequestId::from(bidi_request_id.as_str()),
                url: None,
                method: None,
                post_data: None,
                headers: None,
                intercept_response: false,
            },
        ))
        .await
        .into_complete_parts();
    if let Some(predecessor) = continue_renderer_output_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    assert_eq!(
        continue_result.expect("BiDi continue request should succeed"),
        AutomationResult::Empty
    );
    let mut continue_output = Vec::new();
    drain_scheduler_events_like_scheduler(
        &mut ctx.conn,
        &mut continue_output,
        continue_scheduler_events,
    )
    .await;
    ctx.sent.extend(continue_output);

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        41_014,
        "globalThis.__lm_mixed_intercept_result",
        &json!("mixed-hit"),
        "mixed CDP Fetch / BiDi intercept result",
    )
    .await;
    assert_eq!(resolved["result"]["result"]["value"], "mixed-hit");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_then_bidi_network_intercept_response_stage_chain_completes() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn hit() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "mixed-response-hit",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api/hit", any(hit)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let hit_url = format!("http://{addr}/api/hit");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 41_110,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": hit_url.clone(), "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(41_110, json!({}), Some("SID-1"));

    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit"),
                phases: vec![DevToolsNetworkInterceptPhase::ResponseStarted],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: hit_url.clone(),
                }],
            }),
        )
        .await;
    assert!(
        result.is_ok(),
        "BiDi response-stage add intercept should succeed: {result:?}"
    );
    let api_url = Url::parse(&hit_url).unwrap();
    let response_pause_sessions = ctx
        .conn
        .target_fetch_subresource_interception_snapshot_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-1"),
        )
        .expect("active target fetch snapshot")
        .matching_response_stage_pause_sessions(
            Some("SID-1"),
            DevToolsNetworkResourceType::Fetch,
            &api_url,
        );
    assert_eq!(
        response_pause_sessions
            .iter()
            .map(|session| session.session_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("SID-1"), Some("BIDI-SID")]
    );
    assert_eq!(
        response_pause_sessions[1]
            .blocked_intercepts
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-hit"]
    );
    enable_runtime_async(&mut ctx, "SID-1", 41_111).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 41_112,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_mixed_response_intercept_result = "pending";
  fetch('/api/hit')
    .then(response => response.text())
    .then(text => { globalThis.__lm_mixed_response_intercept_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 41_112);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "CDP Fetch response-stage pause before BiDi response-stage pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["request"]["url"] == json!(hit_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;

    let cdp_pause = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["request"]["url"] == json!(hit_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing CDP response-stage pause: {:?}", ctx.sent));
    assert!(
        cdp_pause["params"]["__moliBlockedInterceptors"].is_null(),
        "CDP Fetch response-stage pause should not carry BiDi blocked marker: {cdp_pause:?}"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("BIDI-SID")
        }),
        "BiDi-owned response pause should wait until the CDP Fetch handler continues"
    );
    let cdp_request_id = cdp_pause["params"]["requestId"]
        .as_str()
        .expect("CDP response-stage request id")
        .to_owned();
    let cdp_owner = crate::conn::CommandOwnerScope::for_session("SID-1");
    let pending_cdp_pause =
        crate::domains::fetch::state::pending_subresource_response_request_for_action_session(
            &mut ctx.conn,
            &cdp_owner,
            Some("SID-1"),
            &cdp_request_id,
        )
        .expect("CDP response-stage pending request");
    let pending_chain = pending_cdp_pause
        .response_stage_pause_state()
        .expect("CDP response-stage pending should have chained BiDi response pause");
    assert_eq!(pending_chain.remaining_sessions.len(), 1);
    assert_eq!(
        pending_chain.remaining_sessions[0].session_id.as_deref(),
        Some("BIDI-SID")
    );
    assert_eq!(
        pending_chain.remaining_sessions[0]
            .blocked_intercepts
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-hit"]
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 41_113,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": cdp_request_id }
    }))
    .await;
    ctx.expect_result(41_113, json!({}), Some("SID-1"));

    let bidi_pause = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("BIDI-SID")
                && message["params"]["request"]["url"] == json!(hit_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing chained BiDi response-stage pause: {:?}", ctx.sent));
    assert_eq!(
        bidi_pause["params"]["__moliBlockedInterceptors"],
        json!(["intercept-hit"]),
        "chained BiDi response-stage pause should carry blocked marker: pause={bidi_pause:?}; sent={:?}",
        ctx.sent
    );
    let bidi_request_id = bidi_pause["params"]["requestId"]
        .as_str()
        .expect("BiDi chained response-stage request id")
        .to_owned();
    ctx.sent.clear();

    let (
        continue_result,
        continue_scheduler_events,
        _continue_protocol_events,
        continue_renderer_output_predecessor,
    ) = ctx
        .conn
        .execute_automation_command(AutomationCommand::ContinueInterceptedResponse(
            DevToolsContinueInterceptedResponseCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                request_id: DevToolsRequestId::from(bidi_request_id.as_str()),
                response_code: None,
                response_headers: None,
                response_phrase: None,
                auth_credentials: None,
            },
        ))
        .await
        .into_complete_parts();
    if let Some(predecessor) = continue_renderer_output_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    assert_eq!(
        continue_result.expect("BiDi continue response should succeed"),
        AutomationResult::Empty
    );
    let mut continue_output = Vec::new();
    drain_scheduler_events_like_scheduler(
        &mut ctx.conn,
        &mut continue_output,
        continue_scheduler_events,
    )
    .await;
    ctx.sent.extend(continue_output);

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        41_114,
        "globalThis.__lm_mixed_response_intercept_result",
        &json!("mixed-response-hit"),
        "mixed CDP Fetch / BiDi response-stage intercept result",
    )
    .await;
    assert_eq!(resolved["result"]["result"]["value"], "mixed-response-hit");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn bidi_response_stage_network_intercept_marks_fetch_continuation_request_id() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn hit() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "hit")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api/hit", any(hit)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let hit_url = format!("http://{addr}/api/hit");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 41_200,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(41_200, json!({}), Some("SID-1"));
    ctx.sent.clear();

    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit"),
                phases: vec![DevToolsNetworkInterceptPhase::ResponseStarted],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: hit_url.clone(),
                }],
            }),
        )
        .await;
    assert_eq!(
        result.expect("BiDi add response-stage intercept should succeed"),
        AutomationResult::AddNetworkIntercept(
            crate::automation::DevToolsAddNetworkInterceptResult {
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit")
            }
        )
    );
    ctx.sent.clear();

    let (evaluate_result, scheduler_events, protocol_events, renderer_output_predecessor) = ctx
        .conn
        .execute_automation_command(AutomationCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                realm_id: None,
                world_name: None,
                expression: r#"(() => {
  globalThis.__lm_bidi_response_intercept_result = "pending";
  fetch('/api/hit').then(r => r.text()).then(text => { globalThis.__lm_bidi_response_intercept_result = text; });
  return "scheduled";
})()"#
                .to_owned(),
                await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await
        .into_complete_parts();
    if let Some(predecessor) = renderer_output_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    evaluate_result.expect("BiDi script.evaluate should start fetch");
    let mut scheduler_output = protocol_events_into_internal_messages(protocol_events);
    drain_scheduler_events_like_scheduler_preserving_internal_fields(
        &mut ctx.conn,
        &mut scheduler_output,
        scheduler_events,
    )
    .await;
    ctx.sent.extend(scheduler_output);
    let paused = wait_for_target_fetch_request_paused(
        &mut ctx,
        None,
        &hit_url,
        Some(200),
        "BiDi response-stage network intercept pause",
    )
    .await;
    assert_eq!(
        paused["params"]["__moliBlockedInterceptors"],
        json!(["intercept-hit"]),
        "response-stage pause should carry the matching BiDi intercept marker: {paused:?}"
    );
    assert!(
        paused["params"]["networkId"].as_str().is_some(),
        "response-stage pause should keep the network request id for CDP correlation: {paused:?}"
    );
    assert_ne!(
        paused["params"]["networkId"], paused["params"]["requestId"],
        "Fetch continuation id should remain distinct from the network id: {paused:?}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn multiple_patterns_can_mix_request_and_response_stage_by_resource_type() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn hit() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "fetch-hit")
    }

    async fn xhr() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "xhr-hit")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api/hit", get(hit))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let hit_url = format!("http://{addr}/api/hit");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 615,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(615, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 616,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": "*/xhr", "requestStage": "Request", "resourceType": "XHR" },
                { "urlPattern": "*/api/hit", "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(616, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 617).await;

    ctx.process_async(json!({
            "id": 618,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": r#"(() => {
  globalThis.__lm_multi_pattern_fetch = "pending";
  globalThis.__lm_multi_pattern_xhr = "pending";
  fetch('/api/hit').then(r => r.text()).then(text => { globalThis.__lm_multi_pattern_fetch = text; });
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/xhr');
  xhr.onload = () => { globalThis.__lm_multi_pattern_xhr = xhr.responseText; };
  xhr.send();
  return "scheduled";
})()"#
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 618);

    let xhr_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(xhr_url)
                && message["params"].get("responseStatusCode").is_none()
        })
        .cloned()
        .expect("xhr request-stage pause");
    let xhr_request_id = xhr_paused["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    let xhr_network_id = xhr_paused["params"]["networkId"]
        .as_str()
        .unwrap()
        .to_owned();

    wait_until_message(&mut ctx, "SID-1", "fetch response-stage pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("XHR")
            && message["params"]["request"]["url"] == json!(hit_url)
            && message["params"]["responseStatusCode"] == json!(200)
    })
    .await;
    let fetch_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(hit_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .expect("fetch response-stage pause");
    let fetch_request_id = fetch_paused["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 619,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": xhr_request_id }
    }))
    .await;
    ctx.expect_result(619, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 620,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(620, json!({}), Some("SID-1"));

    wait_until_scheduler_message(
        &mut ctx,
        "mixed-pattern XHR network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(xhr_network_id)
        },
    )
    .await;

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        621,
        "[globalThis.__lm_multi_pattern_fetch, globalThis.__lm_multi_pattern_xhr].join(',')",
        &json!("fetch-hit,xhr-hit"),
        "mixed request/response-stage pattern completion",
    )
    .await;
    assert_eq!(resolved["result"]["result"]["value"], "fetch-hit,xhr-hit");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn disable_aborts_paused_runtime_fetch_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "subresource body")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 366,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(366, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 367,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(367, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 406).await;

    ctx.process_async(json!({
        "id": 368,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_result = text; })
    .catch(() => { globalThis.__lm_fetch_result = "failed"; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 368);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    assert_eq!(paused["params"]["request"]["url"], api_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 369,
        "method": "Fetch.disable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(369, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["errorText"], "Fetch interception disabled");

    ctx.process_async(json!({
        "id": 370,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(370, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 371,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 371);
    assert_eq!(resolved["result"]["result"]["value"], "failed");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_runtime_fetch_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "ok")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 780,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(780, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 781,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(781, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 782).await;

    ctx.process_async(json!({
        "id": 783,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_result = text; })
    .catch(() => { globalThis.__lm_fetch_result = "failed"; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 783);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    assert_eq!(paused["params"]["request"]["url"], api_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 784,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(784, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["errorText"], "net::ERR_ABORTED");
    assert_eq!(failed["params"]["canceled"], true);

    ctx.process_async(json!({
        "id": 785,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(785, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 786,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 786);
    assert_eq!(resolved["result"]["result"]["value"], "failed");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_aborts_paused_runtime_fetch_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "ok")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 887,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(887, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 888,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(888, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 889).await;

    ctx.process_async(json!({
        "id": 890,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_close_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_close_result = text; })
    .catch(() => { globalThis.__lm_fetch_close_result = "failed"; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 890);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    assert_eq!(paused["params"]["request"]["url"], api_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 891,
        "method": "Page.close",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(891, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["errorText"], "Page closed");

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.detached") && message["sessionId"] == json!("SID-1")
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!("TID-1")
    }));

    ctx.process_async(json!({
        "id": 892,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(892, -32001, "Unknown sessionId");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_fulfill_request_resolves_with_synthetic_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
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
    let api_url = format!("http://{addr}/synthetic");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 366,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(366, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 367,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(367, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 402).await;

    ctx.process_async(json!({
        "id": 368,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_synthetic_fetch = "pending";
  fetch('/synthetic')
    .then(response => response.text())
    .then(text => { globalThis.__lm_synthetic_fetch = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 368);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource fetch request id")
        .to_owned();
    assert_eq!(paused["params"]["request"]["url"], api_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 369,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 202,
            "responseHeaders": [{ "name": "content-type", "value": "text/plain" }],
            "body": "c3ludGhldGljLWJvZHk="
        }
    }))
    .await;
    ctx.expect_result(369, json!({}), Some("SID-1"));

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("network request event");
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 202);
    assert_eq!(response["params"]["response"]["mimeType"], "text/plain");

    ctx.process_async(json!({
        "id": 370,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": network_request_id }
    }))
    .await;
    ctx.expect_result(
        370,
        json!({
            "body": "synthetic-body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        371,
        "globalThis.__lm_synthetic_fetch",
        &json!("synthetic-body"),
        "fulfillRequest fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_fulfill_request_preserves_binary_body() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
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
    let api_url = format!("http://{addr}/synthetic-binary");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_920,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_920, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_921,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_921, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_922).await;

    ctx.process_async(json!({
        "id": 37_923,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_synthetic_binary_fetch = "pending";
  fetch('/synthetic-binary')
    .then(response => response.arrayBuffer())
    .then(buffer => {
      globalThis.__lm_synthetic_binary_fetch = Array.from(new Uint8Array(buffer)).join(',');
    });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_923);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("binary subresource fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("binary subresource fetch request id")
        .to_owned();
    assert_eq!(paused["params"]["request"]["url"], api_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_924,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 202,
            "responseHeaders": [{ "name": "content-type", "value": "application/octet-stream" }],
            "body": "AP9h"
        }
    }))
    .await;
    ctx.expect_result(37_924, json!({}), Some("SID-1"));

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("binary network request event");
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("binary network request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 37_925,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": network_request_id }
    }))
    .await;
    ctx.expect_result(
        37_925,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-1"),
    );

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_926,
        "globalThis.__lm_synthetic_binary_fetch",
        &json!("0,255,97"),
        "binary fulfillRequest fetch result",
    )
    .await;

    server.abort();
}
