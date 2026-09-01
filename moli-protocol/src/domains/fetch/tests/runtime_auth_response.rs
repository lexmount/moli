use super::*;
use crate::devtools_runtime::{
    DevToolsAddNetworkInterceptCommand, DevToolsAuthChallengeAction, DevToolsAuthCredentials,
    DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsContinueInterceptedResponseCommand, DevToolsContinueWithAuthCommand,
    DevToolsNetworkInterceptId, DevToolsNetworkInterceptPattern, DevToolsNetworkInterceptPhase,
    DevToolsProtocol, DevToolsRequestId, DevToolsSessionId, DevToolsTargetId,
};
use crate::testing::drain_scheduler_events_like_scheduler;

fn is_fetch_event_for_request(message: &serde_json::Value, method: &str, request_id: &str) -> bool {
    message["method"] == json!(method)
        && message["params"]["requestId"].as_str() == Some(request_id)
}

fn take_fetch_event_for_request(
    ctx: &mut TestContext,
    method: &str,
    request_id: &str,
) -> serde_json::Value {
    ctx.take_first_matching(method, |message| {
        is_fetch_event_for_request(message, method, request_id)
    })
}

fn network_events_for_request<'a>(
    ctx: &'a TestContext,
    method: &str,
    request_id: &str,
) -> Vec<&'a serde_json::Value> {
    ctx.sent
        .iter()
        .filter(|message| {
            message["method"] == json!(method)
                && message["params"]["requestId"].as_str() == Some(request_id)
        })
        .collect()
}

fn assert_chromium_successful_http_extra_info(
    ctx: &TestContext,
    request_id: &str,
    expected_host: &str,
) {
    let requests = network_events_for_request(ctx, "Network.requestWillBeSent", request_id);
    assert_eq!(requests.len(), 1, "one browser-visible request");

    let request_extra =
        network_events_for_request(ctx, "Network.requestWillBeSentExtraInfo", request_id);
    assert_eq!(request_extra.len(), 1, "one raw request header block");
    let request_headers = request_extra[0]["params"]["headers"]
        .as_object()
        .expect("raw request headers object");
    assert_eq!(
        request_headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("host"))
            .and_then(|(_, value)| value.as_str()),
        Some(expected_host)
    );
    assert!(
        !request_headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization")),
        "Chromium exposes the initial unauthenticated request, not an auth retry"
    );

    let response_extra =
        network_events_for_request(ctx, "Network.responseReceivedExtraInfo", request_id);
    assert_eq!(response_extra.len(), 1, "one raw response header block");
    assert_eq!(response_extra[0]["params"]["statusCode"], 200);

    let responses = network_events_for_request(ctx, "Network.responseReceived", request_id);
    assert_eq!(responses.len(), 1, "one browser-visible response");
    assert_eq!(responses[0]["params"]["response"]["status"], 200);
    assert_eq!(responses[0]["params"]["hasExtraInfo"], true);

    let finished = network_events_for_request(ctx, "Network.loadingFinished", request_id);
    assert_eq!(finished.len(), 1, "one successful network terminal");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_intercept_response_pauses_after_response_until_continue_response()
 {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-fetch-stage", "ok"),
            ],
            "fetch-response-stage",
        )
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
    enable_runtime_async(&mut ctx, "SID-1", 405).await;

    ctx.process_async(json!({
        "id": 384,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_stage = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_stage = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 384);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], api_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 385,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(385, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;

    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    assert_eq!(response_paused["params"]["requestId"], request_id);
    assert_eq!(response_paused["params"]["networkId"], network_id);
    assert_eq!(response_paused["params"]["resourceType"], "XHR");
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);
    assert_eq!(
        response_paused["params"]["responseHeaders"][0]["name"],
        "content-type"
    );
    assert!(ctx.sent.is_empty());

    ctx.process_async(json!({
        "id": 386,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        386,
        json!({
            "body": "fetch-response-stage",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 387,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(387, json!({}), Some("SID-1"));

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
    assert_eq!(request["params"]["type"], "Fetch");
    assert_chromium_successful_http_extra_info(&ctx, &network_request_id, &addr.to_string());
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
        response["params"]["response"]["headers"]["x-fetch-stage"],
        "ok"
    );
    assert_eq!(response["params"]["hasExtraInfo"], true);

    ctx.process_async(json!({
        "id": 388,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_stage" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 388);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "fetch-response-stage"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_response_stage_preserves_binary_body_for_cdp_body_reads() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
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
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_010,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_010, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 36_011).await;

    ctx.process_async(json!({
        "id": 36_012,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_binary_stage = "pending";
  fetch('/api')
    .then(response => response.arrayBuffer())
    .then(buffer => {
      globalThis.__lm_fetch_binary_stage = Array.from(new Uint8Array(buffer)).join(",");
    });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 36_012);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_013,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(36_013, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "binary subresource response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;
    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);

    ctx.process_async(json!({
        "id": 36_014,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        36_014,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 36_015,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        36_015,
        json!({ "stream": "BID-1:TID-1:STREAM-2" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 36_016,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-2" }
    }))
    .await;
    ctx.expect_result(
        36_016,
        json!({ "base64Encoded": true, "data": "AP9h", "eof": true }),
        None,
    );

    ctx.process_async(json!({
        "id": 36_017,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "application/octet-stream" }
            ],
            "body": "AP9h"
        }
    }))
    .await;
    ctx.expect_result(36_017, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        36_018,
        "globalThis.__lm_fetch_binary_stage",
        &json!("0,255,97"),
        "binary fetch response-stage result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_child_frame_fetch_subresource_interception_uses_child_frame_attribution() {
    async fn top() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>child</main></body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-subresource", "child"),
            ],
            "child-fetch",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/top", get(top))
                .route("/child", get(child))
                .route("/api", any(api)),
        )
        .await
        .unwrap();
    });

    let top_url = format!("http://{addr}/top");
    let child_url = format!("http://{addr}/child");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &top_url, "SID-1", "TID-1").await;
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 36_400).await;

    ctx.process_async(json!({
        "id": 36_401,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_401, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 36_402,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_402, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 36_403).await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame navigated before child fetch",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 36_404,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_child_fetch_result = "pending";
  const child = document.querySelector("iframe").contentWindow;
  child.fetch("/api", { method: "POST", body: "child-payload" })
    .then(response => response.text())
    .then(text => { globalThis.__lm_child_fetch_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;

    let evaluate = take_response_by_id(&mut ctx, 36_404);
    assert_eq!(evaluate["id"], 36_404);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame fetch requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["frameId"] == json!(child_frame_id)
                && message["params"]["request"]["url"] == json!(api_url)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["frameId"] == json!(child_frame_id)
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("child frame fetch requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("fetch network id")
        .to_owned();
    assert_eq!(paused["params"]["frameId"], json!(child_frame_id));
    assert_eq!(paused["params"]["request"]["url"], json!(api_url));
    assert_eq!(paused["params"]["request"]["method"], json!("POST"));
    assert_eq!(
        paused["params"]["request"]["postData"],
        json!("child-payload")
    );
    network_request_announced_before_fetch_pause(&ctx, &paused, Some("Fetch"));

    ctx.process_async(json!({
        "id": 36_405,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(36_405, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame fetch network response event",
        |message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("network request event");
    assert_eq!(request["params"]["frameId"], json!(child_frame_id));
    assert_eq!(request["params"]["documentURL"], json!(child_url));
    assert_eq!(request["params"]["request"]["url"], json!(api_url));
    assert_eq!(request["params"]["request"]["method"], json!("POST"));

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["frameId"], json!(child_frame_id));
    assert_eq!(response["params"]["response"]["url"], json!(api_url));

    ctx.process_async(json!({
        "id": 36_406,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_child_fetch_result" }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 36_406);
    assert_eq!(result["result"]["result"]["value"], json!("child-fetch"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_fetch_pattern_pauses_subresource_after_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-fetch-stage", "auto"),
            ],
            "fetch-response-stage-auto",
        )
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
        "id": 389,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(389, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 390,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(390, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 405).await;

    ctx.process_async(json!({
        "id": 391,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_stage_auto = "pending";
  fetch('/api').then(r => r.text()).then(text => { globalThis.__lm_fetch_stage_auto = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 391);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch response-stage requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;

    let pauses = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(pauses.len(), 1);
    let paused = &pauses[0];
    assert_eq!(paused["params"]["resourceType"], "XHR");
    assert_eq!(paused["params"]["request"]["url"], api_url);
    assert_eq!(paused["params"]["responseStatusCode"], 200);
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 392,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(392, json!({}), Some("SID-1"));

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        36_700,
        "globalThis.__lm_fetch_stage_auto",
        &json!("fetch-response-stage-auto"),
        "response-stage fetch pattern result",
    )
    .await;
    assert_eq!(
        resolved["result"]["result"]["value"],
        "fetch-response-stage-auto"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_intercept_response_pauses_after_response_until_continue_response()
{
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-xhr-stage", "ok")],
            "xhr-response-stage",
        )
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
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 389,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(389, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 390,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(390, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 406).await;

    ctx.process_async(json!({
        "id": 391,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_stage = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_stage = xhr.responseText; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 391);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    assert_eq!(request_paused["params"]["resourceType"], "XHR");
    assert_eq!(request_paused["params"]["request"]["url"], xhr_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 392,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(392, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;

    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    assert_eq!(response_paused["params"]["requestId"], request_id);
    assert_eq!(response_paused["params"]["resourceType"], "XHR");
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);
    assert!(ctx.sent.is_empty());

    ctx.process_async(json!({
        "id": 393,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        393,
        json!({
            "body": "xhr-response-stage",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 394,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(394, json!({}), Some("SID-1"));

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
    assert_eq!(request["params"]["type"], "XHR");
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
        response["params"]["response"]["headers"]["x-xhr-stage"],
        "ok"
    );

    ctx.process_async(json!({
        "id": 395,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_stage" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 395);
    assert_eq!(resolved["result"]["result"]["value"], "xhr-response-stage");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_redirect_preserves_request_id_under_fetch_interception() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api_start() -> impl IntoResponse {
        axum::response::Redirect::temporary("/api-final")
    }

    async fn api_final() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-fetch-final", "ok"),
            ],
            "fetch redirected under interception",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api-start", get(api_start))
                .route("/api-final", get(api_final)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let start_url = format!("http://{addr}/api-start");
    let final_url = format!("http://{addr}/api-final");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 396,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(396, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 397,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(397, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 407).await;

    ctx.process_async(json!({
        "id": 398,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_redirect = "pending";
  fetch('/api-start')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_redirect = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 398);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 399,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(399, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch redirect response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;

    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    assert_eq!(response_paused["params"]["requestId"], request_id);
    assert_eq!(response_paused["params"]["networkId"], network_id);
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 400,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(400, json!({}), Some("SID-1"));

    let fetch_requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(fetch_requests.len(), 2);
    assert_eq!(fetch_requests[0]["params"]["request"]["url"], start_url);
    assert_eq!(fetch_requests[1]["params"]["request"]["url"], final_url);
    assert_eq!(
        fetch_requests[1]["params"]["redirectResponse"]["url"],
        start_url
    );
    assert_eq!(
        fetch_requests[1]["params"]["redirectResponse"]["status"],
        307
    );
    assert_eq!(
        fetch_requests[1]["params"]["redirectResponse"]["headers"]["location"],
        "/api-final"
    );

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("redirected response event");
    assert_eq!(response["params"]["response"]["url"], final_url);
    assert_eq!(
        response["params"]["response"]["headers"]["x-fetch-final"],
        "ok"
    );

    ctx.process_async(json!({
        "id": 401,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_redirect" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 401);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "fetch redirected under interception"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_redirect_preserves_request_id_under_fetch_interception() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr_start() -> impl IntoResponse {
        axum::response::Redirect::temporary("/xhr-final")
    }

    async fn xhr_final() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-xhr-final", "ok")],
            "xhr redirected under interception",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr-start", get(xhr_start))
                .route("/xhr-final", get(xhr_final)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let start_url = format!("http://{addr}/xhr-start");
    let final_url = format!("http://{addr}/xhr-final");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 402,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(402, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 403,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(403, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 408).await;

    ctx.process_async(json!({
        "id": 404,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_redirect = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/xhr-start');
  xhr.onload = () => { globalThis.__lm_xhr_redirect = xhr.responseText; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 404);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 405,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(405, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr redirect response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;

    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    assert_eq!(response_paused["params"]["requestId"], request_id);
    assert_eq!(response_paused["params"]["networkId"], network_id);
    assert_eq!(response_paused["params"]["resourceType"], "XHR");
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 406,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(406, json!({}), Some("SID-1"));

    let xhr_requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(xhr_requests.len(), 2);
    assert_eq!(xhr_requests[0]["params"]["request"]["url"], start_url);
    assert_eq!(xhr_requests[1]["params"]["request"]["url"], final_url);
    assert_eq!(
        xhr_requests[1]["params"]["redirectResponse"]["url"],
        start_url
    );
    assert_eq!(xhr_requests[1]["params"]["redirectResponse"]["status"], 307);
    assert_eq!(
        xhr_requests[1]["params"]["redirectResponse"]["headers"]["location"],
        "/xhr-final"
    );

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("redirected response event");
    assert_eq!(response["params"]["response"]["url"], final_url);
    assert_eq!(
        response["params"]["response"]["headers"]["x-xhr-final"],
        "ok"
    );

    ctx.process_async(json!({
        "id": 407,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_redirect" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 407);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "xhr redirected under interception"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_auth_required_then_continue_with_auth_resolves() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "secret fetch",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"test-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 435,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(435, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 436,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(436, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 437).await;

    ctx.process_async(json!({
        "id": 438,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_auth_result = "pending";
  fetch('/protected')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_auth_result = text; })
    .catch(err => { globalThis.__lm_fetch_auth_result = String(err); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 438);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    network_request_announced_before_fetch_pause(&ctx, &request_paused, Some("Fetch"));

    ctx.process_async(json!({
        "id": 439,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(439, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], request_id);
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "basic");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "test-area"
    );

    ctx.process_async(json!({
        "id": 440,
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
    ctx.expect_result(440, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "authenticated subresource network completion",
    )
    .await;

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
    assert_eq!(network_request_id, network_id);
    assert_chromium_successful_http_extra_info(&ctx, &network_request_id, &addr.to_string());
    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 200);
    assert_eq!(response["params"]["hasExtraInfo"], true);

    ctx.process_async(json!({
        "id": 441,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_auth_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 441);
    assert_eq!(resolved["result"]["result"]["value"], "secret fetch");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn multiple_fetch_sessions_chain_subresource_auth_required_pauses() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "secret fetch",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"chain-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );

    for (id, session_id) in [(35_970, "SID-1"), (35_971, "SID-aux")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Fetch.enable",
            "sessionId": session_id,
            "params": { "handleAuthRequests": true }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    enable_runtime_async(&mut ctx, "SID-1", 35_972).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_973,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_auth_chain_result = "pending";
  fetch('/protected')
    .then(response => response.text())
    .then(text => { globalThis.__lm_auth_chain_result = text; })
    .catch(err => { globalThis.__lm_auth_chain_result = String(err); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_973);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "primary request-stage pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["request"]["url"] == json!(protected_url)
        },
    )
    .await;
    let first_request_pause = ctx.take_first_matching("primary request-stage pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["request"]["url"] == json!(protected_url)
    });
    let first_request_id = first_request_pause["params"]["requestId"]
        .as_str()
        .expect("first request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_974,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_result(35_974, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-aux",
        "auxiliary request-stage pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-aux")
                && message["params"]["request"]["url"] == json!(protected_url)
        },
    )
    .await;
    let second_request_pause =
        ctx.take_first_matching("auxiliary request-stage pause", |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-aux")
                && message["params"]["request"]["url"] == json!(protected_url)
        });
    let second_request_id = second_request_pause["params"]["requestId"]
        .as_str()
        .expect("second request id")
        .to_owned();
    assert_ne!(second_request_id, first_request_id);

    ctx.process_async(json!({
        "id": 35_975,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-aux",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(35_975, json!({}), Some("SID-aux"));

    wait_until_message(&mut ctx, "SID-1", "primary authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["request"]["url"] == json!(protected_url)
    })
    .await;
    let first_auth = ctx.take_first_matching("primary authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["request"]["url"] == json!(protected_url)
    });
    let first_auth_request_id = first_auth["params"]["requestId"]
        .as_str()
        .expect("first auth request id")
        .to_owned();
    assert_eq!(first_auth["params"]["authChallenge"]["realm"], "chain-area");

    ctx.process_async(json!({
        "id": 35_976,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": first_auth_request_id,
            "authChallengeResponse": { "response": "Default" }
        }
    }))
    .await;
    ctx.expect_result(35_976, json!({}), Some("SID-1"));

    let second_auth = ctx.take_first_matching("auxiliary authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-aux")
            && message["params"]["request"]["url"] == json!(protected_url)
    });
    let second_auth_request_id = second_auth["params"]["requestId"]
        .as_str()
        .expect("second auth request id")
        .to_owned();
    assert_ne!(second_auth_request_id, second_request_id);
    assert_eq!(
        second_auth["params"]["authChallenge"]["realm"],
        "chain-area"
    );

    ctx.process_async(json!({
        "id": 35_977,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-aux",
        "params": {
            "requestId": second_auth_request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "user",
                "password": "pass"
            }
        }
    }))
    .await;
    ctx.expect_result(35_977, json!({}), Some("SID-aux"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_978,
        "globalThis.__lm_auth_chain_result",
        &json!("secret fetch"),
        "auth chain fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_then_bidi_network_auth_required_terminal_credentials_complete() {
    run_cdp_fetch_then_bidi_network_auth_required_terminal_credentials_complete(
        MixedAuthTerminalCredentialsCommand::ContinueWithAuth,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_then_bidi_network_auth_required_continue_response_credentials_complete() {
    run_cdp_fetch_then_bidi_network_auth_required_terminal_credentials_complete(
        MixedAuthTerminalCredentialsCommand::ContinueResponseAuthCredentials,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_fetch_then_bidi_network_auth_required_terminal_cancel_exposes_401() {
    run_cdp_fetch_then_bidi_network_auth_required_terminal_credentials_complete(
        MixedAuthTerminalCredentialsCommand::ContinueWithAuthCancel,
    )
    .await;
}

#[derive(Clone, Copy, Debug)]
enum MixedAuthTerminalCredentialsCommand {
    ContinueWithAuth,
    ContinueResponseAuthCredentials,
    ContinueWithAuthCancel,
}

async fn run_cdp_fetch_then_bidi_network_auth_required_terminal_credentials_complete(
    terminal_command: MixedAuthTerminalCredentialsCommand,
) {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "secret fetch",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"mixed-auth\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_979,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(35_979, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 35_980,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(35_980, json!({}), Some("SID-1"));

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-auth"),
                phases: vec![DevToolsNetworkInterceptPhase::AuthRequired],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: protected_url.clone(),
                }],
            }),
        )
        .await;
    assert_eq!(
        result.expect("BiDi add auth intercept should succeed"),
        DevToolsCommandResult::AddNetworkIntercept(
            crate::devtools_runtime::DevToolsAddNetworkInterceptResult {
                intercept_id: DevToolsNetworkInterceptId::from("intercept-auth")
            }
        )
    );
    let protected_parsed_url = Url::parse(&protected_url).unwrap();
    let auth_pause_sessions = ctx
        .conn
        .target_fetch_subresource_interception_snapshot_for_session_owner(Some("SID-1"))
        .expect("active target fetch snapshot")
        .matching_auth_required_pause_sessions(Some("SID-1"), &protected_parsed_url);
    assert_eq!(
        auth_pause_sessions
            .iter()
            .map(|session| session.session_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("SID-1"), Some("BIDI-SID")]
    );
    assert_eq!(
        auth_pause_sessions[1]
            .blocked_intercepts
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-auth"]
    );
    enable_runtime_async(&mut ctx, "SID-1", 35_981).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_982,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_mixed_auth_result = "pending";
  fetch('/protected')
    .then(async response => `${response.status}:${await response.text()}`)
    .then(result => { globalThis.__lm_mixed_auth_result = result; })
    .catch(err => { globalThis.__lm_mixed_auth_result = String(err); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_982);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "CDP Fetch request-stage pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["request"]["url"] == json!(protected_url)
        },
    )
    .await;
    let request_pause = ctx.take_first_matching("CDP Fetch request-stage pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["request"]["url"] == json!(protected_url)
    });
    let request_id = request_pause["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_pause["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_983,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(35_983, json!({}), Some("SID-1"));

    wait_until_message(&mut ctx, "SID-1", "CDP authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["request"]["url"] == json!(protected_url)
    })
    .await;
    let cdp_auth = ctx.take_first_matching("CDP authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["request"]["url"] == json!(protected_url)
    });
    assert_eq!(cdp_auth["params"]["authChallenge"]["realm"], "mixed-auth");
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.authRequired")
                && message["sessionId"] == json!("BIDI-SID")
        }),
        "BiDi-owned auth pause should wait for the CDP Fetch Default action"
    );
    let cdp_auth_request_id = cdp_auth["params"]["requestId"]
        .as_str()
        .expect("CDP auth request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_984,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": cdp_auth_request_id,
            "authChallengeResponse": { "response": "Default" }
        }
    }))
    .await;
    ctx.expect_result(35_984, json!({}), Some("SID-1"));

    wait_until_message(&mut ctx, "BIDI-SID", "BiDi authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("BIDI-SID")
            && message["params"]["request"]["url"] == json!(protected_url)
    })
    .await;
    let bidi_auth = ctx.take_first_matching("BiDi authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("BIDI-SID")
            && message["params"]["request"]["url"] == json!(protected_url)
    });
    assert_eq!(bidi_auth["params"]["authChallenge"]["realm"], "mixed-auth");
    let bidi_auth_request_id = bidi_auth["params"]["requestId"]
        .as_str()
        .expect("BiDi auth request id")
        .to_owned();
    ctx.sent.clear();

    let (
        continue_result,
        continue_scheduler_events,
        continue_protocol_events,
        continue_renderer_output_predecessor,
    ) = match terminal_command {
        MixedAuthTerminalCredentialsCommand::ContinueWithAuth => ctx
            .conn
            .execute_devtools_command(DevToolsCommand::ContinueWithAuth(
                DevToolsContinueWithAuthCommand {
                    context: DevToolsCommandContext {
                        protocol: DevToolsProtocol::WebDriverBidi,
                        session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                        target_id: Some(DevToolsTargetId::from("TID-1")),
                        browser_context_id: None,
                    },
                    request_id: DevToolsRequestId::from(bidi_auth_request_id.as_str()),
                    action: DevToolsAuthChallengeAction::ProvideCredentials,
                    username: Some("user".to_owned()),
                    password: Some("pass".to_owned()),
                },
            ))
            .await
            .into_complete_parts(),
        MixedAuthTerminalCredentialsCommand::ContinueResponseAuthCredentials => ctx
            .conn
            .execute_devtools_command(DevToolsCommand::ContinueInterceptedResponse(
                DevToolsContinueInterceptedResponseCommand {
                    context: DevToolsCommandContext {
                        protocol: DevToolsProtocol::WebDriverBidi,
                        session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                        target_id: Some(DevToolsTargetId::from("TID-1")),
                        browser_context_id: None,
                    },
                    request_id: DevToolsRequestId::from(bidi_auth_request_id.as_str()),
                    response_code: None,
                    response_headers: None,
                    response_phrase: None,
                    auth_credentials: Some(DevToolsAuthCredentials {
                        username: "user".to_owned(),
                        password: "pass".to_owned(),
                    }),
                },
            ))
            .await
            .into_complete_parts(),
        MixedAuthTerminalCredentialsCommand::ContinueWithAuthCancel => ctx
            .conn
            .execute_devtools_command(DevToolsCommand::ContinueWithAuth(
                DevToolsContinueWithAuthCommand {
                    context: DevToolsCommandContext {
                        protocol: DevToolsProtocol::WebDriverBidi,
                        session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                        target_id: Some(DevToolsTargetId::from("TID-1")),
                        browser_context_id: None,
                    },
                    request_id: DevToolsRequestId::from(bidi_auth_request_id.as_str()),
                    action: DevToolsAuthChallengeAction::Cancel,
                    username: None,
                    password: None,
                },
            ))
            .await
            .into_complete_parts(),
    };
    if let Some(predecessor) = continue_renderer_output_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    assert_eq!(
        continue_result.expect("BiDi auth terminal action should succeed"),
        DevToolsCommandResult::Empty
    );
    let mut continue_output = continue_protocol_events
        .into_iter()
        .map(|event| event.into_protocol_message())
        .collect::<Vec<_>>();
    drain_scheduler_events_like_scheduler(
        &mut ctx.conn,
        &mut continue_output,
        continue_scheduler_events,
    )
    .await;
    ctx.sent.extend(continue_output);

    let (expected_result, description) = match terminal_command {
        MixedAuthTerminalCredentialsCommand::ContinueWithAuth
        | MixedAuthTerminalCredentialsCommand::ContinueResponseAuthCredentials => (
            json!("200:secret fetch"),
            "mixed CDP Fetch / BiDi auth result",
        ),
        MixedAuthTerminalCredentialsCommand::ContinueWithAuthCancel => (
            json!("401:auth required"),
            "mixed CDP Fetch / BiDi auth cancel result",
        ),
    };
    if matches!(
        terminal_command,
        MixedAuthTerminalCredentialsCommand::ContinueWithAuthCancel
    ) {
        wait_until_message(
            &mut ctx,
            "SID-1",
            "mixed auth cancel network completion",
            |message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(network_id)
            },
        )
        .await;
        let response = ctx.take_first_matching("mixed auth canceled response", |message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        });
        assert_eq!(response["params"]["response"]["status"], 401);
        assert!(
            !ctx.sent.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(network_id)
            }),
            "BiDi cancel must expose the challenged response"
        );
    }

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_985,
        "globalThis.__lm_mixed_auth_result",
        &expected_result,
        description,
    )
    .await;
    assert_eq!(resolved["result"]["result"]["value"], expected_result);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_auth_required_includes_synthesized_cookie_header() {
    async fn page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("set-cookie", "sid=fetch-auth; Path=/protected"),
            ],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(_headers: HeaderMap) -> impl IntoResponse {
        (
            StatusCode::UNAUTHORIZED,
            [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"test-area\"")],
            "auth required",
        )
            .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 440_001,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(440_001, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 440_002,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(440_002, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 440_003).await;

    ctx.process_async(json!({
        "id": 440_004,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  fetch('/protected').catch(() => {});
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 440_004);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 440_005,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(440_005, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch authRequired event with cookie",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(
        auth_required["params"]["request"]["headers"]["Cookie"],
        "sid=fetch-auth"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_proxy_auth_required_then_continue_with_auth_resolves() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("proxy-authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "proxy secret fetch",
            )
                .into_response(),
            _ => (
                StatusCode::PROXY_AUTHENTICATION_REQUIRED,
                [(PROXY_AUTHENTICATE.as_str(), "Basic realm=\"proxy-area\"")],
                "proxy auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 531,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(531, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 532,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(532, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 533).await;

    ctx.process_async(json!({
        "id": 534,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_proxy_auth_result = "pending";
  fetch('/protected')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_proxy_auth_result = text; })
    .catch(err => { globalThis.__lm_fetch_proxy_auth_result = String(err); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 534);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 535,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(535, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch proxy authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Proxy");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "proxy-area"
    );

    ctx.process_async(json!({
        "id": 536,
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
    ctx.expect_result(536, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "authenticated subresource fetch proxy network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    ctx.process_async(json!({
        "id": 537,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_proxy_auth_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 537);
    assert_eq!(resolved["result"]["result"]["value"], "proxy secret fetch");

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 200);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_proxy_digest_auth_then_continue_with_auth_resolves() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = proxy_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                let request = String::from_utf8_lossy(&buf[..read]);
                let first_line = request.lines().next().unwrap_or_default();
                let has_digest_proxy_auth = request.lines().any(|line| {
                    let (name, value) = match line.split_once(':') {
                        Some(parts) => parts,
                        None => return false,
                    };
                    name.eq_ignore_ascii_case("proxy-authorization")
                        && value.trim_start().starts_with("Digest ")
                });

                let response = if first_line.contains("http://example.test/page") {
                    let body = "<!doctype html><html><body>ready</body></html>";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else if has_digest_proxy_auth {
                    let body = "proxy digest secret fetch";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    let body = "proxy digest auth required";
                    format!(
                        "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Digest realm=\"proxy-digest\", nonce=\"feedface\", qop=\"auth\", algorithm=MD5, opaque=\"opaque\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };

                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    let page_url = "http://example.test/page";
    let protected_url = "http://example.test/protected";
    let mut ctx = TestContext::new();
    ctx.conn
        .set_http_proxy_override_async(Some(format!("http://{proxy_addr}")))
        .await;
    with_loaded_http_document(&mut ctx, page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 541,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(541, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 542,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(542, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 543).await;

    ctx.process_async(json!({
        "id": 544,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_proxy_digest_auth_result = "pending";
  fetch('/protected')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_proxy_digest_auth_result = text; })
    .catch(err => { globalThis.__lm_fetch_proxy_digest_auth_result = String(err); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 544);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 545,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(545, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch proxy digest authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Proxy");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "digest");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "proxy-digest"
    );

    ctx.process_async(json!({
        "id": 546,
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
    ctx.expect_result(546, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "authenticated subresource fetch proxy digest network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    ctx.process_async(json!({
        "id": 547,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_proxy_digest_auth_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 547);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "proxy digest secret fetch"
    );

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 200);

    proxy_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_handles_multi_round_basic_auth() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "secret fetch round-2",
            )
                .into_response(),
            value => {
                let realm = if value.is_some() {
                    "round-2"
                } else {
                    "round-1"
                };
                (
                    StatusCode::UNAUTHORIZED,
                    [(
                        WWW_AUTHENTICATE.as_str(),
                        format!("Basic realm=\"{realm}\""),
                    )],
                    format!("auth required {realm}"),
                )
                    .into_response()
            }
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 695,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(695, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 696,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(696, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 697).await;

    ctx.process_async(json!({
        "id": 698,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_auth_round = "pending";
  fetch('/protected')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_auth_round = text; })
    .catch(err => { globalThis.__lm_fetch_auth_round = String(err); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 698);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    network_request_announced_before_fetch_pause(&ctx, &request_paused, Some("Fetch"));

    ctx.process_async(json!({
        "id": 699,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(699, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch first basic authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["authChallenge"]["realm"], "round-1");

    ctx.process_async(json!({
        "id": 700,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "wrong",
                "password": "creds"
            }
        }
    }))
    .await;
    ctx.expect_result(700, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch second basic authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let second_auth_required =
        take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(second_auth_required["method"], "Fetch.authRequired");
    assert_eq!(
        second_auth_required["params"]["requestId"],
        json!(request_id)
    );
    assert!(second_auth_required["params"].get("networkId").is_none());
    assert_eq!(
        second_auth_required["params"]["authChallenge"]["realm"],
        "round-2"
    );

    ctx.process_async(json!({
        "id": 701,
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
    ctx.expect_result(701, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "authenticated subresource fetch basic network completion",
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("network request event");
    assert_eq!(request["params"]["requestId"], json!(network_id));
    assert_chromium_successful_http_extra_info(&ctx, &network_id, &addr.to_string());

    ctx.process_async(json!({
        "id": 702,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_auth_round" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 702);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "secret fetch round-2"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_handles_multi_round_digest_auth() {
    let digest_attempts = Arc::new(AtomicUsize::new(0));

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(digest_attempts: Arc<AtomicUsize>, headers: HeaderMap) -> impl IntoResponse {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if auth.is_some_and(|value| value.starts_with("Digest ")) {
            let attempt = digest_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt >= 1 {
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE.as_str(), "text/plain")],
                    "secret fetch digest round-2",
                )
                    .into_response();
            }
        }
        let realm = if auth.is_some() {
            "digest-round-2"
        } else {
            "digest-round-1"
        };
        (
                StatusCode::UNAUTHORIZED,
                [(
                    WWW_AUTHENTICATE.as_str(),
                    format!(
                        "Digest realm=\"{realm}\", nonce=\"{}\", qop=\"auth\", algorithm=MD5, opaque=\"opaque\"",
                        if auth.is_some() { "feedface" } else { "deadbeef" }
                    ),
                )],
                format!("auth required {realm}"),
            )
                .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let digest_attempts = digest_attempts.clone();
        axum::serve(
            listener,
            Router::new().route("/page", get(page)).route(
                "/protected",
                any(move |headers| protected(digest_attempts.clone(), headers)),
            ),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 712,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(712, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 713,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(713, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 714).await;

    ctx.process_async(json!({
        "id": 715,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_digest_round = "pending";
  fetch('/protected')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_digest_round = text; })
    .catch(err => { globalThis.__lm_fetch_digest_round = String(err); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 715);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    network_request_announced_before_fetch_pause(&ctx, &request_paused, Some("Fetch"));

    ctx.process_async(json!({
        "id": 716,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(716, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch first digest authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "digest");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "digest-round-1"
    );

    ctx.process_async(json!({
        "id": 717,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "digest-user",
                "password": "digest-pass"
            }
        }
    }))
    .await;
    ctx.expect_result(717, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch second digest authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let second_auth_required =
        take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(second_auth_required["method"], "Fetch.authRequired");
    assert_eq!(
        second_auth_required["params"]["requestId"],
        json!(request_id)
    );
    assert!(second_auth_required["params"].get("networkId").is_none());
    assert_eq!(
        second_auth_required["params"]["authChallenge"]["realm"],
        "digest-round-2"
    );

    ctx.process_async(json!({
        "id": 718,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "digest-user",
                "password": "digest-pass"
            }
        }
    }))
    .await;
    ctx.expect_result(718, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "authenticated subresource fetch digest network completion",
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("network request event");
    assert_eq!(request["params"]["requestId"], json!(network_id));
    assert_chromium_successful_http_extra_info(&ctx, &network_id, &addr.to_string());

    ctx.process_async(json!({
        "id": 719,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_digest_round" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 719);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "secret fetch digest round-2"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_auth_required_default_aborts_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "xhr secret",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"xhr-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 442,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(442, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 443,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(443, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 444).await;

    ctx.process_async(json!({
        "id": 445,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_auth_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onload = () => { globalThis.__lm_xhr_auth_result = 'loaded'; };
  xhr.onerror = () => { globalThis.__lm_xhr_auth_result = 'failed'; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 445);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 446,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(446, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "xhr-area"
    );

    ctx.process_async(json!({
        "id": 447,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": { "response": "Default" }
        }
    }))
    .await;
    ctx.expect_result(447, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(
        failed["params"]["errorText"],
        "Fetch auth challenge aborted"
    );

    ctx.process_async(json!({
        "id": 448,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_auth_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 448);
    assert_eq!(resolved["result"]["result"]["value"], "failed");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_auth_required_then_continue_with_auth_resolves() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "xhr secret",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"xhr-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 720,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(720, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 721,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(721, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 722).await;

    ctx.process_async(json!({
        "id": 723,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_auth_success = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onload = () => { globalThis.__lm_xhr_auth_success = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_auth_success = 'failed'; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 723);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    network_request_announced_before_fetch_pause(&ctx, &request_paused, Some("XHR"));

    ctx.process_async(json!({
        "id": 724,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(724, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "xhr-area"
    );

    ctx.process_async(json!({
        "id": 725,
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
    ctx.expect_result(725, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "authenticated subresource xhr network completion",
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("network request event");
    assert_eq!(request["params"]["requestId"], json!(network_id));

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 200);

    ctx.process_async(json!({
        "id": 726,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_auth_success" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 726);
    assert_eq!(resolved["result"]["result"]["value"], "xhr secret");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_handles_multi_round_digest_auth() {
    let digest_attempts = Arc::new(AtomicUsize::new(0));

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(digest_attempts: Arc<AtomicUsize>, headers: HeaderMap) -> impl IntoResponse {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if auth.is_some_and(|value| value.starts_with("Digest ")) {
            let attempt = digest_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt >= 1 {
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE.as_str(), "text/plain")],
                    "xhr digest round-2",
                )
                    .into_response();
            }
        }
        let realm = if auth.is_some() {
            "xhr-digest-round-2"
        } else {
            "xhr-digest-round-1"
        };
        (
                StatusCode::UNAUTHORIZED,
                [(
                    WWW_AUTHENTICATE.as_str(),
                    format!(
                        "Digest realm=\"{realm}\", nonce=\"{}\", qop=\"auth\", algorithm=MD5, opaque=\"opaque\"",
                        if auth.is_some() { "feedface" } else { "deadbeef" }
                    ),
                )],
                format!("auth required {realm}"),
            )
                .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let digest_attempts = digest_attempts.clone();
        axum::serve(
            listener,
            Router::new().route("/page", get(page)).route(
                "/protected",
                any(move |headers| protected(digest_attempts.clone(), headers)),
            ),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 727,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(727, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 728,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(728, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 729).await;

    ctx.process_async(json!({
        "id": 730,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_digest_round = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onload = () => { globalThis.__lm_xhr_digest_round = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_digest_round = 'failed'; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 730);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    network_request_announced_before_fetch_pause(&ctx, &request_paused, Some("XHR"));

    ctx.process_async(json!({
        "id": 731,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(731, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr digest authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "digest");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "xhr-digest-round-1"
    );

    ctx.process_async(json!({
        "id": 732,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "digest-user",
                "password": "digest-pass"
            }
        }
    }))
    .await;
    ctx.expect_result(732, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr second digest authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let second_auth_required =
        take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(second_auth_required["method"], "Fetch.authRequired");
    assert_eq!(
        second_auth_required["params"]["requestId"],
        json!(request_id)
    );
    assert!(second_auth_required["params"].get("networkId").is_none());
    assert_eq!(
        second_auth_required["params"]["authChallenge"]["realm"],
        "xhr-digest-round-2"
    );

    ctx.process_async(json!({
        "id": 733,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "digest-user",
                "password": "digest-pass"
            }
        }
    }))
    .await;
    ctx.expect_result(733, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "authenticated subresource xhr digest network completion",
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("network request event");
    assert_eq!(request["params"]["requestId"], json!(network_id));

    ctx.process_async(json!({
        "id": 734,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_digest_round" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 734);
    assert_eq!(resolved["result"]["result"]["value"], "xhr digest round-2");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_proxy_auth_required_then_continue_with_auth_resolves() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("proxy-authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "xhr proxy secret",
            )
                .into_response(),
            _ => (
                StatusCode::PROXY_AUTHENTICATION_REQUIRED,
                [(
                    PROXY_AUTHENTICATE.as_str(),
                    "Basic realm=\"xhr-proxy-area\"",
                )],
                "proxy auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 735,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(735, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 736,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(736, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 737).await;

    ctx.process_async(json!({
        "id": 738,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_proxy_auth_success = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onload = () => { globalThis.__lm_xhr_proxy_auth_success = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_proxy_auth_success = 'failed'; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 738);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], protected_url);
    network_request_announced_before_fetch_pause(&ctx, &request_paused, Some("XHR"));

    ctx.process_async(json!({
        "id": 739,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(739, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr proxy authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Proxy");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "xhr-proxy-area"
    );

    ctx.process_async(json!({
        "id": 740,
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
    ctx.expect_result(740, json!({}), Some("SID-1"));
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "authenticated subresource xhr proxy network completion",
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .cloned()
        .expect("network request event");
    assert_eq!(request["params"]["requestId"], json!(network_id));

    ctx.process_async(json!({
        "id": 741,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_proxy_auth_success" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 741);
    assert_eq!(resolved["result"]["result"]["value"], "xhr proxy secret");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_proxy_digest_auth_then_continue_with_auth_resolves() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = proxy_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                let request = String::from_utf8_lossy(&buf[..read]);
                let first_line = request.lines().next().unwrap_or_default();
                let has_digest_proxy_auth = request.lines().any(|line| {
                    let (name, value) = match line.split_once(':') {
                        Some(parts) => parts,
                        None => return false,
                    };
                    name.eq_ignore_ascii_case("proxy-authorization")
                        && value.trim_start().starts_with("Digest ")
                });

                let response = if first_line.contains("http://example.test/page") {
                    let body = "<!doctype html><html><body>ready</body></html>";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else if has_digest_proxy_auth {
                    let body = "xhr proxy digest secret";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    let body = "proxy digest auth required";
                    format!(
                        "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Digest realm=\"xhr-proxy-digest\", nonce=\"feedface\", qop=\"auth\", algorithm=MD5, opaque=\"opaque\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };

                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    let mut ctx = TestContext::new();
    ctx.conn
        .set_http_proxy_override_async(Some(format!("http://{proxy_addr}")))
        .await;
    with_loaded_http_document(&mut ctx, "http://example.test/page", "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 742,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(742, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 743,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(743, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 744).await;

    ctx.process_async(json!({
        "id": 745,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_proxy_digest_auth_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onload = () => { globalThis.__lm_xhr_proxy_digest_auth_result = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_proxy_digest_auth_result = 'failed'; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 745);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    assert_eq!(
        request_paused["params"]["request"]["url"],
        "http://example.test/protected"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 746,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(746, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr proxy digest authRequired event",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Proxy");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "digest");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "xhr-proxy-digest"
    );

    ctx.process_async(json!({
        "id": 747,
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
    ctx.expect_result(747, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "authenticated subresource xhr proxy digest network completion",
        |message| message["method"] == json!("Network.loadingFinished"),
    )
    .await;

    ctx.process_async(json!({
        "id": 748,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_proxy_digest_auth_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 748);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "xhr proxy digest secret"
    );

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 200);

    proxy_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn request_stage_runtime_fetch_request_paused_includes_synthesized_cookie_header() {
    async fn page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("set-cookie", "sid=req-fetch; Path=/api"),
            ],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "cookie fetch")
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
        "id": 744_001,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(744_001, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 744_002,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(744_002, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 744_003).await;

    ctx.process_async(json!({
        "id": 744_004,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  fetch('/api').catch(() => {});
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 744_004);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    assert_eq!(request_paused["params"]["request"]["url"], api_url);
    assert_eq!(
        request_paused["params"]["request"]["headers"]["Cookie"],
        "sid=req-fetch"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_continue_response_can_override_status_and_headers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-fetch-stage", "ok"),
            ],
            "fetch-response-stage",
        )
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
        "id": 411,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(411, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 412,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(412, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 413).await;

    ctx.process_async(json!({
        "id": 414,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_override_status = -1;
  globalThis.__lm_fetch_override_header = "pending";
  globalThis.__lm_fetch_override_text = "pending";
  fetch('/api')
    .then(response => {
      globalThis.__lm_fetch_override_status = response.status;
      globalThis.__lm_fetch_override_header = response.headers.get('x-override') ?? "";
      return response.text();
    })
    .then(text => { globalThis.__lm_fetch_override_text = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 414);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    assert_eq!(request_paused["params"]["request"]["url"], api_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 415,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(415, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch override response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;
    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 416,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 201,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" },
                { "name": "x-override", "value": "yes" }
            ],
            "responsePhrase": "Created"
        }
    }))
    .await;
    ctx.expect_result(416, json!({}), Some("SID-1"));

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
    assert_eq!(response["params"]["response"]["status"], 201);
    assert_eq!(
        response["params"]["response"]["headers"]["x-override"],
        "yes"
    );

    ctx.process_async(json!({
            "id": 417,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "JSON.stringify({ status: globalThis.__lm_fetch_override_status, header: globalThis.__lm_fetch_override_header, text: globalThis.__lm_fetch_override_text })"
            }
        })).await;
    let resolved = take_response_by_id(&mut ctx, 417);
    assert_eq!(
        resolved["result"]["result"]["value"],
        r#"{"status":201,"header":"yes","text":"fetch-response-stage"}"#
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_subresource_take_response_body_as_stream_at_response_stage() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "streamed-fetch-response",
        )
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
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 418,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(418, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 419).await;

    ctx.process_async(json!({
        "id": 420,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_streamed = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_streamed = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 420);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 421,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(421, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource fetch stream response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;
    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 422,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        422,
        json!({ "stream": "BID-1:TID-1:STREAM-2" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 423,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-2", "size": 8 }
    }))
    .await;
    ctx.expect_result(
        423,
        json!({
            "base64Encoded": false,
            "data": "streamed",
            "eof": false
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 424,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-2" }
    }))
    .await;
    ctx.expect_result(
        424,
        json!({
            "base64Encoded": false,
            "data": "-fetch-response",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 425,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        425,
        -32000,
        "Can only get response body on requests captured after headers received.",
    );

    ctx.process_async(json!({
        "id": 426,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        426,
        -32602,
        "Unable to continue request as is after body is taken",
    );

    ctx.process_async(json!({
        "id": 427,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "body": "c3RyZWFtZWQtZmV0Y2gtcmVzcG9uc2U="
        }
    }))
    .await;
    ctx.expect_result(427, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 428,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_streamed" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 428);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "streamed-fetch-response"
    );

    ctx.process_async(json!({
        "id": 429,
        "method": "IO.close",
        "params": { "handle": "BID-1:TID-1:STREAM-2" }
    }))
    .await;
    ctx.expect_result(429, json!({}), None);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_take_response_body_as_stream_at_response_stage() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "streamed-xhr-response",
        )
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

    ctx.process_async(json!({
        "id": 453,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(453, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 454).await;

    ctx.process_async(json!({
        "id": 455,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_streamed = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_streamed = xhr.responseText; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 455);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 456,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(456, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr stream response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;
    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 457,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        457,
        json!({ "stream": "BID-1:TID-1:STREAM-2" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 458,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-2", "size": 8 }
    }))
    .await;
    ctx.expect_result(
        458,
        json!({
            "base64Encoded": false,
            "data": "streamed",
            "eof": false
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 459,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-2" }
    }))
    .await;
    ctx.expect_result(
        459,
        json!({
            "base64Encoded": false,
            "data": "-xhr-response",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 460,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "body": "c3RyZWFtZWQteGhyLXJlc3BvbnNl"
        }
    }))
    .await;
    ctx.expect_result(460, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 461,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_streamed" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 461);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "streamed-xhr-response"
    );

    ctx.process_async(json!({
        "id": 462,
        "method": "IO.close",
        "params": { "handle": "BID-1:TID-1:STREAM-2" }
    }))
    .await;
    ctx.expect_result(462, json!({}), None);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_redirect_subresource_take_response_body_as_stream_at_response_stage() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api_start() -> impl IntoResponse {
        axum::response::Redirect::temporary("/api-final")
    }

    async fn api_final() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "streamed-fetch-redirect-response",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api-start", get(api_start))
                .route("/api-final", get(api_final)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let start_url = format!("http://{addr}/api-start");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 463,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(463, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 464).await;

    ctx.process_async(json!({
        "id": 465,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_redirect_streamed = "pending";
  fetch('/api-start')
    .then(response => response.text())
    .then(text => { globalThis.__lm_fetch_redirect_streamed = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 465);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(start_url)
        })
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 466,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(466, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "redirect fetch stream response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;
    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 467,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        467,
        json!({ "stream": "BID-1:TID-1:STREAM-2" }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 468,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-2", "size": 15 }
    }))
    .await;
    ctx.expect_result(
        468,
        json!({
            "base64Encoded": false,
            "data": "streamed-fetch-",
            "eof": false
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 469,
        "method": "IO.read",
        "params": { "handle": "BID-1:TID-1:STREAM-2" }
    }))
    .await;
    ctx.expect_result(
        469,
        json!({
            "base64Encoded": false,
            "data": "redirect-response",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 470,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "body": "c3RyZWFtZWQtZmV0Y2gtcmVkaXJlY3QtcmVzcG9uc2U="
        }
    }))
    .await;
    ctx.expect_result(470, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 471,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_fetch_redirect_streamed" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 471);
    assert_eq!(
        resolved["result"]["result"]["value"],
        "streamed-fetch-redirect-response"
    );

    ctx.process_async(json!({
        "id": 472,
        "method": "IO.close",
        "params": { "handle": "BID-1:TID-1:STREAM-2" }
    }))
    .await;
    ctx.expect_result(472, json!({}), None);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_xhr_subresource_fail_request_at_response_stage_fires_error_and_loading_failed() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-xhr-stage", "ok")],
            "xhr-response-stage",
        )
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

    ctx.process_async(json!({
        "id": 428,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(428, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 429,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(429, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 430).await;

    ctx.process_async(json!({
        "id": 431,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_response_fail = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_response_fail = 'loaded'; };
  xhr.onerror = () => { globalThis.__lm_xhr_response_fail = 'failed'; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 431);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 432,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(432, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr fail response-stage requestPaused event",
        |message| is_fetch_event_for_request(message, "Fetch.requestPaused", &request_id),
    )
    .await;
    let response_paused =
        take_fetch_event_for_request(&mut ctx, "Fetch.requestPaused", &request_id);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");

    ctx.process_async(json!({
        "id": 433,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "errorReason": "Aborted" }
    }))
    .await;
    ctx.expect_result(433, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFailed"))
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Aborted");

    ctx.process_async(json!({
        "id": 434,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_response_fail" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 434);
    assert_eq!(resolved["result"]["result"]["value"], "failed");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_response_stage_runtime_xhr_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-xhr-subresource", "ok"),
            ],
            "xhr:payload",
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
        "id": 787,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(787, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 788,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(788, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 789).await;

    ctx.process_async(json!({
        "id": 790,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_result = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_result = "failed"; };
  xhr.send('payload');
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 790);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr response-stage requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["responseStatusCode"] == json!(200)
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
        .expect("subresource xhr response-stage requestPaused event");
    let request_id = response_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = response_paused["params"]["networkId"].clone();
    assert_eq!(response_paused["params"]["request"]["url"], xhr_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 791,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(791, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "net::ERR_ABORTED");
    assert_eq!(failed["params"]["canceled"], true);

    ctx.process_async(json!({
        "id": 792,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(792, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 793,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 793);
    assert_eq!(resolved["result"]["result"]["value"], "failed");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_runtime_xhr_request_paused_includes_synthesized_cookie_header() {
    async fn page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("set-cookie", "sid=xhr; Path=/xhr"),
            ],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "xhr-ok")
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
        "id": 7_931,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_931, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_932,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(7_932, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 7_933).await;

    ctx.process_async(json!({
        "id": 7_934,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_cookie_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_cookie_result = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_cookie_result = "failed"; };
  xhr.send('payload');
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 7_934);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr response-stage requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["responseStatusCode"] == json!(200)
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
        .expect("subresource xhr response-stage requestPaused event");
    assert_eq!(response_paused["params"]["request"]["url"], xhr_url);
    assert_eq!(
        response_paused["params"]["request"]["headers"]["Cookie"],
        "sid=xhr"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_aborts_paused_response_stage_runtime_xhr_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "xhr-ok")
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
        .active_page_state_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 894,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(894, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 895,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(895, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 896).await;

    ctx.process_async(json!({
        "id": 897,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_close_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_close_result = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_close_result = "failed"; };
  xhr.send('payload');
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 897);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr response-stage requestPaused event before close",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["responseStatusCode"] == json!(200)
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
        .expect("subresource xhr response-stage requestPaused event");
    let request_id = response_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = response_paused["params"]["networkId"].clone();
    assert_eq!(response_paused["params"]["request"]["url"], xhr_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 898,
        "method": "Page.close",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(898, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Page closed");

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.detached") && message["sessionId"] == json!("SID-1")
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!("TID-1")
    }));

    ctx.process_async(json!({
        "id": 899,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(899, -32001, "Unknown sessionId");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_runtime_xhr_auth_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "xhr secret",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"xhr-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7931,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7931, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7932,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(7932, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 7933).await;

    ctx.process_async(json!({
        "id": 7934,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_stop_auth_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onload = () => { globalThis.__lm_xhr_stop_auth_result = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_stop_auth_result = "failed"; };
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 7934);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(protected_url)
        })
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7935,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(7935, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr authRequired event before stopLoading",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7936,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7936, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "net::ERR_ABORTED");
    assert_eq!(failed["params"]["canceled"], true);

    ctx.process_async(json!({
        "id": 7937,
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
    ctx.expect_error(7937, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 7938,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_xhr_stop_auth_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 7938);
    assert_eq!(resolved["result"]["result"]["value"], "failed");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_aborts_paused_runtime_xhr_auth_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "xhr secret",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"xhr-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_state_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7939,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7939, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7940,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(7940, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 7941).await;

    ctx.process_async(json!({
        "id": 7942,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onerror = () => {};
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 7942);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(protected_url)
        })
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7943,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(7943, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr authRequired event before close",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7944,
        "method": "Page.close",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7944, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Page closed");

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.detached") && message["sessionId"] == json!("SID-1")
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!("TID-1")
    }));

    ctx.process_async(json!({
        "id": 7945,
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
    ctx.expect_error(7945, -32001, "Unknown sessionId");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_aborts_paused_runtime_fetch_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn data() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "fetch-ok")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/data", get(data)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let data_url = format!("http://{addr}/data");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 900,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(900, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 901,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(901, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 902).await;

    ctx.process_async(json!({
        "id": 903,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_fetch_crash_result = "pending";
  fetch('/data')
    .then((response) => response.text())
    .then((text) => { globalThis.__lm_fetch_crash_result = text; })
    .catch(() => { globalThis.__lm_fetch_crash_result = "failed"; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 903);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(data_url)
        })
        .cloned()
        .expect("subresource fetch requestPaused event");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 904,
        "method": "Page.crash",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(904, json!({}), Some("SID-1"));

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
    assert_eq!(failed["params"]["errorText"], "Page crashed");
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetCrashed")
            && message["sessionId"] == json!("SID-1")
    }));

    ctx.process_async(json!({
        "id": 905,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(905, -32000, "RequestNotFound");

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        bc.active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
    assert!(!bc.has_loaded_page());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_aborts_paused_response_stage_runtime_xhr_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-xhr-subresource", "ok"),
            ],
            "xhr:payload",
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
        "id": 907,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(907, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 908,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(908, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 909).await;

    ctx.process_async(json!({
        "id": 910,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_xhr_crash_result = "pending";
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { globalThis.__lm_xhr_crash_result = xhr.responseText; };
  xhr.onerror = () => { globalThis.__lm_xhr_crash_result = "failed"; };
  xhr.send('payload');
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 910);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr response-stage requestPaused event before crash",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["responseStatusCode"] == json!(200)
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
        .expect("subresource xhr response-stage requestPaused event");
    let request_id = response_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = response_paused["params"]["networkId"].clone();
    assert_eq!(response_paused["params"]["request"]["url"], xhr_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 911,
        "method": "Page.crash",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(911, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Page crashed");
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetCrashed")
            && message["sessionId"] == json!("SID-1")
    }));

    ctx.process_async(json!({
        "id": 912,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(912, -32000, "RequestNotFound");

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        bc.active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
    assert!(!bc.has_loaded_page());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_aborts_paused_runtime_xhr_auth_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!("Basic {}", super::encode_basic_auth("user", "pass"));
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "xhr secret",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"xhr-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 913,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(913, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 914,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(914, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 915).await;

    ctx.process_async(json!({
        "id": 916,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onerror = () => {};
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 916);

    let request_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(protected_url)
        })
        .cloned()
        .expect("request-stage pause");
    let request_id = request_paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = request_paused["params"]["networkId"]
        .as_str()
        .expect("network id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 917,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(917, json!({}), Some("SID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "subresource xhr authRequired event before crash",
        |message| is_fetch_event_for_request(message, "Fetch.authRequired", &request_id),
    )
    .await;

    let auth_required = take_fetch_event_for_request(&mut ctx, "Fetch.authRequired", &request_id);
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "XHR");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "xhr-area"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 918,
        "method": "Page.crash",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(918, json!({}), Some("SID-1"));

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Page crashed");
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetCrashed")
            && message["sessionId"] == json!("SID-1")
    }));

    ctx.process_async(json!({
        "id": 919,
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
    ctx.expect_error(919, -32000, "RequestNotFound");

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        bc.active_page_state()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
    assert!(!bc.has_loaded_page());

    server.abort();
}
