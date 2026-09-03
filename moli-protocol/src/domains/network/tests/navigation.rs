use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_updates_live_request_overrides_for_current_page() {
    async fn handler(
        State(seen): State<Arc<Mutex<Option<(String, String)>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let test_header = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        *seen.lock() = Some((user_agent, test_header));
        ([(ACCESS_CONTROL_ALLOW_ORIGIN.as_str(), "*")], "ok")
    }

    let seen = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/api", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>ok</body>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 341,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(341, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 342,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "async-network-header" } }
    }))
    .await;
    ctx.expect_result(342, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 343,
        "method": "Network.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": { "userAgent": "async-network-ua" }
    }))
    .await;
    ctx.expect_result(343, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 344,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "navigator.userAgent" }
    }))
    .await;
    let user_agent_response = ctx.take_response_by_id(344);
    assert_eq!(
        user_agent_response["result"]["result"]["value"],
        json!("async-network-ua")
    );

    ctx.process_async(json!({
        "id": 345,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!("fetch('http://{addr}/api').then(r => r.text())")
        }
    }))
    .await;

    flush_until_subresource_finished(
        &mut ctx,
        "Fetch",
        1,
        "runtime fetch after async network override update",
    )
    .await;

    assert_eq!(
        seen.lock().as_ref(),
        Some(&(
            "async-network-ua".to_owned(),
            "async-network-header".to_owned()
        ))
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_offline_runtime_fetch_emits_loading_failed() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><body>offline</body></html>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 361,
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
    ctx.expect_result(361, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 362,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(362, json!({}), Some("SID-1"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 363,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "fetch('http://example.test/api').catch(e => e && String(e))"
        }
    }))
    .await;

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
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_emits_network_events_and_captures_response_body() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html"), ("x-moli", "ok")],
            "<!doctype html><html><body><main>network body</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "main-document network completion",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Network.loadingFinished"))
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    consume_main_document_navigation_start(&mut ctx);

    let request = ctx.take_one();
    assert_eq!(request["method"], "Network.requestWillBeSent");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["loaderId"], LOADER_ID);
    assert_eq!(request["params"]["frameId"], "TID-1");
    assert_eq!(request["params"]["type"], "Document");
    assert_eq!(request["params"]["request"]["url"], url);
    assert_eq!(request["params"]["request"]["method"], "GET");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    let remaining = ctx.take_all();
    let observable_order = remaining
        .iter()
        .map(|message| {
            if message["id"] == json!(2) {
                "Page.navigate result"
            } else {
                message["method"].as_str().expect("protocol event method")
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        observable_order,
        [
            "Network.requestWillBeSentExtraInfo",
            "Page.navigate result",
            "Network.responseReceivedExtraInfo",
            "Network.responseReceived",
            "Page.frameNavigated",
            "DOM.documentUpdated",
            "DOM.documentUpdated",
            "Page.domContentEventFired",
            "Network.dataReceived",
            "Network.loadingFinished",
            "Page.loadEventFired",
            "Page.frameStoppedLoading",
        ],
        "main-document completion should retain stable ExtraInfo and lifecycle order"
    );
    let result = remaining
        .iter()
        .find(|message| message["id"] == json!(2))
        .expect("Page.navigate result");
    assert_eq!(result["id"], 2);
    assert_eq!(result["sessionId"], "SID-1");

    let request_extra_index = remaining
        .iter()
        .position(|message| message["method"] == json!("Network.requestWillBeSentExtraInfo"))
        .expect("request extra info event");
    let request_extra_info = &remaining[request_extra_index];
    assert_eq!(
        request_extra_info["method"],
        "Network.requestWillBeSentExtraInfo"
    );
    assert_eq!(request_extra_info["sessionId"], "SID-1");
    assert_eq!(request_extra_info["params"]["requestId"], request_id);
    assert_eq!(request_extra_info["params"]["associatedCookies"], json!([]));

    let response_extra_index = remaining
        .iter()
        .position(|message| message["method"] == json!("Network.responseReceivedExtraInfo"))
        .expect("response extra info event");
    let response_extra_info = &remaining[response_extra_index];
    assert_eq!(
        response_extra_info["method"],
        "Network.responseReceivedExtraInfo"
    );
    assert_eq!(response_extra_info["sessionId"], "SID-1");
    assert_eq!(response_extra_info["params"]["requestId"], request_id);
    assert_eq!(response_extra_info["params"]["statusCode"], 200);
    assert_eq!(response_extra_info["params"]["headers"]["x-moli"], "ok");
    assert_eq!(response_extra_info["params"]["blockedCookies"], json!([]));

    let response_index = remaining
        .iter()
        .position(|message| message["method"] == json!("Network.responseReceived"))
        .expect("response event");
    let response = &remaining[response_index];
    assert!(request_extra_index < response_extra_index);
    assert!(response_extra_index < response_index);
    assert_eq!(response["method"], "Network.responseReceived");
    assert_eq!(response["sessionId"], "SID-1");
    assert_eq!(response["params"]["requestId"], request_id);
    assert_eq!(response["params"]["loaderId"], LOADER_ID);
    assert_eq!(response["params"]["frameId"], "TID-1");
    assert_eq!(response["params"]["type"], "Document");
    assert_eq!(response["params"]["response"]["url"], url);
    assert_eq!(response["params"]["response"]["status"], 200);
    assert_eq!(
        response["params"]["response"]["headers"]["content-type"],
        "text/html"
    );
    assert_eq!(response["params"]["hasExtraInfo"], json!(true));
    assert_eq!(response["params"]["response"]["headers"]["x-moli"], "ok");

    for method in [
        "Page.frameNavigated",
        "DOM.documentUpdated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.frameStoppedLoading",
    ] {
        assert!(
            remaining
                .iter()
                .any(|message| message["method"] == json!(method)),
            "missing {method}"
        );
    }

    let data_received = remaining
        .iter()
        .find(|message| message["method"] == json!("Network.dataReceived"))
        .expect("dataReceived event");
    assert_eq!(data_received["method"], "Network.dataReceived");
    assert_eq!(data_received["sessionId"], "SID-1");
    assert_eq!(data_received["params"]["requestId"], request_id);
    assert!(
        data_received["params"]["dataLength"]
            .as_u64()
            .is_some_and(|len| len > 0)
    );

    let loading_finished = remaining
        .iter()
        .find(|message| message["method"] == json!("Network.loadingFinished"))
        .expect("loadingFinished event");
    assert_eq!(loading_finished["method"], "Network.loadingFinished");
    assert_eq!(loading_finished["sessionId"], "SID-1");
    assert_eq!(loading_finished["params"]["requestId"], request_id);
    assert!(
        loading_finished["params"]["encodedDataLength"]
            .as_u64()
            .is_some_and(|len| len > 0)
    );

    ctx.process_async(json!({
        "id": 3,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        3,
        json!({
            "body": "<!doctype html><html><body><main>network body</main></body></html>",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_applies_extra_http_headers() {
    async fn handler(headers: axum::http::HeaderMap) -> impl IntoResponse {
        let received = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body><main>{received}</main></body></html>"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works" } }
    }))
    .await;
    ctx.expect_result(2, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let request = ctx.take_first_matching("main document request", |message| {
        message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["request"]["url"] == json!(url)
    });
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "works"
    );
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 4,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        4,
        json!({
            "body": "<!doctype html><html><body><main>works</main></body></html>",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_events_include_synthesized_cookie_header() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>cookie-doc</main></body></html>",
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
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&page_url).unwrap(),
            &[("set-cookie".to_owned(), "sid=1; Path=/page".to_owned())],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 4_1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(4_1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 4_2,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-doc" } }
    }))
    .await;
    ctx.expect_result(4_2, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 4_3,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let messages = ctx.take_all();
    let request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Document")
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .expect("document request should emit requestWillBeSent");
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "works-doc"
    );
    assert_eq!(request["params"]["request"]["headers"]["Cookie"], "sid=1");

    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("document request id")
        .to_owned();
    let extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("document request should emit requestWillBeSentExtraInfo");
    assert_eq!(extra_info["params"]["headers"]["x-cdp-test"], "works-doc");
    assert_eq!(extra_info["params"]["headers"]["Cookie"], "sid=1");

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_deduplicates_manual_cookie_header_when_store_supplies_one() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 8192];
        let read = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..read]);
        *server_seen.lock() = request
            .lines()
            .filter(|line| line.to_ascii_lowercase().starts_with("cookie:"))
            .map(str::to_owned)
            .collect();

        let body = "<!doctype html><html><body><main>ok</main></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        let _ = stream.shutdown().await;
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&page_url).unwrap(),
            &[("set-cookie".to_owned(), "sid=store; Path=/page".to_owned())],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 4_3,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(4_3, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 4_4,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "Cookie": "manual=2", "x-cdp-test": "works-doc" } }
    }))
    .await;
    ctx.expect_result(4_4, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 4_5,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let request = ctx.take_one();
    assert_eq!(request["method"], "Network.requestWillBeSent");
    assert_eq!(
        request["params"]["request"]["headers"]["Cookie"],
        "sid=store"
    );
    assert_eq!(
        request["params"]["request"]["headers"]["x-cdp-test"],
        "works-doc"
    );
    let _ = ctx.take_all();

    assert_eq!(seen.lock().as_slice(), ["Cookie: sid=store"]);

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_redirect_emits_second_request_with_redirect_response() {
    async fn final_page() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/html"),
                ("set-cookie", "reply=1; SameSite=None"),
            ],
            "<!doctype html><html><body><main>redirected</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://localhost:{}/start", addr.port());
    let final_url = format!("http://127.0.0.1:{}/final", addr.port());
    let redirect_target = final_url.clone();
    let server = tokio::spawn(async move {
        let redirect_target = redirect_target.clone();
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/start",
                    get(move || {
                        let redirect_target = redirect_target.clone();
                        async move {
                            axum::http::Response::builder()
                                .status(StatusCode::TEMPORARY_REDIRECT)
                                .header(LOCATION, redirect_target)
                                .header("set-cookie", "redir=1; Path=/")
                                .body(axum::body::Body::empty())
                                .unwrap()
                        }
                    }),
                )
                .route("/final", get(final_page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.set_target_url(format!("http://localhost:{}/origin", addr.port()));
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&final_url).unwrap(),
            &[(
                "set-cookie".to_owned(),
                "strict=1; Path=/; SameSite=Strict".to_owned(),
            )],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": start_url }
    }))
    .await;

    let first_request = ctx.take_first_matching("initial redirect request", |message| {
        message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["request"]["url"] == json!(start_url)
    });
    assert_eq!(first_request["params"]["request"]["url"], start_url);
    let request_id = first_request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    let redirected_request = ctx.take_first_matching("redirected document request", |message| {
        message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["request"]["url"] == json!(final_url)
    });
    assert_eq!(redirected_request["params"]["requestId"], request_id);
    assert_eq!(redirected_request["params"]["request"]["url"], final_url);
    assert_eq!(
        redirected_request["params"]["redirectHasExtraInfo"],
        json!(true)
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["url"],
        start_url
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["status"],
        307
    );
    assert_eq!(
        redirected_request["params"]["redirectResponse"]["headers"]["location"],
        final_url
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["cookie"]["name"],
        "strict"
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["warningReasons"],
        json!(["SameSiteContextDowngradedByRedirect"])
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextDowngradeType"],
        json!("StrictToCross")
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextDowngradeType"],
        json!("StrictToCross")
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextHttpMethod"],
        json!("GET")
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextHttpMethod"],
        json!("GET")
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextRedirectType"],
        json!("CrossSiteRedirect")
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextRedirectType"],
        json!("CrossSiteRedirect")
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContext"],
        json!("CrossSite")
    );
    assert_eq!(
        redirected_request["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContext"],
        json!("CrossSite")
    );

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "redirected main document lifecycle and network completion",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Page.domContentEventFired"))
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.loadEventFired"))
                && messages.iter().any(|message| {
                    message["method"] == json!("Network.loadingFinished")
                        && message["params"]["requestId"] == json!(request_id)
                })
                && messages.iter().any(|message| {
                    message["method"] == json!("Page.frameStoppedLoading")
                        && message["params"]["frameId"] == json!("TID-1")
                })
        },
    )
    .await;

    let remaining = ctx.take_all();
    let redirect_extra_info = remaining
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["statusCode"] == json!(307)
        })
        .expect("redirect hop should emit responseReceivedExtraInfo");
    assert_eq!(
        redirect_extra_info["params"]["cookieReports"][0]["status"]["kind"],
        json!("Accepted")
    );
    let extra_info = remaining
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["requestId"] == redirected_request["params"]["requestId"]
                && message["params"]["cookieAccessReport"]["excludedCookies"][0]["cookie"]["name"]
                    == json!("strict")
        })
        .expect("redirected request should emit requestWillBeSentExtraInfo");
    assert_eq!(
        extra_info["params"]["cookieAccessReport"]["excludedCookies"][0]["cookie"]["name"],
        "strict"
    );

    let result = remaining
        .iter()
        .find(|message| message["id"] == json!(2))
        .expect("page navigate result");
    assert_eq!(result["sessionId"], "SID-1");

    let response = remaining
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("network response event");
    assert_eq!(response["params"]["response"]["url"], final_url);
    assert_eq!(response["params"]["hasExtraInfo"], json!(true));

    let response_extra_info = remaining
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["statusCode"] == json!(200)
        })
        .expect("network response should emit responseReceivedExtraInfo");
    assert_eq!(
        response_extra_info["params"]["cookieReports"][0]["status"]["kind"],
        json!("Rejected")
    );
    assert_eq!(
        response_extra_info["params"]["cookieReports"][0]["status"]["reason"],
        json!("SameSiteNoneRequiresSecure")
    );

    assert!(
        remaining
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        remaining
            .iter()
            .any(|message| message["method"] == json!("DOM.documentUpdated"))
    );
    assert!(
        remaining
            .iter()
            .any(|message| { message["method"] == json!("Page.domContentEventFired") })
    );
    assert!(
        remaining
            .iter()
            .any(|message| message["method"] == json!("Page.loadEventFired"))
    );
    assert!(remaining.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }));
    assert!(remaining.iter().any(|message| {
        message["method"] == json!("Page.frameStoppedLoading")
            && message["params"]["frameId"] == json!("TID-1")
    }));

    ctx.process_async(json!({
        "id": 3,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        3,
        json!({
            "body": "<!doctype html><html><body><main>redirected</main></body></html>",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn main_document_critical_client_hint_restart_matches_chromium_network_chain() {
    type ServerState = Arc<Mutex<Vec<bool>>>;

    async fn page(State(seen_arch): State<ServerState>, headers: HeaderMap) -> impl IntoResponse {
        let has_arch = headers.contains_key("sec-ch-ua-arch");
        let first_request = {
            let mut seen_arch = seen_arch.lock();
            seen_arch.push(has_arch);
            seen_arch.len() == 1
        };
        if first_request {
            axum::http::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header(CONTENT_TYPE, "text/html")
                .header("accept-ch", "Sec-CH-UA-Arch")
                .header("critical-ch", "Sec-CH-UA-Arch")
                .body(axum::body::Body::from("discarded challenge"))
                .unwrap()
        } else {
            axum::http::Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/html")
                .body(axum::body::Body::from(
                    "<!doctype html><html><body>restarted</body></html>",
                ))
                .unwrap()
        }
    }

    let seen_arch = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen_arch = Arc::clone(&seen_arch);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .with_state(server_seen_arch),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 51,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(51, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 52,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "Critical-CH restarted navigation completion",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.responseReceived")
                    && message["params"]["response"]["status"] == json!(200)
            }) && messages
                .iter()
                .any(|message| message["method"] == json!("Network.loadingFinished"))
        },
    )
    .await;

    let messages = ctx.take_all();
    let requests = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Document")
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2, "unexpected event chain: {messages:#?}");
    assert_eq!(
        requests[0]["params"]["requestId"],
        requests[1]["params"]["requestId"]
    );
    assert!(requests[0]["params"]["redirectResponse"].is_null());
    assert_eq!(
        requests[1]["params"]["redirectResponse"]["status"],
        json!(307)
    );
    assert_eq!(
        requests[1]["params"]["redirectResponse"]["statusText"],
        json!("Internal Redirect")
    );
    assert_eq!(requests[1]["params"]["redirectHasExtraInfo"], json!(false));

    let request_id = requests[0]["params"]["requestId"].clone();
    let request_extra = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["requestId"] == request_id
        })
        .collect::<Vec<_>>();
    assert_eq!(
        request_extra.len(),
        2,
        "unexpected event chain: {messages:#?}"
    );
    assert!(request_extra[0]["params"]["headers"]["Sec-CH-UA-Arch"].is_null());
    assert_eq!(
        request_extra[1]["params"]["headers"]["Sec-CH-UA-Arch"],
        json!("\"x86\"")
    );

    let response_extra_statuses = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["requestId"] == request_id
        })
        .map(|message| message["params"]["statusCode"].clone())
        .collect::<Vec<_>>();
    assert_eq!(response_extra_statuses, vec![json!(403), json!(200)]);
    assert_eq!(seen_arch.lock().as_slice(), [false, true]);

    ctx.process_async(json!({
        "id": 53,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "[document.compatMode, document.doctype?.name].join('|')",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        ctx.take_response_by_id(53)["result"]["result"]["value"],
        json!("CSS1Compat|html"),
        "Critical-CH restart must commit the final standards-mode response into a fresh parser"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_multi_hop_redirect_preserves_cookie_downgrade_report() {
    async fn final_page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>redirected</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let start_url = format!("http://localhost:{}/start", addr.port());
    let cross_url = format!("http://127.0.0.1:{}/hop", addr.port());
    let final_url = format!("http://localhost:{}/final", addr.port());
    let cross_target = cross_url.clone();
    let final_target = final_url.clone();
    let server = tokio::spawn(async move {
        let cross_target = cross_target.clone();
        let final_target = final_target.clone();
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/start",
                    get(move || {
                        let cross_target = cross_target.clone();
                        async move {
                            axum::http::Response::builder()
                                .status(StatusCode::TEMPORARY_REDIRECT)
                                .header(LOCATION, cross_target)
                                .body(axum::body::Body::empty())
                                .unwrap()
                        }
                    }),
                )
                .route(
                    "/hop",
                    get(move || {
                        let final_target = final_target.clone();
                        async move {
                            axum::http::Response::builder()
                                .status(StatusCode::TEMPORARY_REDIRECT)
                                .header(LOCATION, final_target)
                                .body(axum::body::Body::empty())
                                .unwrap()
                        }
                    }),
                )
                .route("/final", get(final_page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.set_target_url(format!("http://localhost:{}/origin", addr.port()));
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&final_url).unwrap(),
            &[(
                "set-cookie".to_owned(),
                "strict=1; Path=/; SameSite=Strict".to_owned(),
            )],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 3_1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(3_1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 3_2,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": start_url }
    }))
    .await;

    let requests = ctx
        .take_all()
        .into_iter()
        .filter(|message| message["method"] == json!("Network.requestWillBeSent"))
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 3);

    let first_request = &requests[0];
    assert_eq!(first_request["params"]["request"]["url"], start_url);

    let cross_request = &requests[1];
    assert_eq!(cross_request["params"]["request"]["url"], cross_url);

    let final_request = &requests[2];
    assert_eq!(final_request["params"]["request"]["url"], final_url);
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["cookie"]["name"],
        "strict"
    );
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["warningReasons"],
        json!(["SameSiteContextDowngradedByRedirect"])
    );
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContext"],
        json!("CrossSite")
    );
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContext"],
        json!("CrossSite")
    );
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextDowngradeType"],
        json!("StrictToCross")
    );
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextDowngradeType"],
        json!("StrictToCross")
    );
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextRedirectType"],
        json!("CrossSiteRedirect")
    );
    assert_eq!(
        final_request["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextRedirectType"],
        json!("CrossSiteRedirect")
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn browser_initiated_redirect_does_not_fabricate_renderer_navigation_requests() {
    async fn start() -> impl IntoResponse {
        axum::response::Redirect::temporary("/final")
    }

    async fn final_page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>redirected</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/start", get(start))
                .route("/final", get(final_page)),
        )
        .await
        .unwrap();
    });

    let start_url = format!("http://{addr}/start");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": start_url }
    }))
    .await;

    let messages = ctx.take_all();
    let scheduled = messages
        .iter()
        .filter(|message| message["method"] == json!("Page.frameScheduledNavigation"))
        .count();
    let requested = messages
        .iter()
        .filter(|message| message["method"] == json!("Page.frameRequestedNavigation"))
        .count();
    let cleared = messages
        .iter()
        .filter(|message| message["method"] == json!("Page.frameClearedScheduledNavigation"))
        .count();

    assert_eq!(scheduled, 0);
    assert_eq!(requested, 0);
    assert_eq!(cleared, 0);
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["method"] == json!("Page.frameStartedNavigating"))
            .count(),
        1
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_request_cookie_report_is_snapshotted_before_response_sets_cookie() {
    async fn page() -> impl IntoResponse {
        (
            [
                (SET_COOKIE.as_str(), "sid=1; Path=/"),
                (CONTENT_TYPE.as_str(), "text/html"),
            ],
            "<!doctype html><html><body>cookie response</body></html>",
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
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 401,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(401, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 402,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let messages = ctx.take_all();
    let request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Document")
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .expect("document request should emit requestWillBeSent");
    assert!(
        request["params"]["cookieAccessReport"].is_null(),
        "request-time diagnostics must not see cookies set by the response itself"
    );

    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let response_extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["statusCode"] == json!(200)
        })
        .expect("response should emit responseReceivedExtraInfo");
    assert_eq!(
        response_extra_info["params"]["cookieReports"][0]["status"]["kind"],
        json!("Accepted")
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn first_main_document_transport_failure_commits_error_document() {
    let (addr, failing_server) = spawn_connection_drop_server().await;

    let url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    assert!(bc.assign_attached_session_to_target("TID-1", "SID-2".to_owned()));
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_page_events_for_test(Some("SID-2"));

    for (id, session_id) in [(1, "SID-1"), (11, "SID-2")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Network.enable",
            "sessionId": session_id
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "first error Document load",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    let messages = ctx.take_all();
    let request_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!("SID-1")
        })
        .unwrap_or_else(|| panic!("missing document request: {messages:?}"));
    let request = &messages[request_index];
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["frameId"], "TID-1");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    let failed_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["requestId"] == json!(request_id)
        })
        .unwrap_or_else(|| panic!("missing document loadingFailed: {messages:?}"));
    let failed = &messages[failed_index];
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], request_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["canceled"], false);
    assert!(
        failed["params"]["errorText"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );

    let response = messages
        .iter()
        .find(|message| message["id"] == json!(2))
        .expect("Page.navigate response");
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"]["frameId"], "TID-1");
    assert!(response["result"]["loaderId"].is_string());
    assert_eq!(response["result"]["isDownload"], false);
    assert_eq!(
        response["result"]["errorText"],
        failed["params"]["errorText"]
    );

    let frame_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["sessionId"] == json!("SID-1")
        })
        .unwrap_or_else(|| panic!("missing error Document frame commit: {messages:?}"));
    let finished_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["requestId"] == json!(request_id)
        })
        .unwrap_or_else(|| panic!("missing error Document loadingFinished: {messages:?}"));
    let dom_content_loaded_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Page.domContentEventFired")
                && message["sessionId"] == json!("SID-1")
        })
        .unwrap_or_else(|| panic!("missing error Document DCL: {messages:?}"));
    let load_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!("SID-1")
        })
        .unwrap_or_else(|| panic!("missing error Document load: {messages:?}"));
    assert!(
        request_index < failed_index
            && failed_index < frame_index
            && frame_index < finished_index
            && finished_index < dom_content_loaded_index
            && dom_content_loaded_index < load_index,
        "error Document event order must preserve transport/commit/lifecycle boundaries: {messages:?}"
    );
    assert_eq!(
        messages[frame_index]["params"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        messages[frame_index]["params"]["frame"]["unreachableUrl"],
        url
    );
    assert_eq!(messages[finished_index]["params"]["encodedDataLength"], 0);
    assert!(!messages.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(request_id)
    }));
    for session_id in ["SID-1", "SID-2"] {
        for method in [
            "Network.requestWillBeSent",
            "Network.loadingFailed",
            "Network.loadingFinished",
            "Page.frameNavigated",
            "Page.domContentEventFired",
            "Page.loadEventFired",
        ] {
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| {
                        message["method"] == json!(method)
                            && message["sessionId"] == json!(session_id)
                    })
                    .count(),
                1,
                "error Document {method} must fan out exactly once to {session_id}: {messages:?}"
            );
        }
    }

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify([location.href, document.title, document.readyState])",
            "returnByValue": true
        }
    }))
    .await;
    let evaluate = ctx.take_response_by_id(3);
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(format!(
            "[\"{NETWORK_ERROR_PAGE_URL}\",\"127.0.0.1\",\"complete\"]"
        ))
    );

    ctx.process_async(json!({
        "id": 31,
        "method": "Runtime.evaluate",
        "sessionId": "SID-2",
        "params": {
            "expression": "JSON.stringify([location.href, document.readyState])",
            "returnByValue": true
        }
    }))
    .await;
    let attached_evaluate = ctx.take_response_by_id(31);
    assert_eq!(
        attached_evaluate["result"]["result"]["value"],
        json!(format!("[\"{NETWORK_ERROR_PAGE_URL}\",\"complete\"]"))
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        4,
        -32000,
        "No data found for resource with given identifier",
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = ctx.take_response_by_id(5);
    assert_eq!(history["result"]["currentIndex"], json!(0));
    assert_eq!(
        history["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1
    );
    assert_eq!(history["result"]["entries"][0]["url"], url);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        url
    );

    failing_server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_failure_emits_one_failed_and_finished_terminal_pair() {
    let (addr, failing_server) = spawn_connection_drop_server().await;

    let url = format!("http://{addr}/missing");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    wait_until_messages(&mut ctx, Some("SID-1"), "error Document load", |messages| {
        messages
            .iter()
            .any(|message| message["method"] == json!("Page.loadEventFired"))
    })
    .await;
    let messages = ctx.take_all();
    assert!(!messages.iter().any(|message| {
        matches!(
            message["method"].as_str(),
            Some("Page.frameScheduledNavigation" | "Page.frameRequestedNavigation")
        )
    }));
    assert!(
        messages
            .iter()
            .any(|message| { message["method"] == json!("Page.frameStartedNavigating") })
    );
    assert!(
        messages
            .iter()
            .any(|message| { message["method"] == json!("Page.frameStartedLoading") })
    );
    assert!(
        messages
            .iter()
            .any(|message| message["method"] == json!("Network.loadingFailed"))
    );
    assert!(messages.iter().any(|message| message["id"] == json!(2)));

    let request_id = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Document")
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("main request id");
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .count(),
        1
    );
    assert!(!messages.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(request_id)
    }));
    for method in [
        "Page.frameNavigated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.frameStoppedLoading",
    ] {
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "error Document should emit {method}: {messages:?}"
        );
    }

    failing_server.abort();
}
