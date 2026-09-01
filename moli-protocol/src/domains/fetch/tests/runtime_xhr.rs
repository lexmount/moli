use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_sync_open_runs_without_fetch_interception_pause() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "sync-ok")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_runtime_async(&mut ctx, "SID-1", 37_000).await;

    ctx.process_async(json!({
        "id": 37_001,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  try {
    const xhr = new XMLHttpRequest();
    xhr.open('GET', '/xhr', false);
    xhr.send();
    return `${xhr.status}:${xhr.responseText}`;
  } catch (error) {
    return `${error.name}:${error.message}`;
  }
})()"#
        }
    }))
    .await;

    let result = take_response_by_id(&mut ctx, 37_001);
    assert_eq!(result["result"]["result"]["value"], "200:sync-ok");
    assert!(
        ctx.sent
            .iter()
            .all(|message| { !matches!(message["method"].as_str(), Some("Fetch.requestPaused")) }),
        "sync XHR without Fetch enabled must not pause for interception: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_sync_open_does_not_start_network_before_send() {
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
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_runtime_async(&mut ctx, "SID-1", 37_000).await;

    ctx.process_async(json!({
        "id": 37_001,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  try {
    const xhr = new XMLHttpRequest();
    xhr.open('GET', '/xhr', false);
    return 'not-thrown';
  } catch (error) {
    return `${error.name}:${error.message}`;
  }
})()"#
        }
    }))
    .await;

    let result = take_response_by_id(&mut ctx, 37_001);
    assert_eq!(result["result"]["result"]["value"], "not-thrown");
    assert!(
        ctx.sent.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Fetch.requestPaused")
                    | Some("Network.requestWillBeSent")
                    | Some("Network.responseReceived")
                    | Some("Network.loadingFinished")
            )
        }),
        "sync XHR open without send must not start network work: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_pauses_until_continue_request_then_loads_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-xhr-subresource", "ok"),
            ],
            format!("xhr:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", any(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
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
        "id": 372,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(372, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 373,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(373, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 403).await;

    ctx.process_async(json!({
        "id": 374,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_result = xhr.responseText; };
  xhr.send('payload');
  return "scheduled";
})()"#
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 374);
    assert_eq!(evaluate["id"], 374);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource xhr requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource xhr request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource xhr network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], xhr_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("XHR"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 375,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(375, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource xhr network completion",
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
        .expect("network xhr response event");
    assert_eq!(
        response["params"]["response"]["headers"]["x-xhr-subresource"],
        "ok"
    );

    ctx.process_async(json!({
        "id": 376,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 376);
    assert_eq!(resolved["result"]["result"]["value"], "xhr:payload");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_arraybuffer_preserves_binary_response_bytes() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn binary() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/octet-stream")],
            vec![0x00_u8, 0x80, 0xff, b'a'],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/binary", get(binary)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_runtime_async(&mut ctx, "SID-1", 37_010).await;

    ctx.process_async(json!({
        "id": 37_011,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_binary_bytes = "pending";
  const xhr = new XMLHttpRequest();
  xhr.responseType = "arraybuffer";
  xhr.onload = () => {
    globalThis.__lm_xhr_binary_bytes = Array.from(new Uint8Array(xhr.response)).join(",");
  };
  xhr.onerror = () => { globalThis.__lm_xhr_binary_bytes = "error"; };
  xhr.open("GET", "/binary");
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let scheduled = take_response_by_id(&mut ctx, 37_011);
    assert_eq!(scheduled["result"]["result"]["value"], "scheduled");

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_012,
        "globalThis.__lm_xhr_binary_bytes",
        &json!("0,128,255,97"),
        "async XHR arrayBuffer bytes",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_blob_preserves_binary_response_bytes() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn binary() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/octet-stream")],
            vec![0x00_u8, 0x80, 0xff, b'a'],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/binary", get(binary)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_runtime_async(&mut ctx, "SID-1", 37_020).await;

    ctx.process_async(json!({
        "id": 37_021,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_blob_bytes = "pending";
  const xhr = new XMLHttpRequest();
  xhr.responseType = "blob";
  xhr.onload = () => {
    let responseTextState = "not-checked";
    try {
      void xhr.responseText;
      responseTextState = "readable";
    } catch (error) {
      responseTextState = error && error.name;
    }
    xhr.response.arrayBuffer().then(buffer => {
      const bytes = Array.from(new Uint8Array(buffer)).join(",");
      globalThis.__lm_xhr_blob_bytes = `${bytes}|${responseTextState}`;
    });
  };
  xhr.onerror = () => { globalThis.__lm_xhr_blob_bytes = "error"; };
  xhr.open("GET", "/binary");
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let scheduled = take_response_by_id(&mut ctx, 37_021);
    assert_eq!(scheduled["result"]["result"]["value"], "scheduled");

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_022,
        "globalThis.__lm_xhr_blob_bytes",
        &json!("0,128,255,97|InvalidStateError"),
        "async XHR Blob bytes",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_request_animation_frame_pauses_until_continue_request_then_loads_response()
 {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-xhr-subresource", "raf"),
            ],
            format!("xhr-raf:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", any(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
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
        "id": 377,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(377, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 378,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(378, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 404).await;

    ctx.process_async(json!({
        "id": 379,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_result = "pending";
  requestAnimationFrame(() => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', '/xhr');
    xhr.onload = () => { globalThis.__lm_xhr_result = xhr.responseText; };
    xhr.send('payload');
  });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 379);
    assert_eq!(evaluate["id"], 379);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr requestAnimationFrame requestPaused event",
        |message| message["method"] == json!("Fetch.requestPaused"),
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource xhr requestAnimationFrame requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource xhr request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource xhr network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], xhr_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("XHR"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 380,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(380, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource xhr requestAnimationFrame network completion",
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
        .expect("network xhr response event");
    assert_eq!(
        response["params"]["response"]["headers"]["x-xhr-subresource"],
        "raf"
    );

    ctx.process_async(json!({
        "id": 381,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 381);
    assert_eq!(resolved["result"]["result"]["value"], "xhr-raf:payload");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_queue_microtask_pauses_until_continue_request_then_loads_response()
{
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-xhr-subresource", "microtask"),
            ],
            format!("xhr-microtask:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", any(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
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
        "id": 803,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(803, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 804,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(804, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 410).await;

    ctx.process_async(json!({
        "id": 805,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_result = "pending";
  queueMicrotask(() => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', '/xhr');
    xhr.onload = () => { globalThis.__lm_xhr_result = xhr.responseText; };
    xhr.send('payload');
  });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 805);
    assert_eq!(evaluate["id"], 805);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource xhr queueMicrotask requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource xhr request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource xhr network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], xhr_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("XHR"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 806,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(806, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource xhr queueMicrotask network completion",
    )
    .await;

    ctx.process_async(json!({
        "id": 807,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 807);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "xhr-microtask:payload"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_promise_then_pauses_until_continue_request_then_loads_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-xhr-subresource", "promise"),
            ],
            format!("xhr-promise:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", any(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 813,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(813, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 814,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(814, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 412).await;

    ctx.process_async(json!({
        "id": 815,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_result = "pending";
  Promise.resolve().then(() => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', '/xhr');
    xhr.onload = () => { globalThis.__lm_xhr_result = xhr.responseText; };
    xhr.send('payload');
  });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 815);
    assert_eq!(evaluate["id"], 815);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource xhr promise.then requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource xhr request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource xhr network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], xhr_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("XHR"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 816,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(816, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource xhr promise.then network completion",
    )
    .await;

    ctx.process_async(json!({
        "id": 817,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 817);
    assert_eq!(resolved["result"]["result"]["value"], "xhr-promise:payload");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_request_idle_callback_pauses_until_continue_request_then_loads_response()
 {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-xhr-subresource", "idle"),
            ],
            format!("xhr-idle:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", any(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 387,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(387, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 388,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(388, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 408).await;

    ctx.process_async(json!({
        "id": 389,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_result = "pending";
  requestIdleCallback(() => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', '/xhr');
    xhr.onload = () => { globalThis.__lm_xhr_result = xhr.responseText; };
    xhr.send('payload');
  });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 389);
    assert_eq!(evaluate["id"], 389);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr requestIdleCallback requestPaused event",
        |message| message["method"] == json!("Fetch.requestPaused"),
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource xhr requestIdleCallback requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource xhr request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("subresource xhr network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], xhr_url);
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    let request = network_request_announced_before_fetch_pause(&ctx, &paused, Some("XHR"));
    let network_request_id = request["params"]["requestId"]
        .as_str()
        .expect("network request id")
        .to_owned();
    assert_eq!(network_request_id, network_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 390,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(390, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_request_id,
        "subresource xhr requestIdleCallback network completion",
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
        .expect("network xhr response event");
    assert_eq!(
        response["params"]["response"]["headers"]["x-xhr-subresource"],
        "idle"
    );

    ctx.process_async(json!({
        "id": 391,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 391);
    assert_eq!(resolved["result"]["result"]["value"], "xhr-idle:payload");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_resource_type_filter_pauses_shared_xhr_interception_type() {
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
        "id": 610,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(610, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 611).await;

    ctx.process_async(json!({
        "id": 612,
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
    let _ = take_response_by_id(&mut ctx, 612);

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
        "XHR filter should pause both fetch and XHR: {:?}",
        ctx.sent
    );
    for expected_url in [&fetch_url, &xhr_url] {
        assert!(
            paused
                .iter()
                .any(|event| { event["params"]["request"]["url"] == json!(expected_url) })
        );
    }

    ctx.sent.clear();
    for (offset, event) in paused.into_iter().enumerate() {
        let request_id = event["params"]["requestId"]
            .as_str()
            .expect("fetch-like request id");
        let command_id = 613 + offset as u64;
        ctx.process_async(json!({
            "id": command_id,
            "method": "Fetch.continueRequest",
            "sessionId": "SID-1",
            "params": { "requestId": request_id }
        }))
        .await;
        ctx.expect_result(command_id, json!({}), Some("SID-1"));
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_fulfill_request_loads_synthetic_response() {
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
    let xhr_url = format!("http://{addr}/xhr-synthetic");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 377,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(377, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 378,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(378, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 404).await;

    ctx.process_async(json!({
        "id": 379,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_synthetic = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/xhr-synthetic');
  xhr.onload = () => { globalThis.__lm_xhr_synthetic = xhr.responseText; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 379);

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("subresource xhr requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("subresource xhr request id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], xhr_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 380,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 207,
            "responseHeaders": [{ "name": "content-type", "value": "text/plain" }],
            "body": "eGhyLXN5bnRoZXRpYw=="
        }
    }))
    .await;
    ctx.expect_result(380, json!({}), Some("SID-1"));

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("network xhr request event");
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
        .expect("network xhr response event");
    assert_eq!(response["params"]["response"]["status"], 207);

    ctx.process_async(json!({
        "id": 381,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_synthetic" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 381);
    assert_eq!(resolved["result"]["result"]["value"], "xhr-synthetic");

    server.abort();
}
