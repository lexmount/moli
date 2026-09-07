use super::super::tests_cdp_smoke_fixture::{SmokeFixtureServer, fixture_get};
use super::super::*;
use super::support::{
    attached_smoke_session, evaluate_return_by_value, evaluate_string, navigate_and_take_response,
};
use serde_json::json;

// Capability source: docs/WEB_CAPABILITIES.md proxy/identity request headers.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_emulation_user_agent_override_reaches_navigation() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 116_000).await;
    let token = "chromium-import-ua";

    ctx.process_async(json!({
        "id": 116_005,
        "method": "Emulation.setUserAgentOverride",
        "sessionId": attached.session_id,
        "params": { "userAgent": "MoliChromiumImport/1.0" }
    }))
    .await;
    ctx.expect_result(116_005, json!({}), Some(&attached.session_id));
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        116_006,
        fixture.url(&format!("/profile-headers?token={token}")),
    )
    .await;

    let raw = fixture_get(&fixture, &format!("/profile-result?token={token}")).await;
    let profile: Value = serde_json::from_slice(&raw.body).expect("profile result json");
    assert_eq!(profile["userAgent"], "MoliChromiumImport/1.0");
}

// Chromium source:
// third_party/blink/web_tests/inspector-protocol/page/get-layout-metrics.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_emulation_device_metrics_affect_layout_metrics() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 117_000).await;

    ctx.process_async(json!({
        "id": 117_005,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": attached.session_id,
        "params": {
            "width": 640,
            "height": 480,
            "deviceScaleFactor": 2,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(117_005, json!({}), Some(&attached.session_id));
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        117_006,
        "data:text/html,<body>metrics</body>".to_owned(),
    )
    .await;
    ctx.process_async(json!({
        "id": 117_007,
        "method": "Page.getLayoutMetrics",
        "sessionId": attached.session_id
    }))
    .await;
    let metrics = take_response_by_id(&mut ctx, 117_007);
    assert_eq!(metrics["result"]["cssLayoutViewport"]["clientWidth"], 640);
    assert_eq!(metrics["result"]["cssLayoutViewport"]["clientHeight"], 480);
}

// Chromium source:
// content/browser/devtools/protocol/devtools_protocol_browsertest.cc
// DevToolsProtocolDeviceEmulationTest.DeviceSize
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_emulation_device_metrics_persist_across_navigation_until_clear() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 117_050).await;

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        117_051,
        "data:text/html,<body>baseline</body>".to_owned(),
    )
    .await;
    let baseline = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_052,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;

    ctx.process_async(json!({
        "id": 117_055,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": attached.session_id,
        "params": {
            "width": 800,
            "height": 600,
            "deviceScaleFactor": 1,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(117_055, json!({}), Some(&attached.session_id));

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        117_056,
        "data:text/html,<body>first</body>".to_owned(),
    )
    .await;
    let first = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_057,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;
    assert_eq!(first, r#"{"innerWidth":800,"innerHeight":600}"#);

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        117_058,
        "data:text/html,<body>second</body>".to_owned(),
    )
    .await;
    let second = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_059,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;
    assert_eq!(second, r#"{"innerWidth":800,"innerHeight":600}"#);

    ctx.process_async(json!({
        "id": 117_060,
        "method": "Emulation.clearDeviceMetricsOverride",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(117_060, json!({}), Some(&attached.session_id));
    let cleared = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_061,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;
    assert_eq!(cleared, baseline);
}

// Regression coverage for clearing after an override was installed at
// document start and then replaced by a live metrics override.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_emulation_device_metrics_clear_after_second_live_override() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 117_070).await;

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        117_071,
        "data:text/html,<body>baseline</body>".to_owned(),
    )
    .await;
    let baseline = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_072,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;

    ctx.process_async(json!({
        "id": 117_073,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": attached.session_id,
        "params": {
            "width": 800,
            "height": 600,
            "deviceScaleFactor": 1,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(117_073, json!({}), Some(&attached.session_id));

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        117_074,
        "data:text/html,<body>emulated</body>".to_owned(),
    )
    .await;
    let document_start_surface = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_075,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;
    assert_eq!(
        document_start_surface,
        r#"{"innerWidth":800,"innerHeight":600}"#
    );

    ctx.process_async(json!({
        "id": 117_076,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": attached.session_id,
        "params": {
            "width": 1024,
            "height": 768,
            "deviceScaleFactor": 1,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(117_076, json!({}), Some(&attached.session_id));
    let live_surface = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_077,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;
    assert_eq!(live_surface, r#"{"innerWidth":1024,"innerHeight":768}"#);

    ctx.process_async(json!({
        "id": 117_078,
        "method": "Emulation.clearDeviceMetricsOverride",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(117_078, json!({}), Some(&attached.session_id));
    let cleared = evaluate_string(
        &mut ctx,
        &attached.session_id,
        117_079,
        "JSON.stringify({innerWidth, innerHeight})",
    )
    .await;
    assert_eq!(cleared, baseline);
}

// Chromium source:
// third_party/blink/web_tests/inspector-protocol/emulation/emulation-device-override.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_emulation_device_metrics_hot_apply_window_surface() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 117_100).await;
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        117_106,
        "data:text/html,<body>metrics</body>".to_owned(),
    )
    .await;
    let original_response = evaluate_return_by_value(
        &mut ctx,
        &attached.session_id,
        117_107,
        "JSON.stringify({innerWidth, innerHeight, devicePixelRatio})",
    )
    .await;
    let original_payload: Value = serde_json::from_str(
        original_response["result"]["result"]["value"]
            .as_str()
            .unwrap_or_else(|| panic!("original viewport payload: {original_response}")),
    )
    .expect("original viewport payload json");

    ctx.process_async(json!({
        "id": 117_108,
        "method": "Emulation.setDeviceMetricsOverride",
        "sessionId": attached.session_id,
        "params": {
            "width": 375,
            "height": 667,
            "deviceScaleFactor": 2,
            "mobile": true,
            "screenWidth": 400,
            "screenHeight": 700
        }
    }))
    .await;
    ctx.expect_result(117_108, json!({}), Some(&attached.session_id));

    let response = evaluate_return_by_value(
        &mut ctx,
        &attached.session_id,
        117_109,
        r#"JSON.stringify({
          innerWidth,
          innerHeight,
          outerWidth,
          outerHeight,
          devicePixelRatio,
          screenWidth: screen.width,
          screenHeight: screen.height
        })"#,
    )
    .await;
    let payload: Value = serde_json::from_str(
        response["result"]["result"]["value"]
            .as_str()
            .unwrap_or_else(|| panic!("viewport payload: {response}")),
    )
    .expect("viewport payload json");
    assert_eq!(payload["innerWidth"], 375);
    assert_eq!(payload["innerHeight"], 667);
    assert_eq!(payload["outerWidth"], 375);
    assert_eq!(payload["outerHeight"], 667);
    assert_eq!(payload["devicePixelRatio"], 2);
    assert_eq!(payload["screenWidth"], 400);
    assert_eq!(payload["screenHeight"], 700);

    ctx.process_async(json!({
        "id": 117_110,
        "method": "Emulation.clearDeviceMetricsOverride",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(117_110, json!({}), Some(&attached.session_id));
    let cleared_response = evaluate_return_by_value(
        &mut ctx,
        &attached.session_id,
        117_111,
        "JSON.stringify({innerWidth, innerHeight, devicePixelRatio})",
    )
    .await;
    let cleared_payload: Value = serde_json::from_str(
        cleared_response["result"]["result"]["value"]
            .as_str()
            .unwrap_or_else(|| panic!("cleared viewport payload: {cleared_response}")),
    )
    .expect("cleared viewport payload json");
    assert_eq!(cleared_payload, original_payload);
}

// Capability source: docs/WEB_CAPABILITIES.md browser/device APIs exposed to pages.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_emulation_geolocation_override_runtime_surface() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 118_000).await;
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        118_005,
        fixture.url("/plain"),
    )
    .await;
    ctx.process_async(json!({
        "id": 118_006,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": attached.session_id,
        "params": { "latitude": 48.85837, "longitude": 2.294481, "accuracy": 7 }
    }))
    .await;
    ctx.expect_result(118_006, json!({}), Some(&attached.session_id));

    ctx.process_async(json!({
        "id": 118_007,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "awaitPromise": true,
            "returnByValue": true,
            "expression": r#"
                new Promise((resolve) => {
                    navigator.geolocation.getCurrentPosition(
                        (position) => resolve(JSON.stringify({
                            latitude: position.coords.latitude,
                            longitude: position.coords.longitude,
                            accuracy: position.coords.accuracy
                        })),
                        (error) => resolve(`error:${error.code}:${error.message}`)
                    );
                })
            "#
        }
    }))
    .await;
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native Geolocation result",
        |message| message["id"] == json!(118_007),
    )
    .await;
    let response = take_response_by_id(&mut ctx, 118_007);
    let payload: Value = serde_json::from_str(
        response["result"]["result"]["value"]
            .as_str()
            .unwrap_or_else(|| panic!("geolocation payload: {response}")),
    )
    .expect("geolocation payload json");
    assert_eq!(payload["latitude"], 48.85837);
    assert_eq!(payload["longitude"], 2.294481);
    assert_eq!(payload["accuracy"], 7);
}

// Capability source: docs/WEB_CAPABILITIES.md page interaction/input.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_input_insert_text_into_focused_control() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 119_000).await;
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        119_005,
        "data:text/html,<input id='field'>".to_owned(),
    )
    .await;
    evaluate_string(
        &mut ctx,
        &attached.session_id,
        119_006,
        "document.getElementById('field').focus(); 'focused'",
    )
    .await;
    ctx.process_async(json!({
        "id": 119_007,
        "method": "Input.insertText",
        "sessionId": attached.session_id,
        "params": { "text": "hello" }
    }))
    .await;
    ctx.expect_result(119_007, json!({}), Some(&attached.session_id));
    let value = evaluate_string(
        &mut ctx,
        &attached.session_id,
        119_008,
        "document.getElementById('field').value",
    )
    .await;
    assert_eq!(value, "hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_input_dispatch_mouse_event_uses_layout_hit_testing() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 120_000).await;
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        120_005,
        "data:text/html,<button id='btn' onclick='window.__clicked = true' style='position:absolute;left:0;top:0;width:100px;height:100px'>go</button>".to_owned(),
    )
    .await;
    for (offset, event_type, buttons) in [(0, "mousePressed", 1), (1, "mouseReleased", 0)] {
        ctx.process_async(json!({
            "id": 120_006 + offset,
            "method": "Input.dispatchMouseEvent",
            "sessionId": attached.session_id,
            "params": {
                "type": event_type,
                "x": 10,
                "y": 10,
                "button": "left",
                "buttons": buttons,
                "clickCount": 1
            }
        }))
        .await;
        ctx.expect_result(120_006 + offset, json!({}), Some(&attached.session_id));
    }
    let clicked = evaluate_return_by_value(
        &mut ctx,
        &attached.session_id,
        120_010,
        "Boolean(window.__clicked)",
    )
    .await;
    assert_eq!(clicked["result"]["result"]["value"], true);
}

// Capability source: docs/WEB_CAPABILITIES.md Cookie / state persistence.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_storage_cookie_round_trip_and_clear() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 121_005,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let created = take_response_by_id(&mut ctx, 121_005);
    let browser_context_id = created["result"]["browserContextId"]
        .as_str()
        .unwrap_or_else(|| panic!("browserContextId: {created}"))
        .to_owned();
    ctx.process_async(json!({
        "id": 121_006,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": browser_context_id,
            "cookies": [
                { "name": "session", "value": "abc", "url": "https://state.example/path" }
            ]
        }
    }))
    .await;
    ctx.expect_result(121_006, json!({}), None);
    ctx.process_async(json!({
        "id": 121_007,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let cookies = take_response_by_id(&mut ctx, 121_007);
    assert!(
        cookies["result"]["cookies"]
            .as_array()
            .is_some_and(|items| {
                items.iter().any(|cookie| {
                    cookie["name"] == json!("session") && cookie["value"] == json!("abc")
                })
            }),
        "{cookies}"
    );
    ctx.process_async(json!({
        "id": 121_008,
        "method": "Storage.clearCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    ctx.expect_result(121_008, json!({}), None);
    ctx.process_async(json!({
        "id": 121_009,
        "method": "Storage.getCookies",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;
    let empty = take_response_by_id(&mut ctx, 121_009);
    assert_eq!(empty["result"]["cookies"], json!([]));
}
