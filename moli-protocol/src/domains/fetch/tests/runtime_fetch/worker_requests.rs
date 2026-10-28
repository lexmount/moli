use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_request_stage_pauses_until_continue_request_then_resolves_promise() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api', {
  method: 'POST',
  headers: { 'x-from-worker': 'yes' },
  body: 'payload'
})
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(String(error)));
"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            format!("worker-continued:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_000,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_000, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_001,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_001, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_002).await;

    ctx.process_async(json!({
        "id": 37_003,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_003);

    let paused = wait_for_request_paused(&mut ctx, &api_url, "worker fetch requestPaused").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(
        paused["params"]["request"]["headers"]["x-from-worker"],
        "yes"
    );
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_004,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_004, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch continued network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_005,
        "globalThis.__lm_worker_fetch_result",
        &json!("worker-continued:payload"),
        "worker fetch continueRequest result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn csp_report_request_stage_continue_preserves_service_worker_dispatch() {
    async fn page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                (
                    "Content-Security-Policy",
                    "connect-src 'none'; report-uri /csp-report",
                ),
            ],
            "<!doctype html><html><body>csp report</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
self.addEventListener("install", event => {
  event.waitUntil(Promise.resolve());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});
self.addEventListener("fetch", event => {
  const url = new URL(event.request.url);
  if (url.pathname === "/csp-report") {
    event.respondWith((async () => {
      const matched = await clients.matchAll({ includeUncontrolled: true });
      for (const client of matched) {
        client.postMessage([
          "destination=" + event.request.destination,
          "mode=" + event.request.mode,
          "credentials=" + event.request.credentials,
          "method=" + event.request.method,
          "from=service-worker"
        ].join("|"));
      }
      return new Response("from-service-worker");
    })());
  }
});
"#,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let report_url = format!("http://{addr}/csp-report");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_040,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_040, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_041,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request",
                "resourceType": "CSPViolationReport"
            }]
        }
    }))
    .await;
    ctx.expect_result(37_041, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_042).await;

    ctx.process_async(json!({
        "id": 37_043,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_csp_report_continue_result = "pending";
  navigator.serviceWorker.addEventListener("message", event => {
    globalThis.__lm_csp_report_continue_result = String(event.data);
  });
  (async () => {
    await navigator.serviceWorker.register("/worker.js", { scope: "/" });
    await navigator.serviceWorker.ready;
    await fetch("/blocked-data").catch(() => {});
    globalThis.__lm_csp_report_blocked = "done";
  })().catch(error => {
    globalThis.__lm_csp_report_continue_result =
      "error:" + String(error && error.message);
  });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_043);

    let paused = wait_for_request_paused(&mut ctx, &report_url, "CSP report requestPaused").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("CSP report request id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "CSPViolationReport");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_044,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_044, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_045,
        "globalThis.__lm_csp_report_continue_result",
        &json!(
            "destination=report|mode=no-cors|credentials=same-origin|method=POST|from=service-worker"
        ),
        "CSP report continueRequest should dispatch through Service Worker",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn csp_report_response_stage_take_body_as_stream_observes_report_body() {
    async fn page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                (
                    "Content-Security-Policy",
                    "connect-src 'none'; report-uri /csp-report",
                ),
            ],
            "<!doctype html><html><body>csp report response stream</body></html>",
        )
    }

    async fn report(body: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            format!("csp-report-response-body:{}", body.contains("blocked-data")),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/csp-report", any(report)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let report_url = format!("http://{addr}/csp-report");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_061,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_061, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_062,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Response",
                "resourceType": "CSPViolationReport"
            }]
        }
    }))
    .await;
    ctx.expect_result(37_062, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_063).await;

    ctx.process_async(json!({
        "id": 37_064,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  fetch("/blocked-data").catch(() => {});
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_064);

    let paused = wait_for_request_paused_on_session(
        &mut ctx,
        "SID-1",
        &report_url,
        Some("CSPViolationReport"),
        "CSP report response-stage pause",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("CSP report response-stage request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("CSP report response-stage network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "CSPViolationReport");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["responseStatusCode"], 200);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_065,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    let stream_result = take_response_by_id(&mut ctx, 37_065);
    let stream_handle = stream_result["result"]["stream"]
        .as_str()
        .expect("CSP report response body stream handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 37_066,
        "method": "IO.read",
        "params": { "handle": stream_handle }
    }))
    .await;
    ctx.expect_result(
        37_066,
        json!({
            "base64Encoded": false,
            "data": "csp-report-response-body:true",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 37_067,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    ctx.expect_error(
        37_067,
        -32602,
        "Unable to continue request as is after body is taken",
    );

    ctx.process_async(json!({
        "id": 37_068,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 204,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" },
                { "name": "x-csp-report-response-stream", "value": "synthetic" }
            ],
            "body": "Y3NwLXJlcG9ydC1zdHJlYW0tZnVsZmlsbGVk"
        }
    }))
    .await;
    ctx.expect_result(37_068, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "CSP report response-stage fulfill network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_csp_report_request_stage_continue_records_network_completion() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker csp report</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/javascript"),
                (
                    "Content-Security-Policy",
                    "connect-src 'none'; report-uri /csp-report",
                ),
            ],
            r#"
self.onmessage = async () => {
  await fetch('/blocked-data').catch(() => {});
  postMessage('blocked');
};
"#,
        )
    }

    async fn report(body: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            format!("report-received:{}", body.contains("blocked-data")),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/csp-report", any(report)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let report_url = format!("http://{addr}/csp-report");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_046,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_046, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_047,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request",
                "resourceType": "CSPViolationReport"
            }]
        }
    }))
    .await;
    ctx.expect_result(37_047, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_048).await;
    ctx.process_async(json!({
        "id": 37_049,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_csp_report_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => {
    globalThis.__lm_worker_csp_report_result = String(event.data);
  };
  worker.postMessage('start');
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_049);

    let paused = wait_for_background_request_paused(
        &mut ctx,
        Some("SID-1"),
        &report_url,
        "CSPViolationReport",
        "worker CSP report requestPaused",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker CSP report request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker CSP report network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "CSPViolationReport");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_050,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_050, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker CSP report network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_051,
        "globalThis.__lm_worker_csp_report_result",
        &json!("blocked"),
        "worker should continue after CSP violation report pause",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_csp_report_request_stage_pause_routes_to_popup_session() {
    async fn opener() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>popup opener</body></html>",
        )
    }

    async fn popup() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                (
                    "Content-Security-Policy",
                    "connect-src 'none'; report-uri /csp-report",
                ),
            ],
            r#"<!doctype html>
<html>
<body>
<script>
globalThis.__lm_popup_csp_report_result = "pending";
(async () => {
  await fetch("/blocked-data").catch(() => {});
  globalThis.__lm_popup_csp_report_result = "blocked";
})().catch(error => {
  globalThis.__lm_popup_csp_report_result =
    "error:" + String(error && error.message);
});
</script>
</body>
</html>"#,
        )
    }

    async fn report(body: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            format!("popup-report-received:{}", body.contains("blocked-data")),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/opener", get(opener))
                .route("/popup", get(popup))
                .route("/csp-report", any(report)),
        )
        .await
        .unwrap();
    });

    let opener_url = format!("http://{addr}/opener");
    let popup_url = format!("http://{addr}/popup");
    let report_url = format!("http://{addr}/csp-report");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &opener_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_052,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(37_052, json!({}), None);
    ctx.sent.clear();

    let (popup_target_id, popup_session_id) =
        open_auto_attached_popup_from_session(&mut ctx, 37_053, "SID-1", "about:blank#popup-csp")
            .await;

    ctx.process_async(json!({
        "id": 37_054,
        "method": "Network.enable",
        "sessionId": popup_session_id
    }))
    .await;
    ctx.expect_result(37_054, json!({}), Some(&popup_session_id));

    ctx.process_async(json!({
        "id": 37_055,
        "method": "Fetch.enable",
        "sessionId": popup_session_id,
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request",
                "resourceType": "CSPViolationReport"
            }]
        }
    }))
    .await;
    ctx.expect_result(37_055, json!({}), Some(&popup_session_id));
    ctx.sent.clear();

    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.target_owner_identity_for_session(Some(&popup_session_id))
                .and_then(|(_, target_id)| target_id),
            Some(popup_target_id.clone()),
            "the popup session must remain bound to its own target before navigation"
        )
    });

    ctx.process_async(json!({
        "id": 37_056,
        "method": "Page.navigate",
        "sessionId": popup_session_id,
        "params": { "url": popup_url }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 37_056);
    assert_eq!(navigation["result"]["frameId"], json!(popup_target_id));

    let paused = wait_for_request_paused_on_session(
        &mut ctx,
        &popup_session_id,
        &report_url,
        Some("CSPViolationReport"),
        "popup CSP report requestPaused",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("popup CSP report request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("popup CSP report network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "CSPViolationReport");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["request"]["url"] == json!(report_url)
        }),
        "popup CSP report pause must not be delivered to opener session: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_057,
        "method": "Fetch.continueRequest",
        "sessionId": popup_session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_057, json!({}), Some(&popup_session_id));

    wait_until_messages(
        &mut ctx,
        Some(popup_session_id.as_str()),
        "popup CSP report network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["sessionId"] == json!(popup_session_id)
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn csp_report_request_stage_fail_request_records_network_failure() {
    async fn page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                (
                    "Content-Security-Policy",
                    "connect-src 'none'; report-uri /csp-report",
                ),
            ],
            "<!doctype html><html><body>csp report fail</body></html>",
        )
    }

    async fn report(hits: axum::extract::State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        ([(CONTENT_TYPE.as_str(), "text/plain")], "unexpected-report")
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_hits = hits.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/csp-report", any(report))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let report_url = format!("http://{addr}/csp-report");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_046,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_046, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_047,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request",
                "resourceType": "CSPViolationReport"
            }]
        }
    }))
    .await;
    ctx.expect_result(37_047, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_048).await;

    ctx.process_async(json!({
        "id": 37_049,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  fetch("/blocked-data").catch(() => {});
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_049);

    let paused = wait_for_request_paused(&mut ctx, &report_url, "CSP report fail pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("CSP report request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("CSP report network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "CSPViolationReport");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_050,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "errorReason": "Aborted"
        }
    }))
    .await;
    ctx.expect_result(37_050, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "CSP report failRequest network failure",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(network_id)
                    && message["params"]["type"] == json!("CSPViolationReport")
                    && message["params"]["errorText"] == json!("Aborted")
            })
        },
    )
    .await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn csp_report_request_stage_fulfill_request_records_synthetic_response() {
    async fn page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                (
                    "Content-Security-Policy",
                    "connect-src 'none'; report-uri /csp-report",
                ),
            ],
            "<!doctype html><html><body>csp report fulfill</body></html>",
        )
    }

    async fn report(hits: axum::extract::State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        ([(CONTENT_TYPE.as_str(), "text/plain")], "unexpected-report")
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_hits = hits.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/csp-report", any(report))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let report_url = format!("http://{addr}/csp-report");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_056,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_056, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_057,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request",
                "resourceType": "CSPViolationReport"
            }]
        }
    }))
    .await;
    ctx.expect_result(37_057, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_058).await;

    ctx.process_async(json!({
        "id": 37_059,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  fetch("/blocked-data").catch(() => {});
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_059);

    let paused = wait_for_request_paused(&mut ctx, &report_url, "CSP report fulfill pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("CSP report request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("CSP report network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "CSPViolationReport");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_060,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 204,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" },
                { "name": "x-csp-report", "value": "synthetic" }
            ],
            "body": "c3ludGhldGljLWNzcC1yZXBvcnQ="
        }
    }))
    .await;
    ctx.expect_result(37_060, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "CSP report fulfillRequest network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(network_id)
            && message["params"]["type"] == json!("CSPViolationReport")
            && message["params"]["response"]["status"] == json!(204)
            && message["params"]["response"]["headers"]["x-csp-report"] == json!("synthetic")
    }));
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_request_handles_are_unique_across_workers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch handle uniqueness</body></html>",
        )
    }

    async fn worker_one() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api-one')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn worker_two() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api-two')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn api_one() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "one-body")
    }

    async fn api_two() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "two-body")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker-one.js", get(worker_one))
                .route("/worker-two.js", get(worker_two))
                .route("/worker-api-one", get(api_one))
                .route("/worker-api-two", get(api_two)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_one_url = format!("http://{addr}/worker-api-one");
    let api_two_url = format!("http://{addr}/worker-api-two");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_010,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_010, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_011,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_011, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_012).await;

    ctx.process_async(json!({
        "id": 37_013,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_pair_results = [];
  const worker = new Worker('/worker-one.js');
  worker.onmessage = event => { globalThis.__lm_worker_pair_results.push(event.data); };
  return "scheduled-one";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_013);

    let paused_one =
        wait_for_request_paused(&mut ctx, &api_one_url, "first worker fetch requestPaused").await;
    let request_id_one = paused_one["params"]["requestId"]
        .as_str()
        .expect("first worker fetch request id")
        .to_owned();
    let network_id_one = paused_one["params"]["networkId"]
        .as_str()
        .expect("first worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_014,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id_one }
    }))
    .await;
    ctx.expect_result(37_014, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "first worker fetch network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id_one)
            })
        },
    )
    .await;
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_015,
        "globalThis.__lm_worker_pair_results.join(',')",
        &json!("one-body"),
        "first worker fetch result",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_016,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  const worker = new Worker('/worker-two.js');
  worker.onmessage = event => { globalThis.__lm_worker_pair_results.push(event.data); };
  return "scheduled-two";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_016);

    let paused_two =
        wait_for_request_paused(&mut ctx, &api_two_url, "second worker fetch requestPaused").await;
    let request_id_two = paused_two["params"]["requestId"]
        .as_str()
        .expect("second worker fetch request id")
        .to_owned();
    let network_id_two = paused_two["params"]["networkId"]
        .as_str()
        .expect("second worker fetch network id")
        .to_owned();
    assert_ne!(network_id_one, network_id_two);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_017,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id_two }
    }))
    .await;
    ctx.expect_result(37_017, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "second worker fetch network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id_two)
            })
        },
    )
    .await;
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_018,
        "globalThis.__lm_worker_pair_results.join(',')",
        &json!("one-body,two-body"),
        "second worker fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_publication_surfaces_worker_fetch_request_pause() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch renderer publication</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(String(error)));
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "real-worker-body")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_050,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_050, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_051,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_051, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_052).await;
    ctx.process_async(json!({
        "id": 37_053,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_053);

    let paused = wait_for_background_request_paused(
        &mut ctx,
        Some("SID-1"),
        &api_url,
        "XHR",
        "worker fetch requestPaused should surface through renderer publication",
    )
    .await;
    assert_eq!(paused["sessionId"], json!("SID-1"));
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["method"], "GET");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_publication_surfaces_worker_xhr_request_pause() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker xhr renderer publication</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
const xhr = new XMLHttpRequest();
xhr.open('GET', '/worker-api', true);
xhr.onload = () => postMessage(xhr.responseText);
xhr.onerror = () => postMessage(`error:${xhr.status}`);
xhr.send();
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "real-worker-xhr-body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_060,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_060, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_061,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_061, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_062).await;
    ctx.process_async(json!({
        "id": 37_063,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_xhr_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_xhr_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_063);

    let paused = wait_for_background_request_paused(
        &mut ctx,
        Some("SID-1"),
        &api_url,
        "XHR",
        "worker xhr requestPaused should surface through renderer publication",
    )
    .await;
    assert_eq!(paused["sessionId"], json!("SID-1"));
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["method"], "GET");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_session_fetch_enable_receives_worker_xhr_request_pause() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>aux worker xhr renderer publication</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
const xhr = new XMLHttpRequest();
xhr.open('GET', '/worker-api?aux-worker-xhr=1', true);
xhr.onload = () => postMessage(xhr.responseText);
xhr.onerror = () => postMessage(`error:${xhr.status}`);
xhr.send();
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "aux-worker-xhr-body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api?aux-worker-xhr=1");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.with_fixture(|conn| {
        assert!({
            conn.browser_context
                .as_mut()
                .unwrap()
                .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
        })
    });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_070,
        "method": "Fetch.enable",
        "sessionId": "SID-attached",
        "params": {
            "patterns": [
                { "urlPattern": "*/worker-api*", "requestStage": "Request", "resourceType": "XHR" }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_070, json!({}), Some("SID-attached"));
    enable_runtime_async(&mut ctx, "SID-1", 37_071).await;
    ctx.process_async(json!({
        "id": 37_072,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_aux_worker_xhr_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_aux_worker_xhr_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_072);

    let paused = wait_for_background_request_paused(
        &mut ctx,
        Some("SID-attached"),
        &api_url,
        "XHR",
        "attached-session worker xhr requestPaused should surface through renderer publication",
    )
    .await;
    assert_eq!(paused["sessionId"], json!("SID-attached"));
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["method"], "GET");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_xhr_request_stage_continue_request_resolves_worker_result() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker xhr continue</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
const xhr = new XMLHttpRequest();
xhr.open('POST', '/worker-api', true);
xhr.onload = () => postMessage(xhr.responseText);
xhr.onerror = () => postMessage(`error:${xhr.status}`);
xhr.send('payload');
"#,
        )
    }

    async fn api(headers: HeaderMap, body: String) -> impl IntoResponse {
        let route_header = headers
            .get("x-route-worker")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("missing");
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            format!("worker-xhr:{route_header}:{body}"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", any(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_070,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_070, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_071,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_071, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_072).await;

    ctx.process_async(json!({
        "id": 37_073,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_xhr_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_xhr_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_073);

    let paused = wait_for_request_paused(&mut ctx, &api_url, "worker xhr requestPaused").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker xhr request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker xhr network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["method"], "POST");
    assert_eq!(paused["params"]["request"]["hasPostData"], true);
    assert_eq!(paused["params"]["request"]["postData"], "payload");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_074,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "headers": [
                { "name": "x-route-worker", "value": "continued-from-cdp" }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_074, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker xhr continued network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_075,
        "globalThis.__lm_worker_xhr_result",
        &json!("worker-xhr:continued-from-cdp:payload"),
        "worker xhr continueRequest result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_xhr_request_stage_abort_cleans_pending_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker xhr abort</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
const xhr = new XMLHttpRequest();
xhr.open('GET', '/worker-api', true);
xhr.addEventListener('abort', () => postMessage('aborted'));
xhr.addEventListener('error', () => postMessage(`error:${xhr.readyState}:${xhr.status}`));
onmessage = event => {
  if (event.data === 'abort') {
    xhr.abort();
  }
};
xhr.send();
"#,
        )
    }

    async fn api(hits: axum::extract::State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "unexpected-worker-xhr-body",
        )
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_hits = hits.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_080,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_080, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_081,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_081, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_082).await;

    ctx.process_async(json!({
        "id": 37_083,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_xhr_abort_result = "pending";
  globalThis.__lm_worker = new Worker('/worker.js');
  globalThis.__lm_worker.onmessage = event => {
    globalThis.__lm_worker_xhr_abort_result = event.data;
  };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_083);

    let paused = wait_for_request_paused(&mut ctx, &api_url, "worker xhr abort pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker xhr request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker xhr network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_084,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__lm_worker.postMessage('abort')"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_084);

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker xhr abort network failure",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(network_id)
                    && message["params"]["errorText"] == json!("net::ERR_ABORTED")
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_085,
        "globalThis.__lm_worker_xhr_abort_result",
        &json!("aborted"),
        "worker xhr abort result",
    )
    .await;

    ctx.process_async(json!({
        "id": 37_086,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    let late_continue = take_response_by_id(&mut ctx, 37_086);
    assert_eq!(late_continue["error"]["message"], "RequestNotFound");
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_request_stage_fulfill_request_resolves_worker_promise() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch fulfill</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(String(error)));
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "real-worker-body")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_100,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_100, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_101,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_101, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_102).await;

    ctx.process_async(json!({
        "id": 37_103,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_103);

    let paused = wait_for_request_paused(&mut ctx, &api_url, "worker fetch fulfill pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_104,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "Content-Type", "value": "text/plain" }
            ],
            "body": "d29ya2VyLXN5bnRoZXRpYw=="
        }
    }))
    .await;
    ctx.expect_result(37_104, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch fulfill network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_105,
        "globalThis.__lm_worker_fetch_result",
        &json!("worker-synthetic"),
        "worker fetch fulfillRequest result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_request_stage_fail_request_rejects_worker_promise() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch fail</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api')
  .then(response => response.text())
  .then(text => postMessage(`resolved:${text}`))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "unexpected-body")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_200,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_200, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_201,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_201, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_202).await;

    ctx.process_async(json!({
        "id": 37_203,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_203);

    let paused = wait_for_request_paused(&mut ctx, &api_url, "worker fetch fail pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_204,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "errorReason": "Aborted"
        }
    }))
    .await;
    ctx.expect_result(37_204, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch fail network failure",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(network_id)
                    && message["params"]["errorText"] == json!("Aborted")
            })
        },
    )
    .await;

    let result = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_205,
        "globalThis.__lm_worker_fetch_result",
        &json!("rejected:TypeError: Aborted"),
        "worker fetch failRequest result",
    )
    .await;
    assert!(
        result["result"]["result"]["value"]
            .as_str()
            .expect("worker fetch result")
            .contains("Aborted")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_request_stage_abort_signal_cleans_pending_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch abort</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
const controller = new AbortController();
fetch('/worker-api', { signal: controller.signal })
  .then(response => response.text())
  .then(text => postMessage(`resolved:${text}`))
  .catch(error => postMessage(`rejected:${error.name}:${error.message}`));
onmessage = event => {
  if (event.data === 'abort') {
    controller.abort();
  }
};
"#,
        )
    }

    async fn api(hits: axum::extract::State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        ([(CONTENT_TYPE.as_str(), "text/plain")], "unexpected-body")
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_hits = hits.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api))
                .with_state(server_hits),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_300,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_300, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_301,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_301, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_302).await;

    ctx.process_async(json!({
        "id": 37_303,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  globalThis.__lm_worker = new Worker('/worker.js');
  globalThis.__lm_worker.onmessage = event => {
    globalThis.__lm_worker_fetch_result = event.data;
  };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_303);

    let paused = wait_for_request_paused(&mut ctx, &api_url, "worker fetch abort pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_304,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__lm_worker.postMessage('abort')"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_304);

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch abort network failure",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(network_id)
                    && message["params"]["errorText"] == json!("net::ERR_ABORTED")
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_305,
        "globalThis.__lm_worker_fetch_result",
        &json!("rejected:AbortError:The operation was aborted."),
        "worker fetch AbortSignal rejection",
    )
    .await;

    ctx.process_async(json!({
        "id": 37_370,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    let late_continue = take_response_by_id(&mut ctx, 37_370);
    assert_eq!(late_continue["error"]["message"], "RequestNotFound");
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_auth_required_then_continue_with_auth_resolves() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch auth</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-auth')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn protected(headers: HeaderMap) -> axum::response::Response {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        if headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == expected)
        {
            (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "worker-authenticated",
            )
                .into_response()
        } else {
            (
                StatusCode::UNAUTHORIZED,
                [
                    (CONTENT_TYPE.as_str(), "text/plain"),
                    (
                        WWW_AUTHENTICATE.as_str(),
                        "Bearer realm=\"token-area\", Basic realm=\"worker, area\"",
                    ),
                ],
                "auth required",
            )
                .into_response()
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-auth", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-auth");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_700,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_700, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_701,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(37_701, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_702).await;

    ctx.process_async(json!({
        "id": 37_703,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_auth_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_auth_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_703);

    let paused = wait_for_request_paused(&mut ctx, &api_url, "worker fetch auth pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_704,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_704, json!({}), Some("SID-1"));

    let auth_required =
        wait_for_auth_required(&mut ctx, &request_id, "worker fetch authRequired").await;
    assert_eq!(auth_required["params"]["requestId"], request_id);
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Server");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "basic");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "worker, area"
    );

    ctx.process_async(json!({
        "id": 37_705,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
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
    ctx.expect_result(37_705, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch authenticated network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_706,
        "globalThis.__lm_worker_fetch_auth_result",
        &json!("worker-authenticated"),
        "worker fetch continueWithAuth result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_auth_cancel_pauses_configured_challenged_response_stage() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch auth cancel</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-auth')
  .then(async response => postMessage(
    `resolved:${response.ok}:${response.status}:${await response.text()}`
  ))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn protected() -> impl IntoResponse {
        (
            StatusCode::UNAUTHORIZED,
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                (WWW_AUTHENTICATE.as_str(), "Basic realm=\"worker-area\""),
            ],
            "auth required",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-auth", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-auth");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_720,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_720, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_721,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true,
            "patterns": [
                {
                    "urlPattern": api_url,
                    "requestStage": "Request",
                    "resourceType": "Fetch"
                },
                {
                    "urlPattern": api_url,
                    "requestStage": "Response",
                    "resourceType": "Fetch"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_721, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_722).await;

    ctx.process_async(json!({
        "id": 37_723,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_auth_cancel_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_auth_cancel_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_723);

    let paused =
        wait_for_request_paused(&mut ctx, &api_url, "worker fetch auth cancel pause").await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_724,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_724, json!({}), Some("SID-1"));

    let auth_required = wait_for_auth_required(
        &mut ctx,
        &request_id,
        "worker fetch auth cancel authRequired",
    )
    .await;
    assert_eq!(auth_required["params"]["requestId"], request_id);
    assert!(auth_required["params"].get("networkId").is_none());

    ctx.process_async(json!({
        "id": 37_725,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": { "response": "CancelAuth" }
        }
    }))
    .await;
    ctx.expect_result(37_725, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch auth cancel challenged response-stage pause",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Fetch.requestPaused")
                    && message["params"]["requestId"] == json!(request_id)
                    && message["params"]["networkId"] == json!(network_id)
                    && message["params"]["responseStatusCode"] == json!(401)
            })
        },
    )
    .await;
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_726,
        "globalThis.__lm_worker_fetch_auth_cancel_result",
        &json!("pending"),
        "worker promise at the challenged response stage",
    )
    .await;

    ctx.process_async(json!({
        "id": 37_727,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        37_727,
        json!({ "body": BASE64_STANDARD.encode("auth required"), "base64Encoded": true }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 37_728,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_728, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch auth cancel network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;
    let response =
        ctx.take_first_matching("worker fetch auth cancel challenged response", |message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        });
    assert_eq!(response["params"]["response"]["status"], 401);
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        }),
        "CancelAuth must expose the challenged response instead of failing the request"
    );

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_729,
        "globalThis.__lm_worker_fetch_auth_cancel_result",
        &json!("resolved:false:401:auth required"),
        "worker fetch auth cancel result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_auth_then_response_stage_pauses_authenticated_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch auth response stage</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-auth')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn protected(headers: HeaderMap) -> axum::response::Response {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        if headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == expected)
        {
            (
                StatusCode::OK,
                [
                    (CONTENT_TYPE.as_str(), "text/plain"),
                    ("x-worker-auth-stage", "ok"),
                ],
                "worker-auth-response-stage",
            )
                .into_response()
        } else {
            (
                StatusCode::UNAUTHORIZED,
                [
                    (CONTENT_TYPE.as_str(), "text/plain"),
                    (WWW_AUTHENTICATE.as_str(), "Basic realm=\"worker-area\""),
                ],
                "auth required",
            )
                .into_response()
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-auth", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-auth");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_740,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_740, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_741,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(37_741, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_742).await;

    ctx.process_async(json!({
        "id": 37_743,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_auth_response_stage = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_auth_response_stage = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_743);

    let paused = wait_for_request_paused(
        &mut ctx,
        &api_url,
        "worker fetch auth response-stage request pause",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_744,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(37_744, json!({}), Some("SID-1"));

    let auth_required = wait_for_auth_required(
        &mut ctx,
        &request_id,
        "worker fetch auth response-stage authRequired",
    )
    .await;
    assert_eq!(auth_required["params"]["requestId"], request_id);
    assert!(auth_required["params"].get("networkId").is_none());

    ctx.process_async(json!({
        "id": 37_745,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
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
    ctx.expect_result(37_745, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch auth response-stage pause",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Fetch.requestPaused")
                    && message["params"]["requestId"] == json!(request_id)
                    && message["params"]["networkId"] == json!(network_id)
                    && message["params"]["responseStatusCode"] == json!(200)
            })
        },
    )
    .await;
    let response_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .expect("worker fetch auth response-stage pause event");
    assert!(
        response_paused["params"]["responseHeaders"]
            .as_array()
            .expect("response headers")
            .iter()
            .any(|header| header["name"] == "x-worker-auth-stage" && header["value"] == "ok")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_746,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_746, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch auth response-stage network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_747,
        "globalThis.__lm_worker_fetch_auth_response_stage",
        &json!("worker-auth-response-stage"),
        "worker fetch auth response-stage result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_response_stage_pauses_until_continue_response_then_resolves_promise() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch response stage</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-worker-response", "continue"),
            ],
            "worker-response-stage-body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_400,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_400, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_401,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": "*/worker-api", "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_401, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_402).await;

    ctx.process_async(json!({
        "id": 37_403,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_403);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "worker fetch response-stage pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .expect("worker fetch response-stage requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert!(
        paused["params"]["responseHeaders"]
            .as_array()
            .expect("response headers")
            .iter()
            .any(|header| {
                header["name"] == json!("x-worker-response") && header["value"] == json!("continue")
            }),
        "missing response-stage worker header: {paused:?}"
    );

    let still_pending = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_404,
        "globalThis.__lm_worker_fetch_result",
        &json!("pending"),
        "worker fetch response-stage remains paused before continueResponse",
    )
    .await;
    assert_eq!(still_pending["result"]["result"]["value"], "pending");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_405,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_405, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch response-stage network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_406,
        "globalThis.__lm_worker_fetch_result",
        &json!("worker-response-stage-body"),
        "worker fetch response-stage continueResponse result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_response_stage_get_response_body_preserves_binary_bytes() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch binary response stage</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api')
  .then(response => response.arrayBuffer())
  .then(buffer => postMessage(Array.from(new Uint8Array(buffer)).join(',')))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/octet-stream")],
            vec![0x00_u8, 0xff, b'a'],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_410,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": "*/worker-api", "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_410, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_411).await;

    ctx.process_async(json!({
        "id": 37_412,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_binary = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_binary = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_412);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "worker fetch binary response-stage pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .expect("worker fetch binary response-stage requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_413,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        37_413,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 37_414,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_414, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_415,
        "globalThis.__lm_worker_fetch_binary",
        &json!("0,255,97"),
        "worker fetch binary response-stage result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_response_stage_fulfill_response_replaces_worker_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch response fulfill</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api')
  .then(response => response.text())
  .then(text => postMessage(text))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "real-response")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_500,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_500, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_501,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": "*/worker-api", "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_501, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_502).await;

    ctx.process_async(json!({
        "id": 37_503,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_503);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "worker fetch response-stage fulfill pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .expect("worker fetch response-stage requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_504,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 202,
            "responseHeaders": [
                { "name": "Content-Type", "value": "text/plain" },
                { "name": "x-worker-response", "value": "synthetic" }
            ],
            "body": "d29ya2VyLXJlc3BvbnNlLXN5bnRoZXRpYw=="
        }
    }))
    .await;
    ctx.expect_result(37_504, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch response-stage fulfill network completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            })
        },
    )
    .await;

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_505,
        "globalThis.__lm_worker_fetch_result",
        &json!("worker-response-synthetic"),
        "worker fetch response-stage fulfillRequest result",
    )
    .await;

    ctx.process_async(json!({
        "id": 37_506,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": network_id }
    }))
    .await;
    ctx.expect_result(
        37_506,
        json!({
            "body": "worker-response-synthetic",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_response_stage_fail_response_rejects_worker_promise() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>worker fetch response fail</body></html>",
        )
    }

    async fn worker() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/javascript")],
            r#"
fetch('/worker-api')
  .then(response => response.text())
  .then(text => postMessage(`resolved:${text}`))
  .catch(error => postMessage(`rejected:${String(error)}`));
"#,
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "real-response")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker))
                .route("/worker-api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/worker-api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_600,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(37_600, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37_601,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": "*/worker-api", "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_601, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 37_602).await;

    ctx.process_async(json!({
        "id": 37_603,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_worker_fetch_result = "pending";
  const worker = new Worker('/worker.js');
  worker.onmessage = event => { globalThis.__lm_worker_fetch_result = event.data; };
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 37_603);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "worker fetch response-stage fail pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .expect("worker fetch response-stage requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("worker fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("worker fetch network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_604,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "errorReason": "Aborted"
        }
    }))
    .await;
    ctx.expect_result(37_604, json!({}), Some("SID-1"));

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "worker fetch response-stage fail network failure",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(network_id)
                    && message["params"]["errorText"] == json!("Aborted")
            })
        },
    )
    .await;

    let result = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_605,
        "globalThis.__lm_worker_fetch_result",
        &json!("rejected:TypeError: Aborted"),
        "worker fetch response-stage failRequest result",
    )
    .await;
    assert!(
        result["result"]["result"]["value"]
            .as_str()
            .expect("worker fetch result")
            .contains("Aborted")
    );

    server.abort();
}
