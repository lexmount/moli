use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_service_worker_updates_live_page_fetches() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>service worker bypass</body>",
        )
    }

    async fn service_worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
self.addEventListener("install", event => {
  event.waitUntil(self.skipWaiting());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});
self.addEventListener("fetch", event => {
  const path = new URL(event.request.url).pathname;
  event.respondWith(new Response("worker:" + path));
});
"#,
        )
    }

    async fn probe() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "network:/probe")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(service_worker))
                .route("/probe", get(probe)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(browser_context);

    ctx.process_async(json!({
        "id": 80_000,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(80_000, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 80_001,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = ctx.take_response_by_id(80_001);

    ctx.process_async(json!({
        "id": 80_002,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(async () => {
  await navigator.serviceWorker.register("/worker.js", { scope: "/" });
  await navigator.serviceWorker.ready;
  const response = await fetch("/probe");
  return JSON.stringify({
    body: await response.text(),
    controlled: navigator.serviceWorker.controller !== null
  });
})()
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "service worker fetch before bypass",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(80_002))
        },
    )
    .await;
    let before = ctx.take_response_by_id(80_002);
    assert_eq!(
        before["result"]["result"]["value"],
        json!(r#"{"body":"worker:/probe","controlled":true}"#)
    );

    ctx.process_async(json!({
        "id": 80_003,
        "method": "Network.setBypassServiceWorker",
        "sessionId": "SID-1",
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_result(80_003, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 80_004,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(async () => {
  const response = await fetch("/probe");
  return JSON.stringify({
    body: await response.text(),
    controlled: navigator.serviceWorker.controller !== null
  });
})()
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "service worker fetch after bypass",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(80_004))
        },
    )
    .await;
    let after = ctx.take_response_by_id(80_004);
    assert_eq!(
        after["result"]["result"]["value"],
        json!(r#"{"body":"network:/probe","controlled":true}"#)
    );

    ctx.process_async(json!({
        "id": 80_005,
        "method": "Network.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": { "userAgent": "Moli/ServiceWorkerBypassTest" }
    }))
    .await;
    ctx.expect_result(80_005, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 80_006,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(async () => {
  const response = await fetch("/probe");
  return JSON.stringify({
    body: await response.text(),
    controlled: navigator.serviceWorker.controller !== null
  });
})()
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "service worker fetch after loader rebuild",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(80_006))
        },
    )
    .await;
    let after_rebuild = ctx.take_response_by_id(80_006);
    assert_eq!(
        after_rebuild["result"]["result"]["value"],
        json!(r#"{"body":"network:/probe","controlled":true}"#)
    );

    server.abort();
}
