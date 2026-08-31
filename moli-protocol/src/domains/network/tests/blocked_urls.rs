use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 2800,
        "method": "Network.setBlockedURLs",
        "params": { "urls": ["http://example.test/*"] }
    }))
    .await;
    ctx.expect_error(2800, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({
        "id": 2801,
        "method": "Network.setBlockedURLs",
        "params": {}
    }))
    .await;
    ctx.expect_error(2801, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_updates_browser_context_state() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 2802,
        "method": "Network.setBlockedURLs",
        "params": { "urls": ["http://example.test/*.png", "*://cdn.example.test/*"] }
    }))
    .await;
    ctx.expect_result(2802, json!({}), None);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .network_policy
            .blocked_url_patterns(),
        vec![
            "http://example.test/*.png".to_owned(),
            "*://cdn.example.test/*".to_owned()
        ]
    );

    ctx.process_async(json!({
        "id": 2803,
        "method": "Network.setBlockedURLs",
        "params": { "urls": [] }
    }))
    .await;
    ctx.expect_result(2803, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .network_policy
            .blocked_url_patterns()
            .is_empty()
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_navigation_fails_with_blocked_by_client() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2804,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["http://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(2804, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2805,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/blocked/page" }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let request = ctx.take_one();
    assert_eq!(request["method"], "Network.requestWillBeSent");
    assert_eq!(
        request["params"]["request"]["url"],
        "http://example.test/blocked/page"
    );
    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");
    ctx.expect_error(2805, -32000, "net::ERR_BLOCKED_BY_CLIENT");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_runtime_fetch_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2806,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>blocked</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2807,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["http://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(2807, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2808,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2808, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2809,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_blocked_fetch_result = "pending";
  fetch('http://example.test/blocked/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_blocked_fetch_result = text; })
    .catch(error => { globalThis.__lm_blocked_fetch_result = String(error); });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2809))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "Fetch", "blocked fetch failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");

    ctx.process_async(json!({
        "id": 2810,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_blocked_fetch_result" }
    }))
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2810))
        .cloned()
        .expect("runtime evaluate result");
    assert!(
        result["result"]["result"]["value"]
            .as_str()
            .expect("blocked fetch result")
            .contains("net::ERR_BLOCKED_BY_CLIENT")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn background_set_blocked_urls_updates_loaded_owner_page_without_promotion() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0_u8; 1024];
                loop {
                    let Ok(read) = stream.read(&mut buf).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let (content_type, body): (&str, &[u8]) = if request.starts_with("GET /api ") {
                    ("text/plain", b"unblocked")
                } else {
                    ("text/html", b"<!doctype html><body>background</body>")
                };
                let response = format!(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: {}\r\n",
                        "Content-Length: {}\r\n",
                        "\r\n"
                    ),
                    content_type,
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(body).await;
            });
        }
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = None;
    ctx.sent.clear();

    let background = PageTargetHost::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    );
    let mut inactive = BrowserContext::new("BID-background".to_owned());
    inactive.insert_page_target_host(background);
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
        .await;

    ctx.process_async(json!({
        "id": 2811,
        "method": "Network.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(2811, json!({}), Some("SID-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2812,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(2812, json!({}), Some("SID-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2813,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-background",
        "params": { "urls": [api_url] }
    }))
    .await;
    ctx.expect_result(2813, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 2814,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": format!(
                r#"(() => {{
  globalThis.__lm_background_blocked_fetch_result = "pending";
  fetch({api_url:?})
    .then(response => response.text())
    .then(text => {{ globalThis.__lm_background_blocked_fetch_result = text; }})
    .catch(error => {{ globalThis.__lm_background_blocked_fetch_result = String(error); }});
  return "scheduled";
}})()"#
            )
        }
    }))
    .await;

    wait_until_messages(
        &mut ctx,
        Some("SID-background"),
        "background blocked fetch failure",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["type"] == json!("Fetch")
                    && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_CLIENT")
            })
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 2815,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": { "expression": "globalThis.__lm_background_blocked_fetch_result" }
    }))
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2815))
        .cloned()
        .expect("runtime evaluate result");
    assert!(
        result["result"]["result"]["value"]
            .as_str()
            .expect("blocked fetch result")
            .contains("net::ERR_BLOCKED_BY_CLIENT")
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct background Network policy should not promote the owner"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_worker_fetch_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2811,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>blocked worker fetch</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2812,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["http://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(2812, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2813,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-worker-fetch" } }
    }))
    .await;
    ctx.expect_result(2813, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2814,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2814, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2815,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_blocked_worker_fetch_result = "pending";
  globalThis.__lm_blocked_worker_fetch_done = null;
  const source = `
    fetch("http://example.test/blocked/worker")
      .then(response => response.text())
      .then(text => postMessage("loaded:" + text))
      .catch(error => postMessage(String(error)));
  `;
  const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
  globalThis.__lm_blocked_worker_fetch_done = new Promise(resolve => {
    worker.onmessage = event => {
      globalThis.__lm_blocked_worker_fetch_result = event.data;
      resolve(event.data);
    };
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2815))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "Fetch", "blocked worker fetch failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");
    let request_id = failed["params"]["requestId"]
        .as_str()
        .expect("failed request id");
    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("matching requestWillBeSent");
    assert_eq!(
        request["params"]["request"]["url"],
        "http://example.test/blocked/worker"
    );
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "works-worker-fetch"
    );
    ctx.process_async(json!({
        "id": 2816,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "expression": "globalThis.__lm_blocked_worker_fetch_done"
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "blocked worker fetch awaitPromise result",
        |messages| messages.iter().any(|message| message["id"] == json!(2816)),
    )
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2816))
        .cloned()
        .expect("runtime evaluate result");
    assert!(
        result["result"]["result"]["value"]
            .as_str()
            .expect("blocked worker fetch result")
            .contains("net::ERR_BLOCKED_BY_CLIENT")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_worker_xhr_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2816,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>blocked worker xhr</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2817,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["http://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(2817, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2818,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-worker-xhr" } }
    }))
    .await;
    ctx.expect_result(2818, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2819,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2819, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2820,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_blocked_worker_xhr_result = "pending";
  globalThis.__lm_blocked_worker_xhr_done = null;
  const source = `
    const xhr = new XMLHttpRequest();
    xhr.onload = () => postMessage("loaded:" + xhr.status);
    xhr.onerror = () => postMessage("failed:" + xhr.status);
    xhr.open("GET", "http://example.test/blocked/worker-xhr");
    xhr.send();
  `;
  const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
  globalThis.__lm_blocked_worker_xhr_done = new Promise(resolve => {
    worker.onmessage = event => {
      globalThis.__lm_blocked_worker_xhr_result = event.data;
      resolve(event.data);
    };
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2820))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "XHR", "blocked worker xhr failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");
    let request_id = failed["params"]["requestId"]
        .as_str()
        .expect("failed request id");
    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("matching requestWillBeSent");
    assert_eq!(
        request["params"]["request"]["url"],
        "http://example.test/blocked/worker-xhr"
    );
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "works-worker-xhr"
    );
    ctx.process_async(json!({
        "id": 2821,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "expression": "globalThis.__lm_blocked_worker_xhr_done"
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "blocked worker xhr awaitPromise result",
        |messages| messages.iter().any(|message| message["id"] == json!(2821)),
    )
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2821))
        .cloned()
        .expect("runtime evaluate result");
    assert_eq!(result["result"]["result"]["value"], "failed:0");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_worker_websocket_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2822,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>blocked worker websocket</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2823,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["ws://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(2823, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2824,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-worker-ws" } }
    }))
    .await;
    ctx.expect_result(2824, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2825,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2825, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_and_wait_for_response_async(json!({
        "id": 2826,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_blocked_worker_ws_result = "pending";
  globalThis.__lm_blocked_worker_ws_done = null;
  const source = `
    const events = [];
    const socket = new WebSocket("ws://example.test/blocked/worker-ws");
    socket.onerror = () => events.push("error:" + socket.readyState);
    socket.onclose = event => {
      events.push("close:" + event.code + ":" + event.wasClean + ":" + socket.readyState);
      postMessage(events.join("|"));
    };
  `;
  const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
  globalThis.__lm_blocked_worker_ws_done = new Promise(resolve => {
    worker.onmessage = event => {
    globalThis.__lm_blocked_worker_ws_result = event.data;
      resolve(event.data);
    };
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2826))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "WebSocket", "blocked worker websocket failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "WebSocket");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");
    let request_id = failed["params"]["requestId"]
        .as_str()
        .expect("failed request id");
    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("matching requestWillBeSent");
    assert_eq!(
        request["params"]["request"]["url"],
        "ws://example.test/blocked/worker-ws"
    );
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "works-worker-ws"
    );
    ctx.complete_one_ready_scheduler_input_for_test().await;
    let worker_ws_failures = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["type"] == json!("WebSocket")
                && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_CLIENT")
        })
        .count();
    assert_eq!(worker_ws_failures, 1);
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.webSocketCreated")
            && message["params"]["url"] == json!("ws://example.test/blocked/worker-ws")
    }));

    ctx.process_and_wait_for_response_async(json!({
        "id": 2827,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "expression": "globalThis.__lm_blocked_worker_ws_done"
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "blocked worker websocket awaitPromise result",
        |messages| messages.iter().any(|message| message["id"] == json!(2827)),
    )
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2827))
        .cloned()
        .expect("runtime evaluate result");
    assert_eq!(
        result["result"]["result"]["value"],
        "error:3|close:1006:false:3"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn emulate_network_conditions_offline_worker_fetch_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2821,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>offline worker fetch</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2822,
        "method": "Network.emulateNetworkConditions",
        "sessionId": "SID-1",
        "params": {
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }
    }))
    .await;
    ctx.expect_result(2822, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2823,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2823, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2824,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_offline_worker_fetch_result = "pending";
  globalThis.__lm_offline_worker_fetch_done = null;
  const source = `
    fetch("http://example.test/offline/worker-fetch")
      .then(response => response.text())
      .then(text => postMessage("loaded:" + text))
      .catch(error => postMessage(String(error)));
  `;
  const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
  globalThis.__lm_offline_worker_fetch_done = new Promise(resolve => {
    worker.onmessage = event => {
      globalThis.__lm_offline_worker_fetch_result = event.data;
      resolve(event.data);
    };
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2824))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "Fetch", "offline worker fetch failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["errorText"], "Network emulation offline");
    ctx.process_async(json!({
        "id": 2825,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "expression": "globalThis.__lm_offline_worker_fetch_done"
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "offline worker fetch awaitPromise result",
        |messages| messages.iter().any(|message| message["id"] == json!(2825)),
    )
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2825))
        .cloned()
        .expect("runtime evaluate result");
    assert!(
        result["result"]["result"]["value"]
            .as_str()
            .expect("offline worker fetch result")
            .contains("Network emulation offline")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn emulate_network_conditions_offline_worker_xhr_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2826,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>offline worker xhr</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2827,
        "method": "Network.emulateNetworkConditions",
        "sessionId": "SID-1",
        "params": {
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }
    }))
    .await;
    ctx.expect_result(2827, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2828,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2828, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2829,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_offline_worker_xhr_result = "pending";
  globalThis.__lm_offline_worker_xhr_done = null;
  const source = `
    const xhr = new XMLHttpRequest();
    xhr.onload = () => postMessage("loaded:" + xhr.status);
    xhr.onerror = () => postMessage("failed:" + xhr.status);
    xhr.open("GET", "http://example.test/offline/worker-xhr");
    xhr.send();
  `;
  const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
  globalThis.__lm_offline_worker_xhr_done = new Promise(resolve => {
    worker.onmessage = event => {
      globalThis.__lm_offline_worker_xhr_result = event.data;
      resolve(event.data);
    };
  });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2829))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "XHR", "offline worker xhr failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Network emulation offline");
    ctx.process_async(json!({
        "id": 2830,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "expression": "globalThis.__lm_offline_worker_xhr_done"
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "offline worker xhr awaitPromise result",
        |messages| messages.iter().any(|message| message["id"] == json!(2830)),
    )
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2830))
        .cloned()
        .expect("runtime evaluate result");
    assert_eq!(result["result"]["result"]["value"], "failed:0");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_runtime_xhr_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2811,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>blocked</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2812,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["http://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(2812, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2813,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2813, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2814,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_blocked_xhr_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.onload = () => { globalThis.__lm_blocked_xhr_result = "loaded"; };
  xhr.onerror = () => { globalThis.__lm_blocked_xhr_result = "failed:" + xhr.status; };
  xhr.open('GET', 'http://example.test/blocked/xhr');
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2814))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "XHR", "blocked xhr failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");

    ctx.process_async(json!({
        "id": 2815,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_blocked_xhr_result" }
    }))
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2815))
        .cloned()
        .expect("runtime evaluate result");
    assert_eq!(result["result"]["result"]["value"], "failed:0");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_runtime_websocket_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 2816,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>blocked websocket</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2817,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["ws://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(2817, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2818,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2818, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2819,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_blocked_ws_result = "pending";
  const socket = new WebSocket('ws://example.test/blocked/socket');
  const finish = value => {
    if (globalThis.__lm_blocked_ws_result === "pending") {
      globalThis.__lm_blocked_ws_result = value;
    }
  };
  socket.onopen = () => finish("open");
  socket.onerror = () => finish("error:" + socket.readyState);
  socket.onclose = event => finish("close:" + event.code + ":" + event.wasClean);
  return "scheduled";
})()"#
        }
    }))
    .await;

    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2819))
        .cloned()
        .expect("runtime evaluate result");
    flush_until_subresource_failed(&mut ctx, "WebSocket", "blocked websocket failure").await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["type"] == json!("WebSocket")
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["errorText"], "net::ERR_BLOCKED_BY_CLIENT");
    let websocket_created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.webSocketCreated"))
        .cloned()
        .expect("webSocketCreated event");
    assert_eq!(
        websocket_created["params"]["url"],
        "ws://example.test/blocked/socket"
    );

    ctx.process_async(json!({
        "id": 2820,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_blocked_ws_result" }
    }))
    .await;
    let result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(2820))
        .cloned()
        .expect("runtime evaluate result");
    assert_ne!(result["result"]["result"]["value"], "open");
}
#[tokio::test(flavor = "multi_thread")]
async fn emulate_network_conditions_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 28,
        "method": "Network.emulateNetworkConditions",
        "params": {
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }
    }))
    .await;
    ctx.expect_error(28, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn emulate_network_conditions_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({
        "id": 29,
        "method": "Network.emulateNetworkConditions",
        "params": { "offline": true }
    }))
    .await;
    ctx.expect_error(29, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn emulate_network_conditions_updates_browser_context_state() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 30,
        "method": "Network.emulateNetworkConditions",
        "params": {
            "offline": true,
            "latency": 150,
            "downloadThroughput": 1024,
            "uploadThroughput": 512,
            "connectionType": "cellular3g"
        }
    }))
    .await;
    ctx.expect_result(30, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.network_policy.network_offline());
    assert_eq!(bc.network_policy.emulated_network_latency(), 150.0);
    assert_eq!(bc.network_policy.emulated_download_throughput(), 1024.0);
    assert_eq!(bc.network_policy.emulated_upload_throughput(), 512.0);
    assert_eq!(
        bc.network_policy.emulated_connection_type(),
        Some("cellular3g")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn emulate_network_conditions_offline_navigation_fails_before_completion_events() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 31,
        "method": "Network.emulateNetworkConditions",
        "params": {
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }
    }))
    .await;
    ctx.expect_result(31, json!({}), None);

    ctx.process_async(json!({
        "id": 32,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/offline" }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let request = ctx.take_one();
    assert_eq!(request["method"], "Network.requestWillBeSent");
    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["errorText"], "Network emulation offline");
    ctx.expect_error(32, -32000, "Network emulation offline");
    assert!(ctx.sent.is_empty());
}
#[tokio::test(flavor = "multi_thread")]
async fn emulate_network_conditions_offline_runtime_fetch_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 33,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<html><body>offline</body></html>" }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 34,
        "method": "Network.emulateNetworkConditions",
        "params": {
            "offline": true,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1
        }
    }))
    .await;
    ctx.expect_result(34, json!({}), None);

    ctx.process_async(json!({
        "id": 35,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(35, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 36,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "fetch('http://example.test/api').catch(e => e && String(e))"
        }
    }))
    .await;
    let _ = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(36))
        .cloned()
        .expect("runtime evaluate result");
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "offline runtime fetch loadingFailed",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Network.loadingFailed"))
        },
    )
    .await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["errorText"], "Network emulation offline");
    let request_id = failed["params"]["requestId"]
        .as_str()
        .expect("failed request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 37,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        37,
        -32000,
        "No data found for resource with given identifier",
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_blocked_urls_blocks_parser_external_script_but_preserves_following_parse_work() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script src="http://example.test/blocked/parser-script.js"></script>
<script>
globalThis.__lm_after_blocked_parser_script = true;
</script>
</body></html>"#,
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
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 70_010,
        "method": "Network.setBlockedURLs",
        "sessionId": "SID-1",
        "params": { "urls": ["http://example.test/blocked/*"] }
    }))
    .await;
    ctx.expect_result(70_010, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_011,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    let _ = ctx.take_response_by_id(70_011);

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "parser blocked external script load completion",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 70_012,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify({ after: !!globalThis.__lm_after_blocked_parser_script, blockedType: typeof globalThis.__lm_blocked_parser_script_loaded })"
        }
    }))
    .await;
    let result = ctx.take_response_by_id(70_012);
    assert_eq!(
        result["result"]["result"]["value"],
        json!(r#"{"after":true,"blockedType":"undefined"}"#)
    );

    server.abort();
}
