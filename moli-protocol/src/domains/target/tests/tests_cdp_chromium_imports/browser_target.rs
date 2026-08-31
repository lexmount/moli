use super::super::*;
use super::support::{attached_smoke_session, chromium_like_revision};
use serde_json::json;

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/browser/browser-version.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_import_browser_get_version_shape() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 91_000,
        "method": "Browser.getVersion"
    }))
    .await;
    let version = take_response_by_id(&mut ctx, 91_000);
    let result = &version["result"];
    let protocol_version = result["protocolVersion"]
        .as_str()
        .expect("protocolVersion string");
    assert!(
        protocol_version
            .split_once('.')
            .is_some_and(|(major, minor)| {
                major.chars().all(|ch| ch.is_ascii_digit())
                    && minor.chars().all(|ch| ch.is_ascii_digit())
            }),
        "{version}"
    );
    assert!(
        result["userAgent"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "{version}"
    );
    assert!(
        result["jsVersion"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "{version}"
    );
    assert!(
        result["revision"]
            .as_str()
            .is_some_and(chromium_like_revision),
        "{version}"
    );
    assert!(version.get("sessionId").is_none(), "{version}");
}

// Capability source: docs/WEB_CAPABILITIES.md browser window/session basics.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_browser_window_bounds_round_trip() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let attached = attached_smoke_session(&mut ctx, 101_000).await;

    ctx.process_async(json!({
        "id": 101_005,
        "method": "Browser.getWindowForTarget",
        "sessionId": attached.session_id,
        "params": { "targetId": attached.target_id }
    }))
    .await;
    let first = take_response_by_id(&mut ctx, 101_005);
    let window_id = first["result"]["windowId"]
        .as_i64()
        .unwrap_or_else(|| panic!("window id: {first}"));

    ctx.process_async(json!({
        "id": 101_006,
        "method": "Browser.setWindowBounds",
        "sessionId": attached.session_id,
        "params": {
            "windowId": window_id,
            "bounds": { "left": 11, "top": 22, "width": 800, "height": 600 }
        }
    }))
    .await;
    ctx.expect_result(101_006, json!({}), Some(&attached.session_id));

    ctx.process_async(json!({
        "id": 101_007,
        "method": "Browser.getWindowForTarget",
        "sessionId": attached.session_id,
        "params": { "targetId": attached.target_id }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 101_007);
    assert_eq!(updated["result"]["bounds"]["left"], 11);
    assert_eq!(updated["result"]["bounds"]["top"], 22);
    assert_eq!(updated["result"]["bounds"]["width"], 800);
    assert_eq!(updated["result"]["bounds"]["height"], 600);
}

// Capability source: docs/WEB_CAPABILITIES.md download ability.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_browser_download_behavior_contract() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 102_005,
        "method": "Browser.setDownloadBehavior",
        "params": {
            "behavior": "allowAndName",
            "downloadPath": "/tmp/moli-chromium-import-downloads",
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(102_005, json!({}), None);
    assert_eq!(ctx.conn.download_behavior.behavior, "allowAndName");
    assert_eq!(
        ctx.conn.download_behavior.download_path.as_deref(),
        Some("/tmp/moli-chromium-import-downloads")
    );
    assert!(!ctx.conn.download_behavior.automation_events_enabled);
    assert_eq!(
        ctx.conn.download_behavior.browser_event_session_ids(),
        vec![None]
    );
}

// Puppeteer Browser.target().createCDPSession() discovers the browser agent
// host and then attaches to it through the generic Target.attachToTarget path.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_puppeteer_browser_target_session_contract() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 102_100,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(102_100, json!({}), None);
    let browser_created = ctx.take_first_matching("browser targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_target_id = browser_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("browser target id")
        .to_owned();
    assert_eq!(
        browser_created["params"]["targetInfo"],
        json!({
            "targetId": browser_target_id,
            "type": "browser",
            "title": "",
            "url": "",
            "attached": true,
            "canAccessOpener": false
        })
    );

    ctx.process_async(json!({
        "id": 102_101,
        "method": "Target.getTargetInfo",
        "params": { "targetId": browser_target_id.clone() }
    }))
    .await;
    let target_info = take_response_by_id(&mut ctx, 102_101);
    assert_eq!(
        target_info["result"]["targetInfo"],
        browser_created["params"]["targetInfo"]
    );

    ctx.process_async(json!({
        "id": 102_102,
        "method": "Target.getTargets",
        "params": { "filter": [{}] }
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 102_102);
    assert!(
        targets["result"]["targetInfos"]
            .as_array()
            .expect("targetInfos")
            .iter()
            .all(|target| target["type"] != json!("browser")),
        "Chromium does not include its browser agent host in Target.getTargets: {targets:?}"
    );

    ctx.process_async(json!({
        "id": 102_103,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": browser_target_id.clone(),
            "flatten": true
        }
    }))
    .await;
    let attached = take_response_by_id(&mut ctx, 102_103);
    let browser_session_id = attached["result"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    let attached_event = ctx.take_first_matching("browser attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(browser_session_id)
    });
    assert_eq!(
        attached_event["params"]["targetInfo"]["targetId"],
        json!(browser_target_id)
    );
    assert_eq!(
        attached_event["params"]["targetInfo"]["type"],
        json!("browser")
    );
    assert_eq!(
        ctx.conn.session_route(Some(&browser_session_id)),
        Some(crate::conn::CdpSessionRoute::Browser)
    );

    ctx.process_async(json!({
        "id": 102_104,
        "method": "Browser.setDownloadBehavior",
        "sessionId": browser_session_id,
        "params": {
            "behavior": "allowAndName",
            "downloadPath": "/tmp/moli-puppeteer-downloads",
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(102_104, json!({}), Some(&browser_session_id));
    assert_eq!(
        ctx.conn.download_behavior.download_path.as_deref(),
        Some("/tmp/moli-puppeteer-downloads")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_page_session_can_attach_discovered_browser_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = attached_smoke_session(&mut ctx, 102_200).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 102_205,
        "method": "Target.setDiscoverTargets",
        "sessionId": page.session_id,
        "params": { "discover": true, "filter": [{}] }
    }))
    .await;
    ctx.expect_result(102_205, json!({}), Some(&page.session_id));
    let browser_created = ctx.take_first_matching("page-owned browser targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["sessionId"] == json!(page.session_id)
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_target_id = browser_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("browser target id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 102_206,
        "method": "Target.attachToTarget",
        "sessionId": page.session_id,
        "params": { "targetId": browser_target_id, "flatten": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 102_206);
    assert_eq!(response["sessionId"], json!(page.session_id));
    let browser_session_id = response["result"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    let attached = ctx.take_first_matching("page-owned browser attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["sessionId"] == json!(page.session_id)
            && message["params"]["sessionId"] == json!(browser_session_id)
    });
    assert_eq!(attached["params"]["targetInfo"]["type"], "browser");
    assert_eq!(
        ctx.conn.session_route(Some(&browser_session_id)),
        Some(crate::conn::CdpSessionRoute::Browser)
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 102_207,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": page.session_id }
    }))
    .await;
    ctx.expect_result(102_207, json!({}), None);
    assert_eq!(
        ctx.conn.session_route(Some(&browser_session_id)),
        None,
        "detaching the owner page session must release its browser child session"
    );
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Target.detachedFromTarget")
                || message["params"]["sessionId"] != json!(browser_session_id)
        }),
        "Chromium silently releases a browser child when its owner is detached: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_explicit_browser_session_detach_emits_owned_event() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = attached_smoke_session(&mut ctx, 102_250).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 102_255,
        "method": "Target.setDiscoverTargets",
        "sessionId": page.session_id,
        "params": { "discover": true, "filter": [{}] }
    }))
    .await;
    let browser_created = ctx.take_first_matching("page-owned browser targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["sessionId"] == json!(page.session_id)
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_target_id = browser_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("browser target id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 102_256,
        "method": "Target.attachToTarget",
        "sessionId": page.session_id,
        "params": { "targetId": browser_target_id, "flatten": true }
    }))
    .await;
    let browser_session_id = take_response_by_id(&mut ctx, 102_256)["result"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 102_257,
        "method": "Target.detachFromTarget",
        "sessionId": page.session_id,
        "params": { "sessionId": browser_session_id }
    }))
    .await;
    ctx.expect_result(102_257, json!({}), Some(&page.session_id));
    let detached = ctx.take_first_matching("page-owned browser detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["sessionId"] == json!(page.session_id)
            && message["params"]["sessionId"] == json!(browser_session_id)
    });
    assert_eq!(
        detached["params"]["targetId"],
        json!(super::super::browser_context::DEVTOOLS_BROWSER_TARGET_ID)
    );
    assert_eq!(ctx.conn.session_route(Some(&browser_session_id)), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_attach_to_browser_target_rejects_page_session() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = attached_smoke_session(&mut ctx, 102_300).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 102_305,
        "method": "Target.attachToBrowserTarget",
        "sessionId": page.session_id
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 102_305);
    assert_eq!(response["sessionId"], json!(page.session_id));
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(response["error"]["message"], "Not allowed");
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Target.attachedToTarget")
                || message["params"]["targetInfo"]["type"] != json!("browser")
        }),
        "rejected specialized attach must not create a browser session: {:?}",
        ctx.sent
    );
}

// Capability source: docs/WEB_CAPABILITIES.md profile/proxy state.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_target_browser_context_proxy_and_enumeration() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 103_005,
        "method": "Target.createBrowserContext",
        "params": {
            "proxyServer": "http://proxy.example:8080",
            "proxyBypassList": "example.com, <-loopback>"
        }
    }))
    .await;
    let created = take_response_by_id(&mut ctx, 103_005);
    let browser_context_id = created["result"]["browserContextId"]
        .as_str()
        .unwrap_or_else(|| panic!("browserContextId: {created}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 103_006,
        "method": "Target.getBrowserContexts"
    }))
    .await;
    let contexts = take_response_by_id(&mut ctx, 103_006);
    assert!(
        contexts["result"]["browserContextIds"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == &json!(browser_context_id))),
        "{contexts}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|context| context.default_http_proxy_override.as_deref()),
        Some("http://proxy.example:8080")
    );
}

// Capability source: docs/WEB_CAPABILITIES.md multi-tab/session target control.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_capability_target_create_attach_detach_close_contract() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 104_005,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let context = take_response_by_id(&mut ctx, 104_005);
    let browser_context_id = context["result"]["browserContextId"]
        .as_str()
        .unwrap_or_else(|| panic!("browserContextId: {context}"))
        .to_owned();
    ctx.conn.set_root_target_discovery_enabled(true);

    ctx.process_async(json!({
        "id": 104_006,
        "method": "Target.createTarget",
        "params": { "browserContextId": browser_context_id, "url": "about:blank#created" }
    }))
    .await;
    let created_event = ctx.take_one();
    assert_eq!(created_event["method"], "Target.targetCreated");
    let target_id = created_event["params"]["targetInfo"]["targetId"]
        .as_str()
        .unwrap_or_else(|| panic!("targetCreated: {created_event}"))
        .to_owned();
    ctx.expect_result(104_006, json!({ "targetId": target_id }), None);

    ctx.process_async(json!({
        "id": 104_007,
        "method": "Target.getTargets"
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 104_007);
    assert!(
        targets["result"]["targetInfos"]
            .as_array()
            .is_some_and(|infos| infos
                .iter()
                .any(|info| info["targetId"] == json!(target_id))),
        "{targets}"
    );

    ctx.process_async(json!({
        "id": 104_008,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id, "flatten": true }
    }))
    .await;
    let attached = take_response_by_id(&mut ctx, 104_008);
    let session_id = attached["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("sessionId: {attached}"))
        .to_owned();
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    }));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 104_009,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": session_id }
    }))
    .await;
    ctx.expect_result(104_009, json!({}), None);
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["sessionId"] == json!(session_id)
    }));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 104_010,
        "method": "Target.closeTarget",
        "params": { "targetId": target_id }
    }))
    .await;
    let closed = take_response_by_id(&mut ctx, 104_010);
    assert_eq!(closed["result"]["success"], true);
}
