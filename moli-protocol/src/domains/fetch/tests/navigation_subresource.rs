use super::*;

async fn wait_for_navigation_subresource_pause(
    ctx: &mut TestContext,
    subresource_url: &str,
    description: &str,
) {
    wait_until_message(ctx, "SID-1", description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(subresource_url)
    })
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(subresource_url)
        })
        .expect("navigation subresource Fetch pause");
    network_request_announced_before_fetch_pause(ctx, paused, None);
}

fn take_http_main_document_response_after_extra_info(
    ctx: &mut TestContext,
    request_id: &str,
    status: u16,
) -> Value {
    let request_extra_info = ctx.take_one();
    assert_eq!(
        request_extra_info["method"],
        "Network.requestWillBeSentExtraInfo"
    );
    assert_eq!(request_extra_info["params"]["requestId"], json!(request_id));

    let response_extra_info = ctx.take_one();
    assert_eq!(
        response_extra_info["method"],
        "Network.responseReceivedExtraInfo"
    );
    assert_eq!(
        response_extra_info["params"]["requestId"],
        json!(request_id)
    );
    assert_eq!(response_extra_info["params"]["statusCode"], status);

    let response = ctx.take_one();
    assert_eq!(response["method"], "Network.responseReceived");
    assert_eq!(response["params"]["requestId"], json!(request_id));
    assert_eq!(response["params"]["response"]["status"], status);
    assert_eq!(response["params"]["hasExtraInfo"], true);
    response
}

#[tokio::test(flavor = "multi_thread")]
async fn request_stage_navigation_request_paused_includes_synthesized_cookie_header() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>request-stage</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    let url = format!("http://{addr}/page");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&url).unwrap(),
            &[(
                "set-cookie".to_owned(),
                "sid=req-nav; Path=/page".to_owned(),
            )],
        );
    }
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 72_101,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(72_101, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 72_102,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(
        paused["params"]["request"]["headers"]["Cookie"],
        "sid=req-nav"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_enable_persists_subresource_interception_across_navigation() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
fetch('/api', { method: 'POST', body: 'nav-payload' })
  .then(response => response.text())
  .then(text => { document.body.setAttribute('data-fetch-result', text); });
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "ok"),
            ],
            format!("nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 73,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(73, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 74,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(main_document_paused["method"], "Fetch.requestPaused");
    assert_eq!(main_document_paused["params"]["request"]["url"], page_url);
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 75,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(75, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "navigation fetch subresource should pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        },
    )
    .await;
    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("navigation-created page should preserve fetch subresource interception");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    let navigate_result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(74))
        .cloned()
        .expect("navigate result");
    assert_eq!(
        navigate_result["result"],
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID })
    );
    let initial_subresource_request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("subresource pause should already emit Network.requestWillBeSent");
    assert_eq!(
        initial_subresource_request["params"]["request"]["url"],
        api_url
    );
    assert_eq!(
        initial_subresource_request["params"]["request"]["method"],
        "POST"
    );
    assert_eq!(
        initial_subresource_request["params"]["request"]["postData"],
        "nav-payload"
    );
    let request_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .expect("subresource requestWillBeSent position");
    let pause_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["requestId"] == json!(subresource_request_id)
        })
        .expect("subresource requestPaused position");
    assert!(
        request_index < pause_index,
        "Network.requestWillBeSent must be emitted before Fetch.requestPaused for the same networkId"
    );

    ctx.process_async(json!({
        "id": 76,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(76, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued subresource should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(request["params"]["request"]["postData"], "nav-payload");

    enable_runtime_async(&mut ctx, "SID-1", 77).await;
    ctx.process_async(json!({
        "id": 78,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-fetch-result')" }
    }))
    .await;
    ctx.expect_result(
        78,
        json!({
            "result": {
                "type": "string",
                "value": "nav:nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_dom_content_loaded_fetch_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
document.addEventListener('DOMContentLoaded', () => {
  fetch('/api', { method: 'POST', body: 'domcontentloaded-nav-payload' })
    .then(response => response.text())
    .then(text => { document.body.setAttribute('data-fetch-result', text); });
});
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "domcontentloaded"),
            ],
            format!("domcontentloaded-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 740,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(740, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 741,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 742,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(742, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("DOMContentLoaded fetch should be intercepted");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 743,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(743, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued DOMContentLoaded fetch should emit request");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "domcontentloaded-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 744).await;
    ctx.process_async(json!({
        "id": 745,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-fetch-result')" }
    }))
    .await;
    ctx.expect_result(
        745,
        json!({
            "result": {
                "type": "string",
                "value": "domcontentloaded-nav:domcontentloaded-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_window_load_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
window.addEventListener('load', () => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/api');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('load-nav-payload');
});
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "load"),
            ],
            format!("load-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 746,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(746, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 747,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 748,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(748, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("load xhr should be intercepted");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 749,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(749, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued load xhr should emit request");
    assert_eq!(request["params"]["type"], "XHR");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(request["params"]["request"]["postData"], "load-nav-payload");

    enable_runtime_async(&mut ctx, "SID-1", 750).await;
    ctx.process_async(json!({
        "id": 751,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        751,
        json!({
            "result": {
                "type": "string",
                "value": "load-nav:load-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_window_post_message_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
window.addEventListener('message', () => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/api');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('postmessage-nav-payload');
}, { once: true });
window.postMessage('go', '*');
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "postmessage"),
            ],
            format!("postmessage-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 752,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(752, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 753,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 754,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(754, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("postMessage xhr should be intercepted");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 755,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(755, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued postMessage xhr should emit request");
    assert_eq!(request["params"]["type"], "XHR");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "postmessage-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 756).await;
    ctx.process_async(json!({
        "id": 757,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        757,
        json!({
            "result": {
                "type": "string",
                "value": "postmessage-nav:postmessage-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_mutation_observer_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
const observer = new MutationObserver(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/api');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('mutation-nav-payload');
  observer.disconnect();
});
observer.observe(document.body, { attributes: true });
document.body.setAttribute('data-trigger', '1');
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "mutation"),
            ],
            format!("mutation-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 758,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(758, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 759,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 760,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(760, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("mutation observer xhr should be intercepted");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 761,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(761, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued mutation xhr should emit request");
    assert_eq!(request["params"]["type"], "XHR");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "mutation-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 762).await;
    ctx.process_async(json!({
        "id": 763,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        763,
        json!({
            "result": {
                "type": "string",
                "value": "mutation-nav:mutation-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_intersection_observer_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><div id="target">watch</div><script>
const observer = new IntersectionObserver(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/api');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('intersection-nav-payload');
  observer.disconnect();
});
observer.observe(document.getElementById('target'));
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "intersection"),
            ],
            format!("intersection-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 764,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(764, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 765,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 766,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(766, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("intersection observer xhr should be intercepted");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 767,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(767, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued intersection xhr should emit request");
    assert_eq!(request["params"]["type"], "XHR");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "intersection-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 768).await;
    ctx.process_async(json!({
        "id": 769,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        769,
        json!({
            "result": {
                "type": "string",
                "value": "intersection-nav:intersection-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_set_timeout_fetch_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
setTimeout(() => {
  fetch('/api', { method: 'POST', body: 'timeout-nav-payload' })
    .then(response => response.text())
    .then(text => { document.body.setAttribute('data-fetch-result', text); });
}, 0);
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "timeout"),
            ],
            format!("timeout-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 750,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(750, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 751,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(main_document_paused["method"], "Fetch.requestPaused");
    assert_eq!(main_document_paused["params"]["request"]["url"], page_url);
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 752,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(752, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("delayed navigation fetch should still pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    let navigate_result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(751))
        .cloned()
        .expect("navigate result");
    assert_eq!(
        navigate_result["result"],
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID })
    );

    ctx.process_async(json!({
        "id": 753,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(753, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued delayed fetch should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "timeout-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 754).await;
    ctx.process_async(json!({
        "id": 755,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-fetch-result')" }
    }))
    .await;
    ctx.expect_result(
        755,
        json!({
            "result": {
                "type": "string",
                "value": "timeout-nav:timeout-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_set_interval_fetch_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
const id = setInterval(() => {
  clearInterval(id);
  fetch('/api', { method: 'POST', body: 'interval-nav-payload' })
    .then(response => response.text())
    .then(text => { document.body.setAttribute('data-fetch-result', text); });
}, 0);
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "interval"),
            ],
            format!("interval-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 756,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(756, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 757,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 758,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(758, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "setInterval navigation subresource should pause",
    )
    .await;
    let paused_events = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        paused_events.len(),
        1,
        "setInterval fetch should pause once after clearInterval"
    );
    let subresource_paused = paused_events[0].clone();
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 759,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(759, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        1,
        "setInterval fetch should emit one Network.requestWillBeSent"
    );
    let request = requests[0].clone();
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "interval-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 760).await;
    ctx.process_async(json!({
        "id": 761,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-fetch-result')" }
    }))
    .await;
    ctx.expect_result(
        761,
        json!({
            "result": {
                "type": "string",
                "value": "interval-nav:interval-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_queue_microtask_fetch_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
queueMicrotask(() => {
  fetch('/api', { method: 'POST', body: 'microtask-nav-payload' })
    .then(response => response.text())
    .then(text => { document.body.setAttribute('data-fetch-result', text); });
});
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "microtask"),
            ],
            format!("microtask-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 786,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(786, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 787,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 788,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(788, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("microtask navigation fetch should pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 789,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(789, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued microtask fetch should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "microtask-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 790).await;
    ctx.process_async(json!({
        "id": 791,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-fetch-result')" }
    }))
    .await;
    ctx.expect_result(
        791,
        json!({
            "result": {
                "type": "string",
                "value": "microtask-nav:microtask-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_promise_then_fetch_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
Promise.resolve().then(() => {
  fetch('/api', { method: 'POST', body: 'promise-nav-payload' })
    .then(response => response.text())
    .then(text => { document.body.setAttribute('data-fetch-result', text); });
});
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "promise"),
            ],
            format!("promise-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 792,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(792, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 793,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 794,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(794, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("promise navigation fetch should pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 795,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(795, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued promise fetch should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "promise-nav-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 796).await;
    ctx.process_async(json!({
        "id": 797,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-fetch-result')" }
    }))
    .await;
    ctx.expect_result(
        797,
        json!({
            "result": {
                "type": "string",
                "value": "promise-nav:promise-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_set_timeout_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
setTimeout(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('timeout-xhr-payload');
}, 0);
</script></body></html>"#,
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-xhr", "timeout"),
            ],
            format!("timeout-xhr:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");

    ctx.process_async(json!({
        "id": 756,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(756, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 757,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(main_document_paused["method"], "Fetch.requestPaused");
    assert_eq!(main_document_paused["params"]["request"]["url"], page_url);
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 758,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(758, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "delayed navigation xhr should pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        },
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
        .expect("delayed navigation xhr should still pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    let navigate_result = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(757))
        .cloned()
        .expect("navigate result");
    assert_eq!(
        navigate_result["result"],
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID })
    );

    ctx.process_async(json!({
        "id": 759,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(759, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued delayed xhr should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "timeout-xhr-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 760).await;
    ctx.process_async(json!({
        "id": 761,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        761,
        json!({
            "result": {
                "type": "string",
                "value": "timeout-xhr:timeout-xhr-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_set_interval_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
const id = setInterval(() => {
  clearInterval(id);
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('interval-xhr-payload');
}, 0);
</script></body></html>"#,
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-xhr", "interval"),
            ],
            format!("interval-xhr:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");

    ctx.process_async(json!({
        "id": 7820,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7820, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7821,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 7822,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(7822, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &xhr_url,
        "setInterval navigation xhr should pause",
    )
    .await;
    let paused_events = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        paused_events.len(),
        1,
        "setInterval xhr should pause once after clearInterval"
    );
    let subresource_paused = paused_events[0].clone();
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 7823,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(7823, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        1,
        "setInterval xhr should emit one Network.requestWillBeSent"
    );
    let request = requests[0].clone();
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "interval-xhr-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 7824).await;
    ctx.process_async(json!({
        "id": 7825,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        7825,
        json!({
            "result": {
                "type": "string",
                "value": "interval-xhr:interval-xhr-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_queue_microtask_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
queueMicrotask(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('microtask-xhr-payload');
});
</script></body></html>"#,
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-xhr", "microtask"),
            ],
            format!("microtask-xhr:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");

    ctx.process_async(json!({
        "id": 792,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(792, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 793,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 794,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(794, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &xhr_url,
        "microtask navigation xhr should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
        .expect("microtask navigation xhr should pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 795,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(795, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued microtask xhr should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "microtask-xhr-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 796).await;
    ctx.process_async(json!({
        "id": 797,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        797,
        json!({
            "result": {
                "type": "string",
                "value": "microtask-xhr:microtask-xhr-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_promise_then_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
Promise.resolve().then(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('promise-xhr-payload');
});
</script></body></html>"#,
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-xhr", "promise"),
            ],
            format!("promise-xhr:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");

    ctx.process_async(json!({
        "id": 816,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(816, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 817,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 818,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(818, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &xhr_url,
        "promise navigation xhr should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
        .expect("promise navigation xhr should pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 819,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(819, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued promise xhr should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(
        request["params"]["request"]["postData"],
        "promise-xhr-payload"
    );

    enable_runtime_async(&mut ctx, "SID-1", 820).await;
    ctx.process_async(json!({
        "id": 821,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        821,
        json!({
            "result": {
                "type": "string",
                "value": "promise-xhr:promise-xhr-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_request_idle_callback_fetch_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
requestIdleCallback(deadline => {
  document.body.setAttribute('data-idle-meta', String(deadline.didTimeout === false && deadline.timeRemaining() > 0));
  fetch('/api', { method: 'POST', body: 'idle-nav-payload' })
    .then(response => response.text())
    .then(text => { document.body.setAttribute('data-fetch-result', text); });
});
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "idle"),
            ],
            format!("idle-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 762,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(762, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 763,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(main_document_paused["method"], "Fetch.requestPaused");
    assert_eq!(main_document_paused["params"]["request"]["url"], page_url);
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 764,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(764, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "idle navigation fetch should pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        },
    )
    .await;
    let subresource_paused =
        ctx.take_first_matching("idle navigation fetch should pause", |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        });
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 765,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(765, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued idle fetch should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(request["params"]["request"]["postData"], "idle-nav-payload");

    enable_runtime_async(&mut ctx, "SID-1", 766).await;
    ctx.process_async(json!({
            "id": 767,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": { "expression": "[document.body.getAttribute('data-idle-meta'), document.body.getAttribute('data-fetch-result')].join(':')" }
        })).await;
    ctx.expect_result(
        767,
        json!({
            "result": {
                "type": "string",
                "value": "true:idle-nav:idle-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_request_idle_callback_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
requestIdleCallback(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('idle-xhr-payload');
});
</script></body></html>"#,
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-nav-xhr", "idle")],
            format!("idle-xhr:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");

    ctx.process_async(json!({
        "id": 768,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(768, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 769,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(main_document_paused["method"], "Fetch.requestPaused");
    assert_eq!(main_document_paused["params"]["request"]["url"], page_url);
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 770,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(770, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "idle navigation xhr should pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        },
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
        .expect("idle navigation xhr should pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 771,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(771, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued idle xhr should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(request["params"]["request"]["postData"], "idle-xhr-payload");

    enable_runtime_async(&mut ctx, "SID-1", 772).await;
    ctx.process_async(json!({
        "id": 773,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        773,
        json!({
            "result": {
                "type": "string",
                "value": "idle-xhr:idle-xhr-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_request_animation_frame_fetch_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
requestAnimationFrame(() => {
  fetch('/api', { method: 'POST', body: 'raf-nav-payload' })
    .then(response => response.text())
    .then(text => { document.body.setAttribute('data-fetch-result', text); });
});
</script></body></html>"#,
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-nav-subresource", "raf"),
            ],
            format!("raf-nav:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");

    ctx.process_async(json!({
        "id": 774,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(774, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 775,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 776,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(776, json!({}), Some("SID-1"));

    wait_for_navigation_subresource_pause(
        &mut ctx,
        &api_url,
        "navigation subresource should pause",
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("raf navigation fetch should pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 777,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(777, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued raf fetch should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], api_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(request["params"]["request"]["postData"], "raf-nav-payload");

    enable_runtime_async(&mut ctx, "SID-1", 778).await;
    ctx.process_async(json!({
        "id": 779,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-fetch-result')" }
    }))
    .await;
    ctx.expect_result(
        779,
        json!({
            "result": {
                "type": "string",
                "value": "raf-nav:raf-nav-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_request_animation_frame_xhr_subresource_pauses_until_continue_request() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
requestAnimationFrame(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => { document.body.setAttribute('data-xhr-result', xhr.responseText); };
  xhr.send('raf-xhr-payload');
});
</script></body></html>"#,
        )
    }

    async fn xhr(body: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-nav-xhr", "raf")],
            format!("raf-xhr:{body}"),
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");

    ctx.process_async(json!({
        "id": 780,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(780, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 781,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let main_document_paused = take_main_document_request_pause(&mut ctx).await;
    let main_document_request_id = main_document_paused["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 782,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": main_document_request_id }
    }))
    .await;
    ctx.expect_result(782, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "raf navigation xhr should pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        },
    )
    .await;
    let subresource_paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
        .expect("raf navigation xhr should pause");
    assert_eq!(subresource_paused["params"]["resourceType"], "XHR");
    let subresource_request_id = subresource_paused["params"]["requestId"]
        .as_str()
        .expect("subresource request id")
        .to_owned();
    let subresource_network_id = subresource_paused["params"]["networkId"]
        .as_str()
        .expect("subresource network id")
        .to_owned();

    ctx.process_async(json!({
        "id": 783,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": subresource_request_id }
    }))
    .await;
    ctx.expect_result(783, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued navigation subresource network completion",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(subresource_network_id)
        },
    )
    .await;

    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(subresource_network_id)
        })
        .cloned()
        .expect("continued raf xhr should emit Network.requestWillBeSent");
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert_eq!(request["params"]["request"]["postData"], "raf-xhr-payload");

    enable_runtime_async(&mut ctx, "SID-1", 784).await;
    ctx.process_async(json!({
        "id": 785,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "document.body.getAttribute('data-xhr-result')" }
    }))
    .await;
    ctx.expect_result(
        785,
        json!({
            "result": {
                "type": "string",
                "value": "raf-xhr:raf-xhr-payload"
            }
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn document_resource_type_filter_only_pauses_main_document() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script>
fetch('/api').catch(() => {});
</script></body></html>"#,
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

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let page_url = format!("http://{addr}/page");

    ctx.process_async(json!({
            "id": 603,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": {
                "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Document" }]
            }
        })).await;
    ctx.expect_result(603, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 604,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["params"]["resourceType"], "Document");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("document request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 605,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(605, json!({}), Some("SID-1"));

    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] != json!("Document")
        }),
        "document-only filter should not pause subresources"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_default_aborts_pending_navigation() {
    async fn handler() -> impl IntoResponse {
        (
            StatusCode::UNAUTHORIZED,
            [
                (WWW_AUTHENTICATE.as_str(), r#"Basic realm="test-area""#),
                (CONTENT_TYPE.as_str(), "text/plain"),
            ],
            "auth required",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/auth", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(attached_browser_context());
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 73,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(73, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 74,
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
        "id": 75,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(75, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.authRequired");

    ctx.process_async(json!({
        "id": 76,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": { "response": "Default" }
        }
    }))
    .await;
    ctx.expect_result(76, json!({}), Some("SID-1"));
    ctx.expect_error(74, -32000, "Fetch auth challenge aborted");
    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        !bc.active_page_state()
            .fetch_owner
            .has_pending_fetch_state_for_test()
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_success_clears_pending_auth_navigation() {
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
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 81,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(81, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 82,
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
        "id": 83,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(83, json!({}), Some("SID-1"));
    assert_eq!(ctx.take_one()["method"], "Fetch.authRequired");

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        bc.active_page_state()
            .fetch_owner
            .has_pending_fetch_auth_navigation_for_test("INT-1")
    );

    ctx.process_async(json!({
        "id": 84,
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
    ctx.expect_result(84, json!({}), Some("SID-1"));

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        !bc.active_page_state()
            .fetch_owner
            .has_pending_fetch_state_for_test()
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_retries_navigation_with_basic_proxy_credentials() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let authorization = headers
            .get("proxy-authorization")
            .and_then(|value| value.to_str().ok());
        if authorization != Some("Basic YWxhZGRpbjpvcGVuc2VzYW1l") {
            return (
                StatusCode::PROXY_AUTHENTICATION_REQUIRED,
                [
                    (PROXY_AUTHENTICATE.as_str(), r#"Basic realm="proxy-area""#),
                    (CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "proxy auth required",
            )
                .into_response();
        }

        (
            StatusCode::OK,
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>proxy authorized</main></body></html>",
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
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 84,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(84, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 85,
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
        "id": 86,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(86, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Proxy");
    assert_eq!(
        auth_required["params"]["authChallenge"]["origin"],
        format!("http://{addr}")
    );
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "proxy-area"
    );

    ctx.process_async(json!({
        "id": 87,
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
    ctx.expect_result(87, json!({}), Some("SID-1"));

    ctx.expect_result(
        85,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let response = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 200);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_retries_navigation_with_digest_proxy_credentials() {
    let (proxy_url, proxy_server) = spawn_digest_proxy(
        "text/html",
        "<!doctype html><html><body><main>proxy digest authorized</main></body></html>",
        "proxy-digest",
        "deadbeef",
    )
    .await;

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.conn
        .set_http_proxy_override_async(Some(proxy_url))
        .await;
    let url = "http://example.test/auth";

    ctx.process_async(json!({
        "id": 870,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(870, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 871,
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
        "id": 872,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(872, json!({}), Some("SID-1"));

    let auth_required = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.authRequired"))
        .cloned()
        .expect("auth required event");
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Proxy");
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "digest");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "proxy-digest"
    );

    ctx.process_async(json!({
        "id": 873,
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
    ctx.expect_result(873, json!({}), Some("SID-1"));

    ctx.expect_result(
        871,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    let response = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .cloned()
        .expect("network response event");
    assert_eq!(response["params"]["response"]["status"], 200);

    let request_extra = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSentExtraInfo"))
        .expect("proxy navigation request ExtraInfo");
    let observed_headers = request_extra["params"]["headers"]
        .as_object()
        .expect("request ExtraInfo headers");
    assert!(
        !observed_headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("proxy-authorization")),
        "Chromium exposes the initial unauthenticated proxy request headers"
    );
    assert!(
        !observed_headers
            .keys()
            .any(|name| name.starts_with("GET http")),
        "the absolute-form request line must not be parsed as a header"
    );

    proxy_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_https_proxy_connect_auth_emits_407_without_extra_info_and_fails_navigation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let proxy_server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
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
                assert!(
                    request.starts_with("CONNECT example.test:443 HTTP/1.1\r\n"),
                    "expected HTTPS proxy CONNECT, got {request:?}"
                );
                let body = "proxy auth required";
                let response = format!(
                    "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"connect-proxy\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    ctx.conn
        .set_http_proxy_override_async(Some(format!("http://{proxy_addr}")))
        .await;
    let url = "https://example.test/proxy";

    ctx.process_async(json!({
        "id": 880,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(880, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 881,
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
        "id": 882,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(882, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["authChallenge"]["source"], "Proxy");
    assert_eq!(
        auth_required["params"]["authChallenge"]["origin"],
        format!("http://{proxy_addr}")
    );
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "connect-proxy"
    );

    ctx.process_async(json!({
        "id": 883,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": { "response": "CancelAuth" }
        }
    }))
    .await;
    ctx.expect_result(883, json!({}), Some("SID-1"));

    let response = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["response"]["url"] == json!(url)
        })
        .expect("proxy CONNECT response event");
    assert_eq!(response["params"]["response"]["status"], 407);
    assert_eq!(response["params"]["hasExtraInfo"], false);
    let network_request_id = response["params"]["requestId"]
        .as_str()
        .expect("network request id");
    assert!(!ctx.sent.iter().any(|message| {
        matches!(
            message["method"].as_str(),
            Some("Network.requestWillBeSentExtraInfo" | "Network.responseReceivedExtraInfo")
        ) && message["params"]["requestId"] == json!(network_request_id)
    }));
    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_request_id)
        })
        .expect("proxy CONNECT navigation failure event");
    assert_eq!(
        failed["params"]["errorText"],
        "net::ERR_HTTP_RESPONSE_CODE_FAILURE"
    );
    let navigate = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(881))
        .expect("Page.navigate terminal result");
    assert_eq!(
        navigate["result"]["errorText"],
        "net::ERR_HTTP_RESPONSE_CODE_FAILURE"
    );

    proxy_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_handles_multi_round_basic_navigation_challenge() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        let expected = format!(
            "Basic {}",
            super::encode_basic_auth("aladdin", "opensesame")
        );
        if auth == Some(expected.as_str()) {
            return (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>authorized-round-2</main></body></html>",
            )
                .into_response();
        }
        let realm = if auth.is_some() { "round-2" } else { "round-1" };
        (
            StatusCode::UNAUTHORIZED,
            [
                (
                    WWW_AUTHENTICATE.as_str(),
                    format!("Basic realm=\"{realm}\""),
                ),
                (CONTENT_TYPE.as_str(), "text/plain".to_owned()),
            ],
            format!("auth required {realm}"),
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
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 690,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(690, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 691,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 692,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(692, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], "INT-1");
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["authChallenge"]["realm"], "round-1");

    ctx.process_async(json!({
        "id": 693,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "wrong",
                "password": "creds"
            }
        }
    }))
    .await;
    ctx.expect_result(693, json!({}), Some("SID-1"));

    let second_auth_required = ctx.take_one();
    assert_eq!(second_auth_required["method"], "Fetch.authRequired");
    assert_eq!(second_auth_required["params"]["requestId"], "INT-1");
    assert!(second_auth_required["params"].get("networkId").is_none());
    assert_eq!(
        second_auth_required["params"]["authChallenge"]["realm"],
        "round-2"
    );

    ctx.process_async(json!({
        "id": 694,
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
    ctx.expect_result(694, json!({}), Some("SID-1"));

    ctx.expect_result(
        691,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    take_http_main_document_response_after_extra_info(&mut ctx, &network_id, 200);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_handles_digest_navigation_challenge() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if auth.is_some_and(|value| value.starts_with("Digest ")) {
            return (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>digest-authorized</main></body></html>",
            )
                .into_response();
        }
        (
                StatusCode::UNAUTHORIZED,
                [
                    (
                        WWW_AUTHENTICATE.as_str(),
                        "Digest realm=\"digest-area\", nonce=\"deadbeef\", qop=\"auth\", algorithm=MD5, opaque=\"opaque\"",
                    ),
                    (CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "digest auth required",
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
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 703,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(703, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 704,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 705,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(705, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], "INT-1");
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "digest");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "digest-area"
    );

    ctx.process_async(json!({
        "id": 706,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "digest-user",
                "password": "digest-pass"
            }
        }
    }))
    .await;
    ctx.expect_result(706, json!({}), Some("SID-1"));

    ctx.expect_result(
        704,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    take_http_main_document_response_after_extra_info(&mut ctx, &network_id, 200);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_handles_multi_round_digest_navigation_challenge() {
    let digest_attempts = Arc::new(AtomicUsize::new(0));

    async fn handler(digest_attempts: Arc<AtomicUsize>, headers: HeaderMap) -> impl IntoResponse {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok());
        if auth.is_some_and(|value| value.starts_with("Digest ")) {
            let attempt = digest_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt >= 1 {
                return (
                        StatusCode::OK,
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main>digest-authorized-round-2</main></body></html>",
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
                [
                    (
                        WWW_AUTHENTICATE.as_str(),
                        format!(
                            "Digest realm=\"{realm}\", nonce=\"{}\", qop=\"auth\", algorithm=MD5, opaque=\"opaque\"",
                            if auth.is_some() { "feedface" } else { "deadbeef" }
                        ),
                    ),
                    (CONTENT_TYPE.as_str(), "text/plain".to_owned()),
                ],
                format!("digest auth required {realm}"),
            )
                .into_response()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let digest_attempts = digest_attempts.clone();
        axum::serve(
            listener,
            Router::new().route(
                "/auth",
                get(move |headers| handler(digest_attempts.clone(), headers)),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);
    let url = format!("http://{addr}/auth");

    ctx.process_async(json!({
        "id": 707,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(707, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 708,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();

    ctx.process_async(json!({
        "id": 709,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(709, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], "INT-1");
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "digest");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "digest-round-1"
    );

    ctx.process_async(json!({
        "id": 710,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "digest-user",
                "password": "digest-pass"
            }
        }
    }))
    .await;
    ctx.expect_result(710, json!({}), Some("SID-1"));

    let second_auth_required = ctx.take_one();
    assert_eq!(second_auth_required["method"], "Fetch.authRequired");
    assert_eq!(second_auth_required["params"]["requestId"], "INT-1");
    assert!(second_auth_required["params"].get("networkId").is_none());
    assert_eq!(
        second_auth_required["params"]["authChallenge"]["realm"],
        "digest-round-2"
    );

    ctx.process_async(json!({
        "id": 711,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-1",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "digest-user",
                "password": "digest-pass"
            }
        }
    }))
    .await;
    ctx.expect_result(711, json!({}), Some("SID-1"));

    ctx.expect_result(
        708,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-1"),
    );

    take_http_main_document_response_after_extra_info(&mut ctx, &network_id, 200);

    server.abort();
}
