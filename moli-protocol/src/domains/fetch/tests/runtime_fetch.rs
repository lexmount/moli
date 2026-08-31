use super::*;
use crate::devtools_runtime::{
    DevToolsAddNetworkInterceptCommand, DevToolsCommand, DevToolsCommandContext,
    DevToolsCommandResult, DevToolsContinueInterceptedRequestCommand,
    DevToolsContinueInterceptedResponseCommand, DevToolsEvaluateScriptCommand,
    DevToolsNetworkInterceptId, DevToolsNetworkInterceptPattern, DevToolsNetworkInterceptPhase,
    DevToolsNetworkResourceType, DevToolsProtocol, DevToolsRequestId, DevToolsResultOwnership,
    DevToolsSessionId, DevToolsTargetId,
};
use crate::testing::{
    drain_scheduler_events_like_scheduler,
    drain_scheduler_events_like_scheduler_preserving_internal_fields,
    protocol_events_into_internal_messages, wait_until_scheduler_message,
};

async fn wait_for_request_paused(ctx: &mut TestContext, url: &str, description: &str) -> Value {
    wait_for_request_paused_on_session(ctx, "SID-1", url, None, description).await
}

async fn wait_for_request_paused_on_session(
    ctx: &mut TestContext,
    session_id: &str,
    url: &str,
    resource_type: Option<&str>,
    description: &str,
) -> Value {
    wait_until_messages(ctx, Some(session_id), description, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!(session_id)
                && message["params"]["request"]["url"] == json!(url)
                && resource_type
                    .is_none_or(|expected| message["params"]["resourceType"] == json!(expected))
        })
    })
    .await;
    ctx.take_first_matching("Fetch.requestPaused event", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["sessionId"] == json!(session_id)
            && message["params"]["request"]["url"] == json!(url)
            && resource_type
                .is_none_or(|expected| message["params"]["resourceType"] == json!(expected))
    })
}

async fn open_auto_attached_popup_from_session(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    url: &str,
) -> (String, String) {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": format!("window.open('{url}', '_blank') !== null"),
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing popup open response in {messages:?}"));
    assert_eq!(response["result"]["result"]["value"], json!(true));
    let created = messages
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .unwrap_or_else(|| panic!("missing popup targetCreated in {messages:?}"));
    let target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let attached = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        })
        .unwrap_or_else(|| panic!("missing popup attachedToTarget in {messages:?}"));
    let popup_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup session id")
        .to_owned();
    (target_id, popup_session_id)
}

async fn wait_for_auth_required(
    ctx: &mut TestContext,
    request_id: &str,
    description: &str,
) -> Value {
    wait_until_messages(ctx, Some("SID-1"), description, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Fetch.authRequired")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["requestId"] == json!(request_id)
        })
    })
    .await;
    ctx.take_first_matching("Fetch.authRequired event", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-1")
            && message["params"]["requestId"] == json!(request_id)
    })
}

async fn wait_for_auxiliary_fetch_request_paused(
    ctx: &mut TestContext,
    session_id: &str,
    url: &str,
    response_status: Option<u16>,
    description: &str,
) -> Value {
    wait_for_target_fetch_request_paused(ctx, Some(session_id), url, response_status, description)
        .await
}

async fn wait_for_target_fetch_request_paused(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    url: &str,
    response_status: Option<u16>,
    description: &str,
) -> Value {
    wait_until_scheduler_message(ctx, description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"] == json!(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!("XHR")
            && match response_status {
                Some(status) => message["params"]["responseStatusCode"] == json!(status),
                None => true,
            }
    })
    .await;
    ctx.take_first_matching(description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"] == json!(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!("XHR")
            && match response_status {
                Some(status) => message["params"]["responseStatusCode"] == json!(status),
                None => true,
            }
    })
}

async fn wait_for_background_request_paused(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    url: &str,
    resource_type: &str,
    description: &str,
) -> Value {
    wait_until_scheduler_message(ctx, description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"].as_str() == Some(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!(resource_type)
    })
    .await;
    ctx.take_first_matching(description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && session_id.is_none_or(|session_id| message["sessionId"].as_str() == Some(session_id))
            && message["params"]["request"]["url"] == json!(url)
            && message["params"]["resourceType"] == json!(resource_type)
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_session_fetch_enable_receives_subresource_request_pause() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        "auxiliary fetch body"
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
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_900,
        "method": "Fetch.enable",
        "sessionId": "SID-aux",
        "params": {
            "patterns": [
                { "urlPattern": "*/api", "requestStage": "Request", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_900, json!({}), Some("SID-aux"));
    enable_runtime_async(&mut ctx, "SID-1", 35_901).await;

    ctx.process_async(json!({
        "id": 35_902,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_aux_fetch_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_aux_fetch_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_902);

    let paused = ctx
        .wait_for_scheduler_message("auxiliary-session Fetch.requestPaused", |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-aux")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["resourceType"] == json!("XHR")
        })
        .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("auxiliary fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_903,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    ctx.expect_error(35_903, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 35_904,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-aux",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(35_904, json!({}), Some("SID-aux"));

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_905,
        "globalThis.__lm_aux_fetch_result",
        &json!("auxiliary fetch body"),
        "auxiliary-session fetch result",
    )
    .await;
    assert_eq!(
        resolved["result"]["result"]["value"],
        "auxiliary fetch body"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_session_chain_uses_enable_insertion_order_not_session_id_sort() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        "ordered fetch body"
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
    with_loaded_http_document(&mut ctx, &page_url, "SID-z", "TID-1").await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-a".to_owned())
    );
    ctx.sent.clear();

    for (id, session_id) in [(35_916, "SID-z"), (35_917, "SID-a")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Fetch.enable",
            "sessionId": session_id,
            "params": {
                "patterns": [
                    { "urlPattern": "*/api", "requestStage": "Request", "resourceType": "Fetch" }
                ]
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    enable_runtime_async(&mut ctx, "SID-z", 35_918).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_919,
        "method": "Runtime.evaluate",
        "sessionId": "SID-z",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_ordered_fetch_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_ordered_fetch_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_919);

    wait_until_messages(
        &mut ctx,
        Some("SID-z"),
        "first ordered Fetch pause",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Fetch.requestPaused")
                    && message["params"]["request"]["url"] == json!(api_url)
            })
        },
    )
    .await;
    let first_pause = ctx.take_first_matching("first ordered Fetch pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(api_url)
    });
    assert_eq!(
        first_pause["sessionId"], "SID-z",
        "Fetch handler order should follow enable/attach order, not session id sort"
    );
    let first_request_id = first_pause["params"]["requestId"]
        .as_str()
        .expect("first request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_920,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-z",
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_result(35_920, json!({}), Some("SID-z"));

    let second_pause = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-a",
        &api_url,
        None,
        "second ordered Fetch pause",
    )
    .await;
    let second_request_id = second_pause["params"]["requestId"]
        .as_str()
        .expect("second request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_921,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-a",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(35_921, json!({}), Some("SID-a"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-z",
        35_922,
        "globalThis.__lm_ordered_fetch_result",
        &json!("ordered fetch body"),
        "ordered fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_session_detach_removes_fetch_enable_interception() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        "detached auxiliary fetch body"
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
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_910,
        "method": "Fetch.enable",
        "sessionId": "SID-aux",
        "params": {
            "patterns": [
                { "urlPattern": "*/api", "requestStage": "Request", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_910, json!({}), Some("SID-aux"));
    enable_runtime_async(&mut ctx, "SID-1", 35_911).await;

    ctx.process_async(json!({
        "id": 35_912,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": "SID-aux" }
    }))
    .await;
    ctx.expect_result(35_912, json!({}), None);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_913,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_detached_aux_fetch_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_detached_aux_fetch_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_913);

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_914,
        "globalThis.__lm_detached_aux_fetch_result",
        &json!("detached auxiliary fetch body"),
        "fetch after auxiliary detach",
    )
    .await;
    assert_eq!(
        resolved["result"]["result"]["value"],
        "detached auxiliary fetch body"
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Fetch.requestPaused")),
        "detached auxiliary Fetch session must not keep intercepting requests: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_session_fetch_response_stage_body_commands_use_session_owner() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(axum::extract::Query(query): axum::extract::Query<Value>) -> impl IntoResponse {
        let mode = query.get("mode").and_then(Value::as_str).unwrap_or("body");
        match mode {
            "stream" => "auxiliary stream body",
            _ => "auxiliary fetch body",
        }
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
    let body_url = format!("http://{addr}/api?mode=body");
    let stream_url = format!("http://{addr}/api?mode=stream");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_920,
        "method": "Fetch.enable",
        "sessionId": "SID-aux",
        "params": {
            "patterns": [
                { "urlPattern": "*/api*", "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_920, json!({}), Some("SID-aux"));
    enable_runtime_async(&mut ctx, "SID-1", 35_921).await;

    ctx.process_async(json!({
        "id": 35_922,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_aux_fetch_body_result = "pending";
  fetch('/api?mode=body')
    .then(response => response.text())
    .then(text => { globalThis.__lm_aux_fetch_body_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_922);

    let paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-aux",
        &body_url,
        Some(200),
        "auxiliary response-stage body pause",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("auxiliary fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_931,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    ctx.expect_error(35_931, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 35_923,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    ctx.expect_result(
        35_923,
        json!({ "body": "auxiliary fetch body", "base64Encoded": false }),
        Some("SID-aux"),
    );

    ctx.process_async(json!({
        "id": 35_924,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-aux",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(35_924, json!({}), Some("SID-aux"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_925,
        "globalThis.__lm_aux_fetch_body_result",
        &json!("auxiliary fetch body"),
        "auxiliary fetch body result",
    )
    .await;

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 35_926,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_aux_fetch_stream_result = "pending";
  fetch('/api?mode=stream')
    .then(response => response.text())
    .then(text => { globalThis.__lm_aux_fetch_stream_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_926);

    let paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-aux",
        &stream_url,
        Some(200),
        "auxiliary response-stage stream pause",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("auxiliary fetch stream request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_932,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    ctx.expect_error(35_932, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 35_927,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-aux",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    let stream = take_response_by_id(&mut ctx, 35_927)["result"]["stream"]
        .as_str()
        .expect("stream handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_928,
        "method": "IO.read",
        "sessionId": "SID-aux",
        "params": { "handle": stream }
    }))
    .await;
    ctx.expect_result(
        35_928,
        json!({ "base64Encoded": false, "data": "auxiliary stream body", "eof": true }),
        Some("SID-aux"),
    );

    ctx.process_async(json!({
        "id": 35_929,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-aux",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "body": "YXV4aWxpYXJ5IHN0cmVhbSBib2R5"
        }
    }))
    .await;
    ctx.expect_result(35_929, json!({}), Some("SID-aux"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_930,
        "globalThis.__lm_aux_fetch_stream_result",
        &json!("auxiliary stream body"),
        "auxiliary fetch stream result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn multiple_fetch_sessions_chain_subresource_response_stage_pauses() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        "multi response body"
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
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );

    for (id, session_id) in [(35_940, "SID-1"), (35_941, "SID-aux")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Fetch.enable",
            "sessionId": session_id,
            "params": {
                "patterns": [
                    { "urlPattern": "*/api", "requestStage": "Response", "resourceType": "Fetch" }
                ]
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    enable_runtime_async(&mut ctx, "SID-1", 35_942).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_943,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_multi_response_stage_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_multi_response_stage_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_943);

    let first_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-1",
        &api_url,
        Some(200),
        "first response-stage pause on primary session",
    )
    .await;
    let first_request_id = first_paused["params"]["requestId"]
        .as_str()
        .expect("first response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_944,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_result(35_944, json!({}), Some("SID-1"));

    let second_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-aux",
        &api_url,
        Some(200),
        "second response-stage pause on auxiliary session",
    )
    .await;
    let second_request_id = second_paused["params"]["requestId"]
        .as_str()
        .expect("second response-stage request id")
        .to_owned();
    assert_ne!(second_request_id, first_request_id);

    ctx.process_async(json!({
        "id": 35_945,
        "method": "Fetch.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": second_request_id.clone() }
    }))
    .await;
    ctx.expect_result(
        35_945,
        json!({ "body": "multi response body", "base64Encoded": false }),
        Some("SID-aux"),
    );

    ctx.process_async(json!({
        "id": 35_946,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-aux",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(35_946, json!({}), Some("SID-aux"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_947,
        "globalThis.__lm_multi_response_stage_result",
        &json!("multi response body"),
        "multi response-stage fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_body_stream_taken_blocks_chained_response_stage_continue() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        "multi stream body"
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
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );

    for (id, session_id) in [(35_948, "SID-1"), (35_949, "SID-aux")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Fetch.enable",
            "sessionId": session_id,
            "params": {
                "patterns": [
                    { "urlPattern": "*/api", "requestStage": "Response", "resourceType": "Fetch" }
                ]
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    enable_runtime_async(&mut ctx, "SID-1", 35_950).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_951,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_multi_stream_taken_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_multi_stream_taken_result = text; })
    .catch(error => { globalThis.__lm_multi_stream_taken_result = String(error); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_951);

    let first_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-1",
        &api_url,
        Some(200),
        "first response-stage pause before body stream taken",
    )
    .await;
    let first_request_id = first_paused["params"]["requestId"]
        .as_str()
        .expect("first response-stage request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_952,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": first_request_id }
    }))
    .await;
    let stream_result = take_response_by_id(&mut ctx, 35_952);
    let stream_handle = stream_result["result"]["stream"]
        .as_str()
        .expect("response body stream handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_953,
        "method": "IO.read",
        "params": { "handle": stream_handle }
    }))
    .await;
    ctx.expect_result(
        35_953,
        json!({
            "base64Encoded": false,
            "data": "multi stream body",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 35_954,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_error(
        35_954,
        -32602,
        "Unable to continue request as is after body is taken",
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-aux")
                && message["params"]["request"]["url"] == json!(api_url)
        }),
        "body-taken response should not advance to next Fetch handler"
    );

    ctx.process_async(json!({
        "id": 35_955,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": first_request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "body": "bXVsdGkgc3RyZWFtIGJvZHk="
        }
    }))
    .await;
    ctx.expect_result(35_955, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_956,
        "globalThis.__lm_multi_stream_taken_result",
        &json!("multi stream body"),
        "body-taken terminal fulfill fetch result",
    )
    .await;

    ctx.process_async(json!({
        "id": 35_957,
        "method": "IO.close",
        "params": { "handle": stream_handle }
    }))
    .await;
    ctx.expect_result(35_957, json!({}), None);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_body_stream_taken_blocks_chained_bidi_response_stage_pause() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn hit() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "mixed body taken")
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
        "id": 35_958,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [
                { "urlPattern": hit_url.clone(), "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_958, json!({}), Some("SID-1"));

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        .target_fetch_subresource_interception_snapshot_for_session_owner(Some("SID-1"))
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
    enable_runtime_async(&mut ctx, "SID-1", 35_959).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_960,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_mixed_stream_taken_result = "pending";
  fetch('/api/hit')
    .then(response => response.text())
    .then(text => { globalThis.__lm_mixed_stream_taken_result = text; })
    .catch(error => { globalThis.__lm_mixed_stream_taken_result = String(error); });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_960);

    let cdp_pause = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-1",
        &hit_url,
        Some(200),
        "CDP response-stage pause before BiDi response-stage pause",
    )
    .await;
    assert!(
        cdp_pause["params"]["__moliBlockedInterceptors"].is_null(),
        "CDP Fetch pause should not carry the later BiDi blocked marker: {cdp_pause:?}"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("BIDI-SID")
                && message["params"]["request"]["url"] == json!(hit_url)
        }),
        "BiDi response-stage pause should wait until the CDP Fetch handler continues"
    );
    let cdp_request_id = cdp_pause["params"]["requestId"]
        .as_str()
        .expect("CDP response-stage request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_961,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": "SID-1",
        "params": { "requestId": cdp_request_id.clone() }
    }))
    .await;
    let stream_result = take_response_by_id(&mut ctx, 35_961);
    let stream_handle = stream_result["result"]["stream"]
        .as_str()
        .expect("response body stream handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_962,
        "method": "IO.read",
        "params": { "handle": stream_handle }
    }))
    .await;
    ctx.expect_result(
        35_962,
        json!({
            "base64Encoded": false,
            "data": "mixed body taken",
            "eof": true
        }),
        None,
    );

    ctx.process_async(json!({
        "id": 35_963,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": cdp_request_id.clone() }
    }))
    .await;
    ctx.expect_error(
        35_963,
        -32602,
        "Unable to continue request as is after body is taken",
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("BIDI-SID")
                && message["params"]["request"]["url"] == json!(hit_url)
        }),
        "body-taken response must not advance to the later BiDi Network stage"
    );

    ctx.process_async(json!({
        "id": 35_964,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": cdp_request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "body": "bWl4ZWQgZnVsZmlsbGVkIGJvZHk="
        }
    }))
    .await;
    ctx.expect_result(35_964, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_965,
        "globalThis.__lm_mixed_stream_taken_result",
        &json!("mixed fulfilled body"),
        "body-taken mixed-chain terminal fulfill fetch result",
    )
    .await;

    ctx.process_async(json!({
        "id": 35_966,
        "method": "IO.close",
        "params": { "handle": stream_handle }
    }))
    .await;
    ctx.expect_result(35_966, json!({}), None);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_head_override_is_visible_to_chained_fetch_session() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-original", "yes")],
            "override body",
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
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );

    for (id, session_id) in [(35_950, "SID-1"), (35_951, "SID-aux")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Fetch.enable",
            "sessionId": session_id,
            "params": {
                "patterns": [
                    { "urlPattern": "*/api", "requestStage": "Response", "resourceType": "Fetch" }
                ]
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    enable_runtime_async(&mut ctx, "SID-1", 35_952).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_953,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_response_override_chain_result = "pending";
  fetch('/api')
    .then(async response => {
      globalThis.__lm_response_override_chain_result =
        `${response.status}:${response.headers.get('x-chain')}:${await response.text()}`;
    });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_953);

    let first_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-1",
        &api_url,
        Some(200),
        "first response-stage pause before head override",
    )
    .await;
    let first_request_id = first_paused["params"]["requestId"]
        .as_str()
        .expect("first response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_954,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": {
            "requestId": first_request_id,
            "responseCode": 201,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" },
                { "name": "x-chain", "value": "primary" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_954, json!({}), Some("SID-1"));

    let second_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-aux",
        &api_url,
        Some(201),
        "second response-stage pause after head override",
    )
    .await;
    let second_headers = second_paused["params"]["responseHeaders"]
        .as_array()
        .expect("second pause response headers");
    assert!(
        second_headers
            .iter()
            .any(|header| header["name"] == "x-chain" && header["value"] == "primary")
    );
    assert!(
        !second_headers
            .iter()
            .any(|header| header["name"] == "x-original" && header["value"] == "yes")
    );
    let second_request_id = second_paused["params"]["requestId"]
        .as_str()
        .expect("second response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_955,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-aux",
        "params": {
            "requestId": second_request_id,
            "responsePhrase": "Created"
        }
    }))
    .await;
    ctx.expect_result(35_955, json!({}), Some("SID-aux"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_956,
        "globalThis.__lm_response_override_chain_result",
        &json!("201:primary:override body"),
        "response-stage override chain fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_empty_headers_override_is_visible_to_chained_fetch_session() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-original", "yes")],
            "empty header body",
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
            "params": {
                "patterns": [
                    { "urlPattern": "*/api", "requestStage": "Response", "resourceType": "Fetch" }
                ]
            }
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
  globalThis.__lm_empty_response_headers_override_result = "pending";
  fetch('/api')
    .then(async response => {
      globalThis.__lm_empty_response_headers_override_result =
        `${response.status}:${response.headers.get('x-original')}:${response.headers.get('content-type')}:${await response.text()}`;
    });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 35_973);

    let first_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-1",
        &api_url,
        Some(200),
        "first response-stage pause before empty headers override",
    )
    .await;
    let first_request_id = first_paused["params"]["requestId"]
        .as_str()
        .expect("first response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_974,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": {
            "requestId": first_request_id,
            "responseCode": 201,
            "responseHeaders": []
        }
    }))
    .await;
    ctx.expect_result(35_974, json!({}), Some("SID-1"));

    let second_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-aux",
        &api_url,
        Some(201),
        "second response-stage pause after empty headers override",
    )
    .await;
    let second_headers = second_paused["params"]["responseHeaders"]
        .as_array()
        .expect("second pause response headers");
    assert!(
        second_headers.is_empty(),
        "explicit empty responseHeaders override should clear original headers: {second_paused:?}"
    );
    let second_request_id = second_paused["params"]["requestId"]
        .as_str()
        .expect("second response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 35_975,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-aux",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(35_975, json!({}), Some("SID-aux"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_976,
        "globalThis.__lm_empty_response_headers_override_result",
        &json!("201:null:null:empty header body"),
        "response-stage empty headers override chain fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_stage_partial_head_override_is_rejected_and_preserves_chain() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-original", "yes")],
            "partial body",
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
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );

    for (id, session_id) in [(36_020, "SID-1"), (36_021, "SID-aux")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Fetch.enable",
            "sessionId": session_id,
            "params": {
                "patterns": [
                    { "urlPattern": "*/api", "requestStage": "Response", "resourceType": "Fetch" }
                ]
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    enable_runtime_async(&mut ctx, "SID-1", 36_022).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_023,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_partial_response_override_result = "pending";
  fetch('/api')
    .then(async response => {
      globalThis.__lm_partial_response_override_result =
        `${response.status}:${response.headers.get('x-original')}:${await response.text()}`;
    });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 36_023);

    let first_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-1",
        &api_url,
        Some(200),
        "first response-stage pause before partial head override",
    )
    .await;
    let first_request_id = first_paused["params"]["requestId"]
        .as_str()
        .expect("first response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 36_024,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": {
            "requestId": first_request_id,
            "responseCode": 202
        }
    }))
    .await;
    ctx.expect_error(
        36_024,
        -32602,
        "Cannot override only status or headers, both should be provided",
    );

    ctx.process_async(json!({
        "id": 36_025,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": first_request_id }
    }))
    .await;
    ctx.expect_result(36_025, json!({}), Some("SID-1"));

    let second_paused = wait_for_auxiliary_fetch_request_paused(
        &mut ctx,
        "SID-aux",
        &api_url,
        Some(200),
        "second response-stage pause after rejected partial override",
    )
    .await;
    let second_request_id = second_paused["params"]["requestId"]
        .as_str()
        .expect("second response-stage request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 36_026,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-aux",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(36_026, json!({}), Some("SID-aux"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        36_027,
        "globalThis.__lm_partial_response_override_result",
        &json!("200:yes:partial body"),
        "response-stage partial override rejection fetch result",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_request_returns_before_delayed_subresource_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(notify: axum::extract::State<Arc<tokio::sync::Notify>>) -> impl IntoResponse {
        notify.notified().await;
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-subresource", "delayed"),
            ],
            "delayed-body",
        )
    }

    let release_response = Arc::new(tokio::sync::Notify::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_release = release_response.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api))
                .with_state(server_release),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.conn.browser_context.as_mut().unwrap().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_000,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_000, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 36_001,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(36_001, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 36_002).await;

    ctx.process_async(json!({
        "id": 36_003,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_delayed_fetch_result = "pending";
  fetch('/api')
    .then(response => response.text())
    .then(text => { globalThis.__lm_delayed_fetch_result = text; });
  return "scheduled";
})()"#
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 36_003);

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
    assert_eq!(paused["params"]["request"]["url"], api_url);
    network_request_announced_before_fetch_pause(&ctx, &paused, Some("Fetch"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36_004,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(36_004, json!({}), Some("SID-1"));
    assert!(
        ctx.sent.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Network.requestWillBeSent")
                    | Some("Network.responseReceived")
                    | Some("Network.loadingFinished")
            )
        }),
        "continueRequest must not synchronously emit network completion events: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    release_response.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "delayed subresource Network.loadingFinished",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(network_id)
        },
    )
    .await;

    assert!(ctx.sent.iter().all(|message| {
        message["method"] != json!("Network.requestWillBeSent")
            || message["params"]["requestId"] != json!(network_id)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(network_id)
            && message["params"]["response"]["headers"]["x-subresource"] == json!("delayed")
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(network_id)
    }));

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 36_045,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_delayed_fetch_result" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 36_045);
    assert_eq!(resolved["result"]["result"]["value"], "delayed-body");

    server.abort();
}

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

    assert_eq!(
        ctx.conn
            .target_owner_identity_for_session(Some(&popup_session_id))
            .and_then(|(_, target_id)| target_id),
        Some(popup_target_id.clone()),
        "the popup session must remain bound to its own target before navigation"
    );

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
async fn auxiliary_session_fetch_enable_receives_worker_xhr_request_pause() {
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
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 37_070,
        "method": "Fetch.enable",
        "sessionId": "SID-aux",
        "params": {
            "patterns": [
                { "urlPattern": "*/worker-api*", "requestStage": "Request", "resourceType": "XHR" }
            ]
        }
    }))
    .await;
    ctx.expect_result(37_070, json!({}), Some("SID-aux"));
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
        Some("SID-aux"),
        &api_url,
        "XHR",
        "auxiliary-session worker xhr requestPaused should surface through renderer publication",
    )
    .await;
    assert_eq!(paused["sessionId"], json!("SID-aux"));
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
        json!({ "body": "auth required", "base64Encoded": false }),
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
    ctx.conn.browser_context.as_mut().unwrap().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
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
    ctx.conn.browser_context.as_mut().unwrap().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
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
    ctx.conn.browser_context.as_mut().unwrap().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
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
    ctx.conn.browser_context.as_mut().unwrap().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
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
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        DevToolsCommandResult::AddNetworkIntercept(
            crate::devtools_runtime::DevToolsAddNetworkInterceptResult {
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit")
            }
        )
    );
    ctx.sent.clear();

    let (evaluate_result, scheduler_events, protocol_events, renderer_output_predecessor) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        .execute_devtools_command(DevToolsCommand::ContinueInterceptedRequest(
            DevToolsContinueInterceptedRequestCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        DevToolsCommandResult::Empty
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
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        .target_fetch_subresource_interception_snapshot_for_session_owner(Some("SID-1"))
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
    let pending_cdp_pause =
        crate::domains::fetch::state::pending_subresource_response_request_for_action_session(
            &mut ctx.conn,
            Some("SID-1"),
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
        .execute_devtools_command(DevToolsCommand::ContinueInterceptedResponse(
            DevToolsContinueInterceptedResponseCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        DevToolsCommandResult::Empty
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
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
        DevToolsCommandResult::AddNetworkIntercept(
            crate::devtools_runtime::DevToolsAddNetworkInterceptResult {
                intercept_id: DevToolsNetworkInterceptId::from("intercept-hit")
            }
        )
    );
    ctx.sent.clear();

    let (evaluate_result, scheduler_events, protocol_events, renderer_output_predecessor) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
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
    assert_eq!(failed["params"]["errorText"], "Navigation stopped");

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
    ctx.conn.browser_context.as_mut().unwrap().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
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

    ctx.process_async(json!({
        "id": 371,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_synthetic_fetch" }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 371);
    assert_eq!(resolved["result"]["result"]["value"], "synthetic-body");

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
