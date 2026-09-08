use super::super::tests_cdp_smoke_fixture::{SmokeFixtureServer, fixture_get};
use super::super::*;
use super::support::{
    attached_smoke_session, navigate_and_take_response, request_will_be_sent_for_suffix,
    response_received_for_suffix,
};
use crate::testing::wait_until_messages;
use serde_json::json;

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/network/set-extra-http-headers.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_network_extra_http_headers_reach_navigation() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 99_000).await;
    let token = "chromium-import-extra-headers";

    ctx.process_async(json!({
        "id": 99_005,
        "method": "Network.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(99_005, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 99_006,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": attached.session_id,
        "params": { "headers": { "X-DevTools-Test": "Hello, world!" } }
    }))
    .await;
    ctx.expect_result(99_006, json!({}), Some(&attached.session_id));
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        99_007,
        fixture.url(&format!("/profile-headers?token={token}")),
    )
    .await;

    let raw = fixture_get(&fixture, &format!("/profile-result?token={token}")).await;
    let profile: Value = serde_json::from_slice(&raw.body).expect("profile result json");
    assert_eq!(profile["devtoolsTest"], "Hello, world!");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/network/url-blocking.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_network_blocked_urls_fail_fetch() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 115_000).await;
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        115_005,
        fixture.url("/plain?blocked-urls"),
    )
    .await;

    ctx.process_async(json!({
        "id": 115_006,
        "method": "Network.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(115_006, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 115_007,
        "method": "Network.setBlockedURLs",
        "sessionId": attached.session_id,
        "params": { "urls": ["*api*"] }
    }))
    .await;
    ctx.expect_result(115_007, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 115_008,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": format!(
                "fetch({:?}).then(() => 'resolved', error => error.constructor.name)",
                fixture.url("/api")
            ),
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 115_008);
    assert_eq!(result["result"]["result"]["value"], "TypeError");
    wait_until_messages(
        &mut ctx,
        attached.session_id.as_str(),
        "blocked runtime fetch loadingFailed",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"].is_string()
            })
        },
    )
    .await;
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.loadingFailed")
            && message["params"]["requestId"].is_string()
    }));
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/fetch/calls-while-not-enabled.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_fetch_paused_request_methods_error_when_not_enabled() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 95_000).await;

    for (offset, method) in [
        "Fetch.fulfillRequest",
        "Fetch.failRequest",
        "Fetch.continueRequest",
        "Fetch.continueWithAuth",
        "Fetch.getResponseBody",
        "Fetch.takeResponseBodyAsStream",
    ]
    .into_iter()
    .enumerate()
    {
        let id = 95_005 + offset as u64;
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": attached.session_id,
            "params": {
                "requestId": "does-not-matter",
                "responseCode": 404,
                "errorReason": "Failed",
                "authChallengeResponse": { "response": "Default" }
            }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["sessionId"], attached.session_id);
        assert!(response.get("error").is_some(), "{method}: {response}");
    }
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/fetch/navigation-request-no-body.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_fetch_fulfill_navigation_without_body() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 122_000).await;

    ctx.process_async(json!({
        "id": 122_005,
        "method": "Network.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(122_005, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 122_006,
        "method": "Fetch.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(122_006, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 122_007,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": { "url": fixture.url("/plain?fulfill-no-body") }
    }))
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .unwrap_or_else(|| panic!("missing requestPaused: {:?}", ctx.sent));
    let request_id = paused["params"]["requestId"]
        .as_str()
        .unwrap_or_else(|| panic!("paused request id: {paused}"))
        .to_owned();
    ctx.process_async(json!({
        "id": 122_008,
        "method": "Fetch.fulfillRequest",
        "sessionId": attached.session_id,
        "params": { "requestId": request_id, "responseCode": 200 }
    }))
    .await;
    ctx.expect_result(122_008, json!({}), Some(&attached.session_id));
    crate::testing::wait_until_navigation_document_load(
        &mut ctx,
        122_007,
        Some(&attached.session_id),
    )
    .await;
    let navigation = take_response_by_id(&mut ctx, 122_007);
    assert_eq!(navigation["result"]["frameId"], attached.target_id);
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["response"]["status"] == json!(200)
    }));
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/fetch/fetch-take-body-invalid-id.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_fetch_take_response_body_stream_invalid_id_errors() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 100_000).await;

    ctx.process_async(json!({
        "id": 100_005,
        "method": "Fetch.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(100_005, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 100_006,
        "method": "Fetch.takeResponseBodyAsStream",
        "sessionId": attached.session_id,
        "params": { "requestId": "I'm not there" }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 100_006);
    assert_eq!(response["sessionId"], attached.session_id);
    assert!(response["error"].is_object(), "{response}");
}

// Chromium source:
// third_party/blink/web_tests/inspector-protocol/network/resource-type.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_network_resource_type_matrix() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery_and_optional_resource_fetch_mask(
        false,
        moli_core::OptionalResourceFetchMask::ALL,
    );
    let attached = attached_smoke_session(&mut ctx, 96_000).await;

    ctx.process_async(json!({
        "id": 96_004,
        "method": "Page.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(96_004, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 96_005,
        "method": "Network.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(96_005, json!({}), Some(&attached.session_id));
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        96_006,
        fixture.url("/chromium-resource-type-page"),
    )
    .await;
    wait_until_messages(
        &mut ctx,
        Some(attached.session_id.as_str()),
        "Page.loadEventFired before reading the parser-created XHR promise",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;
    ctx.process_async(json!({
        "id": 96_007,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "globalThis.__smokeResourceXhrDone",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_messages(
        &mut ctx,
        Some(attached.session_id.as_str()),
        "Runtime.evaluate XHR completion response",
        |messages| {
            messages
                .iter()
                .any(|message| message["id"] == json!(96_007))
        },
    )
    .await;
    let xhr_done = take_response_by_id(&mut ctx, 96_007);
    assert_eq!(xhr_done["result"]["result"]["value"]["status"], 200);

    wait_until_messages(
        &mut ctx,
        Some(attached.session_id.as_str()),
        "Network.loadingFinished for Chromium resource matrix",
        |messages| {
            [
                "/chromium-resource-type-page",
                "/chromium-resource-style.css",
                "/chromium-resource-script.js",
                "/chromium-resource-image.png",
                "/chromium-resource-audio.wav",
                "/chromium-resource-video.ogv",
                "/chromium-resource-captions.vtt",
                "/chromium-resource-xhr.bin",
            ]
            .iter()
            .all(|suffix| {
                response_received_for_suffix(messages, suffix).is_some()
                    && request_will_be_sent_for_suffix(messages, suffix).is_some()
            })
        },
    )
    .await;

    for (suffix, expected_type, expected_body) in [
        ("/chromium-resource-type-page", "Document", None),
        (
            "/chromium-resource-style.css",
            "Stylesheet",
            Some(("main { color: rgb(31, 41, 59); }", false)),
        ),
        (
            "/chromium-resource-script.js",
            "Script",
            Some(("globalThis.__smokeChromiumResourceScript = true;", false)),
        ),
        (
            "/chromium-resource-image.png",
            "Image",
            Some((
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=",
                true,
            )),
        ),
        (
            "/chromium-resource-audio.wav",
            "Media",
            Some(("AP9tb2xpLW1lZGlh", true)),
        ),
        (
            "/chromium-resource-video.ogv",
            "Media",
            Some(("AP9tb2xpLW1lZGlh", true)),
        ),
        (
            "/chromium-resource-captions.vtt",
            "TextTrack",
            Some(("WEBVTT\n\n00:00.000 --> 00:01.000\ncaption\n", false)),
        ),
        (
            "/chromium-resource-xhr.bin",
            "XHR",
            Some(("AP9tb2xpLXhocg==", true)),
        ),
    ] {
        let request = request_will_be_sent_for_suffix(&ctx.sent, suffix)
            .unwrap_or_else(|| panic!("requestWillBeSent for {suffix}: {:?}", ctx.sent));
        assert_eq!(
            request["params"]["type"], expected_type,
            "{suffix}: {request}"
        );
        let response = response_received_for_suffix(&ctx.sent, suffix)
            .unwrap_or_else(|| panic!("responseReceived for {suffix}: {:?}", ctx.sent));
        assert_eq!(
            response["params"]["type"], expected_type,
            "{suffix}: {response}"
        );

        if let Some((expected_body, expected_base64_encoded)) = expected_body {
            let request_id = response["params"]["requestId"]
                .as_str()
                .unwrap_or_else(|| panic!("request id for {suffix}: {response}"))
                .to_owned();
            ctx.process_async(json!({
                "id": 96_020,
                "method": "Network.getResponseBody",
                "sessionId": attached.session_id,
                "params": { "requestId": request_id }
            }))
            .await;
            wait_until_messages(
                &mut ctx,
                Some(attached.session_id.as_str()),
                "Network.getResponseBody resource matrix response",
                |messages| {
                    messages
                        .iter()
                        .any(|message| message["id"] == json!(96_020))
                },
            )
            .await;
            let body = take_response_by_id(&mut ctx, 96_020);
            assert!(body.get("error").is_none(), "{suffix}: {body}");
            assert_eq!(
                body["result"]["base64Encoded"],
                json!(expected_base64_encoded),
                "{suffix}: {body}"
            );
            assert_eq!(
                body["result"]["body"],
                json!(expected_body),
                "{suffix}: {body}"
            );
        }
    }
}
