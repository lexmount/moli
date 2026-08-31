use super::super::*;
use serde_json::{Value, json};

fn response(messages: &[Value], id: u64) -> &Value {
    messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing response {id} in {messages:?}"))
}

fn event<'a>(messages: &'a [Value], method: &str) -> &'a Value {
    messages
        .iter()
        .find(|message| message["method"] == json!(method))
        .unwrap_or_else(|| panic!("missing {method} event in {messages:?}"))
}

fn active_context_id(ctx: &TestContext) -> Option<&str> {
    ctx.conn
        .browser_context
        .as_ref()
        .map(|context| context.id.as_str())
}

async fn create_browser_context(ctx: &mut TestContext, id: u64) -> String {
    ctx.process_async(json!({
        "id": id,
        "method": "Target.createBrowserContext"
    }))
    .await;
    take_response_by_id(ctx, id)["result"]["browserContextId"]
        .as_str()
        .expect("browserContextId")
        .to_owned()
}

async fn create_target(
    ctx: &mut TestContext,
    id: u64,
    browser_context_id: &str,
    url: &str,
) -> String {
    ctx.process_async(json!({
        "id": id,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": url
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "Target.createTarget without Target.setDiscoverTargets should not emit targetCreated: {messages:?}"
    );
    response(&messages, id)["result"]["targetId"]
        .as_str()
        .expect("targetId")
        .to_owned()
}

async fn attach_browser_session(ctx: &mut TestContext, id: u64) -> String {
    ctx.process_async(json!({
        "id": id,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let messages = ctx.take_all();
    let session_id = event(&messages, "Target.attachedToTarget")["params"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    assert_eq!(response(&messages, id)["result"]["sessionId"], session_id);
    session_id
}

async fn set_auto_attach(ctx: &mut TestContext, id: u64, auto_attach: bool) {
    ctx.process_async(json!({
        "id": id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": auto_attach,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(id, json!({}), None);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_get_browser_contexts_includes_default_context_id() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 261_001,
        "method": "Target.getBrowserContexts"
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 261_001);
    assert_eq!(response["result"]["browserContextIds"], json!([]));
    assert!(
        response["result"]["defaultBrowserContextId"]
            .as_str()
            .is_some(),
        "Chrome exposes the default browser context id in Target.getBrowserContexts: {response:?}"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_browser_context_first_becomes_active() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    let browser_context_id = create_browser_context(&mut ctx, 261_002).await;

    assert_eq!(active_context_id(&ctx), Some(browser_context_id.as_str()));
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_browser_context_second_is_inactive_and_listed() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_003).await;
    let second_id = create_browser_context(&mut ctx, 261_004).await;

    ctx.process_async(json!({
        "id": 261_005,
        "method": "Target.getBrowserContexts"
    }))
    .await;
    let contexts = take_response_by_id(&mut ctx, 261_005);

    assert_eq!(active_context_id(&ctx), Some(first_id.as_str()));
    let listed = contexts["result"]["browserContextIds"]
        .as_array()
        .expect("browserContextIds array");
    assert_eq!(listed.len(), 2);
    assert!(listed.contains(&json!(first_id)));
    assert!(listed.contains(&json!(second_id)));
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_browser_context_proxy_server_is_recorded() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 261_006,
        "method": "Target.createBrowserContext",
        "params": { "proxyServer": "http://proxy.example:8080" }
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 261_006)["result"]["browserContextId"]
        .as_str()
        .expect("browserContextId")
        .to_owned();

    assert_eq!(
        ctx.conn
            .browser_context_by_id(&browser_context_id)
            .and_then(|context| context.default_http_proxy_override.as_deref()),
        Some("http://proxy.example:8080")
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_browser_context_proxy_bypass_normalizes_loopback_token() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 261_007,
        "method": "Target.createBrowserContext",
        "params": {
            "proxyServer": "http://proxy.example:8080",
            "proxyBypassList": " localhost , <-loopback>, .example.com, "
        }
    }))
    .await;
    let browser_context_id = take_response_by_id(&mut ctx, 261_007)["result"]["browserContextId"]
        .as_str()
        .expect("browserContextId")
        .to_owned();

    assert_eq!(
        ctx.conn
            .browser_context_by_id(&browser_context_id)
            .and_then(|context| context.default_http_no_proxy_override.as_deref()),
        Some("localhost,.example.com")
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_dispose_browser_context_requires_id() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 261_008,
        "method": "Target.disposeBrowserContext"
    }))
    .await;

    ctx.expect_error(261_008, -32602, "InvalidParams");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_dispose_browser_context_unknown_id_errors() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 261_009,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-missing" }
    }))
    .await;

    ctx.expect_error(
        261_009,
        -32000,
        "Failed to find context with id BID-missing",
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_dispose_inactive_context_preserves_active_context() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_010).await;
    let second_id = create_browser_context(&mut ctx, 261_011).await;

    ctx.process_async(json!({
        "id": 261_012,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": second_id }
    }))
    .await;

    ctx.expect_result(261_012, json!({}), None);
    assert_eq!(active_context_id(&ctx), Some(first_id.as_str()));
    assert!(ctx.conn.browser_context_by_id(&second_id).is_none());
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_dispose_active_context_promotes_remaining_context() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_013).await;
    let second_id = create_browser_context(&mut ctx, 261_014).await;

    ctx.process_async(json!({
        "id": 261_015,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": first_id }
    }))
    .await;

    ctx.expect_result(261_015, json!({}), None);
    assert_eq!(active_context_id(&ctx), Some(second_id.as_str()));
    assert!(ctx.conn.browser_context_by_id(&first_id).is_none());
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_dispose_context_clears_context_scoped_download_behavior() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let browser_context_id = create_browser_context(&mut ctx, 261_016).await;
    ctx.conn.download_behavior.set_browser_context(
        browser_context_id.clone(),
        "allow".into(),
        Some("/tmp/moli-target-contexts".into()),
        true,
    );

    ctx.process_async(json!({
        "id": 261_017,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": browser_context_id }
    }))
    .await;

    ctx.expect_result(261_017, json!({}), None);
    assert_eq!(
        ctx.conn.download_behavior,
        crate::conn::BrowserDownloadBehavior::default()
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_in_inactive_context_restores_active_context() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_018).await;
    let second_id = create_browser_context(&mut ctx, 261_019).await;

    let target_id = create_target(
        &mut ctx,
        261_020,
        &second_id,
        "https://example.com/inactive",
    )
    .await;

    assert_eq!(active_context_id(&ctx), Some(first_id.as_str()));
    assert_eq!(
        ctx.conn
            .browser_context_by_id(&second_id)
            .and_then(|context| context.active_target_id()),
        Some(target_id.as_str())
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_in_inactive_context_emits_context_id() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let _first_id = create_browser_context(&mut ctx, 261_021).await;
    let second_id = create_browser_context(&mut ctx, 261_022).await;
    ctx.conn.set_root_target_discovery_enabled(true);

    ctx.process_async(json!({
        "id": 261_023,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": second_id,
            "url": "about:blank#inactive"
        }
    }))
    .await;
    let messages = ctx.take_all();

    assert_eq!(
        event(&messages, "Target.targetCreated")["params"]["targetInfo"]["browserContextId"],
        second_id
    );
    assert!(
        response(&messages, 261_023)["result"]["targetId"]
            .as_str()
            .is_some()
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/get-target-info.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_get_target_info_in_inactive_context_restores_active_context() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_024).await;
    let second_id = create_browser_context(&mut ctx, 261_025).await;
    let target_id = create_target(&mut ctx, 261_026, &second_id, "about:blank#info").await;

    ctx.process_async(json!({
        "id": 261_027,
        "method": "Target.getTargetInfo",
        "params": { "targetId": target_id }
    }))
    .await;
    let info = take_response_by_id(&mut ctx, 261_027);

    assert_eq!(active_context_id(&ctx), Some(first_id.as_str()));
    assert_eq!(info["result"]["targetInfo"]["browserContextId"], second_id);
    assert_eq!(info["result"]["targetInfo"]["targetId"], target_id);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_attach_to_inactive_context_target_selects_target_context() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_028).await;
    let second_id = create_browser_context(&mut ctx, 261_029).await;
    let target_id = create_target(&mut ctx, 261_030, &second_id, "about:blank#attach").await;

    ctx.process_async(json!({
        "id": 261_031,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id, "flatten": true }
    }))
    .await;
    let messages = ctx.take_all();

    assert_eq!(active_context_id(&ctx), Some(second_id.as_str()));
    let session_id = response(&messages, 261_031)["result"]["sessionId"]
        .as_str()
        .expect("session id");
    assert_eq!(
        event(&messages, "Target.attachedToTarget")["params"]["sessionId"],
        session_id
    );
    assert_eq!(
        ctx.conn
            .browser_context_by_id(&second_id)
            .and_then(|context| context.active_session_id()),
        Some(session_id)
    );
    assert!(ctx.conn.browser_context_by_id(&first_id).is_some());
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_browser_session_attach_inactive_target_scopes_event_to_browser() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_032).await;
    let second_id = create_browser_context(&mut ctx, 261_033).await;
    let target_id = create_target(&mut ctx, 261_034, &second_id, "about:blank#aux").await;
    let browser_session_id = attach_browser_session(&mut ctx, 261_035).await;

    ctx.process_async(json!({
        "id": 261_036,
        "method": "Target.attachToTarget",
        "sessionId": browser_session_id,
        "params": { "targetId": target_id, "flatten": true }
    }))
    .await;
    let messages = ctx.take_all();

    assert_eq!(active_context_id(&ctx), Some(first_id.as_str()));
    assert_eq!(
        response(&messages, 261_036)["sessionId"],
        browser_session_id
    );
    let attached = event(&messages, "Target.attachedToTarget");
    assert_eq!(attached["sessionId"], browser_session_id);
    assert_eq!(
        attached["params"]["targetInfo"]["browserContextId"],
        second_id
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_existing_targets_across_contexts_restores_active() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_037).await;
    let second_id = create_browser_context(&mut ctx, 261_038).await;
    let first_target_id = create_target(&mut ctx, 261_039, &first_id, "about:blank#first").await;
    let second_target_id = create_target(&mut ctx, 261_040, &second_id, "about:blank#second").await;

    set_auto_attach(&mut ctx, 261_041, true).await;
    let messages = ctx.take_all();

    assert_eq!(active_context_id(&ctx), Some(first_id.as_str()));
    let attached = messages
        .iter()
        .filter(|message| message["method"] == "Target.attachedToTarget")
        .collect::<Vec<_>>();
    assert_eq!(attached.len(), 2, "{messages:?}");
    assert!(attached.iter().any(|message| {
        message["params"]["targetInfo"]["targetId"] == json!(first_target_id)
            && message["params"]["targetInfo"]["browserContextId"] == json!(first_id)
    }));
    assert!(attached.iter().any(|message| {
        message["params"]["targetInfo"]["targetId"] == json!(second_target_id)
            && message["params"]["targetInfo"]["browserContextId"] == json!(second_id)
    }));
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_false_detaches_sessions_across_contexts() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_042).await;
    let second_id = create_browser_context(&mut ctx, 261_043).await;
    let first_target_id = create_target(&mut ctx, 261_044, &first_id, "about:blank#first").await;
    let second_target_id = create_target(&mut ctx, 261_045, &second_id, "about:blank#second").await;
    set_auto_attach(&mut ctx, 261_046, true).await;
    ctx.sent.clear();

    set_auto_attach(&mut ctx, 261_047, false).await;
    let messages = ctx.take_all();

    assert_eq!(active_context_id(&ctx), Some(first_id.as_str()));
    let detached = messages
        .iter()
        .filter(|message| message["method"] == "Target.detachedFromTarget")
        .collect::<Vec<_>>();
    assert_eq!(detached.len(), 2, "{messages:?}");
    assert!(
        detached
            .iter()
            .any(|message| message["params"]["targetId"] == json!(first_target_id))
    );
    assert!(
        detached
            .iter()
            .any(|message| message["params"]["targetId"] == json!(second_target_id))
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_get_targets_lists_context_ids_for_multiple_contexts() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let first_id = create_browser_context(&mut ctx, 261_048).await;
    let second_id = create_browser_context(&mut ctx, 261_049).await;
    let first_target_id = create_target(&mut ctx, 261_050, &first_id, "about:blank#first").await;
    let second_target_id = create_target(&mut ctx, 261_051, &second_id, "about:blank#second").await;

    ctx.process_async(json!({
        "id": 261_052,
        "method": "Target.getTargets"
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 261_052);
    let target_infos = targets["result"]["targetInfos"].as_array().unwrap();

    assert!(target_infos.iter().any(|target| {
        target["targetId"] == json!(first_target_id)
            && target["browserContextId"] == json!(first_id)
    }));
    assert!(target_infos.iter().any(|target| {
        target["targetId"] == json!(second_target_id)
            && target["browserContextId"] == json!(second_id)
    }));
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-browser-context.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_popup_target_keeps_opener_browser_context_id() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    // Chromium creates the auxiliary browsing context and returns from the
    // causing Runtime command before the popup's requested URL finishes
    // loading. Use the production scheduler boundary here: leaving this test
    // on TestContext's legacy inline-navigation mode makes an unrelated
    // example.com response part of the renderer-output cursor fence.
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-context",
                "TID-popup-context-opener",
                "<main>opener</main>",
            )
            .await;

            ctx.process_async(json!({
                "id": 261_053,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "window.open('data:text/html,context-popup', '_blank') !== null",
                    "returnByValue": true
                }
            }))
            .await;
            let messages = ctx.take_all();

            let popup = event(&messages, "Target.targetCreated");
            assert_eq!(
                popup["params"]["targetInfo"]["browserContextId"],
                "BID-popup-context"
            );
            assert_eq!(
                popup["params"]["targetInfo"]["openerId"],
                "TID-popup-context-opener"
            );
        })
        .await;
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-info-changed.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_named_popup_reuse_keeps_browser_context_id() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-named-context",
        "TID-named-context-opener",
        "<main>opener</main>",
    )
    .await;

    ctx.process_async(json!({
        "id": 261_054,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('https://example.com/first', 'report') !== null",
            "returnByValue": true
        }
    }))
    .await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 261_055,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('https://example.com/second', 'report') !== null",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();

    let changed = event(&messages, "Target.targetInfoChanged");
    assert_eq!(
        changed["params"]["targetInfo"]["browserContextId"],
        "BID-named-context"
    );
    assert_eq!(
        changed["params"]["targetInfo"]["url"],
        "https://example.com/second"
    );
}
