use super::*;
use crate::devtools_runtime::{
    AutomationEvent, DevToolsAddNetworkInterceptCommand, DevToolsAuthChallengeAction,
    DevToolsAuthCredentials, DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsContinueInterceptedResponseCommand, DevToolsContinueWithAuthCommand,
    DevToolsNetworkInterceptId, DevToolsNetworkInterceptPattern, DevToolsNetworkInterceptPhase,
    DevToolsProtocol, DevToolsRequestId, DevToolsSessionId, DevToolsTargetId,
};

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_retries_navigation_with_basic_credentials() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if authorization != Some("Basic YWxhZGRpbjpvcGVuc2VzYW1l") {
            return (
                StatusCode::UNAUTHORIZED,
                [
                    (WWW_AUTHENTICATE.as_str(), r#"Basic realm="test-area""#),
                    (CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "auth required",
            )
                .into_response();
        }

        (
            StatusCode::OK,
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>authorized</main></body></html>",
        )
            .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/auth", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 68,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(68, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 69,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    assert_eq!(paused["params"]["networkId"], LOADER_ID);

    ctx.process_async(json!({
        "id": 70,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(70, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], "INT-1");
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Server");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "basic");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "test-area"
    );

    ctx.process_async(json!({
        "id": 71,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "aladdin",
                "password": "opensesame"
            }
        }
    }))
    .await;
    ctx.expect_result(71, json!({}), Some("SID-1"));

    ctx.expect_result(
        69,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

    let request_extra_info = ctx.take_one();
    assert_eq!(
        request_extra_info["method"],
        "Network.requestWillBeSentExtraInfo"
    );
    assert_eq!(request_extra_info["params"]["requestId"], LOADER_ID);
    let response_extra_info = ctx.take_one();
    assert_eq!(
        response_extra_info["method"],
        "Network.responseReceivedExtraInfo"
    );
    assert_eq!(response_extra_info["params"]["requestId"], LOADER_ID);
    assert_eq!(response_extra_info["params"]["statusCode"], 200);
    let response = ctx.take_one();
    assert_eq!(response["method"], "Network.responseReceived");
    assert_eq!(response["params"]["requestId"], LOADER_ID);
    assert_eq!(response["params"]["response"]["status"], 200);

    assert_eq!(ctx.take_one()["method"], "Page.frameNavigated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");
    let data = ctx.take_one();
    assert_eq!(data["method"], "Network.dataReceived");
    assert_eq!(data["params"]["requestId"], LOADER_ID);
    assert_eq!(ctx.take_one()["method"], "Network.loadingFinished");
    assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");
    assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");

    ctx.process_async(json!({
        "id": 72,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": LOADER_ID }
    }))
    .await;
    ctx.expect_result(
        72,
        json!({
            "body": "<!doctype html><html><body><main>authorized</main></body></html>",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_continue_response_credentials_retries_auth_navigation() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if authorization != Some("Basic YWxhZGRpbjpvcGVuc2VzYW1l") {
            return (
                StatusCode::UNAUTHORIZED,
                [
                    (WWW_AUTHENTICATE.as_str(), r#"Basic realm="test-area""#),
                    (CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "auth required",
            )
                .into_response();
        }

        (
            StatusCode::OK,
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>authorized</main></body></html>",
        )
            .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/auth", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 73_001,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(73_001, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 73_002,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 73_003,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(73_003, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], "INT-1");

    let outcome = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::ContinueInterceptedResponse(
            DevToolsContinueInterceptedResponseCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
                    session_id: None,
                    target_id: Some("TID-1".into()),
                    browser_context_id: Some("BID-1".into()),
                },
                request_id: DevToolsRequestId::from("INT-1"),
                response_code: None,
                response_headers: None,
                response_phrase: None,
                auth_credentials: Some(DevToolsAuthCredentials {
                    username: "aladdin".to_owned(),
                    password: "opensesame".to_owned(),
                }),
            },
        ))
        .await;
    let (result, scheduler_events, events) = outcome.into_parts_with_protocol_events();
    assert_eq!(
        result.expect("continueResponse credentials should succeed"),
        crate::devtools_runtime::DevToolsCommandResult::Empty
    );
    let event_parts = events
        .clone()
        .into_iter()
        .map(|event| event.into_parts())
        .collect::<Vec<_>>();
    assert!(
        event_parts
            .iter()
            .any(|(_, event)| matches!(event, Some(AutomationEvent::NetworkResponseStarted(_)))),
        "continueResponse credentials should expose typed responseStarted event: {event_parts:?}"
    );
    ctx.route_direct_command_output_for_test(events, scheduler_events)
        .await;
    wait_until_scheduler_message(
        &mut ctx,
        "authorized navigation loadingFinished after direct continueResponse",
        |event| event["method"] == "Network.loadingFinished",
    )
    .await;
    assert!(
        ctx.sent
            .iter()
            .any(|event| event["method"] == "Network.loadingFinished"),
        "continueResponse credentials should schedule response completion activity: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_and_intercept_response_pauses_before_authorized_body_eof() {
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = first.read(&mut buf).await.unwrap();
        let challenge_body = "auth required";
        let challenge = format!(
            "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test-area\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{challenge_body}",
            challenge_body.len()
        );
        first.write_all(challenge.as_bytes()).await.unwrap();
        let _ = first.shutdown().await;

        let (mut second, _) = listener.accept().await.unwrap();
        let read = second.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..read]);
        assert!(
            request.lines().any(|line| {
                line.eq_ignore_ascii_case("Authorization: Basic YWxhZGRpbjpvcGVuc2VzYW1l")
            }),
            "auth retry should send Basic credentials before response headers are streamed"
        );

        let head = "<!doctype html><html><body><main>";
        let tail = "authorized tail</main></body></html>";
        let response_head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        second.write_all(response_head.as_bytes()).await.unwrap();
        second
            .write_all(format!("{:X}\r\n{head}\r\n", head.len()).as_bytes())
            .await
            .unwrap();
        let _ = tail_rx.await;
        second
            .write_all(format!("{:X}\r\n{tail}\r\n0\r\n\r\n", tail.len()).as_bytes())
            .await
            .unwrap();
        let _ = second.shutdown().await;
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 72_100,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(72_100, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 72_101,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 72_102,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(72_102, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], "INT-1");
    assert!(auth_required["params"].get("networkId").is_none());

    tokio::time::timeout(
        std::time::Duration::from_millis(300),
        ctx.process_async(json!({
            "id": 72_103,
            "method": "Fetch.continueWithAuth",
            "sessionId": "SID-1",
            "params": {
                "requestId": "INT-1",
                "authChallengeResponse": {
                    "response": "ProvideCredentials",
                    "username": "aladdin",
                    "password": "opensesame"
                }
            }
        })),
    )
    .await
    .expect("auth retry response-stage pause should not wait for body EOF");
    ctx.expect_result(72_103, json!({}), Some("SID-1"));

    let response_paused = take_main_document_response_pause(&mut ctx);
    assert_eq!(response_paused["method"], "Fetch.requestPaused");
    assert_eq!(response_paused["params"]["requestId"], "INT-1");
    assert_eq!(response_paused["params"]["networkId"], LOADER_ID);
    assert_eq!(response_paused["params"]["responseStatusCode"], 200);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.loaded_page())
            .is_none(),
        "authorized document should not commit before Fetch.continueResponse"
    );

    tail_tx.send(()).unwrap();
    ctx.process_async(json!({
        "id": 72_104,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_result(72_104, json!({}), Some("SID-1"));
    ctx.expect_result(
        72_101,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("authorized tail")
    );

    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_non_basic_auth_and_intercept_response_fails_explicitly_without_retry() {
    async fn handler(request_count: Arc<AtomicUsize>) -> impl IntoResponse {
        request_count.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::UNAUTHORIZED,
            [
                (WWW_AUTHENTICATE.as_str(), r#"Negotiate realm="corp-area""#),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            "auth required",
        )
    }

    let request_count = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_request_count = Arc::clone(&request_count);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/auth",
                get(move || handler(Arc::clone(&server_request_count))),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 72_200,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(72_200, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 72_201,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 72_202,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "interceptResponse": true }
    }))
    .await;
    ctx.expect_result(72_202, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], "INT-1");
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Server");
    assert_eq!(
        auth_required["params"]["authChallenge"]["scheme"],
        "negotiate"
    );

    ctx.process_async(json!({
        "id": 72_203,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "aladdin",
                "password": "opensesame"
            }
        }
    }))
    .await;
    ctx.expect_result(72_203, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["params"]["requestId"], LOADER_ID);
    assert_eq!(
        failed["params"]["errorText"],
        "Fetch response-stage interception after Negotiate authentication is not supported for navigation without buffering"
    );

    let navigate_error = ctx.take_one();
    assert_eq!(navigate_error["id"], 72_201);
    assert_eq!(navigate_error["error"]["code"], -32000);
    assert_eq!(
        navigate_error["error"]["message"],
        "Fetch response-stage interception after Negotiate authentication is not supported for navigation without buffering"
    );
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "unsupported response-stage auth must fail before issuing a buffered retry"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_auth_required_includes_synthesized_cookie_header() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let cookie = headers.get("cookie").and_then(|value| value.to_str().ok());
        assert_eq!(cookie, Some("sid=nav-auth"));

        (
            StatusCode::UNAUTHORIZED,
            [
                (WWW_AUTHENTICATE.as_str(), r#"Basic realm="test-area""#),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            "auth required",
        )
            .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/auth", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    let url = format!("http://{addr}/auth");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&url).unwrap(),
            &[(
                "set-cookie".to_owned(),
                "sid=nav-auth; Path=/auth".to_owned(),
            )],
        );
    }
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 72_001,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(72_001, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 72_002,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 72_003,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(72_003, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(
        auth_required["params"]["request"]["headers"]["Cookie"],
        "sid=nav-auth"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_prefers_supported_navigation_challenge_over_unsupported_one() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if authorization != Some("Basic YWxhZGRpbjpvcGVuc2VzYW1l") {
            return (
                StatusCode::UNAUTHORIZED,
                [
                    (WWW_AUTHENTICATE.as_str(), r#"Bearer realm="token-area""#),
                    (WWW_AUTHENTICATE.as_str(), r#"Basic realm="test-area""#),
                    (CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "auth required",
            )
                .into_response();
        }

        (
            StatusCode::OK,
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>authorized</main></body></html>",
        )
            .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/auth", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 681,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(681, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 682,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 683,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(683, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Server");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "basic");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "test-area"
    );

    ctx.process_async(json!({
        "id": 684,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "aladdin",
                "password": "opensesame"
            }
        }
    }))
    .await;
    ctx.expect_result(684, json!({}), Some("SID-1"));

    ctx.expect_result(
        682,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_cdp_fetch_then_bidi_network_auth_required_continue_with_auth_complete() {
    run_navigation_cdp_fetch_then_bidi_network_auth_required_terminal(
        NavigationMixedAuthTerminalCommand::ContinueWithAuth,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_cdp_fetch_then_bidi_network_auth_required_continue_response_credentials_complete()
 {
    run_navigation_cdp_fetch_then_bidi_network_auth_required_terminal(
        NavigationMixedAuthTerminalCommand::ContinueResponseAuthCredentials,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_cdp_fetch_then_bidi_network_auth_required_terminal_cancel_commits_401() {
    run_navigation_cdp_fetch_then_bidi_network_auth_required_terminal(
        NavigationMixedAuthTerminalCommand::ContinueWithAuthCancel,
    )
    .await;
}

#[derive(Clone, Copy, Debug)]
enum NavigationMixedAuthTerminalCommand {
    ContinueWithAuth,
    ContinueResponseAuthCredentials,
    ContinueWithAuthCancel,
}

async fn run_navigation_cdp_fetch_then_bidi_network_auth_required_terminal(
    terminal_command: NavigationMixedAuthTerminalCommand,
) {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let expected = format!(
            "Basic {}",
            super::encode_basic_auth("aladdin", "opensesame")
        );
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>authorized</main></body></html>",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [
                    (
                        WWW_AUTHENTICATE.as_str(),
                        r#"Basic realm="mixed-navigation""#,
                    ),
                    (CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/auth", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-fetch".to_owned()));
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 73_100,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(73_100, json!({}), Some("SID-1"));

    let fetch_enable_params = if matches!(
        terminal_command,
        NavigationMixedAuthTerminalCommand::ContinueWithAuthCancel
    ) {
        json!({
            "handleAuthRequests": true,
            "patterns": [
                { "urlPattern": url, "requestStage": "Request" },
                { "urlPattern": url, "requestStage": "Response" }
            ]
        })
    } else {
        json!({ "handleAuthRequests": true })
    };
    ctx.process_async(json!({
        "id": 73_101,
        "method": "Fetch.enable",
        "sessionId": "SID-fetch",
        "params": fetch_enable_params
    }))
    .await;
    ctx.expect_result(73_101, json!({}), Some("SID-fetch"));

    let (
        result,
        add_intercept_scheduler_events,
        add_intercept_protocol_events,
        add_intercept_renderer_output_predecessor,
    ) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::AddNetworkIntercept(
            DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("BIDI-SID")),
                    target_id: Some(DevToolsTargetId::from("TID-1")),
                    browser_context_id: None,
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-auth"),
                phases: vec![DevToolsNetworkInterceptPhase::AuthRequired],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: url.clone(),
                }],
            },
        ))
        .await
        .into_complete_parts();
    assert!(add_intercept_protocol_events.is_empty());
    assert!(add_intercept_renderer_output_predecessor.is_none());
    assert_eq!(
        result.expect("BiDi navigation auth intercept should succeed"),
        DevToolsCommandResult::AddNetworkIntercept(
            crate::devtools_runtime::DevToolsAddNetworkInterceptResult {
                intercept_id: DevToolsNetworkInterceptId::from("intercept-auth")
            }
        )
    );
    ctx.route_direct_command_output_for_test(Vec::new(), add_intercept_scheduler_events)
        .await;
    ctx.sent.clear();

    let parsed_url = Url::parse(&url).unwrap();
    let auth_pause_sessions = ctx
        .conn
        .target_fetch_subresource_interception_snapshot_for_session_owner(Some("SID-1"))
        .expect("active target fetch snapshot")
        .matching_auth_required_pause_sessions(Some("SID-1"), &parsed_url);
    assert_eq!(
        auth_pause_sessions
            .iter()
            .map(|session| session.session_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("SID-fetch"), Some("BIDI-SID")]
    );
    assert_eq!(
        auth_pause_sessions[1]
            .blocked_intercepts
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-auth"]
    );

    ctx.process_async(json!({
        "id": 73_102,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = ctx.take_first_matching("auxiliary navigation Fetch.requestPaused", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["sessionId"] == json!("SID-fetch")
            && message["params"]["resourceType"] == json!("Document")
    });
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    assert_eq!(paused["params"]["networkId"], LOADER_ID);
    assert_eq!(paused["sessionId"], "SID-fetch");

    ctx.process_async(json!({
        "id": 73_103,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-fetch",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(73_103, json!({}), Some("SID-fetch"));

    let cdp_auth = ctx.take_first_matching("CDP navigation authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("SID-fetch")
            && message["params"]["request"]["url"] == json!(url)
    });
    assert_eq!(cdp_auth["params"]["requestId"], "INT-1");
    assert_eq!(
        cdp_auth["params"]["authChallenge"]["realm"],
        "mixed-navigation"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.authRequired")
                && message["sessionId"] == json!("BIDI-SID")
        }),
        "BiDi auth pause should wait for CDP Fetch Default"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 73_104,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-fetch",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": { "response": "Default" }
        }
    }))
    .await;
    ctx.expect_result(73_104, json!({}), Some("SID-fetch"));

    let bidi_auth = ctx.take_first_matching("BiDi navigation authRequired pause", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["sessionId"] == json!("BIDI-SID")
            && message["params"]["request"]["url"] == json!(url)
    });
    assert_ne!(bidi_auth["params"]["requestId"], "INT-1");
    assert_eq!(
        bidi_auth["params"]["authChallenge"]["realm"],
        "mixed-navigation"
    );
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
        NavigationMixedAuthTerminalCommand::ContinueWithAuth => ctx
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
                    username: Some("aladdin".to_owned()),
                    password: Some("opensesame".to_owned()),
                },
            ))
            .await
            .into_complete_parts(),
        NavigationMixedAuthTerminalCommand::ContinueResponseAuthCredentials => ctx
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
                        username: "aladdin".to_owned(),
                        password: "opensesame".to_owned(),
                    }),
                },
            ))
            .await
            .into_complete_parts(),
        NavigationMixedAuthTerminalCommand::ContinueWithAuthCancel => ctx
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
    assert!(continue_renderer_output_predecessor.is_none());
    assert_eq!(
        continue_result.expect("BiDi navigation auth terminal action should succeed"),
        DevToolsCommandResult::Empty
    );
    ctx.route_direct_command_output_for_test(continue_protocol_events, continue_scheduler_events)
        .await;

    match terminal_command {
        NavigationMixedAuthTerminalCommand::ContinueWithAuth
        | NavigationMixedAuthTerminalCommand::ContinueResponseAuthCredentials => {
            ctx.expect_result(
                73_102,
                json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
                Some("SID-1"),
            );
            let response = ctx.take_first_matching("authorized navigation response", |message| {
                message["method"] == json!("Network.responseReceived")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["requestId"] == json!(LOADER_ID)
            });
            assert_eq!(response["params"]["response"]["status"], 200);
        }
        NavigationMixedAuthTerminalCommand::ContinueWithAuthCancel => {
            let response_pause = ctx.take_first_matching(
                "navigation auth canceled response-stage pause",
                |message| {
                    message["method"] == json!("Fetch.requestPaused")
                        && message["sessionId"] == json!("SID-fetch")
                        && message["params"]["requestId"] == json!("INT-1")
                        && message["params"]["responseStatusCode"] == json!(401)
                },
            );
            assert_eq!(response_pause["params"]["networkId"], LOADER_ID);
            assert!(
                !ctx.sent.iter().any(|message| {
                    message["method"] == json!("Fetch.requestPaused")
                        && message["sessionId"] == json!("BIDI-SID")
                        && message["params"]["responseStatusCode"] == json!(401)
                }),
                "the auth action session must not replace the configured response-stage owner"
            );

            ctx.process_async(json!({
                "id": 73_105,
                "method": "Fetch.getResponseBody",
                "sessionId": "SID-fetch",
                "params": { "requestId": "INT-1" }
            }))
            .await;
            ctx.expect_result(
                73_105,
                json!({ "body": "auth required", "base64Encoded": false }),
                Some("SID-fetch"),
            );

            ctx.process_async(json!({
                "id": 73_106,
                "method": "Fetch.continueResponse",
                "sessionId": "SID-fetch",
                "params": { "requestId": "INT-1" }
            }))
            .await;
            ctx.expect_result(73_106, json!({}), Some("SID-fetch"));

            ctx.expect_result(
                73_102,
                json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
                Some("SID-1"),
            );
            let response =
                ctx.take_first_matching("navigation auth canceled response", |message| {
                    message["method"] == json!("Network.responseReceived")
                        && message["sessionId"] == json!("SID-1")
                        && message["params"]["requestId"] == json!(LOADER_ID)
                });
            assert_eq!(response["params"]["response"]["status"], 401);
            wait_until_message(
                &mut ctx,
                "SID-1",
                "navigation auth cancel loadingFinished",
                |message| {
                    message["method"] == json!("Network.loadingFinished")
                        && message["params"]["requestId"] == json!(LOADER_ID)
                },
            )
            .await;
            ctx.take_first_matching("navigation auth cancel loadingFinished", |message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["sessionId"] == json!("SID-1")
                    && message["params"]["requestId"] == json!(LOADER_ID)
            });
            assert!(
                !ctx.sent.iter().any(|message| {
                    message["method"] == json!("Network.loadingFailed")
                        && message["params"]["requestId"] == json!(LOADER_ID)
                }),
                "CancelAuth navigation must commit the challenged response"
            );
            assert!(
                !ctx.conn
                    .has_pending_document_navigation_for_session_owner(Some("SID-1")),
                "CancelAuth navigation must release document navigation ownership after commit"
            );
            wait_until_message(
                &mut ctx,
                "SID-1",
                "navigation auth cancel loadEventFired",
                |message| message["method"] == json!("Page.loadEventFired"),
            )
            .await;
            ctx.take_first_matching("navigation auth cancel loadEventFired", |message| {
                message["method"] == json!("Page.loadEventFired")
                    && message["sessionId"] == json!("SID-1")
            });

            ctx.process_async(json!({
                "id": 73_107,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": { "expression": "document.body.textContent" }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 73_107)["result"]["result"]["value"],
                json!("auth required")
            );
        }
    }

    server.abort();
}
