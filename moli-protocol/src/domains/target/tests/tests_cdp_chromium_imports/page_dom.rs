use super::super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::super::*;
use super::support::{attached_smoke_session, evaluate_string, navigate_and_take_response};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;

// Chromium source:
// third_party/blink/web_tests/inspector-protocol/page/add-script-to-evaluate-on-load.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_preload_scripts_run_in_addition_order_and_remove() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 93_000).await;

    ctx.process_async(json!({
        "id": 93_005,
        "method": "Page.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(93_005, json!({}), Some(&attached.session_id));

    let mut identifiers = Vec::new();
    for offset in 0..5 {
        let id = 93_010 + offset;
        ctx.process_async(json!({
            "id": id,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": attached.session_id,
            "params": {
                "source": format!(
                    "globalThis.__chromiumImportOrder = globalThis.__chromiumImportOrder || []; \
                     globalThis.__chromiumImportOrder.push({offset});"
                )
            }
        }))
        .await;
        let installed = take_response_by_id(&mut ctx, id);
        identifiers.push(
            installed["result"]["identifier"]
                .as_str()
                .unwrap_or_else(|| panic!("preload identifier: {installed}"))
                .to_owned(),
        );
    }

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        93_020,
        fixture.url("/plain?preload-order"),
    )
    .await;
    ctx.process_async(json!({
        "id": 93_021,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "globalThis.__chromiumImportOrder.join(',')",
            "returnByValue": true
        }
    }))
    .await;
    let order = take_response_by_id(&mut ctx, 93_021);
    assert_eq!(order["result"]["result"]["value"], "0,1,2,3,4");

    for (offset, identifier) in identifiers.into_iter().enumerate() {
        let id = 93_030 + offset as u64;
        ctx.process_async(json!({
            "id": id,
            "method": "Page.removeScriptToEvaluateOnNewDocument",
            "sessionId": attached.session_id,
            "params": { "identifier": identifier }
        }))
        .await;
        let removed = take_response_by_id(&mut ctx, id);
        assert!(removed.get("error").is_none(), "{removed}");
    }

    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        93_040,
        fixture.url("/plain?preload-removed"),
    )
    .await;
    ctx.process_async(json!({
        "id": 93_041,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": {
            "expression": "typeof globalThis.__chromiumImportOrder",
            "returnByValue": true
        }
    }))
    .await;
    let removed_value = take_response_by_id(&mut ctx, 93_041);
    assert_eq!(removed_value["result"]["result"]["value"], "undefined");
}

// Capability source: docs/WEB_CAPABILITIES.md page screenshot.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_page_capture_screenshot_returns_png() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 110_000).await;

    ctx.process_async(json!({
        "id": 110_005,
        "method": "Page.captureScreenshot",
        "sessionId": attached.session_id,
        "params": { "format": "png" }
    }))
    .await;
    let screenshot = take_response_by_id(&mut ctx, 110_005);
    let encoded = screenshot["result"]["data"]
        .as_str()
        .unwrap_or_else(|| panic!("screenshot response missing base64 data: {screenshot}"));
    let png = STANDARD
        .decode(encoded)
        .expect("screenshot response must contain valid base64");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(screenshot["sessionId"], json!(attached.session_id));
}

// Capability source: docs/WEB_CAPABILITIES.md navigation/session state.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_page_navigation_history_round_trip() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 111_000).await;
    let first_url = fixture.url("/plain?history=first");
    let second_url = fixture.url("/plain?history=second");

    navigate_and_take_response(&mut ctx, &attached.session_id, 111_005, first_url.clone()).await;
    navigate_and_take_response(&mut ctx, &attached.session_id, 111_006, second_url.clone()).await;
    ctx.process_async(json!({
        "id": 111_007,
        "method": "Page.getNavigationHistory",
        "sessionId": attached.session_id
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 111_007);
    assert_eq!(history["result"]["currentIndex"], 2);
    assert_eq!(history["result"]["entries"][0]["url"], "about:blank");
    assert_eq!(
        history["result"]["entries"][0]["userTypedURL"],
        "about:blank"
    );
    assert_eq!(
        history["result"]["entries"][0]["transitionType"],
        "auto_toplevel"
    );
    assert_eq!(history["result"]["entries"][1]["url"], first_url);
    assert_eq!(history["result"]["entries"][1]["userTypedURL"], first_url);
    assert_eq!(history["result"]["entries"][2]["url"], second_url);
    let first_entry_id = history["result"]["entries"][1]["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("first history id: {history}"));

    ctx.process_async(json!({
        "id": 111_008,
        "method": "Page.navigateToHistoryEntry",
        "sessionId": attached.session_id,
        "params": { "entryId": first_entry_id }
    }))
    .await;
    ctx.expect_result(111_008, json!({}), Some(&attached.session_id));
    let href = evaluate_string(&mut ctx, &attached.session_id, 111_009, "location.href").await;
    assert_eq!(href, first_url);
}

// Capability source: docs/WEB_CAPABILITIES.md download ability.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_page_set_download_behavior_alias_contract() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 112_000).await;

    ctx.process_async(json!({
        "id": 112_005,
        "method": "Page.setDownloadBehavior",
        "sessionId": attached.session_id,
        "params": {
            "behavior": "allow",
            "downloadPath": "/tmp/moli-page-downloads",
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(112_005, json!({}), Some(&attached.session_id));
    let (browser_context_id, _) = ctx
        .conn
        .target_owner_identity_for_session(Some(&attached.session_id))
        .expect("attached page target should have a browser context");
    let settings = ctx
        .conn
        .download_policy_for_browser_context(Some(browser_context_id.as_str()));
    assert_eq!(
        settings.behavior,
        moli_core::browser::DownloadBehavior::Allow
    );
    assert_eq!(
        settings.download_path.as_deref(),
        Some("/tmp/moli-page-downloads")
    );
    assert!(
        !ctx.conn
            .automation_download_events_enabled_for_context(Some(&browser_context_id))
    );
    assert!(
        !ctx.conn
            .automation_download_events_enabled_for_context(None),
        "Page.setDownloadBehavior must not enable Browser download events"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/page/navigate-loader-id.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_page_navigate_loader_id_matches_network_event() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 98_000).await;

    ctx.process_async(json!({
        "id": 98_005,
        "method": "Page.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(98_005, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 98_006,
        "method": "Network.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(98_006, json!({}), Some(&attached.session_id));
    let navigation = navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        98_007,
        fixture.url("/plain?navigate-loader-id"),
    )
    .await;
    let request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(attached.session_id)
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/plain?navigate-loader-id"))
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing navigation requestWillBeSent: {:?}", ctx.sent));
    assert_eq!(
        navigation["result"]["loaderId"], request["params"]["loaderId"],
        "navigation={navigation} request={request}"
    );
}

// Chromium source:
// third_party/blink/web_tests/inspector-protocol/dom/resolve-node.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_dom_resolve_node_then_call_function() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 113_000).await;
    ctx.process_async(json!({
        "id": 113_010,
        "method": "Page.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(113_010, json!({}), Some(&attached.session_id));
    navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        113_005,
        fixture.url("/chromium-cdp-dom-page"),
    )
    .await;
    crate::testing::wait_until_message(
        &mut ctx,
        attached.session_id.as_str(),
        "Page.loadEventFired after chromium DOM page navigation",
        |message| {
            message["sessionId"] == json!(attached.session_id)
                && message["method"] == json!("Page.loadEventFired")
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 113_006,
        "method": "DOM.getDocument",
        "sessionId": attached.session_id
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 113_006);
    let root_id = document["result"]["root"]["nodeId"]
        .as_i64()
        .unwrap_or_else(|| panic!("root node id: {document}"));
    ctx.process_async(json!({
        "id": 113_007,
        "method": "DOM.querySelector",
        "sessionId": attached.session_id,
        "params": { "nodeId": root_id, "selector": "p.class1" }
    }))
    .await;
    let selected = take_response_by_id(&mut ctx, 113_007);
    let node_id = selected["result"]["nodeId"]
        .as_i64()
        .unwrap_or_else(|| panic!("selected node id: {selected}"));
    assert_ne!(node_id, 0, "querySelector should find p.class1: {selected}");
    ctx.process_async(json!({
        "id": 113_008,
        "method": "DOM.resolveNode",
        "sessionId": attached.session_id,
        "params": { "nodeId": node_id }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 113_008);
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "resolved object for root_id={root_id}, node_id={node_id}; document={document}; selected={selected}; resolved={resolved}"
            )
        })
        .to_owned();
    ctx.process_async(json!({
        "id": 113_009,
        "method": "Runtime.callFunctionOn",
        "sessionId": attached.session_id,
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.textContent; }",
            "returnByValue": true
        }
    }))
    .await;
    let text = take_response_by_id(&mut ctx, 113_009);
    assert_eq!(text["result"]["result"]["value"], "Paragraph Text");
}

// Chromium source:
// third_party/blink/web_tests/inspector-protocol/dom/get-box-model.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_dom_get_box_model_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 114_000).await;
    let navigation = navigate_and_take_response(
        &mut ctx,
        &attached.session_id,
        114_005,
        fixture.url("/chromium-cdp-hit-test-page"),
    )
    .await;
    let loader_id = navigation["result"]["loaderId"]
        .as_str()
        .expect("navigation loader id");
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        Some(&attached.session_id),
        &attached.target_id,
        loader_id,
    )
    .await;

    ctx.process_async(json!({
        "id": 114_006,
        "method": "DOM.getDocument",
        "sessionId": attached.session_id
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 114_006);
    let root_id = document["result"]["root"]["nodeId"]
        .as_i64()
        .unwrap_or_else(|| panic!("root node id: {document}"));
    ctx.process_async(json!({
        "id": 114_007,
        "method": "DOM.querySelector",
        "sessionId": attached.session_id,
        "params": { "nodeId": root_id, "selector": "#hit-target" }
    }))
    .await;
    let selected = take_response_by_id(&mut ctx, 114_007);
    let node_id = selected["result"]["nodeId"]
        .as_i64()
        .unwrap_or_else(|| panic!("selected node id: {selected}"));
    ctx.process_async(json!({
        "id": 114_008,
        "method": "DOM.getBoxModel",
        "sessionId": attached.session_id,
        "params": { "nodeId": node_id }
    }))
    .await;
    let box_model = take_response_by_id(&mut ctx, 114_008);
    assert_eq!(
        box_model["result"]["model"]["content"]
            .as_array()
            .expect("content quad")
            .len(),
        8
    );
}

// Chromium source:
// third_party/blink/web_tests/inspector-protocol/page/createIsolatedWorld.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_create_isolated_world_reports_context() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 94_000).await;

    ctx.process_async(json!({
        "id": 94_005,
        "method": "Runtime.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(94_005, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 94_006,
        "method": "Page.enable",
        "sessionId": attached.session_id
    }))
    .await;
    ctx.expect_result(94_006, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 94_007,
        "method": "Page.getFrameTree",
        "sessionId": attached.session_id
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 94_007);
    let main_frame_id = frame_tree["result"]["frameTree"]["frame"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("main frame id: {frame_tree}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 94_008,
        "method": "Page.createIsolatedWorld",
        "sessionId": attached.session_id,
        "params": { "frameId": main_frame_id, "worldName": "Test world" }
    }))
    .await;
    let created = take_response_by_id(&mut ctx, 94_008);
    let execution_context_id = created["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_else(|| panic!("executionContextId: {created}"));

    let context_event = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!(attached.session_id)
                && message["params"]["context"]["id"] == json!(execution_context_id)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing isolated world event: {:?}", ctx.sent));
    assert_eq!(context_event["params"]["context"]["name"], "Test world");
    assert_eq!(
        context_event["params"]["context"]["auxData"]["frameId"],
        attached.target_id
    );
    assert_eq!(
        context_event["params"]["context"]["auxData"]["isDefault"],
        false
    );
    assert_eq!(
        context_event["params"]["context"]["auxData"]["type"],
        "isolated"
    );
}
