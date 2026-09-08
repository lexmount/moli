use super::super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::super::*;
use super::support::{
    CdpPageHarness, request_will_be_sent_for_suffix, response_received_for_suffix,
};
use crate::testing::wait_until_messages;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde_json::{Value, json};

async fn enable_response_stage(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url_pattern: &str,
    resource_type: &str,
) {
    page.expect_empty_command(
        ctx,
        id,
        "Fetch.enable",
        json!({
            "patterns": [{
                "urlPattern": url_pattern,
                "requestStage": "Response",
                "resourceType": resource_type
            }]
        }),
    )
    .await;
}

async fn enable_fetch_response_stage(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url_pattern: &str,
) {
    enable_response_stage(ctx, page, id, url_pattern, "Fetch").await;
}

async fn enable_request_stage(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url_pattern: &str,
    resource_type: &str,
) {
    page.expect_empty_command(
        ctx,
        id,
        "Fetch.enable",
        json!({
            "patterns": [{
                "urlPattern": url_pattern,
                "requestStage": "Request",
                "resourceType": resource_type
            }]
        }),
    )
    .await;
}

async fn enable_fetch_request_stage(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url_pattern: &str,
) {
    enable_request_stage(ctx, page, id, url_pattern, "Fetch").await;
}

async fn enable_xhr_request_stage(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url_pattern: &str,
) {
    enable_request_stage(ctx, page, id, url_pattern, "XHR").await;
}

async fn enable_xhr_response_stage(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url_pattern: &str,
) {
    enable_response_stage(ctx, page, id, url_pattern, "XHR").await;
}

async fn enable_document_response_stage(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url_pattern: &str,
) {
    enable_response_stage(ctx, page, id, url_pattern, "Document").await;
}

async fn start_page_navigation(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    url: impl Into<String>,
) {
    ctx.process_async(json!({
        "id": id,
        "method": "Page.navigate",
        "sessionId": page.session_id,
        "params": { "url": url.into() }
    }))
    .await;
}

async fn wait_for_response_stage_pause(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    resource_type: &str,
    label: &str,
) -> Value {
    wait_until_messages(ctx, Some(page.session_id.as_str()), label, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!(resource_type)
                && message["params"]["responseStatusCode"] == json!(200)
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with(suffix))
        })
    })
    .await;
    ctx.sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!(resource_type)
                && message["params"]["responseStatusCode"] == json!(200)
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with(suffix))
        })
        .cloned()
        .unwrap_or_else(|| panic!("{label}: {:?}", ctx.sent))
}

async fn wait_for_fetch_response_stage_pause(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    label: &str,
) -> Value {
    // Chromium maps the Fetch filter to Blink's XHR bucket and reports the
    // bucket name, not the initiating JavaScript API, in Fetch events.
    wait_for_response_stage_pause(ctx, page, suffix, "XHR", label).await
}

async fn wait_for_request_stage_pause(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    resource_type: &str,
    label: &str,
) -> Value {
    wait_until_messages(ctx, Some(page.session_id.as_str()), label, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!(resource_type)
                && message["params"]["responseStatusCode"].is_null()
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with(suffix))
        })
    })
    .await;
    ctx.sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!(resource_type)
                && message["params"]["responseStatusCode"].is_null()
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with(suffix))
        })
        .cloned()
        .unwrap_or_else(|| panic!("{label}: {:?}", ctx.sent))
}

async fn wait_for_fetch_request_stage_pause(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    label: &str,
) -> Value {
    wait_for_request_stage_pause(ctx, page, suffix, "XHR", label).await
}

async fn wait_for_xhr_request_stage_pause(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    label: &str,
) -> Value {
    wait_for_request_stage_pause(ctx, page, suffix, "XHR", label).await
}

async fn wait_for_xhr_response_stage_pause(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    label: &str,
) -> Value {
    wait_for_response_stage_pause(ctx, page, suffix, "XHR", label).await
}

async fn wait_for_document_response_stage_pause(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    label: &str,
) -> Value {
    wait_for_response_stage_pause(ctx, page, suffix, "Document", label).await
}

fn paused_request_id(paused: &Value) -> String {
    paused["params"]["requestId"]
        .as_str()
        .unwrap_or_else(|| panic!("request id: {paused}"))
        .to_owned()
}

async fn open_paused_response_stream(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    request_id: &str,
) -> String {
    let stream = page
        .command(
            ctx,
            id,
            "Fetch.takeResponseBodyAsStream",
            json!({ "requestId": request_id }),
        )
        .await;
    stream["result"]["stream"]
        .as_str()
        .unwrap_or_else(|| panic!("stream handle: {stream}"))
        .to_owned()
}

async fn read_response_stream(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    handle: &str,
    offset: Option<u64>,
    size: Option<u64>,
) -> Value {
    let mut params = json!({ "handle": handle });
    if let Some(offset) = offset {
        params["offset"] = json!(offset);
    }
    if let Some(size) = size {
        params["size"] = json!(size);
    }
    page.command(ctx, id, "IO.read", params).await
}

async fn read_paused_response_stream(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    open_id: u64,
    read_id: u64,
    request_id: &str,
) -> Value {
    let handle = open_paused_response_stream(ctx, page, open_id, request_id).await;
    read_response_stream(ctx, page, read_id, &handle, None, None).await
}

async fn continue_paused_response(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    paused: &Value,
) {
    page.expect_empty_command(
        ctx,
        id,
        "Fetch.continueResponse",
        json!({ "requestId": paused["params"]["requestId"] }),
    )
    .await;
}

async fn continue_paused_request_with_response_interception(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    paused: &Value,
) {
    page.expect_empty_command(
        ctx,
        id,
        "Fetch.continueRequest",
        json!({
            "requestId": paused["params"]["requestId"],
            "interceptResponse": true
        }),
    )
    .await;
}

async fn get_paused_response_body(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    paused: &Value,
) -> Value {
    page.command(
        ctx,
        id,
        "Fetch.getResponseBody",
        json!({ "requestId": paused["params"]["requestId"] }),
    )
    .await
}

fn assert_response_stage_header(paused: &Value, name: &str, value: &str) {
    assert!(
        paused["params"]["responseHeaders"]
            .as_array()
            .is_some_and(|headers| headers.iter().any(|header| {
                header["name"]
                    .as_str()
                    .is_some_and(|header_name| header_name.eq_ignore_ascii_case(name))
                    && header["value"] == json!(value)
            })),
        "{paused}"
    );
}

async fn fulfill_paused_response(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    paused: &Value,
    response_code: u16,
    body: &str,
) {
    page.expect_empty_command(
        ctx,
        id,
        "Fetch.fulfillRequest",
        json!({
            "requestId": paused["params"]["requestId"],
            "responseCode": response_code,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" },
                { "name": "x-p0-fulfilled", "value": "yes" }
            ],
            "body": body
        }),
    )
    .await;
}

async fn fail_paused_response(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    paused: &Value,
) {
    page.expect_empty_command(
        ctx,
        id,
        "Fetch.failRequest",
        json!({
            "requestId": paused["params"]["requestId"],
            "errorReason": "Aborted"
        }),
    )
    .await;
}

async fn assert_document_response_stage_network_contract(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    suffix: &str,
    paused: &Value,
) {
    let paused_network_id = paused["params"]["networkId"]
        .as_str()
        .unwrap_or_else(|| panic!("paused networkId: {paused}"));
    wait_until_messages(
        ctx,
        Some(page.session_id.as_str()),
        "Document response-stage Network completion",
        |messages| {
            request_will_be_sent_for_suffix(messages, suffix).is_some()
                && response_received_for_suffix(messages, suffix).is_some()
        },
    )
    .await;
    let request = request_will_be_sent_for_suffix(&ctx.sent, suffix)
        .unwrap_or_else(|| panic!("document requestWillBeSent for {suffix}: {:?}", ctx.sent));
    assert_eq!(
        request["params"]["requestId"],
        json!(paused_network_id),
        "{request}"
    );
    let response = response_received_for_suffix(&ctx.sent, suffix)
        .unwrap_or_else(|| panic!("document responseReceived for {suffix}: {:?}", ctx.sent));
    assert_eq!(
        response["params"]["requestId"],
        json!(paused_network_id),
        "{response}"
    );
    assert_eq!(response["params"]["type"], json!("Document"), "{response}");
}

async fn wait_for_global_string(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    id: u64,
    name: &str,
) -> Value {
    page.evaluate_await_value(
        ctx,
        id,
        format!(
            r#"
            new Promise(resolve => {{
                const started = Date.now();
                const timer = setInterval(() => {{
                    if (globalThis[{name:?}] !== 'pending' || Date.now() - started > 2000) {{
                        clearInterval(timer);
                        resolve(globalThis[{name:?}]);
                    }}
                }}, 10);
            }})
            "#
        ),
    )
    .await
}

async fn wait_for_global_string_value(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    mut next_id: u64,
    name: &str,
    expected: &str,
    label: &str,
) -> Value {
    let mut issued_ids = Vec::new();
    let mut last = None;
    for _ in 0..64 {
        ctx.complete_one_ready_scheduler_input_for_test().await;
        let id = next_id;
        next_id += 1;
        issued_ids.push(id);
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": page.session_id,
            "params": {
                "expression": format!("globalThis[{name:?}]"),
                "returnByValue": true
            }
        }))
        .await;
        while let Some(position) = ctx.sent.iter().position(|message| {
            message["id"]
                .as_u64()
                .is_some_and(|message_id| issued_ids.contains(&message_id))
        }) {
            let response = ctx.sent.remove(position);
            if response["result"]["result"]["value"] == json!(expected) {
                return response;
            }
            last = Some(response);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!(
        "timed out waiting for {label}; last={last:?}; sent={:?}",
        ctx.sent
    );
}

async fn wait_for_loading_finished(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    network_id: &str,
    label: &str,
) {
    wait_until_messages(ctx, Some(page.session_id.as_str()), label, |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(network_id)
        })
    })
    .await;
}

fn assert_no_late_request_pause(ctx: &TestContext, path: &str, label: &str) {
    assert!(
        ctx.sent.iter().all(|message| {
            !(message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with(path)))
        }),
        "{label}: {:?}",
        ctx.sent
    );
}

async fn assert_redirect_response_stage_network_contract(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    label: &str,
    paused_network_id: &str,
) {
    wait_until_messages(ctx, Some(page.session_id.as_str()), label, |messages| {
        request_will_be_sent_for_suffix(messages, "/api-redirect-start").is_some()
            && request_will_be_sent_for_suffix(messages, "/api-redirect-final").is_some()
            && response_received_for_suffix(messages, "/api-redirect-final").is_some()
    })
    .await;
    let start_request = request_will_be_sent_for_suffix(&ctx.sent, "/api-redirect-start")
        .unwrap_or_else(|| panic!("redirect start request: {:?}", ctx.sent));
    let final_request = request_will_be_sent_for_suffix(&ctx.sent, "/api-redirect-final")
        .unwrap_or_else(|| panic!("redirect final request: {:?}", ctx.sent));
    assert_eq!(
        final_request["params"]["requestId"],
        json!(paused_network_id),
        "response-stage pause should use the final redirect request id: {:?}",
        ctx.sent
    );
    assert_eq!(
        final_request["params"]["requestId"], start_request["params"]["requestId"],
        "redirect chain should preserve request id continuity: {:?}",
        ctx.sent
    );
    assert_eq!(
        final_request["params"]["redirectResponse"]["headers"]["x-smoke-redirect"],
        json!("start"),
        "{final_request}"
    );
}

async fn assert_document_redirect_response_stage_network_contract(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    paused_network_id: &str,
) {
    wait_until_messages(
        ctx,
        Some(page.session_id.as_str()),
        "Document redirect response-stage Network completion",
        |messages| {
            request_will_be_sent_for_suffix(messages, "/redirect-start").is_some()
                && request_will_be_sent_for_suffix(messages, "/redirect-final").is_some()
                && response_received_for_suffix(messages, "/redirect-final").is_some()
        },
    )
    .await;
    let start_request = request_will_be_sent_for_suffix(&ctx.sent, "/redirect-start")
        .unwrap_or_else(|| panic!("document redirect start request: {:?}", ctx.sent));
    let final_request = request_will_be_sent_for_suffix(&ctx.sent, "/redirect-final")
        .unwrap_or_else(|| panic!("document redirect final request: {:?}", ctx.sent));
    assert_eq!(
        final_request["params"]["requestId"], start_request["params"]["requestId"],
        "document redirect chain should preserve request id continuity: {:?}",
        ctx.sent
    );
    assert_eq!(
        final_request["params"]["requestId"],
        json!(paused_network_id),
        "document response-stage pause should use the final redirect request id: {:?}",
        ctx.sent
    );
    assert_eq!(
        final_request["params"]["redirectResponse"]["headers"]["cache-control"],
        json!("no-store"),
        "{final_request}"
    );
    let response = response_received_for_suffix(&ctx.sent, "/redirect-final")
        .unwrap_or_else(|| panic!("document redirect responseReceived: {:?}", ctx.sent));
    assert_eq!(response["params"]["type"], json!("Document"), "{response}");
    assert_eq!(
        response["params"]["requestId"],
        json!(paused_network_id),
        "{response}"
    );
}

// P0 browser contract source:
// Chromium Network post body plus Playwright request.postData expectations.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_network_fetch_post_data_and_response_body_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 136_000).await;

    page.navigate(&mut ctx, 136_005, fixture.url("/plain?post-data"))
        .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-echo");
    let response = page
        .evaluate_await_value(
            &mut ctx,
            136_006,
            format!(
                r#"(async () => {{
                    const response = await fetch({fetch_url:?}, {{
                        method: 'POST',
                        headers: {{
                            'content-type': 'text/plain',
                            'x-smoke-post': 'p0'
                        }},
                        body: 'p0-body'
                    }});
                    return await response.json();
                }})()"#
            ),
        )
        .await;
    let value = &response["result"]["result"]["value"];
    assert_eq!(value["method"], json!("POST"), "{response}");
    assert_eq!(value["body"], json!("p0-body"), "{response}");
    assert_eq!(value["customHeader"], json!("p0"), "{response}");

    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "Network.requestWillBeSent post data",
        |messages| {
            request_will_be_sent_for_suffix(messages, "/api-echo").is_some_and(|message| {
                message["params"]["request"]["method"] == json!("POST")
                    && message["params"]["request"]["hasPostData"] == json!(true)
                    && message["params"]["request"]["postData"] == json!("p0-body")
            })
        },
    )
    .await;
}

// P0 browser contract source:
// Chromium Network.getResponseBody binary response contract.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_network_binary_response_body_base64_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 137_000).await;

    page.navigate(&mut ctx, 137_005, fixture.url("/plain?binary-body"))
        .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-binary");
    let response = page
        .evaluate_await_value(
            &mut ctx,
            137_006,
            format!(
                r#"(async () => {{
                    const response = await fetch({fetch_url:?});
                    const bytes = new Uint8Array(await response.arrayBuffer());
                    return Array.from(bytes).join(',');
                }})()"#
            ),
        )
        .await;
    assert_eq!(response["result"]["result"]["value"], json!("0,255,97"));

    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "Network.responseReceived for binary response",
        |messages| response_received_for_suffix(messages, "/api-binary").is_some(),
    )
    .await;
    let response_event = response_received_for_suffix(&ctx.sent, "/api-binary")
        .unwrap_or_else(|| panic!("responseReceived for /api-binary: {:?}", ctx.sent));
    let request_id = response_event["params"]["requestId"]
        .as_str()
        .unwrap_or_else(|| panic!("requestId: {response_event}"))
        .to_owned();

    let body = page
        .command(
            &mut ctx,
            137_007,
            "Network.getResponseBody",
            json!({ "requestId": request_id }),
        )
        .await;
    assert_eq!(body["result"]["base64Encoded"], json!(true), "{body}");
    assert_eq!(body["result"]["body"], json!("AP9h"), "{body}");
}

// P0 browser contract source:
// Chromium Network redirect ordering and Playwright request redirect chain behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_network_fetch_redirect_chain_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 142_000).await;

    page.navigate(&mut ctx, 142_005, fixture.url("/plain?redirect-chain"))
        .await;
    ctx.sent.clear();

    let redirect_url = fixture.url("/api-redirect-start");
    let response = page
        .evaluate_await_value(
            &mut ctx,
            142_006,
            format!(
                r#"(async () => {{
                    const response = await fetch({redirect_url:?});
                    return await response.json();
                }})()"#
            ),
        )
        .await;
    assert_eq!(
        response["result"]["result"]["value"],
        json!({"redirected": true, "method": "GET"}),
        "{response}"
    );

    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "Network redirect chain",
        |messages| {
            request_will_be_sent_for_suffix(messages, "/api-redirect-start").is_some()
                && request_will_be_sent_for_suffix(messages, "/api-redirect-final").is_some_and(
                    |message| {
                        message["params"]["redirectResponse"]["status"] == json!(307)
                            || message["params"]["redirectResponse"]["status"] == json!(302)
                    },
                )
                && response_received_for_suffix(messages, "/api-redirect-final").is_some()
        },
    )
    .await;
    let start_request = request_will_be_sent_for_suffix(&ctx.sent, "/api-redirect-start")
        .unwrap_or_else(|| panic!("redirect start request: {:?}", ctx.sent));
    let final_request = request_will_be_sent_for_suffix(&ctx.sent, "/api-redirect-final")
        .unwrap_or_else(|| panic!("redirect final request: {:?}", ctx.sent));
    assert_eq!(
        final_request["params"]["requestId"], start_request["params"]["requestId"],
        "redirect should preserve requestId continuity: {:?}",
        ctx.sent
    );
    assert_eq!(
        final_request["params"]["redirectResponse"]["headers"]["x-smoke-redirect"],
        json!("start"),
        "{final_request}"
    );
}

// P0 browser contract source:
// Chromium Fetch response-stage interception for a main document navigation.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_document_response_stage_get_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 155_000).await;

    enable_document_response_stage(&mut ctx, &page, 155_005, "*document-response-stage*").await;
    start_page_navigation(
        &mut ctx,
        &page,
        155_006,
        fixture.url("/document-response-stage"),
    )
    .await;

    let paused = wait_for_document_response_stage_pause(
        &mut ctx,
        &page,
        "/document-response-stage",
        "Document response-stage pause",
    )
    .await;
    assert_response_stage_header(&paused, "x-smoke-document-stage", "paused");

    let body = get_paused_response_body(&mut ctx, &page, 155_007, &paused).await;
    assert_eq!(body["result"]["base64Encoded"], json!(false), "{body}");
    assert_eq!(
        body["result"]["body"],
        json!("<!doctype html><main>document response-stage body</main>"),
        "{body}"
    );

    continue_paused_response(&mut ctx, &page, 155_008, &paused).await;
    crate::testing::wait_until_navigation_document_load(&mut ctx, 155_006, Some(&page.session_id))
        .await;
    let navigation = take_response_by_id(&mut ctx, 155_006);
    assert_eq!(navigation["result"]["frameId"], page.target_id);
    assert!(
        navigation["result"]["loaderId"].as_str().is_some(),
        "{navigation}"
    );

    assert_document_response_stage_network_contract(
        &mut ctx,
        &page,
        "/document-response-stage",
        &paused,
    )
    .await;
    let text = page
        .evaluate_string(&mut ctx, 155_009, "document.body.textContent")
        .await;
    assert_eq!(text, "document response-stage body");
}

// P0 browser contract source:
// Playwright route.fulfill-like main document response-stage override.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_document_response_stage_fulfill_overrides_body() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 156_000).await;

    enable_document_response_stage(&mut ctx, &page, 156_005, "*document-response-stage*").await;
    start_page_navigation(
        &mut ctx,
        &page,
        156_006,
        fixture.url("/document-response-stage?fulfill"),
    )
    .await;

    let paused = wait_for_document_response_stage_pause(
        &mut ctx,
        &page,
        "/document-response-stage?fulfill",
        "Document response-stage fulfill pause",
    )
    .await;
    page.expect_empty_command(
        &mut ctx,
        156_007,
        "Fetch.fulfillRequest",
        json!({
            "requestId": paused["params"]["requestId"],
            "responseCode": 201,
            "responseHeaders": [
                { "name": "content-type", "value": "text/html; charset=utf-8" },
                { "name": "x-p0-document-fulfilled", "value": "yes" }
            ],
            "body": BASE64_STANDARD.encode("<!doctype html><main>document fulfilled</main>")
        }),
    )
    .await;

    crate::testing::wait_until_navigation_document_load(&mut ctx, 156_006, Some(&page.session_id))
        .await;
    let navigation = take_response_by_id(&mut ctx, 156_006);
    assert_eq!(navigation["result"]["frameId"], page.target_id);
    let text = page
        .evaluate_string(&mut ctx, 156_008, "document.body.textContent")
        .await;
    assert_eq!(text, "document fulfilled");

    let response = response_received_for_suffix(&ctx.sent, "/document-response-stage?fulfill")
        .unwrap_or_else(|| panic!("document fulfill responseReceived: {:?}", ctx.sent));
    assert_eq!(
        response["params"]["response"]["status"],
        json!(201),
        "{response}"
    );
    assert_eq!(
        response["params"]["response"]["headers"]["x-p0-document-fulfilled"],
        json!("yes"),
        "{response}"
    );
}

// P0 browser contract source:
// Playwright route.abort-like main document response-stage failure.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_document_response_stage_fail_aborts_navigation() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 157_000).await;

    enable_document_response_stage(&mut ctx, &page, 157_005, "*document-response-stage*").await;
    start_page_navigation(
        &mut ctx,
        &page,
        157_006,
        fixture.url("/document-response-stage?fail"),
    )
    .await;

    let paused = wait_for_document_response_stage_pause(
        &mut ctx,
        &page,
        "/document-response-stage?fail",
        "Document response-stage fail pause",
    )
    .await;
    fail_paused_response(&mut ctx, &page, 157_007, &paused).await;

    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "original navigation reply",
        |message| {
            message["id"] == json!(157_006) && message["sessionId"] == json!(&page.session_id)
        },
    )
    .await;
    ctx.expect_error(157_006, -32000, "Aborted");
    let network_id = paused["params"]["networkId"]
        .as_str()
        .unwrap_or_else(|| panic!("paused networkId: {paused}"));
    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        })
        .cloned()
        .unwrap_or_else(|| panic!("document loadingFailed: {:?}", ctx.sent));
    assert_eq!(failed["params"]["type"], json!("Document"), "{failed}");
}

// P0 browser contract source:
// Chromium Fetch response-stage interception after a main document redirect.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_document_redirect_response_stage_get_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 160_000).await;

    enable_document_response_stage(&mut ctx, &page, 160_005, "*redirect-final*").await;
    start_page_navigation(&mut ctx, &page, 160_006, fixture.url("/redirect-start")).await;

    let paused = wait_for_document_response_stage_pause(
        &mut ctx,
        &page,
        "/redirect-final",
        "Document redirect response-stage pause",
    )
    .await;
    let paused_network_id = paused["params"]["networkId"]
        .as_str()
        .unwrap_or_else(|| panic!("paused networkId: {paused}"))
        .to_owned();
    assert_eq!(
        paused["params"]["responseStatusCode"],
        json!(200),
        "{paused}"
    );

    let body = get_paused_response_body(&mut ctx, &page, 160_007, &paused).await;
    assert_eq!(body["result"]["base64Encoded"], json!(false), "{body}");
    assert_eq!(
        body["result"]["body"],
        json!("<!doctype html><main>redirect final</main>"),
        "{body}"
    );

    continue_paused_response(&mut ctx, &page, 160_008, &paused).await;
    crate::testing::wait_until_navigation_document_load(&mut ctx, 160_006, Some(&page.session_id))
        .await;
    let navigation = take_response_by_id(&mut ctx, 160_006);
    assert_eq!(navigation["result"]["frameId"], page.target_id);
    assert!(
        navigation["result"]["loaderId"].as_str().is_some(),
        "{navigation}"
    );

    let text = page
        .evaluate_string(&mut ctx, 160_009, "document.body.textContent")
        .await;
    assert_eq!(text, "redirect final");
    assert_document_redirect_response_stage_network_contract(&mut ctx, &page, &paused_network_id)
        .await;
}

// P0 browser contract source:
// Chromium Fetch response-stage URL patterns should be matched against the final
// main-document response URL after redirects.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_document_redirect_response_stage_final_url_mismatch_completes() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 160_100).await;

    enable_document_response_stage(&mut ctx, &page, 160_105, "*redirect-start*").await;
    start_page_navigation(&mut ctx, &page, 160_106, fixture.url("/redirect-start")).await;

    let navigation = take_response_by_id(&mut ctx, 160_106);
    assert_eq!(navigation["result"]["frameId"], page.target_id);
    assert!(
        navigation["result"]["loaderId"].as_str().is_some(),
        "{navigation}"
    );

    let text = page
        .evaluate_string(&mut ctx, 160_107, "document.body.textContent")
        .await;
    assert_eq!(text, "redirect final");
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
        }),
        "Document response-stage should not pause when only the initial redirect URL matches: {:?}",
        ctx.sent
    );
    wait_until_messages(
        &mut ctx,
        Some(page.session_id.as_str()),
        "Document redirect final response completed without Fetch pause",
        |messages| {
            request_will_be_sent_for_suffix(messages, "/redirect-start").is_some()
                && request_will_be_sent_for_suffix(messages, "/redirect-final").is_some()
                && response_received_for_suffix(messages, "/redirect-final").is_some()
        },
    )
    .await;
}

// P0 browser contract source:
// Chromium Fetch response-stage interception after a subresource redirect.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_redirect_response_stage_get_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 150_000).await;

    enable_fetch_response_stage(&mut ctx, &page, 150_005, "*api-redirect-final*").await;
    page.navigate(
        &mut ctx,
        150_006,
        fixture.url("/plain?redirect-response-stage"),
    )
    .await;
    ctx.sent.clear();

    let redirect_url = fixture.url("/api-redirect-start");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            150_007,
            format!(
                r#"
                globalThis.__p0RedirectResponseStage = 'pending';
                fetch({redirect_url:?})
                    .then(response => response.json())
                    .then(value => {{
                        globalThis.__p0RedirectResponseStage = value.method + ':' + value.redirected;
                    }})
                    .catch(error => {{
                        globalThis.__p0RedirectResponseStage = 'error:' + error.name;
                    }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_response_stage_pause(
        &mut ctx,
        &page,
        "/api-redirect-final",
        "Fetch redirect response-stage pause",
    )
    .await;
    let paused_network_id = paused["params"]["networkId"]
        .as_str()
        .unwrap_or_else(|| panic!("paused networkId: {paused}"));

    let body = get_paused_response_body(&mut ctx, &page, 150_008, &paused).await;
    assert_eq!(body["result"]["base64Encoded"], json!(false), "{body}");
    let body_value: Value = serde_json::from_str(
        body["result"]["body"]
            .as_str()
            .unwrap_or_else(|| panic!("redirect response body: {body}")),
    )
    .unwrap_or_else(|error| panic!("redirect response body json error: {error}: {body}"));
    assert_eq!(
        body_value,
        json!({"redirected": true, "method": "GET"}),
        "{body}"
    );

    continue_paused_response(&mut ctx, &page, 150_009, &paused).await;
    let completed =
        wait_for_global_string(&mut ctx, &page, 150_010, "__p0RedirectResponseStage").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!("GET:true"),
        "{completed}"
    );

    assert_redirect_response_stage_network_contract(
        &mut ctx,
        &page,
        "Network redirect response-stage completion",
        paused_network_id,
    )
    .await;
}

// P0 browser contract source:
// Playwright CDPSession XHR response-stage interception after a subresource redirect.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_xhr_redirect_response_stage_get_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 151_000).await;

    enable_xhr_response_stage(&mut ctx, &page, 151_005, "*api-redirect-final*").await;
    page.navigate(
        &mut ctx,
        151_006,
        fixture.url("/plain?xhr-redirect-response-stage"),
    )
    .await;
    ctx.sent.clear();

    let redirect_url = fixture.url("/api-redirect-start");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            151_007,
            format!(
                r#"
                globalThis.__p0XhrRedirectResponseStage = 'pending';
                const xhr = new XMLHttpRequest();
                xhr.open('GET', {redirect_url:?}, true);
                xhr.onload = () => {{
                    globalThis.__p0XhrRedirectResponseStage = JSON.stringify({{
                        status: xhr.status,
                        body: xhr.responseText
                    }});
                }};
                xhr.onerror = () => {{
                    globalThis.__p0XhrRedirectResponseStage = 'error';
                }};
                xhr.send();
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_xhr_response_stage_pause(
        &mut ctx,
        &page,
        "/api-redirect-final",
        "XHR redirect response-stage pause",
    )
    .await;
    let paused_network_id = paused["params"]["networkId"]
        .as_str()
        .unwrap_or_else(|| panic!("paused networkId: {paused}"));

    let body = get_paused_response_body(&mut ctx, &page, 151_008, &paused).await;
    assert_eq!(body["result"]["base64Encoded"], json!(false), "{body}");
    let body_value: Value = serde_json::from_str(
        body["result"]["body"]
            .as_str()
            .unwrap_or_else(|| panic!("XHR redirect response body: {body}")),
    )
    .unwrap_or_else(|error| panic!("XHR redirect response body json error: {error}: {body}"));
    assert_eq!(
        body_value,
        json!({"redirected": true, "method": "GET"}),
        "{body}"
    );

    continue_paused_response(&mut ctx, &page, 151_009, &paused).await;
    let completed =
        wait_for_global_string(&mut ctx, &page, 151_010, "__p0XhrRedirectResponseStage").await;
    let completed_value: Value = serde_json::from_str(
        completed["result"]["result"]["value"]
            .as_str()
            .unwrap_or_else(|| panic!("XHR redirect completion value: {completed}")),
    )
    .unwrap_or_else(|error| panic!("XHR redirect completion json error: {error}: {completed}"));
    assert_eq!(completed_value["status"], json!(200), "{completed}");
    let completed_body: Value = serde_json::from_str(
        completed_value["body"]
            .as_str()
            .unwrap_or_else(|| panic!("XHR redirect completion body: {completed_value}")),
    )
    .unwrap_or_else(|error| {
        panic!("XHR redirect completion body json error: {error}: {completed_value}")
    });
    assert_eq!(
        completed_body,
        json!({"redirected": true, "method": "GET"}),
        "{completed}"
    );

    assert_redirect_response_stage_network_contract(
        &mut ctx,
        &page,
        "Network XHR redirect response-stage completion",
        paused_network_id,
    )
    .await;
}

// P0 browser contract source:
// Chromium Fetch response-stage body inspection and continueResponse behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_response_stage_get_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 143_000).await;

    enable_fetch_response_stage(&mut ctx, &page, 143_005, "*api-response-stage*").await;
    page.navigate(
        &mut ctx,
        143_006,
        fixture.url("/plain?fetch-response-stage"),
    )
    .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            143_007,
            format!(
                r#"
                globalThis.__p0ResponseStageBody = 'pending';
                fetch({fetch_url:?})
                    .then(response => response.text())
                    .then(text => {{ globalThis.__p0ResponseStageBody = text; }})
                    .catch(error => {{ globalThis.__p0ResponseStageBody = 'error:' + error.name; }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "Fetch response-stage pause",
    )
    .await;
    assert_response_stage_header(&paused, "x-smoke-response-stage", "paused");

    let body = get_paused_response_body(&mut ctx, &page, 143_008, &paused).await;
    assert_eq!(
        body["result"]["body"],
        json!("response-stage body"),
        "{body}"
    );
    assert_eq!(body["result"]["base64Encoded"], json!(false), "{body}");

    continue_paused_response(&mut ctx, &page, 143_009, &paused).await;
    let completed = wait_for_global_string(&mut ctx, &page, 143_010, "__p0ResponseStageBody").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!("response-stage body"),
        "{completed}"
    );
}

// P0 browser contract source:
// Chromium Fetch response-stage stream body lifecycle.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_response_stage_stream_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 144_000).await;

    enable_fetch_response_stage(&mut ctx, &page, 144_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 144_006, fixture.url("/plain?fetch-stream"))
        .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            144_007,
            format!(
                r#"
                globalThis.__p0ResponseStageStreamBody = 'pending';
                fetch({fetch_url:?})
                    .then(response => response.text())
                    .then(text => {{ globalThis.__p0ResponseStageStreamBody = text; }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "Fetch response-stage stream pause",
    )
    .await;
    let request_id = paused_request_id(&paused);

    let chunk = read_paused_response_stream(&mut ctx, &page, 144_008, 144_009, &request_id).await;
    assert_eq!(
        chunk["result"]["data"],
        json!("response-stage body"),
        "{chunk}"
    );
    assert_eq!(chunk["result"]["base64Encoded"], json!(false), "{chunk}");
    assert_eq!(chunk["result"]["eof"], json!(true), "{chunk}");

    let continue_after_taken = page
        .command(
            &mut ctx,
            144_010,
            "Fetch.continueResponse",
            json!({ "requestId": paused["params"]["requestId"] }),
        )
        .await;
    assert_eq!(
        continue_after_taken["error"]["message"],
        json!("Unable to continue request as is after body is taken"),
        "{continue_after_taken}"
    );

    fulfill_paused_response(
        &mut ctx,
        &page,
        144_011,
        &paused,
        200,
        "cmVzcG9uc2Utc3RhZ2UgYm9keQ==",
    )
    .await;
    let completed =
        wait_for_global_string(&mut ctx, &page, 144_012, "__p0ResponseStageStreamBody").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!("response-stage body"),
        "{completed}"
    );
}

// P0 browser contract source:
// Chromium Fetch response-stage binary stream body contract.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_response_stage_binary_stream_body() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 145_000).await;

    enable_fetch_response_stage(&mut ctx, &page, 145_005, "*api-binary*").await;
    page.navigate(&mut ctx, 145_006, fixture.url("/plain?binary-stream"))
        .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-binary");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            145_007,
            format!(
                r#"
                globalThis.__p0BinaryStreamBody = 'pending';
                fetch({fetch_url:?})
                    .then(response => response.arrayBuffer())
                    .then(buffer => {{
                        globalThis.__p0BinaryStreamBody = Array.from(new Uint8Array(buffer)).join(',');
                    }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_response_stage_pause(
        &mut ctx,
        &page,
        "/api-binary",
        "Fetch binary response-stage stream pause",
    )
    .await;
    let request_id = paused_request_id(&paused);

    let chunk = read_paused_response_stream(&mut ctx, &page, 145_008, 145_009, &request_id).await;
    assert_eq!(chunk["result"]["data"], json!("AP9h"), "{chunk}");
    assert_eq!(chunk["result"]["base64Encoded"], json!(true), "{chunk}");
    assert_eq!(chunk["result"]["eof"], json!(true), "{chunk}");

    let continue_after_taken = page
        .command(
            &mut ctx,
            145_010,
            "Fetch.continueResponse",
            json!({ "requestId": paused["params"]["requestId"] }),
        )
        .await;
    assert_eq!(
        continue_after_taken["error"]["message"],
        json!("Unable to continue request as is after body is taken"),
        "{continue_after_taken}"
    );

    fulfill_paused_response(&mut ctx, &page, 145_011, &paused, 200, "AP9h").await;
    let completed = wait_for_global_string(&mut ctx, &page, 145_012, "__p0BinaryStreamBody").await;
    assert_eq!(completed["result"]["result"]["value"], json!("0,255,97"));
}

// P0 browser contract source:
// Playwright CDPSession response-stage stream offset and IO.close behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_response_stage_stream_offset_and_close() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 146_000).await;

    enable_fetch_response_stage(&mut ctx, &page, 146_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 146_006, fixture.url("/plain?stream-offset"))
        .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            146_007,
            format!(
                r#"
                globalThis.__p0StreamOffsetBody = 'pending';
                fetch({fetch_url:?})
                    .then(response => response.text())
                    .then(text => {{ globalThis.__p0StreamOffsetBody = text; }})
                    .catch(error => {{ globalThis.__p0StreamOffsetBody = 'error:' + error.name; }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "Fetch response-stage stream offset pause",
    )
    .await;
    let request_id = paused_request_id(&paused);
    let handle = open_paused_response_stream(&mut ctx, &page, 146_008, &request_id).await;

    let offset_chunk =
        read_response_stream(&mut ctx, &page, 146_009, &handle, Some(9), Some(5)).await;
    assert_eq!(
        offset_chunk["result"]["data"],
        json!("stage"),
        "{offset_chunk}"
    );
    assert_eq!(
        offset_chunk["result"]["base64Encoded"],
        json!(false),
        "{offset_chunk}"
    );
    assert_eq!(
        offset_chunk["result"]["eof"],
        json!(false),
        "{offset_chunk}"
    );

    let first_chunk =
        read_response_stream(&mut ctx, &page, 146_010, &handle, Some(0), Some(8)).await;
    assert_eq!(
        first_chunk["result"]["data"],
        json!("response"),
        "{first_chunk}"
    );
    assert_eq!(first_chunk["result"]["eof"], json!(false), "{first_chunk}");

    let tail_chunk = read_response_stream(&mut ctx, &page, 146_011, &handle, None, None).await;
    assert_eq!(
        tail_chunk["result"]["data"],
        json!("-stage body"),
        "{tail_chunk}"
    );
    assert_eq!(tail_chunk["result"]["eof"], json!(true), "{tail_chunk}");

    let continue_after_taken = page
        .command(
            &mut ctx,
            146_012,
            "Fetch.continueResponse",
            json!({ "requestId": paused["params"]["requestId"] }),
        )
        .await;
    assert_eq!(
        continue_after_taken["error"]["message"],
        json!("Unable to continue request as is after body is taken"),
        "{continue_after_taken}"
    );

    fulfill_paused_response(
        &mut ctx,
        &page,
        146_013,
        &paused,
        200,
        "cmVzcG9uc2Utc3RhZ2UgYm9keQ==",
    )
    .await;
    let completed = wait_for_global_string(&mut ctx, &page, 146_014, "__p0StreamOffsetBody").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!("response-stage body"),
        "{completed}"
    );

    page.expect_empty_command(&mut ctx, 146_015, "IO.close", json!({ "handle": handle }))
        .await;
    let read_after_close =
        read_response_stream(&mut ctx, &page, 146_016, &handle, None, None).await;
    assert_eq!(
        read_after_close["error"]["code"],
        json!(-32000),
        "{read_after_close}"
    );
    assert_eq!(
        read_after_close["error"]["message"],
        json!("StreamHandleNotFound"),
        "{read_after_close}"
    );
}

// P0 browser contract source:
// Chromium/Playwright response-stage fulfillRequest body override behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_response_stage_fulfill_overrides_body() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 147_000).await;

    enable_fetch_response_stage(&mut ctx, &page, 147_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 147_006, fixture.url("/plain?response-fulfill"))
        .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            147_007,
            format!(
                r#"
                globalThis.__p0ResponseStageFulfill = 'pending';
                fetch({fetch_url:?})
                    .then(async response => {{
                        globalThis.__p0ResponseStageFulfill = JSON.stringify({{
                            status: response.status,
                            header: response.headers.get('x-p0-fulfilled'),
                            body: await response.text()
                        }});
                    }})
                    .catch(error => {{ globalThis.__p0ResponseStageFulfill = 'error:' + error.name; }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "Fetch response-stage fulfill pause",
    )
    .await;
    fulfill_paused_response(
        &mut ctx,
        &page,
        147_008,
        &paused,
        202,
        "ZnVsZmlsbGVkLXJlc3BvbnNl",
    )
    .await;

    let completed =
        wait_for_global_string(&mut ctx, &page, 147_009, "__p0ResponseStageFulfill").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!(r#"{"status":202,"header":"yes","body":"fulfilled-response"}"#),
        "{completed}"
    );
}

// P0 browser contract source:
// Chromium/Playwright response-stage failRequest rejection behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_response_stage_fail_rejects_fetch() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 148_000).await;

    enable_fetch_response_stage(&mut ctx, &page, 148_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 148_006, fixture.url("/plain?response-fail"))
        .await;
    ctx.sent.clear();

    let fetch_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            148_007,
            format!(
                r#"
                globalThis.__p0ResponseStageFail = 'pending';
                fetch({fetch_url:?})
                    .then(response => response.text())
                    .then(text => {{ globalThis.__p0ResponseStageFail = 'unexpected:' + text; }})
                    .catch(error => {{ globalThis.__p0ResponseStageFail = 'error:' + error.name; }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "Fetch response-stage fail pause",
    )
    .await;
    fail_paused_response(&mut ctx, &page, 148_008, &paused).await;

    let completed = wait_for_global_string(&mut ctx, &page, 148_009, "__p0ResponseStageFail").await;
    let value = completed["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("response-stage fail result: {completed}"));
    assert!(
        value.starts_with("error:") && value != "error:",
        "response-stage fail should reject page fetch: {completed}"
    );
}

// P0 browser contract source:
// Chromium http/tests/inspector-protocol/fetch/disable-with-response-in-flight.js.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_fetch_disable_with_response_in_flight_completes_request() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 158_000).await;

    enable_fetch_request_stage(&mut ctx, &page, 158_005, "*api-response-stage*").await;
    page.navigate(
        &mut ctx,
        158_006,
        fixture.url("/plain?disable-response-in-flight"),
    )
    .await;
    ctx.sent.clear();

    let fetch_path = "/api-response-stage?delay=0.2&disable-in-flight";
    let fetch_url = fixture.url(fetch_path);
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            158_007,
            format!(
                r#"
                globalThis.__p0DisableResponseInFlight = 'pending';
                fetch({fetch_url:?})
                    .then(response => response.text())
                    .then(text => {{ globalThis.__p0DisableResponseInFlight = text; }})
                    .catch(error => {{ globalThis.__p0DisableResponseInFlight = 'error:' + error.name; }});
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_fetch_request_stage_pause(
        &mut ctx,
        &page,
        fetch_path,
        "Fetch request-stage pause before disable",
    )
    .await;
    let network_id = paused["params"]["networkId"]
        .as_str()
        .unwrap_or_else(|| panic!("request-stage networkId: {paused}"))
        .to_owned();
    continue_paused_request_with_response_interception(&mut ctx, &page, 158_008, &paused).await;
    ctx.sent.clear();

    page.expect_empty_command(&mut ctx, 158_009, "Fetch.disable", json!({}))
        .await;

    wait_for_loading_finished(
        &mut ctx,
        &page,
        &network_id,
        "Fetch.disable response in-flight completion",
    )
    .await;
    let completed = wait_for_global_string_value(
        &mut ctx,
        &page,
        158_010,
        "__p0DisableResponseInFlight",
        "response-stage body",
        "Fetch.disable response in-flight page completion",
    )
    .await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!("response-stage body"),
        "{completed}"
    );
    assert_no_late_request_pause(
        &ctx,
        fetch_path,
        "Fetch.disable should let the in-flight response complete without a late pause",
    );
}

// P0 browser contract source:
// Chromium http/tests/inspector-protocol/fetch/disable-with-response-in-flight.js,
// mirrored onto XHR because Playwright route() and CDPSession users rely on the
// same subresource response-stage disable semantics for XHR.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_xhr_disable_with_response_in_flight_completes_request() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 159_000).await;

    enable_xhr_request_stage(&mut ctx, &page, 159_005, "*api-response-stage*").await;
    page.navigate(
        &mut ctx,
        159_006,
        fixture.url("/plain?xhr-disable-response-in-flight"),
    )
    .await;
    ctx.sent.clear();

    let xhr_path = "/api-response-stage?delay=0.2&xhr-disable-in-flight";
    let xhr_url = fixture.url(xhr_path);
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            159_007,
            format!(
                r#"
                globalThis.__p0XhrDisableResponseInFlight = 'pending';
                const xhr = new XMLHttpRequest();
                xhr.open('GET', {xhr_url:?}, true);
                xhr.onload = () => {{
                    globalThis.__p0XhrDisableResponseInFlight = xhr.responseText;
                }};
                xhr.onerror = () => {{
                    globalThis.__p0XhrDisableResponseInFlight = 'error';
                }};
                xhr.send();
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_xhr_request_stage_pause(
        &mut ctx,
        &page,
        xhr_path,
        "XHR request-stage pause before disable",
    )
    .await;
    let network_id = paused["params"]["networkId"]
        .as_str()
        .unwrap_or_else(|| panic!("XHR request-stage networkId: {paused}"))
        .to_owned();
    continue_paused_request_with_response_interception(&mut ctx, &page, 159_008, &paused).await;
    ctx.sent.clear();

    page.expect_empty_command(&mut ctx, 159_009, "Fetch.disable", json!({}))
        .await;

    wait_for_loading_finished(
        &mut ctx,
        &page,
        &network_id,
        "Fetch.disable XHR response in-flight completion",
    )
    .await;
    let completed = wait_for_global_string_value(
        &mut ctx,
        &page,
        159_010,
        "__p0XhrDisableResponseInFlight",
        "response-stage body",
        "Fetch.disable XHR response in-flight page completion",
    )
    .await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!("response-stage body"),
        "{completed}"
    );
    assert_no_late_request_pause(
        &ctx,
        xhr_path,
        "Fetch.disable should let the in-flight XHR response complete without a late pause",
    );
}

// P0 browser contract source:
// Playwright CDPSession XHR response-stage paused body inspection and continue.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_xhr_response_stage_get_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 149_000).await;

    enable_xhr_response_stage(&mut ctx, &page, 149_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 149_006, fixture.url("/plain?xhr-response-stage"))
        .await;
    ctx.sent.clear();

    let xhr_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            149_007,
            format!(
                r#"
                globalThis.__p0XhrResponseStage = 'pending';
                const xhr = new XMLHttpRequest();
                xhr.open('GET', {xhr_url:?}, true);
                xhr.onload = () => {{
                    globalThis.__p0XhrResponseStage = JSON.stringify({{
                        status: xhr.status,
                        body: xhr.responseText
                    }});
                }};
                xhr.onerror = () => {{ globalThis.__p0XhrResponseStage = 'error'; }};
                xhr.send();
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_xhr_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "XHR response-stage pause",
    )
    .await;
    assert_response_stage_header(&paused, "x-smoke-response-stage", "paused");

    let body = get_paused_response_body(&mut ctx, &page, 149_008, &paused).await;
    assert_eq!(
        body["result"]["body"],
        json!("response-stage body"),
        "{body}"
    );
    assert_eq!(body["result"]["base64Encoded"], json!(false), "{body}");

    continue_paused_response(&mut ctx, &page, 149_009, &paused).await;
    let completed = wait_for_global_string(&mut ctx, &page, 149_010, "__p0XhrResponseStage").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!(r#"{"status":200,"body":"response-stage body"}"#),
        "{completed}"
    );
}

// P0 browser contract source:
// Playwright CDPSession XHR response-stage stream body lifecycle.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_xhr_response_stage_stream_body_then_continue() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 152_000).await;

    enable_xhr_response_stage(&mut ctx, &page, 152_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 152_006, fixture.url("/plain?xhr-stream"))
        .await;
    ctx.sent.clear();

    let xhr_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            152_007,
            format!(
                r#"
                globalThis.__p0XhrResponseStageStreamBody = 'pending';
                const xhr = new XMLHttpRequest();
                xhr.open('GET', {xhr_url:?}, true);
                xhr.onload = () => {{
                    globalThis.__p0XhrResponseStageStreamBody = xhr.responseText;
                }};
                xhr.onerror = () => {{ globalThis.__p0XhrResponseStageStreamBody = 'error'; }};
                xhr.send();
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_xhr_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "XHR response-stage stream pause",
    )
    .await;
    let request_id = paused_request_id(&paused);

    let chunk = read_paused_response_stream(&mut ctx, &page, 152_008, 152_009, &request_id).await;
    assert_eq!(
        chunk["result"]["data"],
        json!("response-stage body"),
        "{chunk}"
    );
    assert_eq!(chunk["result"]["base64Encoded"], json!(false), "{chunk}");
    assert_eq!(chunk["result"]["eof"], json!(true), "{chunk}");

    let continue_after_taken = page
        .command(
            &mut ctx,
            152_010,
            "Fetch.continueResponse",
            json!({ "requestId": paused["params"]["requestId"] }),
        )
        .await;
    assert_eq!(
        continue_after_taken["error"]["message"],
        json!("Unable to continue request as is after body is taken"),
        "{continue_after_taken}"
    );

    fulfill_paused_response(
        &mut ctx,
        &page,
        152_011,
        &paused,
        200,
        "cmVzcG9uc2Utc3RhZ2UgYm9keQ==",
    )
    .await;
    let completed =
        wait_for_global_string(&mut ctx, &page, 152_012, "__p0XhrResponseStageStreamBody").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!("response-stage body"),
        "{completed}"
    );
}

// P0 browser contract source:
// Playwright route.fulfill-like XHR response-stage body override behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_xhr_response_stage_fulfill_overrides_body() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 153_000).await;

    enable_xhr_response_stage(&mut ctx, &page, 153_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 153_006, fixture.url("/plain?xhr-fulfill"))
        .await;
    ctx.sent.clear();

    let xhr_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            153_007,
            format!(
                r#"
                globalThis.__p0XhrResponseStageFulfill = 'pending';
                const xhr = new XMLHttpRequest();
                xhr.open('GET', {xhr_url:?}, true);
                xhr.onload = () => {{
                    globalThis.__p0XhrResponseStageFulfill = JSON.stringify({{
                        status: xhr.status,
                        header: xhr.getResponseHeader('x-p0-fulfilled'),
                        body: xhr.responseText
                    }});
                }};
                xhr.onerror = () => {{ globalThis.__p0XhrResponseStageFulfill = 'error'; }};
                xhr.send();
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_xhr_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "XHR response-stage fulfill pause",
    )
    .await;
    fulfill_paused_response(
        &mut ctx,
        &page,
        153_008,
        &paused,
        203,
        "eGhyLWZ1bGZpbGxlZA==",
    )
    .await;

    let completed =
        wait_for_global_string(&mut ctx, &page, 153_009, "__p0XhrResponseStageFulfill").await;
    assert_eq!(
        completed["result"]["result"]["value"],
        json!(r#"{"status":203,"header":"yes","body":"xhr-fulfilled"}"#),
        "{completed}"
    );
}

// P0 browser contract source:
// Playwright route.abort-like XHR response-stage failRequest behavior.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_xhr_response_stage_fail_rejects_xhr() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 154_000).await;

    enable_xhr_response_stage(&mut ctx, &page, 154_005, "*api-response-stage*").await;
    page.navigate(&mut ctx, 154_006, fixture.url("/plain?xhr-fail"))
        .await;
    ctx.sent.clear();

    let xhr_url = fixture.url("/api-response-stage");
    let scheduled = page
        .evaluate_value(
            &mut ctx,
            154_007,
            format!(
                r#"
                globalThis.__p0XhrResponseStageFail = 'pending';
                const xhr = new XMLHttpRequest();
                xhr.open('GET', {xhr_url:?}, true);
                xhr.onload = () => {{
                    globalThis.__p0XhrResponseStageFail = 'unexpected:' + xhr.responseText;
                }};
                xhr.onerror = () => {{
                    globalThis.__p0XhrResponseStageFail = 'error:' + xhr.status;
                }};
                xhr.onabort = () => {{
                    globalThis.__p0XhrResponseStageFail = 'abort:' + xhr.status;
                }};
                xhr.send();
                'scheduled'
                "#
            ),
        )
        .await;
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    let paused = wait_for_xhr_response_stage_pause(
        &mut ctx,
        &page,
        "/api-response-stage",
        "XHR response-stage fail pause",
    )
    .await;
    fail_paused_response(&mut ctx, &page, 154_008, &paused).await;

    let completed =
        wait_for_global_string(&mut ctx, &page, 154_009, "__p0XhrResponseStageFail").await;
    let value = completed["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("XHR response-stage fail result: {completed}"));
    assert!(
        value == "error:0" || value == "abort:0",
        "response-stage fail should reject XHR without delivering the response body: {completed}"
    );
}
