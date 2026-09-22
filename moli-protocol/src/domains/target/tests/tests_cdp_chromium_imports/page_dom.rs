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
        .download_behavior
        .effective_for_browser_context(Some(browser_context_id.as_str()));
    assert_eq!(settings.behavior, "allow");
    assert_eq!(
        settings.download_path.as_deref(),
        Some("/tmp/moli-page-downloads")
    );
    assert!(!settings.automation_events_enabled);
    assert!(
        !ctx.conn.download_behavior.automation_events_enabled,
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
    ctx.capture_fixture_layout(Some(&attached.session_id)).await;
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

// Session history must survive destruction of the old top-level renderer,
// including visits to the same URL with a different Document and entry state.
#[tokio::test(flavor = "multi_thread")]
async fn top_level_history_restores_structured_state_and_entry_identity_before_scripts() {
    let fixture = SmokeFixtureServer::start().await;
    let other = SmokeFixtureServer::start().await;
    for cross_origin in [false, true] {
        let mut ctx = TestContext::new_with_target_discovery(false);
        let attached = attached_smoke_session(&mut ctx, 211_000).await;
        let first_url = fixture.url("/plain?first#start");
        let middle_url = if cross_origin {
            other.url("/plain?middle")
        } else {
            fixture.url("/plain?middle")
        };
        ctx.process_async(json!({
            "id": 211_004, "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": attached.session_id,
            "params": {"source": "globalThis.restoredBeforeScripts = history.state;"}
        }))
        .await;
        let preload = take_response_by_id(&mut ctx, 211_004);
        assert!(preload.get("error").is_none(), "{preload}");
        navigate_and_take_response(&mut ctx, &attached.session_id, 211_005, first_url.clone())
            .await;
        let original = evaluate_string(
            &mut ctx,
            &attached.session_id,
            211_006,
            r#"
            (() => {
              if (navigation.activation.from !== null) throw new Error('initial Document exposed as activation.from');
              const state = {map: new Map([['bytes', new Uint8Array([7, 9])]])};
              state.self = state;
              history.replaceState(state, '');
              navigation.updateCurrentEntry({state: new Set(['first'])});
              return JSON.stringify([navigation.currentEntry.id, navigation.currentEntry.key]);
            })()
        "#,
        )
        .await;
        navigate_and_take_response(&mut ctx, &attached.session_id, 211_007, middle_url.clone())
            .await;
        navigate_and_take_response(&mut ctx, &attached.session_id, 211_008, first_url.clone())
            .await;
        evaluate_string(
            &mut ctx,
            &attached.session_id,
            211_009,
            "history.replaceState('last', ''); globalThis.lastDocument = true; 'ok'",
        )
        .await;
        ctx.process_async(json!({"id": 211_010, "method": "Page.getNavigationHistory", "sessionId": attached.session_id})).await;
        let history = take_response_by_id(&mut ctx, 211_010);
        let first_id = history["result"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["url"] == first_url)
            .unwrap()["id"]
            .clone();
        ctx.process_async(json!({"id": 211_011, "method": "Page.navigateToHistoryEntry", "sessionId": attached.session_id,
            "params": {"entryId": first_id}})).await;
        ctx.expect_result(211_011, json!({}), Some(&attached.session_id));
        let restored = evaluate_string(&mut ctx, &attached.session_id, 211_012, r#"
            JSON.stringify({
                identity: [navigation.currentEntry.id, navigation.currentEntry.key],
                state: history.state === history.state.self && history.state.map instanceof Map &&
                       history.state.map.get('bytes') instanceof Uint8Array && history.state.map.get('bytes')[1] === 9,
                beforeScripts: restoredBeforeScripts.self === restoredBeforeScripts && restoredBeforeScripts.map.get('bytes')[0] === 7,
                navigationState: navigation.currentEntry.getState() instanceof Set && navigation.currentEntry.getState().has('first'),
                replacedDocument: !globalThis.lastDocument,
                type: navigation.activation.navigationType,
                from: navigation.activation.from?.url ?? null,
                urls: navigation.entries().map(entry => entry.url)
            })
        "#).await;
        let restored: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(
            restored["identity"],
            serde_json::from_str::<serde_json::Value>(&original).unwrap()
        );
        for field in [
            "state",
            "beforeScripts",
            "navigationState",
            "replacedDocument",
        ] {
            assert_eq!(restored[field], true, "{field}: {restored}");
        }
        assert_eq!(restored["type"], "traverse");
        if cross_origin {
            assert_eq!(restored["from"], serde_json::Value::Null);
            assert_eq!(restored["urls"], json!([first_url]));
        } else {
            assert_eq!(restored["from"], first_url);
            assert_eq!(restored["urls"], json!([first_url, middle_url, first_url]));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn top_level_history_uses_committed_source_updates_after_navigation_is_requested() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 212_000).await;
    let first_url = fixture.url("/plain?source");
    let fragment_url = format!("{first_url}#state");
    let second_url = fixture.url("/plain?destination");
    ctx.process_async(
        json!({"id": 212_004, "method": "Page.enable", "sessionId": attached.session_id}),
    )
    .await;
    ctx.expect_result(212_004, json!({}), Some(&attached.session_id));
    ctx.process_async(json!({
        "id": 212_010, "method": "Page.setLifecycleEventsEnabled", "sessionId": attached.session_id,
        "params": {"enabled": true}
    }))
    .await;
    ctx.expect_result(212_010, json!({}), Some(&attached.session_id));
    let first_navigation =
        navigate_and_take_response(&mut ctx, &attached.session_id, 212_005, first_url.clone())
            .await;
    let first_loader = first_navigation["result"]["loaderId"].as_str().unwrap();
    // Location assignment before load completion replaces the current entry.
    // This fixture needs a push from a completely loaded source Document.
    ctx.wait_for_scheduler_message("source Document load", |message| {
        message["method"] == "Page.lifecycleEvent"
            && message["sessionId"] == attached.session_id
            && message["params"]["loaderId"] == first_loader
            && message["params"]["name"] == "load"
    })
    .await;
    let fragment_entries = evaluate_string(
        &mut ctx,
        &attached.session_id,
        212_006,
        r#"
        history.replaceState('first', '');
        history.pushState('before', '', '#state');
        JSON.stringify(navigation.entries().map(entry => [entry.url, entry.index]))
    "#,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fragment_entries).unwrap(),
        json!([[first_url, 0], [fragment_url, 1]])
    );
    let original = evaluate_string(
        &mut ctx,
        &attached.session_id,
        212_007,
        format!(
            r#"
        location.href = {};
        history.replaceState('late', '');
        navigation.updateCurrentEntry({{state: 'nav-late'}});
        JSON.stringify([navigation.currentEntry.id, navigation.currentEntry.key])
    "#,
            serde_json::to_string(&second_url).unwrap()
        ),
    )
    .await;
    ctx.wait_for_scheduler_message("destination Document commit", |message| {
        message["method"] == "Page.frameNavigated"
            && message["sessionId"] == attached.session_id
            && message["params"]["frame"]["url"] == second_url
    })
    .await;
    evaluate_string(
        &mut ctx,
        &attached.session_id,
        212_008,
        "history.back(); 'scheduled'",
    )
    .await;
    ctx.wait_for_scheduler_message("source Document traversal", |message| {
        message["method"] == "Page.frameNavigated"
            && message["sessionId"] == attached.session_id
            && message["params"]["frame"]["url"] == fragment_url
    })
    .await;
    let restored = evaluate_string(
        &mut ctx,
        &attached.session_id,
        212_009,
        r#"
        JSON.stringify({state: history.state, navstate: navigation.currentEntry.getState(),
            identity: [navigation.currentEntry.id, navigation.currentEntry.key],
            urls: navigation.entries().map(entry => entry.url),
            type: navigation.activation.navigationType})
    "#,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&restored).unwrap(),
        json!({
            "state": "late", "navstate": "nav-late", "identity": serde_json::from_str::<serde_json::Value>(&original).unwrap(),
            "urls": [first_url, fragment_url, second_url], "type": "traverse"
        })
    );
}
