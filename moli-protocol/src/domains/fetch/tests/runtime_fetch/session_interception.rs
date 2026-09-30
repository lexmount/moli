use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn runtime_local_fetch_and_xhr_bypass_request_and_response_interception() {
    for request_stage in ["Request", "Response"] {
        let mut ctx = TestContext::new();
        with_loaded_http_document(
            &mut ctx,
            "data:text/html,<html><body>ready</body></html>",
            "SID-1",
            "TID-1",
        )
        .await;
        enable_runtime_async(&mut ctx, "SID-1", 1).await;
        ctx.process_async(json!({
            "id": 2,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": { "patterns": [{ "urlPattern": "*", "requestStage": request_stage }] }
        }))
        .await;
        ctx.expect_result(2, json!({}), Some("SID-1"));

        ctx.process_async(json!({
            "id": 3,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": r#"
globalThis.__localInterceptionResult = 'pending';
(async () => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  const blob = URL.createObjectURL(new Blob(['payload'], {type: 'text/plain'}));
  const revoked = URL.createObjectURL(new Blob(['revoked']));
  URL.revokeObjectURL(revoked);
  try {
    for (const [url, method, success] of [
      [blob + '#fragment', 'GET', true],
      [blob, 'POST', false],
      ['data:text/plain,payload', 'POST', true],
      [revoked, 'GET', false],
      ['data:invalid', 'GET', false],
    ]) {
      let response;
      try { response = await fetch(url, {method}); }
      catch (error) {
        check(!success && error instanceof TypeError, 'fetch rejection: ' + error);
      }
      check(Boolean(response) === success, 'fetch outcome: ' + url);
      if (response) {
        check(response.status === 200 && await response.text() === 'payload', 'fetch response');
      }
      for (const async of [true, false]) {
        const xhr = new XMLHttpRequest();
        xhr.open(method, url, async);
        const label = [url, method, async].join(':');
        if (async) {
          await new Promise((resolve, reject) => {
            let returned = false;
            const events = [];
            xhr.onload = () => events.push('load');
            xhr.onerror = () => events.push('error');
            xhr.onloadend = () => {
              try {
                check(returned, label + ': completion during send');
                check(events.join(',') === (success ? 'load' : 'error'), label + ': events');
                resolve();
              } catch (error) { reject(error); }
            };
            xhr.send();
            returned = true;
          });
        } else {
          let failure;
          try { xhr.send(); } catch (error) { failure = error; }
          check(success ? failure === undefined : failure instanceof DOMException && failure.name === 'NetworkError', label + ': exception');
        }
        check(xhr.readyState === 4 && xhr.status === (success ? 200 : 0), label + ': state/status');
        check(xhr.responseText === (success ? 'payload' : ''), label + ': body');
      }
    }
  } finally { URL.revokeObjectURL(blob); }
})().then(
  () => { globalThis.__localInterceptionResult = 'ok'; },
  error => { globalThis.__localInterceptionResult = String(error); },
);
'scheduled'
"#
            }
        }))
        .await;
        let scheduled = take_response_by_id(&mut ctx, 3);
        assert_eq!(scheduled["result"]["result"]["value"], "scheduled");
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != "Fetch.requestPaused"),
            "local requests must bypass {request_stage} interception: {:?}",
            ctx.sent
        );

        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            4,
            "globalThis.__localInterceptionResult",
            &json!("ok"),
            "local fetch and XHR completion with Fetch enabled",
        )
        .await;
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != "Fetch.requestPaused"),
            "local responses and errors must bypass {request_stage} interception: {:?}",
            ctx.sent
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_session_fetch_enable_receives_subresource_request_pause() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        "attached fetch body"
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_900,
        "method": "Fetch.enable",
        "sessionId": "SID-attached",
        "params": {
            "patterns": [
                { "urlPattern": "*/api", "requestStage": "Request", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_900, json!({}), Some("SID-attached"));
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
        .wait_for_scheduler_message("attached-session Fetch.requestPaused", |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-attached")
                && message["params"]["request"]["url"] == json!(api_url)
                && message["params"]["resourceType"] == json!("XHR")
        })
        .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("attached fetch request id")
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
        "sessionId": "SID-attached",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(35_904, json!({}), Some("SID-attached"));

    let resolved = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_905,
        "globalThis.__lm_aux_fetch_result",
        &json!("attached fetch body"),
        "attached-session fetch result",
    )
    .await;
    assert_eq!(resolved["result"]["result"]["value"], "attached fetch body");

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
            .assign_attached_session_to_target("TID-1", "SID-a".to_owned())
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

    let second_pause = wait_for_attached_fetch_request_paused(
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
async fn attached_session_detach_removes_fetch_enable_interception() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        "detached attached fetch body"
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_910,
        "method": "Fetch.enable",
        "sessionId": "SID-attached",
        "params": {
            "patterns": [
                { "urlPattern": "*/api", "requestStage": "Request", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_910, json!({}), Some("SID-attached"));
    enable_runtime_async(&mut ctx, "SID-1", 35_911).await;

    ctx.process_async(json!({
        "id": 35_912,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": "SID-attached" }
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
        &json!("detached attached fetch body"),
        "fetch after attached detach",
    )
    .await;
    assert_eq!(
        resolved["result"]["result"]["value"],
        "detached attached fetch body"
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Fetch.requestPaused")),
        "detached attached Fetch session must not keep intercepting requests: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_session_fetch_response_stage_body_commands_use_session_owner() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api(axum::extract::Query(query): axum::extract::Query<Value>) -> impl IntoResponse {
        let mode = query.get("mode").and_then(Value::as_str).unwrap_or("body");
        match mode {
            "stream" => "attached stream body",
            _ => "attached fetch body",
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35_920,
        "method": "Fetch.enable",
        "sessionId": "SID-attached",
        "params": {
            "patterns": [
                { "urlPattern": "*/api*", "requestStage": "Response", "resourceType": "Fetch" }
            ]
        }
    }))
    .await;
    ctx.expect_result(35_920, json!({}), Some("SID-attached"));
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

    let paused = wait_for_attached_fetch_request_paused(
        &mut ctx,
        "SID-attached",
        &body_url,
        Some(200),
        "attached response-stage body pause",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("attached fetch request id")
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
        "sessionId": "SID-attached",
        "params": { "requestId": request_id.clone() }
    }))
    .await;
    ctx.expect_result(
        35_923,
        json!({ "body": "attached fetch body", "base64Encoded": false }),
        Some("SID-attached"),
    );

    ctx.process_async(json!({
        "id": 35_924,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-attached",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(35_924, json!({}), Some("SID-attached"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_925,
        "globalThis.__lm_aux_fetch_body_result",
        &json!("attached fetch body"),
        "attached fetch body result",
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

    let paused = wait_for_attached_fetch_request_paused(
        &mut ctx,
        "SID-attached",
        &stream_url,
        Some(200),
        "attached response-stage stream pause",
    )
    .await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("attached fetch stream request id")
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
        "sessionId": "SID-attached",
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
        "sessionId": "SID-attached",
        "params": { "handle": stream }
    }))
    .await;
    ctx.expect_result(
        35_928,
        json!({ "base64Encoded": false, "data": "attached stream body", "eof": true }),
        Some("SID-attached"),
    );

    ctx.process_async(json!({
        "id": 35_929,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-attached",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "body": "YXR0YWNoZWQgc3RyZWFtIGJvZHk="
        }
    }))
    .await;
    ctx.expect_result(35_929, json!({}), Some("SID-attached"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        35_930,
        "globalThis.__lm_aux_fetch_stream_result",
        &json!("attached stream body"),
        "attached fetch stream result",
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(35_940, "SID-1"), (35_941, "SID-attached")] {
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

    let first_paused = wait_for_attached_fetch_request_paused(
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

    let second_paused = wait_for_attached_fetch_request_paused(
        &mut ctx,
        "SID-attached",
        &api_url,
        Some(200),
        "second response-stage pause on attached session",
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
        "sessionId": "SID-attached",
        "params": { "requestId": second_request_id.clone() }
    }))
    .await;
    ctx.expect_result(
        35_945,
        json!({ "body": "multi response body", "base64Encoded": false }),
        Some("SID-attached"),
    );

    ctx.process_async(json!({
        "id": 35_946,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-attached",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(35_946, json!({}), Some("SID-attached"));
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(35_948, "SID-1"), (35_949, "SID-attached")] {
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

    let first_paused = wait_for_attached_fetch_request_paused(
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
                && message["sessionId"] == json!("SID-attached")
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

    let cdp_pause = wait_for_attached_fetch_request_paused(
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(35_950, "SID-1"), (35_951, "SID-attached")] {
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

    let first_paused = wait_for_attached_fetch_request_paused(
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

    let second_paused = wait_for_attached_fetch_request_paused(
        &mut ctx,
        "SID-attached",
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
        "sessionId": "SID-attached",
        "params": {
            "requestId": second_request_id,
            "responsePhrase": "Created"
        }
    }))
    .await;
    ctx.expect_result(35_955, json!({}), Some("SID-attached"));
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(35_970, "SID-1"), (35_971, "SID-attached")] {
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

    let first_paused = wait_for_attached_fetch_request_paused(
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

    let second_paused = wait_for_attached_fetch_request_paused(
        &mut ctx,
        "SID-attached",
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
        "sessionId": "SID-attached",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(35_975, json!({}), Some("SID-attached"));
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
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(36_020, "SID-1"), (36_021, "SID-attached")] {
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

    let first_paused = wait_for_attached_fetch_request_paused(
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

    let second_paused = wait_for_attached_fetch_request_paused(
        &mut ctx,
        "SID-attached",
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
        "sessionId": "SID-attached",
        "params": { "requestId": second_request_id }
    }))
    .await;
    ctx.expect_result(36_026, json!({}), Some("SID-attached"));
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
