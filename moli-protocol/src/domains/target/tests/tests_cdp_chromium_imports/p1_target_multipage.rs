use super::super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::super::*;
use crate::{CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand};
use serde_json::{Value, json};

fn event<'a>(messages: &'a [Value], method: &str) -> &'a Value {
    messages
        .iter()
        .find(|message| message["method"] == json!(method))
        .unwrap_or_else(|| panic!("missing {method} event in {messages:?}"))
}

fn response(messages: &[Value], id: u64) -> &Value {
    messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing response {id} in {messages:?}"))
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

async fn set_auto_attach(ctx: &mut TestContext, id: u64, auto_attach: bool) {
    ctx.process_async(json!({
        "id": id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": auto_attach,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(id, json!({}), None);
}

async fn set_auto_attach_waiting_for_debugger(ctx: &mut TestContext, id: u64) {
    ctx.process_async(json!({
        "id": id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(id, json!({}), None);
}

async fn create_target(
    ctx: &mut TestContext,
    id: u64,
    browser_context_id: Option<&str>,
    url: &str,
) -> String {
    let mut params = json!({ "url": url });
    if let Some(browser_context_id) = browser_context_id {
        params["browserContextId"] = json!(browser_context_id);
    }
    ctx.process_async(json!({
        "id": id,
        "method": "Target.createTarget",
        "params": params
    }))
    .await;
    let messages = ctx.take_all();
    response(&messages, id)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned()
}

async fn attach_to_target(
    ctx: &mut TestContext,
    id: u64,
    browser_session_id: Option<&str>,
    target_id: &str,
) -> String {
    let mut command = json!({
        "id": id,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id, "flatten": true }
    });
    if let Some(browser_session_id) = browser_session_id {
        command["sessionId"] = json!(browser_session_id);
    }
    ctx.process_async(command).await;
    let response = take_response_by_id(ctx, id);
    response["result"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_owned()
}

async fn open_popup_from_runtime(ctx: &mut TestContext, id: u64, expression: &str) -> Vec<Value> {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true
        }
    }))
    .await;
    ctx.take_all()
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_set_discover_targets_true_returns_empty() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 260_001,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;

    ctx.expect_result(260_001, json!({}), None);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_set_discover_targets_false_returns_empty() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 260_002,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": false }
    }))
    .await;

    ctx.expect_result(260_002, json!({}), None);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_set_auto_attach_true_records_global_policy() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    set_auto_attach(&mut ctx, 260_003, true).await;

    assert!(ctx.conn.auto_attach);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_set_auto_attach_false_records_global_policy() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.conn.auto_attach = true;

    set_auto_attach(&mut ctx, 260_004, false).await;

    assert!(!ctx.conn.auto_attach);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_set_auto_attach_requires_params() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    ctx.process_async(json!({
        "id": 260_005,
        "method": "Target.setAutoAttach"
    }))
    .await;

    ctx.expect_error(260_005, -32602, "InvalidParams");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_without_context_creates_context() {
    let mut ctx = TestContext::new_with_target_discovery(false);

    let target_id = create_target(&mut ctx, 260_006, None, "about:blank").await;

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(browser_context.active_target_id(), Some(target_id.as_str()));
    assert_eq!(browser_context.id, "BID-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromiumoxide_loaded_target_replays_renderer_lifecycle_when_enabled() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let target_id = create_target(&mut ctx, 2_600_061, None, "about:blank").await;
    let session_id = attach_to_target(&mut ctx, 2_600_062, None, target_id.as_str()).await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2_600_063,
        "method": "Page.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(2_600_063, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 2_600_064,
        "method": "Page.getFrameTree",
        "sessionId": session_id
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 2_600_064);
    let loader_id = frame_tree["result"]["frameTree"]["frame"]["loaderId"]
        .as_str()
        .expect("initial document loader id")
        .to_owned();
    assert_eq!(
        frame_tree["result"]["frameTree"]["frame"]["id"],
        json!(target_id)
    );

    ctx.process_async(json!({
        "id": 2_600_065,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": session_id,
        "params": { "enabled": true }
    }))
    .await;
    let messages = ctx.take_all();

    for name in ["DOMContentLoaded", "load"] {
        assert!(
            messages.iter().any(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["sessionId"] == json!(session_id)
                    && message["params"]["name"] == json!(name)
                    && message["params"]["frameId"] == json!(target_id)
                    && message["params"]["loaderId"] == json!(loader_id)
            }),
            "missing {name} lifecycle replay in {messages:?}"
        );
    }
    assert_eq!(response(&messages, 2_600_065)["result"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_restart_does_not_inherit_the_stale_renderer_stream() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let target_id = create_target(&mut ctx, 2_600_070, None, "about:blank").await;
    let session_id = attach_to_target(&mut ctx, 2_600_071, None, &target_id).await;
    ctx.take_all();

    // Start the utility-world command on the initial renderer, but deliberately
    // leave its completed turn pending at the protocol boundary. Navigating now
    // replaces the attachment before that completion is decoded, deterministically
    // exercising the same stale-completion restart as popup initialization.
    let command = ParsedCdpCommand::parse_value(json!({
        "id": 2_600_072,
        "method": "Page.createIsolatedWorld",
        "sessionId": session_id,
        "params": {
            "frameId": target_id,
            "worldName": "__playwright_utility_world_page"
        }
    }))
    .expect("createIsolatedWorld command should parse");
    let mut command_context = CommandDispatchContext::default();
    let CdpCommandTaskStep::Pending(first_pending) = ctx
        .conn
        .start_parsed_command_dispatch_with_context(&command, &mut command_context)
    else {
        panic!("createIsolatedWorld should start on the initial renderer");
    };

    ctx.process_async(json!({
        "id": 2_600_073,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": fixture.url("/plain?replacement=isolated-world") }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 2_600_073);
    assert_eq!(navigation["result"]["frameId"], json!(target_id));

    let first_completed = first_pending.wait().await;
    let CdpCommandTaskStep::Pending(restarted) = ctx
        .conn
        .complete_pending_command_dispatch_with_context(first_completed, &mut command_context)
        .await
    else {
        panic!("the stale initial-renderer completion should restart on the replacement");
    };
    assert!(
        command_context.take_renderer_output_predecessor().is_none(),
        "an abandoned renderer stream must not become the final response predecessor"
    );

    let replacement_completed = restarted.wait().await;
    let CdpCommandTaskStep::Complete(outcome) = ctx
        .conn
        .complete_pending_command_dispatch_with_context(replacement_completed, &mut command_context)
        .await
    else {
        panic!("the replacement renderer should complete createIsolatedWorld");
    };
    let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
    assert!(
        response(&messages, 2_600_072)["result"]["executionContextId"]
            .as_i64()
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromiumoxide_loaded_background_target_replays_own_renderer_lifecycle() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let active_target_id = create_target(&mut ctx, 2_600_066, None, "about:blank").await;
    ctx.process_async(json!({
        "id": 2_600_067,
        "method": "Target.createTarget",
        "params": { "url": "about:blank", "background": true }
    }))
    .await;
    let background_target_id = take_response_by_id(&mut ctx, 2_600_067)["result"]["targetId"]
        .as_str()
        .expect("background target id")
        .to_owned();
    ctx.take_all();
    assert_ne!(background_target_id, active_target_id);
    let session_id =
        attach_to_target(&mut ctx, 2_600_068, None, background_target_id.as_str()).await;
    ctx.take_all();

    ctx.process_async(json!({
        "id": 2_600_069,
        "method": "Page.getFrameTree",
        "sessionId": session_id
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 2_600_069);
    let loader_id = frame_tree["result"]["frameTree"]["frame"]["loaderId"]
        .as_str()
        .expect("background initial document loader id")
        .to_owned();

    ctx.process_async(json!({
        "id": 2_600_070,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": session_id,
        "params": { "enabled": true }
    }))
    .await;
    let messages = ctx.take_all();

    for name in ["DOMContentLoaded", "load"] {
        assert!(
            messages.iter().any(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["sessionId"] == json!(session_id)
                    && message["params"]["name"] == json!(name)
                    && message["params"]["frameId"] == json!(background_target_id)
                    && message["params"]["loaderId"] == json!(loader_id)
            }),
            "missing background {name} lifecycle replay in {messages:?}"
        );
    }
    assert_eq!(response(&messages, 2_600_070)["result"], json!({}));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some(active_target_id.as_str())
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_in_matching_context_stages_active_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let browser_context_id = create_browser_context(&mut ctx, 260_007).await;

    let target_id = create_target(
        &mut ctx,
        260_008,
        Some(&browser_context_id),
        "about:blank#active",
    )
    .await;

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(browser_context.active_target_id(), Some(target_id.as_str()));
    assert_eq!(browser_context.target_url(), "about:blank#active");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_unknown_context_errors() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc(&mut ctx, "BID-known");

    ctx.process_async(json!({
        "id": 260_009,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": "BID-missing",
            "url": "about:blank"
        }
    }))
    .await;

    ctx.expect_error(260_009, -31998, "UnknownBrowserContextId");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/tab-target.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_created_event_has_page_fields() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    let browser_context_id = create_browser_context(&mut ctx, 260_010).await;
    ctx.process_async(json!({
        "id": 2_600_101,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(2_600_101, json!({}), None);

    ctx.process_async(json!({
        "id": 260_011,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": browser_context_id,
            "url": "https://example.com/page"
        }
    }))
    .await;

    let messages = ctx.take_all();
    let target_info = &event(&messages, "Target.targetCreated")["params"]["targetInfo"];
    assert_eq!(target_info["type"], "page");
    assert_eq!(target_info["url"], "https://example.com/page");
    assert_eq!(target_info["attached"], false);
    assert_eq!(target_info["canAccessOpener"], false);
    assert_eq!(target_info["browserContextId"], browser_context_id);
    assert!(target_info["targetId"].as_str().is_some());
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_auto_attach_emits_attached() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    set_auto_attach(&mut ctx, 260_012, true).await;

    ctx.process_async(json!({
        "id": 260_013,
        "method": "Target.createTarget",
        "params": { "url": "about:blank#attached" }
    }))
    .await;

    let messages = ctx.take_all();
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "autoAttach without Target.setDiscoverTargets should not emit targetCreated: {messages:?}"
    );
    let attached = event(&messages, "Target.attachedToTarget");
    let attached_id = attached["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("attached target")
        .to_owned();
    assert_eq!(attached["params"]["targetInfo"]["attached"], true);
    assert!(attached["params"]["sessionId"].as_str().is_some());
    assert_eq!(
        response(&messages, 260_013)["result"]["targetId"],
        attached_id
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_create_target_auto_attach_marks_get_targets_attached() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    set_auto_attach(&mut ctx, 260_014, true).await;
    let target_id = create_target(&mut ctx, 260_015, None, "about:blank#get-targets").await;

    ctx.process_async(json!({
        "id": 260_016,
        "method": "Target.getTargets"
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 260_016);

    assert!(
        targets["result"]["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["targetId"] == json!(target_id)
                && target["attached"] == json!(true)),
        "{targets}"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_second_create_target_activates_new_target_by_default() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-multi",
        "TID-000000000A",
        "<main>first target</main>",
    )
    .await;

    let target_id = create_target(
        &mut ctx,
        260_017,
        Some("BID-multi"),
        "about:blank#background",
    )
    .await;

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(browser_context.active_target_id(), Some(target_id.as_str()));
    assert_eq!(browser_context.background_targets.len(), 1);
    assert_eq!(
        browser_context.background_targets[0].target_id(),
        "TID-000000000A"
    );

    let first_session_id = attach_to_target(&mut ctx, 2_600_171, None, "TID-000000000A").await;
    ctx.process_async(json!({
        "id": 2_600_172,
        "sessionId": first_session_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "JSON.stringify({ hidden: document.hidden, visibilityState: document.visibilityState })",
            "returnByValue": true
        }
    }))
    .await;
    let visibility = take_response_by_id(&mut ctx, 2_600_172);
    assert_eq!(
        visibility["result"]["result"]["value"],
        json!(r#"{"hidden":true,"visibilityState":"hidden"}"#)
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/get-target-info.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_get_targets_includes_active_and_background_pages() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-list", "TID-active");
    push_background_target(
        &mut ctx,
        "TID-background",
        "https://example.com/background",
        None,
    );

    ctx.process_async(json!({
        "id": 260_018,
        "method": "Target.getTargets"
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 260_018);
    let infos = targets["result"]["targetInfos"].as_array().unwrap();

    assert!(
        infos
            .iter()
            .any(|target| target["targetId"] == "TID-active")
    );
    assert!(
        infos
            .iter()
            .any(|target| target["targetId"] == "TID-background"
                && target["url"] == "https://example.com/background")
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/get-target-info.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_get_target_info_returns_background_target_info() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-info", "TID-active");
    push_background_target(&mut ctx, "TID-background", "https://example.com/bg", None);

    ctx.process_async(json!({
        "id": 260_019,
        "method": "Target.getTargetInfo",
        "params": { "targetId": "TID-background" }
    }))
    .await;
    let info = take_response_by_id(&mut ctx, 260_019);

    assert_eq!(info["result"]["targetInfo"]["targetId"], "TID-background");
    assert_eq!(
        info["result"]["targetInfo"]["url"],
        "https://example.com/bg"
    );
    assert_eq!(info["result"]["targetInfo"]["attached"], false);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/get-target-info.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_get_target_info_unknown_target_errors() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-info", "TID-active");

    ctx.process_async(json!({
        "id": 260_020,
        "method": "Target.getTargetInfo",
        "params": { "targetId": "TID-missing" }
    }))
    .await;

    ctx.expect_error(260_020, -31998, "UnknownTargetId");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-windowOpen.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_window_open_blank_creates_popup_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(&mut ctx, "BID-popup", "TID-opener", "<main>opener</main>")
        .await;

    let messages = open_popup_from_runtime(
        &mut ctx,
        260_021,
        "window.open('https://example.com/popup', '_blank') !== null",
    )
    .await;

    let popup = event(&messages, "Target.targetCreated");
    assert_eq!(
        popup["params"]["targetInfo"]["url"],
        "https://example.com/popup"
    );
    assert_eq!(popup["params"]["targetInfo"]["openerId"], "TID-opener");
    assert_eq!(popup["params"]["targetInfo"]["canAccessOpener"], true);
    assert_eq!(
        response(&messages, 260_021)["result"]["result"]["value"],
        true
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-windowOpen.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_window_open_auto_attached_popup_materializes_initial_document() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-auto-load",
                "TID-auto-load-opener",
                "<main>opener</main>",
            )
            .await;
            set_auto_attach(&mut ctx, 260_210, true).await;
            ctx.take_all();

            let popup_url = "data:text/html,%3Cmain%3Enatural-popup%3C/main%3E";
            let messages = open_popup_from_runtime(
                &mut ctx,
                260_211,
                &format!("window.open('{popup_url}', '_blank') !== null"),
            )
            .await;
            let popup = event(&messages, "Target.targetCreated");
            let popup_target_id = popup["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("popup target id");
            let attached = event(&messages, "Target.attachedToTarget");
            let popup_session_id = attached["params"]["sessionId"]
                .as_str()
                .expect("popup session id");
            ctx.wait_until_scheduler_state("auto-attached popup navigation commit", |conn| {
                conn.browser_context_by_id("BID-popup-auto-load")
                    .and_then(|browser_context| {
                        loaded_page_for_target(browser_context, popup_target_id)
                    })
                    .is_some_and(|page| page.final_url().as_str() == popup_url)
            })
            .await;
            let popup_page = ctx
                .conn
                .browser_context
                .as_ref()
                .and_then(|browser_context| {
                    loaded_page_for_target(browser_context, popup_target_id)
                })
                .expect("window.open lifecycle should have loaded the popup document");
            assert_eq!(popup_page.final_url().as_str(), popup_url);

            ctx.process_async(json!({
                "id": 260_212,
                "method": "Page.enable",
                "sessionId": popup_session_id
            }))
            .await;
            take_response_by_id(&mut ctx, 260_212);

            ctx.process_async(json!({
                "id": 260_213,
                "method": "Page.setLifecycleEventsEnabled",
                "sessionId": popup_session_id,
                "params": { "enabled": true }
            }))
            .await;
            take_response_by_id(&mut ctx, 260_213);
            // Chromium replays only lifecycle milestones already reached when
            // `setLifecycleEventsEnabled` runs. With flattened auto-attach, a popup
            // can still be at `commit`; its later DCL/load arrive as ordinary
            // renderer notifications after the command response.
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "background popup DOMContentLoaded lifecycle",
                |message| {
                    message["method"] == json!("Page.lifecycleEvent")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["name"] == json!("DOMContentLoaded")
                        && message["params"]["frameId"] == json!(popup_target_id)
                },
            )
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "background popup load lifecycle tail",
                |message| {
                    message["method"] == json!("Page.lifecycleEvent")
                        && message["params"]["name"] == json!("load")
                        && message["params"]["frameId"] == json!(popup_target_id)
                },
            )
            .await;

            ctx.process_async(json!({
                "id": 260_214,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "document.querySelector('main').textContent",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_214)["result"]["result"]["value"],
                "natural-popup"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_window_open_waiting_popup_routes_initial_document_after_resume() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-wait-route",
        "TID-wait-route-opener",
        "<main>opener</main>",
    )
    .await;
    set_auto_attach_waiting_for_debugger(&mut ctx, 260_215).await;
    ctx.take_all();

    let popup_url = fixture.url("/plain?popup=wait-route");
    let messages = open_popup_from_runtime(
        &mut ctx,
        260_216,
        &format!("window.open('{popup_url}', '_blank') !== null"),
    )
    .await;
    let popup = event(&messages, "Target.targetCreated");
    let popup_target_id = popup["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id");
    let attached = event(&messages, "Target.attachedToTarget");
    let popup_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup session id");

    ctx.process_async(json!({
        "id": 260_217,
        "method": "Page.enable",
        "sessionId": popup_session_id
    }))
    .await;
    take_response_by_id(&mut ctx, 260_217);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| {
                loaded_page_for_target(browser_context, popup_target_id)
            })
            .is_some_and(|page| page.final_url().as_str() == "about:blank"),
        "popup target lifecycle should already expose the initial about:blank document"
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                || (message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["request"]["url"] == json!(popup_url))
        }),
        "Page.enable must not start the real popup URL before Runtime.runIfWaitingForDebugger: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 260_218,
        "method": "Fetch.enable",
        "sessionId": popup_session_id,
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "resourceType": "Document",
                "requestStage": "Request"
            }]
        }
    }))
    .await;
    ctx.expect_result(260_218, json!({}), Some(popup_session_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 260_219,
        "method": "Runtime.runIfWaitingForDebugger",
        "sessionId": popup_session_id
    }))
    .await;
    take_response_by_id(&mut ctx, 260_219);
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "Runtime.runIfWaitingForDebugger should start the popup document navigation: {:?}",
                ctx.sent
            )
        });
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(popup_session_id)
                && message["params"]["frameId"] == json!(popup_target_id)
        }),
        "popup initial document Network.requestWillBeSent should be emitted on the popup owner session: {:?}",
        ctx.sent
    );
    assert_eq!(paused["sessionId"], popup_session_id);
    assert_eq!(paused["params"]["request"]["url"], popup_url);
    assert_eq!(paused["params"]["resourceType"], "Document");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 260_220,
        "method": "Page.createIsolatedWorld",
        "sessionId": popup_session_id,
        "params": {
            "frameId": popup_target_id,
            "worldName": "__playwright_utility_world_page",
            "grantUniveralAccess": true
        }
    }))
    .await;
    let isolated = take_response_by_id(&mut ctx, 260_220);
    assert!(
        isolated["result"]["executionContextId"].as_i64().is_some(),
        "createIsolatedWorld should resolve while popup initial document is paused: {isolated:?}"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "createIsolatedWorld should not start a second popup document navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 260_221,
        "method": "Fetch.fulfillRequest",
        "sessionId": popup_session_id,
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/html; charset=utf-8" }
            ],
            "body": "PCFkb2N0eXBlIGh0bWw+PG1haW4+cm91dGVkLXBvcHVwPC9tYWluPg=="
        }
    }))
    .await;
    ctx.expect_result(260_221, json!({}), Some(popup_session_id));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "resumed popup load lifecycle",
        |message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!(popup_session_id)
        },
    )
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["sessionId"] == json!(popup_session_id)
                && message["params"]["frame"]["id"] == json!(popup_target_id)
        }),
        "resumed popup navigation events should use the popup target as frame id: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 260_222,
        "method": "Runtime.evaluate",
        "sessionId": popup_session_id,
        "params": {
            "expression": "document.querySelector('main').textContent",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 260_222)["result"]["result"]["value"],
        "routed-popup"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-windowOpen-empty-url.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_window_open_empty_url_creates_about_blank_popup() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-empty",
        "TID-empty-opener",
        "<main>opener</main>",
    )
    .await;

    let messages =
        open_popup_from_runtime(&mut ctx, 260_022, "window.open('', '_blank') !== null").await;

    let popup = event(&messages, "Target.targetCreated");
    assert_eq!(popup["params"]["targetInfo"]["url"], "about:blank");
    assert_eq!(
        popup["params"]["targetInfo"]["openerId"],
        "TID-empty-opener"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-windowOpen-javascript-url.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_window_open_javascript_url_still_reports_popup_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-js",
        "TID-js-opener",
        "<main>opener</main>",
    )
    .await;

    let messages = open_popup_from_runtime(
        &mut ctx,
        260_023,
        "window.open('javascript:42', '_blank') !== null",
    )
    .await;

    let popup = event(&messages, "Target.targetCreated");
    assert_eq!(popup["params"]["targetInfo"]["url"], "javascript:42");
    assert_eq!(popup["params"]["targetInfo"]["openerId"], "TID-js-opener");
}

// Chromium/WPT source:
// third_party/blink/web_tests/external/wpt/html/semantics/links/
// links-created-by-a-and-area-elements/target_blank_implicit_noopener.html
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_anchor_blank_uses_implicit_noopener() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-anchor",
        "TID-anchor-opener",
        "<main>opener</main>",
    )
    .await;

    let messages = open_popup_from_runtime(
        &mut ctx,
        260_024,
        "document.body.innerHTML = '<a id=\"a\" href=\"https://example.com/a\" target=\"_blank\">a</a>'; document.getElementById('a').click(); 'clicked'",
    )
    .await;

    assert_eq!(
        response(&messages, 260_024)["result"]["result"]["value"],
        "clicked"
    );
    let popup = event(&messages, "Target.targetCreated");
    assert_eq!(
        popup["params"]["targetInfo"]["url"],
        "https://example.com/a"
    );
    assert_eq!(popup["params"]["targetInfo"]["canAccessOpener"], false);
    assert_eq!(
        popup["params"]["targetInfo"]["openerId"],
        "TID-anchor-opener"
    );
    assert_eq!(
        popup["params"]["targetInfo"]["openerFrameId"],
        "TID-anchor-opener"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-windowOpen.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_window_open_self_does_not_create_popup_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-self",
        "TID-self-opener",
        "<main>opener</main>",
    )
    .await;

    let messages = open_popup_from_runtime(
        &mut ctx,
        260_025,
        "window.open('data:text/html,<main>self</main>', '_self') === null",
    )
    .await;

    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == "Target.targetCreated"),
        "{messages:?}"
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        "data:text/html,<main>self</main>"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-info-changed.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_window_open_named_target_reuses_existing_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-name",
        "TID-name-opener",
        "<main>opener</main>",
    )
    .await;

    let first = open_popup_from_runtime(
        &mut ctx,
        260_026,
        "window.open('https://example.com/one', 'named') !== null",
    )
    .await;
    let target_id = event(&first, "Target.targetCreated")["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("named target id")
        .to_owned();

    let second = open_popup_from_runtime(
        &mut ctx,
        260_027,
        "window.open('https://example.com/two', 'named') !== null",
    )
    .await;

    assert!(
        !second
            .iter()
            .any(|message| message["method"] == "Target.targetCreated"),
        "{second:?}"
    );
    let changed = event(&second, "Target.targetInfoChanged");
    assert_eq!(changed["params"]["targetInfo"]["targetId"], target_id);
    assert_eq!(
        changed["params"]["targetInfo"]["url"],
        "https://example.com/two"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-info-changed.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_info_changed_is_emitted_for_named_popup_reuse() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-change",
        "TID-change-opener",
        "<main>opener</main>",
    )
    .await;

    let _first = open_popup_from_runtime(
        &mut ctx,
        260_028,
        "window.open('https://example.com/first', 'reuse') !== null",
    )
    .await;
    let second = open_popup_from_runtime(
        &mut ctx,
        260_029,
        "window.open('https://example.com/second', 'reuse') !== null",
    )
    .await;

    assert_eq!(
        event(&second, "Target.targetInfoChanged")["params"]["targetInfo"]["url"],
        "https://example.com/second"
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-windowOpen.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_resetting_opener_clears_popup_opener_reference() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-popup-reset",
        "TID-reset-opener",
        "<main>opener</main>",
    )
    .await;

    let messages = open_popup_from_runtime(
        &mut ctx,
        260_030,
        "window.open('https://example.com/reset', '_blank') !== null",
    )
    .await;
    let target_id = event(&messages, "Target.targetCreated")["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();

    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .reset_active_target_slot_to_empty_async()
        .await;

    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .target_opener_ids
            .contains_key(&target_id)
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .target_opener_frame_ids
            .get(&target_id)
            .map(String::as_str),
        Some("TID-reset-opener")
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_existing_active_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-auto-active", "TID-active");

    set_auto_attach(&mut ctx, 260_031, true).await;
    let attached = ctx.take_one();

    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(attached["params"]["targetInfo"]["targetId"], "TID-active");
    assert_eq!(attached["params"]["targetInfo"]["attached"], true);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_existing_background_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc(&mut ctx, "BID-auto-bg");
    push_background_target(&mut ctx, "TID-background", "about:blank#bg", None);

    set_auto_attach(&mut ctx, 260_032, true).await;
    let attached = ctx.take_one();

    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        "TID-background"
    );
    assert_eq!(attached["params"]["targetInfo"]["attached"], true);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_existing_active_and_background_targets() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-auto-both", "TID-active");
    push_background_target(&mut ctx, "TID-background", "about:blank#bg", None);

    set_auto_attach(&mut ctx, 260_033, true).await;
    let messages = ctx.take_all();

    assert_eq!(
        messages
            .iter()
            .filter(|message| message["method"] == "Target.attachedToTarget")
            .count(),
        2,
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["params"]["targetInfo"]["targetId"] == "TID-active")
    );
    assert!(
        messages
            .iter()
            .any(|message| message["params"]["targetInfo"]["targetId"] == "TID-background")
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_does_not_reattach_existing_session() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-auto-existing", "TID-active");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-existing");

    set_auto_attach(&mut ctx, 260_034, true).await;

    assert!(ctx.sent.is_empty());
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_session_id(),
        Some("SID-existing")
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_false_detaches_active_session() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-detach-active", "TID-active");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    set_auto_attach(&mut ctx, 260_035, false).await;
    let detached = ctx.take_one();

    assert_eq!(detached["method"], "Target.detachedFromTarget");
    assert_eq!(detached["params"]["targetId"], "TID-active");
    assert_eq!(detached["params"]["sessionId"], "SID-active");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_auto_attach_false_detaches_background_session() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-detach-bg", "TID-active");
    push_background_target(&mut ctx, "TID-background", "about:blank#bg", Some("SID-bg"));
    ctx.conn.auto_attach = true;

    set_auto_attach(&mut ctx, 260_036, false).await;
    let detached = ctx.take_one();

    assert_eq!(detached["method"], "Target.detachedFromTarget");
    assert_eq!(detached["params"]["targetId"], "TID-background");
    assert_eq!(detached["params"]["sessionId"], "SID-bg");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_attach_to_browser_target_emits_browser_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc(&mut ctx, "BID-browser-session");

    ctx.process_async(json!({
        "id": 260_037,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let messages = ctx.take_all();

    let attached = event(&messages, "Target.attachedToTarget");
    assert_eq!(attached["params"]["targetInfo"]["type"], "browser");
    assert_eq!(attached["params"]["targetInfo"]["targetId"], "browser");
    assert_eq!(attached["params"]["targetInfo"]["attached"], true);
    assert_eq!(
        response(&messages, 260_037)["result"]["sessionId"],
        attached["params"]["sessionId"]
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/browser-auto-attach-tab.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_attach_to_target_from_browser_session_creates_auxiliary_session()
{
    let mut ctx = TestContext::new_with_target_discovery(false);
    let browser_context_id = create_browser_context(&mut ctx, 260_038).await;
    let target_id =
        create_target(&mut ctx, 260_039, Some(&browser_context_id), "about:blank").await;
    ctx.process_async(json!({
        "id": 260_040,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let browser_session_id =
        event(&ctx.take_all(), "Target.attachedToTarget")["params"]["sessionId"]
            .as_str()
            .expect("browser session")
            .to_owned();

    let session_id =
        attach_to_target(&mut ctx, 260_041, Some(&browser_session_id), &target_id).await;
    let attached = ctx.take_one();

    assert_eq!(attached["sessionId"], browser_session_id);
    assert_eq!(attached["params"]["sessionId"], session_id);
    assert_eq!(attached["params"]["targetInfo"]["targetId"], target_id);
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/message-to-detached-session.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_detach_from_target_emits_detached_event() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-detach", "TID-active");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 260_042,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": "SID-active" }
    }))
    .await;
    let messages = ctx.take_all();

    assert_eq!(response(&messages, 260_042)["result"], json!({}));
    let detached = event(&messages, "Target.detachedFromTarget");
    assert_eq!(detached["params"]["targetId"], "TID-active");
    assert_eq!(detached["params"]["sessionId"], "SID-active");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-send-message.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_send_message_to_target_wraps_nested_result() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-send-message",
        "TID-active",
        "<main>send message</main>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 260_043,
        "method": "Target.sendMessageToTarget",
        "params": {
            "sessionId": "SID-active",
            "message": "{\"id\":1,\"method\":\"Runtime.evaluate\",\"params\":{\"expression\":\"1 + 2\",\"returnByValue\":true}}"
        }
    }))
    .await;
    let messages = ctx.take_all();

    assert_eq!(response(&messages, 260_043)["result"], json!({}));
    let received = event(&messages, "Target.receivedMessageFromTarget");
    assert_eq!(received["params"]["sessionId"], "SID-active");
    let nested: Value = serde_json::from_str(
        received["params"]["message"]
            .as_str()
            .expect("nested protocol message"),
    )
    .expect("nested protocol JSON");
    assert_eq!(nested["id"], 1);
    assert_eq!(nested["result"]["result"]["value"], 3, "{nested}");

    for (outer_id, nested) in [
        (
            260_044,
            json!({
                "id": 2,
                "method": "Emulation.setScriptExecutionDisabled",
                "params": { "value": true }
            }),
        ),
        (
            260_045,
            json!({ "id": 3, "method": "Performance.enable", "params": {} }),
        ),
        (
            260_046,
            json!({ "id": 4, "method": "Performance.getMetrics", "params": {} }),
        ),
    ] {
        ctx.process_async(json!({
            "id": outer_id,
            "method": "Target.sendMessageToTarget",
            "params": {
                "sessionId": "SID-active",
                "message": nested.to_string(),
            }
        }))
        .await;
        let messages = ctx.take_all();
        assert_eq!(response(&messages, outer_id)["result"], json!({}));
        let received = event(&messages, "Target.receivedMessageFromTarget");
        let nested_response: Value = serde_json::from_str(
            received["params"]["message"]
                .as_str()
                .expect("nested protocol message"),
        )
        .expect("nested protocol JSON");
        assert_eq!(nested_response["id"], nested["id"]);
        assert!(nested_response.get("result").is_some(), "{nested_response}");
        if nested["method"] == "Performance.getMetrics" {
            assert!(
                nested_response["result"]["metrics"].is_array(),
                "{nested_response}"
            );
        }
    }
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/message-to-detached-session.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_send_message_to_target_invalid_session_errors() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-send-invalid", "TID-active");

    ctx.process_async(json!({
        "id": 260_044,
        "method": "Target.sendMessageToTarget",
        "params": {
            "sessionId": "SID-missing",
            "message": "{\"id\":1,\"method\":\"Runtime.evaluate\",\"params\":{\"expression\":\"1\"}}"
        }
    }))
    .await;

    ctx.expect_error(260_044, -31998, "InvalidSessionId");
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/target-setAutoAttach-new-page.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_close_background_target_detaches_sessions() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-close-bg", "TID-active");
    push_background_target(&mut ctx, "TID-background", "about:blank#bg", Some("SID-bg"));
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-background", "SID-aux".into())
    );

    ctx.process_async(json!({
        "id": 260_045,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-background" }
    }))
    .await;
    let messages = ctx.take_all();

    assert_eq!(
        response(&messages, 260_045)["result"],
        json!({ "success": true })
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["method"] == "Target.detachedFromTarget")
            .count(),
        2,
        "{messages:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .background_target("TID-background")
            .is_none()
    );
}

// Chromium source:
// third_party/blink/web_tests/http/tests/inspector-protocol/target/tab-target.js
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_chromium_target_activate_background_target_promotes_it() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_target(&mut ctx, "BID-activate", "TID-active");
    push_background_target(
        &mut ctx,
        "TID-background",
        "https://example.com/background",
        None,
    );

    ctx.process_async(json!({
        "id": 260_046,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-background" }
    }))
    .await;

    ctx.expect_result(260_046, json!({}), None);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_target_id(),
        Some("TID-background")
    );
}
