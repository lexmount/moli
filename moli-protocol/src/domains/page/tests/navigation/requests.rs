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

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_cancels_inflight_unpaused_navigation_transport() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let token = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .start_document_navigation_for_active_target("LOADER-inflight-stop".to_owned())
        .unwrap();
    let cancellation = ctx
        .conn
        .document_navigation_cancellation_handle(&token)
        .unwrap();
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .claim_background_navigation_completion(&token, None)
        .expect("current navigation should be claimable");
    assert!(!cancellation.is_cancelled());

    ctx.process_async(json!({
        "id": 901,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(901, json!({}), Some("SID-1"));
    assert!(
        cancellation.is_cancelled(),
        "stopLoading must cancel an ordinary HTTP navigation, not only Fetch-paused requests"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_before_response_preserves_document_and_allows_next_navigation() {
    assert_stopped_provisional_navigation_preserves_document(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_during_xml_body_preserves_document_and_allows_next_navigation() {
    assert_stopped_provisional_navigation_preserves_document(Some((200, "application/xml"))).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_during_http_error_body_preserves_document_and_allows_next_navigation() {
    assert_stopped_provisional_navigation_preserves_document(Some((500, "text/html"))).await;
}

async fn assert_stopped_provisional_navigation_preserves_document(
    response_head: Option<(u16, &'static str)>,
) {
    let request_received = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_response = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_received = request_received.clone();
    let handler_release = release_response.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/held",
                axum::routing::get(move || {
                    let received = handler_received.clone();
                    let release = handler_release.clone();
                    async move {
                        received.notify_one();
                        let (status, content_type) = if let Some(head) = response_head {
                            head
                        } else {
                            release.notified().await;
                            return axum::http::Response::builder()
                                .header("content-type", "text/html")
                                .body(axum::body::Body::from("<body>cancelled response</body>"))
                                .unwrap();
                        };
                        let body = futures_util::stream::once(async move {
                            release.notified().await;
                            Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(
                                b"<body>cancelled response</body>",
                            ))
                        });
                        axum::http::Response::builder()
                            .status(status)
                            .header("content-type", content_type)
                            .body(axum::body::Body::from_stream(body))
                            .unwrap()
                    }
                }),
            )
            .route(
                "/ready",
                axum::routing::get(|| async {
                    axum::response::Html("<body>subsequent document</body>")
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let initial_url = "data:text/html,<body>previous committed document</body>";
    ctx.install_navigation_fixture_for_session_owner(initial_url, Some("SID-1"))
        .await;
    wait_until_renderer_document_load(&mut ctx, Some("SID-1"), "TID-1", LOADER_ID).await;
    for (id, method) in [
        (910, "Page.enable"),
        (911, "DOM.enable"),
        (912, "Network.enable"),
        (913, "Runtime.enable"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": "SID-1"
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    let previous_html = loaded_page_html_for_test(&mut ctx).await;
    ctx.sent.clear();
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_async(json!({
                "id": 914,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/held") }
            }))
            .await;
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                request_received.notified(),
            )
            .await
            .expect("the ordinary HTTP navigation should reach the held response");
            if let Some((status, _)) = response_head {
                wait_until_scheduler_message(
                    &mut ctx,
                    "held document response headers",
                    |message| {
                        message["method"] == json!("Network.responseReceived")
                            && message["params"]["type"] == json!("Document")
                            && message["params"]["response"]["status"] == json!(status)
                    },
                )
                .await;
            } else {
                assert!(ctx.sent.iter().all(|message| message["id"] != json!(914)));
            }

            ctx.process_and_wait_for_response_async(json!({
                "id": 915,
                "method": "Page.stopLoading",
                "sessionId": "SID-1"
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 915)["result"], json!({}));
            wait_until_scheduler_message(&mut ctx, "cancelled navigation response", |message| {
                message["id"] == json!(914)
            })
            .await;
            let navigation = take_response_by_id(&mut ctx, 914);
            assert_eq!(navigation["result"]["frameId"], json!("TID-1"));
            if response_head.is_some_and(|(status, _)| status == 200) {
                // Successful response headers acknowledge navigation before
                // XML buffering completes; cancellation must not send a
                // second reply or replace the still-committed old Document.
                assert!(navigation["result"].get("errorText").is_none());
            } else {
                assert_eq!(navigation["result"]["errorText"], json!("net::ERR_ABORTED"));
            }
            wait_until_scheduler_message(
                &mut ctx,
                "cancelled navigation network event",
                |message| {
                    message["method"] == json!("Network.loadingFailed")
                        && message["params"]["errorText"] == json!("net::ERR_ABORTED")
                },
            )
            .await;
            let failure = ctx
                .sent
                .iter()
                .find(|message| message["method"] == json!("Network.loadingFailed"))
                .expect("cancelled request failure");
            assert_eq!(failure["params"]["canceled"], json!(true));
            assert_eq!(failure["params"]["type"], json!("Document"));
            assert!(ctx.sent.iter().all(|message| message["id"] != json!(914)));
            assert_eq!(loaded_page_html_for_test(&mut ctx).await, previous_html);
            assert_eq!(
                ctx.conn.browser_context.as_ref().unwrap().target_url(),
                initial_url
            );
            for method in [
                "Page.frameNavigated",
                "DOM.documentUpdated",
                "Runtime.executionContextsCleared",
                "Runtime.executionContextCreated",
                "Page.domContentEventFired",
                "Page.loadEventFired",
            ] {
                assert!(
                    ctx.sent
                        .iter()
                        .all(|message| message["method"] != json!(method)),
                    "cancelled pre-response navigation must not replace the document: {method}"
                );
            }

            // The response remains held until cancellation has completed. No
            // server delay or timing race can masquerade as transport abort.
            release_response.notify_one();
            ctx.sent.clear();
            ctx.process_and_wait_for_response_async(json!({
                "id": 916,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/ready") }
            }))
            .await;
            let navigation = take_response_by_id(&mut ctx, 916);
            assert!(navigation["result"].get("errorText").is_none());
            wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
            let html = loaded_page_html_for_test(&mut ctx).await;
            assert!(html.contains("subsequent document"));
            assert!(!html.contains("cancelled response"));
            assert_eq!(
                ctx.conn.browser_context.as_ref().unwrap().target_url(),
                format!("http://{addr}/ready")
            );
        })
        .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_after_commit_cancels_transport_without_replacing_partial_document() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let prefix_parsed = std::sync::Arc::new(tokio::sync::Notify::new());
    let transport_closed = std::sync::Arc::new(tokio::sync::Notify::new());
    let server_parsed = prefix_parsed.clone();
    let server_closed = transport_closed.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let parsed = server_parsed.clone();
            let closed = server_closed.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    if socket.read(&mut byte).await.unwrap() == 0 {
                        return;
                    }
                    request.push(byte[0]);
                }
                if request.starts_with(b"GET /stream ") {
                    let prefix = b"<!doctype html><body><main id='prefix'>committed prefix</main><script>fetch('/parsed')</script>";
                    let tail = b"<main id='tail'>unreceived tail</main></body>";
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        prefix.len() + tail.len()
                    );
                    socket.write_all(headers.as_bytes()).await.unwrap();
                    socket.write_all(prefix).await.unwrap();
                    socket.flush().await.unwrap();
                    // The server never supplies EOF or the tail. Completion
                    // here can only come from the client's transport close.
                    let read = socket.read(&mut byte).await;
                    assert!(
                        matches!(read, Ok(0)) || read.is_err(),
                        "client must close the held transfer: {read:?}"
                    );
                    closed.notify_one();
                } else if request.starts_with(b"GET /parsed ") {
                    socket
                        .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                    parsed.notify_one();
                } else {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 26\r\nConnection: close\r\n\r\n<body>complete page</body>").await.unwrap();
                }
            });
        }
    });
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_and_wait_for_response_async(json!({
                "id": 920, "method": "Page.navigate", "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/stream") }
            }))
            .await;
            assert!(
                take_response_by_id(&mut ctx, 920)["result"]
                    .get("errorText")
                    .is_none()
            );
            wait_until_scheduler_message(&mut ctx, "streaming document commit", |message| {
                message["method"] == json!("Page.frameNavigated")
            })
            .await;
            tokio::time::timeout(std::time::Duration::from_secs(5), prefix_parsed.notified())
                .await
                .expect("committed prefix script must execute before stopping");
            assert!(
                loaded_page_html_for_test(&mut ctx)
                    .await
                    .contains("committed prefix")
            );
            ctx.sent.clear();
            ctx.process_and_wait_for_response_async(json!({
                "id": 921, "method": "Page.stopLoading", "sessionId": "SID-1"
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 921)["result"], json!({}));
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                transport_closed.notified(),
            )
            .await
            .expect("explicit document stop must cancel the committed response transport");
            let html = loaded_page_html_for_test(&mut ctx).await;
            assert!(html.contains("committed prefix"));
            assert!(!html.contains("unreceived tail"));
            assert_eq!(
                ctx.conn.browser_context.as_ref().unwrap().target_url(),
                format!("http://{addr}/stream")
            );
            assert!(
                ctx.sent
                    .iter()
                    .all(|message| message["method"] != json!("Page.frameNavigated"))
            );

            ctx.sent.clear();
            ctx.process_and_wait_for_response_async(json!({
                "id": 922, "method": "Page.navigate", "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/complete") }
            }))
            .await;
            assert!(
                take_response_by_id(&mut ctx, 922)["result"]
                    .get("errorText")
                    .is_none()
            );
            wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
            let completed_html = loaded_page_html_for_test(&mut ctx).await;
            assert!(completed_html.contains("complete page"));
            ctx.process_and_wait_for_response_async(json!({
                "id": 923, "method": "Page.stopLoading", "sessionId": "SID-1"
            }))
            .await;
            assert_eq!(take_response_by_id(&mut ctx, 923)["result"], json!({}));
            assert_eq!(loaded_page_html_for_test(&mut ctx).await, completed_html);
        })
        .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_reloads_current_url_and_returns_empty_result() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn page(
        axum::extract::State(counter): axum::extract::State<Arc<AtomicUsize>>,
    ) -> impl axum::response::IntoResponse {
        let next = counter.fetch_add(1, Ordering::SeqCst) + 1;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{next}</body></html>"),
        )
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_counter = counter.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .with_state(server_counter),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 239,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let navigate = take_response_by_id(&mut ctx, 239);
    assert_eq!(
        navigate["result"],
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID })
    );

    let first_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        first_html.contains(">1<"),
        "expected first navigation html to contain counter 1, got {first_html}"
    );

    ctx.process_async(json!({
        "id": 241,
        "method": "Page.reload",
        "sessionId": "SID-1"
    }))
    .await;
    let reload = take_response_by_id(&mut ctx, 241);
    assert_eq!(reload["result"], json!({}));

    let second_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        second_html.contains(">2<"),
        "expected reloaded html to contain counter 2, got {second_html}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_csp_controls_response_policy_across_reload() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("content-security-policy", "script-src 'none'"),
            ],
            r#"<!doctype html>
<title>CSP Locked</title>
<script>
globalThis.__cspRan = true;
document.title = "CSP Bypassed";
</script>"#,
        )
    }

    async fn evaluate_csp_state(ctx: &mut TestContext, id: u64) -> serde_json::Value {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-CSP",
            "params": {
                "expression": "[globalThis.__cspRan === true, document.title]",
                "returnByValue": true
            }
        }))
        .await;
        take_response_by_id(ctx, id)["result"]["result"]["value"].clone()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/csp", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-CSP", "TID-CSP", "SID-CSP", "about:blank");
    ctx.process_async(json!({
        "id": 24_000,
        "method": "Page.enable",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_000);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_001,
        "method": "Page.navigate",
        "sessionId": "SID-CSP",
        "params": { "url": format!("http://{addr}/csp") }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_001);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_002).await,
        json!([false, "CSP Locked"])
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_003,
        "method": "Page.setBypassCSP",
        "sessionId": "SID-CSP",
        "params": { "enabled": true }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_003);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 24_004,
        "method": "Page.reload",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_004);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_005).await,
        json!([true, "CSP Bypassed"])
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_006,
        "method": "Page.setBypassCSP",
        "sessionId": "SID-CSP",
        "params": { "enabled": false }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_006);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 24_007,
        "method": "Page.reload",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_007);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_008).await,
        json!([false, "CSP Locked"])
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_response_csp_controls_parser_and_dynamic_inline_scripts() {
    async fn parent() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><iframe src="/child"></iframe>"#,
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("content-security-policy", "script-src 'nonce-allowed'"),
            ],
            r#"<!doctype html><body>
<script nonce="allowed">
globalThis.__allowedParserInlineRan = true;
globalThis.__inlineCspViolations = [];
addEventListener("securitypolicyviolation", event => {
  __inlineCspViolations.push(`${event.effectiveDirective}:${event.disposition}`);
});
</script>
<script>globalThis.__blockedParserInlineRan = true;</script>
<script nonce="allowed">
const script = document.createElement("script");
script.textContent = "globalThis.__blockedDynamicInlineRan = true";
document.body.append(script);
const allowedScript = document.createElement("script");
allowedScript.nonce = "allowed";
allowedScript.textContent = "globalThis.__allowedDynamicInlineRan = true";
document.body.append(allowedScript);
allowedScript.nonce = "changed-after-connection";
</script>
</body>"#,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(parent))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-CHILD-CSP",
        "TID-CHILD-CSP",
        "SID-CHILD-CSP",
        "about:blank",
    );
    ctx.process_async(json!({
        "id": 24_100,
        "method": "Page.enable",
        "sessionId": "SID-CHILD-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_100);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_101,
        "method": "Page.navigate",
        "sessionId": "SID-CHILD-CSP",
        "params": { "url": format!("http://{addr}/parent") }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_101);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CHILD-CSP").await;
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 24_102).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_103,
        "method": "Runtime.enable",
        "sessionId": "SID-CHILD-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_103);
    let child_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_104,
        "method": "Runtime.evaluate",
        "sessionId": "SID-CHILD-CSP",
        "params": {
            "contextId": child_context_id,
            "expression": "[globalThis.__blockedParserInlineRan === true, globalThis.__allowedParserInlineRan === true, globalThis.__blockedDynamicInlineRan === true, globalThis.__allowedDynamicInlineRan === true, globalThis.__inlineCspViolations]",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 24_104)["result"]["result"]["value"],
        json!([
            false,
            true,
            false,
            true,
            ["script-src-elem:enforce", "script-src-elem:enforce"]
        ])
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_targets_background_owner_without_activation() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn page(
        axum::extract::State(counter): axum::extract::State<Arc<AtomicUsize>>,
    ) -> impl axum::response::IntoResponse {
        let next = counter.fetch_add(1, Ordering::SeqCst) + 1;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{next}</body></html>"),
        )
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_counter = counter.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .with_state(server_counter),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let page_url = format!("http://{addr}/page");
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        page_url.clone(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<title>Active</title><main>active</main>".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
        .await;
    let initial_html = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|browser_context| {
            browser_context
                .background_target_mut("TID-background")
                .and_then(PageTargetHost::loaded_page_mut)
        })
        .expect("loaded background page")
        .serialize_html_async()
        .await
        .expect("background page should serialize HTML");
    assert!(
        initial_html.contains(">1<"),
        "expected first background load to contain counter 1"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 242,
        "method": "Page.reload",
        "sessionId": "SID-background"
    }))
    .await;
    let reload = take_response_by_id(&mut ctx, 242);
    assert_eq!(reload["sessionId"], json!("SID-background"));
    assert_eq!(reload["result"], json!({}));

    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Page.reload should not activate the target"
    );
    let background = browser_context
        .background_target_mut("TID-background")
        .expect("background target should remain background");
    let reloaded_html = background
        .loaded_page_mut()
        .expect("loaded background page after reload")
        .serialize_html_async()
        .await
        .expect("background page should serialize HTML");
    assert!(
        reloaded_html.contains(">2<"),
        "expected background reload html to contain counter 2, got {reloaded_html}"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn repeated_http_navigation_after_runtime_enable_replaces_the_context_group() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>runtime context replacement</body></html>",
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
                axum::routing::get(move || {
                    let request_count = server_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        page().await
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

    ctx.process_async(json!({
        "id": 404,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url.clone() }
    }))
    .await;
    let first_loader_id = take_response_by_id(&mut ctx, 404)["result"]["loaderId"]
        .as_str()
        .expect("first HTTP navigation should have a loader")
        .to_owned();
    // Page.navigate acknowledges the navigation before the renderer reaches
    // load. Chromium clients wait for a lifecycle event when they need a
    // fully completed document; reading readyState immediately after the
    // command response is allowed to observe "interactive".
    wait_until_renderer_document_load(&mut ctx, Some("SID-1"), "TID-1", &first_loader_id).await;
    ctx.process_async(json!({
        "id": 4041,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.readyState",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 4041)["result"]["result"]["value"],
        json!("complete"),
        "the benchmark enables Runtime only after the first document has completed"
    );
    let state_before_enable = ctx
        .conn
        .navigation_load_inputs_for_session_owner(Some("SID-1"))
        .runtime_inspector_session_restore_snapshots
        .into_iter()
        .find(|restore| restore.inspector_session_id.is_none())
        .and_then(|restore| restore.v8_attach.reattach_state().cloned())
        .expect("Runtime.evaluate should establish the primary V8 session state");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 405,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "Runtime.enable should report the first HTTP document context: {:?}",
        ctx.sent
    );
    let state_after_enable = ctx
        .conn
        .navigation_load_inputs_for_session_owner(Some("SID-1"))
        .runtime_inspector_session_restore_snapshots
        .into_iter()
        .find(|restore| restore.inspector_session_id.is_none())
        .and_then(|restore| restore.v8_attach.reattach_state().cloned())
        .expect("Runtime.enable should retain the primary V8 session state");
    assert_ne!(
        state_before_enable, state_after_enable,
        "Runtime.enable must replace the pre-enable V8 session cookie"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 406,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    let second_loader_id = take_response_by_id(&mut ctx, 406)["result"]["loaderId"]
        .as_str()
        .expect("second HTTP navigation should have a loader")
        .to_owned();

    assert_ne!(first_loader_id, second_loader_id);
    assert_eq!(
        request_count.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "same-URL Page.navigate should fetch and commit a replacement document"
    );
    assert_runtime_navigation_context_reset(&ctx.sent, "SID-1", "TID-1");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_after_crash_emits_target_reloaded_after_crash() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>reload-after-crash</body>",
    );
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();
    bc.active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 248,
        "method": "Page.reload",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 248);

    let events = ctx.take_all();
    assert!(events.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_crash_emits_target_reloaded_after_crash() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>before-crash</body>",
    );
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();
    bc.active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 249,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>after-crash</body>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 249);

    let events = ctx.take_all();
    assert!(events.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
    assert_eq!(
        loaded_page_html_for_test(&mut ctx).await,
        "<html><head></head><body>after-crash</body></html>"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_crash_without_inspector_enabled_clears_crash_without_event() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>before-crash</body>",
    );
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 250,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>after-crash</body>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 250);

    let events = ctx.take_all();
    assert!(
        !events
            .iter()
            .any(|message| message["method"] == json!("Inspector.targetReloadedAfterCrash"))
    );
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_script_location_navigation_suppresses_aborted_document_dcl() {
    async fn challenge() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><script>window.location.href = '/final'</script>",
        )
    }

    async fn final_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Final document</title><main>final content</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/challenge", axum::routing::get(challenge))
                .route("/final", axum::routing::get(final_page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 251,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/challenge") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 251);
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "successor document lifecycle after parser-script navigation",
        |messages| {
            messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameNavigated"))
                .count()
                >= 2
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.domContentEventFired"))
        },
    )
    .await;

    let events = ctx.take_all();
    let domcontentloaded = events
        .iter()
        .filter(|message| message["method"] == json!("Page.domContentEventFired"))
        .collect::<Vec<_>>();
    assert_eq!(
        domcontentloaded.len(),
        1,
        "the synchronously aborted challenge document must not emit DCL: {events:?}"
    );
    let final_frame_commit_index = events
        .iter()
        .rposition(|message| message["method"] == json!("Page.frameNavigated"))
        .expect("final document frame commit");
    let domcontentloaded_index = events
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .expect("final document DCL");
    assert!(
        final_frame_commit_index < domcontentloaded_index,
        "the only DCL must belong to the final document: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        format!("http://{addr}/final")
    );
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("final content")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_script_location_navigation_continues_after_target_response() {
    async fn challenge() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><script>window.location.href = '/final'</script>",
        )
    }

    let final_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let final_release = std::sync::Arc::new(tokio::sync::Notify::new());
    let requested_for_handler = std::sync::Arc::clone(&final_requested);
    let release_for_handler = std::sync::Arc::clone(&final_release);
    let final_handler = move || {
        let requested = std::sync::Arc::clone(&requested_for_handler);
        let release = std::sync::Arc::clone(&release_for_handler);
        async move {
            requested.notify_one();
            release.notified().await;
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><title>Delayed final</title><main>delayed final content</main>",
            )
        }
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/challenge", axum::routing::get(challenge))
                .route("/final", axum::routing::get(final_handler)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 252,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/challenge") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 252);

    // Chromium returns Page.navigate once the new document commits. Parser
    // continuation (and therefore this script navigation) is admitted only
    // after that response boundary has been flushed.
    let release_response = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            final_requested.notified(),
        )
        .await
        .expect("successor navigation should request the gated final response");
        final_release.notify_one();
    });
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "response-gated successor document lifecycle",
        |messages| {
            messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameNavigated"))
                .count()
                >= 2
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.domContentEventFired"))
        },
    )
    .await;
    release_response
        .await
        .expect("gated final-response release task");

    let events = ctx.take_all();
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.domContentEventFired"))
            .count(),
        1,
        "the response-gated source document must stay terminated before DCL: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        format!("http://{addr}/final")
    );

    server.abort();
}
