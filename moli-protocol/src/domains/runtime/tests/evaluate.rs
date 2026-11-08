use super::*;

async fn assert_empty_http_error_navigation(ctx: &mut TestContext, error_url: &str) {
    ctx.process_and_wait_for_response_async(json!({
        "id": 42,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": error_url }
    }))
    .await;

    let navigate = take_response_by_id(ctx, 42);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert!(navigate["result"]["loaderId"].is_string());
    assert_eq!(navigate["result"]["isDownload"], json!(false));
    assert_eq!(
        navigate["result"]["errorText"],
        json!("net::ERR_HTTP_RESPONSE_CODE_FAILURE")
    );
    wait_until_message(
        ctx,
        Some("SID-1"),
        "empty HTTP error Document stop loading",
        |message| message["method"] == json!("Page.frameStoppedLoading"),
    )
    .await;
    assert!(
        ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Page.loadEventFired")),
        "browser-owned error Document should publish load before frameStoppedLoading"
    );

    let response_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Document")
                && message["params"]["response"]["status"] == json!(429)
                && message["params"]["response"]["url"] == json!(error_url)
        })
        .unwrap_or_else(|| panic!("missing original HTTP 429 response: {:?}", ctx.sent));
    let failure_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["type"] == json!("Document")
                && message["params"]["errorText"] == json!("net::ERR_HTTP_RESPONSE_CODE_FAILURE")
        })
        .unwrap_or_else(|| panic!("missing HTTP response-code failure: {:?}", ctx.sent));
    assert!(
        response_index < failure_index,
        "Chromium publishes the real HTTP response before failing the empty response body"
    );

    let frame_navigated = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Page.frameNavigated"))
        .unwrap_or_else(|| panic!("missing browser error Document commit: {:?}", ctx.sent));
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_navigated["params"]["frame"]["unreachableUrl"],
        error_url
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 43,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "({href: location.href, title: document.title, ready: document.readyState, text: document.body?.innerText || ''})",
            "returnByValue": true
        }
    }))
    .await;
    let state = take_response_by_id(ctx, 43)["result"]["result"]["value"].clone();
    assert_eq!(state["href"], NETWORK_ERROR_PAGE_URL);
    assert_eq!(state["title"], "127.0.0.1");
    assert_eq!(state["ready"], "complete");
    assert!(
        state["text"]
            .as_str()
            .is_some_and(|text| text.contains("HTTP ERROR 429")),
        "browser-owned error Document should be usable by text automation: {state:?}"
    );
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.browser_context
                .as_ref()
                .expect("browser context should exist")
                .target_url(),
            error_url
        )
    });
}

mod child_await;
mod contexts;
mod dom_integration;
mod evaluation;
mod navigation;
mod promise_waiting;
mod remote_objects;
mod subresource_network;
