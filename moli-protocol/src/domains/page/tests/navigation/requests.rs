use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_file_url_fails_before_navigation_events_or_document_replacement() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_107,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "file:///moli-policy-must-not-open" }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 90_107);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Navigation to a local file URL requires an explicitly granted browser capability.")
    );
    assert!(
        ctx.sent.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Page.frameStartedNavigating")
                    | Some("Page.frameStartedLoading")
                    | Some("Network.requestWillBeSent")
            )
        }),
        "rejected file navigation must not start a browser load: {:?}",
        ctx.sent
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        "about:blank"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_request_completes_paused_navigation_before_commit_events() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>continued</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
        "id": 241,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(241, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 242,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 243,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;

    let continue_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(243))
        .expect("Fetch.continueRequest response");
    let navigate_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(242))
        .expect("Page.navigate response");
    let commit_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameNavigated"))
        .expect("navigation commit event");
    assert!(
        continue_index < navigate_index,
        "Fetch command response should precede the completed Page.navigate response: {:?}",
        ctx.sent
    );
    assert!(
        navigate_index < commit_index,
        "Page.navigate response should precede commit events: {:?}",
        ctx.sent
    );

    let continue_response = ctx.sent.remove(continue_index);
    assert_eq!(continue_response["result"], json!({}));
    assert_eq!(continue_response["sessionId"], json!("SID-1"));
    let navigate_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(242))
        .expect("Page.navigate response after removing continue response");
    let navigate_response = ctx.sent.remove(navigate_index);
    assert_eq!(navigate_response["sessionId"], json!("SID-1"));
    assert_eq!(navigate_response["result"]["frameId"], json!("TID-1"));
    assert!(
        navigate_response["result"].get("loaderId").is_some(),
        "continued main-document navigation should return a loader id: {navigate_response:?}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_request_stage_navigation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    let committed_document_token = bc
        .start_document_navigation_for_active_target("LOADER-committed-stop".to_owned())
        .expect("active target should start committed document navigation");
    bc.commit_document_navigation_if_matches(&committed_document_token);

    ctx.process_async(json!({
        "id": 233,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(233, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 234,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/stop-loading" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], "SID-1");
    assert_eq!(paused["params"]["resourceType"], "Document");
    let network_id = paused["params"]["networkId"].clone();

    ctx.process_async(json!({
        "id": 235,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(235, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Navigation stopped");

    let error = ctx.take_one();
    assert_eq!(error["id"], 234);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Navigation stopped");
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .accepts_document_body_completion_event(&committed_document_token),
        "Page.stopLoading should preserve the previously committed document"
    );

    let messages = ctx.take_all();
    for method in [
        "Page.frameClearedScheduledNavigation",
        "Page.frameNavigated",
        "DOM.documentUpdated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.lifecycleEvent",
        "Page.frameStoppedLoading",
        "Network.responseReceived",
        "Network.loadingFinished",
    ] {
        assert!(
            !messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "unexpected completion event after stopLoading: {method}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_without_browser_context_returns_empty_result() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 160,
        "method": "Page.stopLoading"
    }))
    .await;

    ctx.expect_result(160, json!({}), None);
    assert!(ctx.take_all().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_response_stage_navigation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("x-stage", "response"),
            ],
            "<!doctype html><html><body>stop-loading</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
            "id": 236,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": {
                "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "Document" }]
            }
        })).await;
    ctx.expect_result(236, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 237,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let request = ctx.take_one();
    assert_eq!(request["method"], "Network.requestWillBeSent");
    let network_id = request["params"]["requestId"].clone();

    let paused = take_main_document_response_pause_after_extra_info(&mut ctx, &network_id, 200);
    assert_eq!(paused["sessionId"], "SID-1");
    assert_eq!(paused["params"]["resourceType"], "Document");
    assert_eq!(paused["params"]["networkId"], network_id);
    assert_eq!(paused["params"]["responseStatusCode"], 200);

    ctx.process_async(json!({
        "id": 238,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(238, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Navigation stopped");

    let error = ctx.take_one();
    assert_eq!(error["id"], 237);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Navigation stopped");

    let messages = ctx.take_all();
    for method in [
        "Page.frameClearedScheduledNavigation",
        "Page.frameNavigated",
        "DOM.documentUpdated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.lifecycleEvent",
        "Page.frameStoppedLoading",
        "Network.responseReceived",
        "Network.loadingFinished",
    ] {
        assert!(
            !messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "unexpected completion event after stopLoading: {method}"
        );
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_auth_navigation() {
    async fn auth(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let expected = "Basic YWxhZGRpbjpvcGVuc2VzYW1l";
        let authorization = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        if authorization != Some(expected) {
            return axum::response::IntoResponse::into_response((
                axum::http::StatusCode::UNAUTHORIZED,
                [
                    (
                        axum::http::header::WWW_AUTHENTICATE.as_str(),
                        r#"Basic realm="stop-area""#,
                    ),
                    (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "auth required",
            ));
        }

        axum::response::IntoResponse::into_response((
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>authorized</body></html>",
        ))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/auth", axum::routing::get(auth)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
        "id": 254,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(254, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 255,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/auth") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 256,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(256, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "Document");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 257,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(257, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Navigation stopped");

    let error = ctx.take_one();
    assert_eq!(error["id"], 255);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Navigation stopped");

    ctx.process_async(json!({
        "id": 258,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "aladdin",
                "password": "opensesame"
            }
        }
    }))
    .await;
    ctx.expect_error(258, -32000, "RequestNotFound");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_passes_referrer_header() {
    async fn page(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{referer}</body></html>"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 401,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": format!("http://{addr}/page"),
            "referrer": "https://www.google.com/"
        }
    }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains(">https://www.google.com/<"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_protocol_navigate_to_same_url_does_not_inherit_referrer_or_reload_headers() {
    async fn page(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let cache_control = headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!(
                "<!doctype html><html><body>referer={referer};cache={cache_control}</body></html>"
            ),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_request_count = request_count.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/page",
                axum::routing::get(move |headers| {
                    let request_count = server_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        page(headers).await
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let url = format!("http://{addr}/page");

    let mut loader_ids = Vec::new();
    for id in [402, 403] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": { "url": url.clone() }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        loader_ids.push(
            response["result"]["loaderId"]
                .as_str()
                .expect("a repeated fragment-free URL remains a cross-document navigation")
                .to_owned(),
        );
        ctx.sent.clear();
    }

    assert_ne!(
        loader_ids[0], loader_ids[1],
        "Chromium assigns a new loader to a repeated fragment-free Page.navigate"
    );
    assert_eq!(
        request_count.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the second Page.navigate must load the URL instead of taking the fragment path"
    );

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">referer=;cache=<"),
        "protocol Page.navigate without an explicit referrer should stay browser-initiated even when navigating to the current URL, got {html}"
    );

    server.abort();
}
