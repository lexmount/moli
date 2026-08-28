use super::super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::super::*;
use crate::conn::NETWORK_ERROR_PAGE_URL;
use crate::testing::spawn_connection_drop_server;
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

async fn enable_popup_document_response_stage(
    ctx: &mut TestContext,
    session_id: &str,
    id: u64,
    url_pattern: &str,
) {
    ctx.process_async(json!({
        "id": id,
        "method": "Fetch.enable",
        "sessionId": session_id,
        "params": {
            "patterns": [{
                "urlPattern": url_pattern,
                "resourceType": "Document",
                "requestStage": "Response"
            }]
        }
    }))
    .await;
    ctx.expect_result(id, json!({}), Some(session_id));
    ctx.take_all();
}

async fn fulfill_popup_document_response_stage(
    ctx: &mut TestContext,
    session_id: &str,
    final_url: &str,
    id: u64,
    body: &str,
) {
    crate::testing::wait_until_scheduler_message(
        ctx,
        "COOP redirect final response-stage pause",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!(session_id)
                && message["params"]["request"]["url"] == json!(final_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;
    let paused_request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!(session_id)
                && message["params"]["request"]["url"] == json!(final_url)
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("COOP redirect response-stage request id")
        .to_owned();
    ctx.process_async(json!({
        "id": id,
        "method": "Fetch.fulfillRequest",
        "sessionId": session_id,
        "params": {
            "requestId": paused_request_id,
            "responseCode": 200,
            "responseHeaders": [{
                "name": "content-type",
                "value": "text/html; charset=utf-8"
            }],
            "body": body
        }
    }))
    .await;
    ctx.expect_result(id, json!({}), Some(session_id));
}

async fn enable_popup_page_runtime_network(ctx: &mut TestContext, session_id: &str, first_id: u64) {
    for (offset, method) in ["Page.enable", "Runtime.enable", "Network.enable"]
        .into_iter()
        .enumerate()
    {
        let id = first_id + offset as u64;
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": session_id,
            "params": {}
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }
    ctx.take_all();
}

async fn wait_for_coop_sandbox_blocked_error_document(
    ctx: &mut TestContext,
    browser_context_id: &str,
    popup_target_id: &str,
    popup_session_id: &str,
    description: &str,
) {
    crate::testing::wait_until_scheduler_message(ctx, description, |message| {
        message["method"] == json!("Network.loadingFailed")
            && message["sessionId"] == json!(popup_session_id)
            && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_RESPONSE")
            && message["params"]["blockedReason"]
                == json!("CoopSandboxedIframeCannotNavigateToCoopPage")
    })
    .await;
    crate::testing::wait_until_scheduler_message(ctx, description, |message| {
        message["method"] == json!("Page.frameNavigated")
            && message["sessionId"] == json!(popup_session_id)
            && message["params"]["frame"]["id"] == json!(popup_target_id)
            && message["params"]["frame"]["url"] == json!(NETWORK_ERROR_PAGE_URL)
    })
    .await;
    crate::testing::wait_until_scheduler_message(ctx, description, |message| {
        message["method"] == json!("Page.loadEventFired")
            && message["sessionId"] == json!(popup_session_id)
    })
    .await;
    ctx.wait_until_scheduler_state(description, |conn| {
        !conn.has_pending_document_navigation_for_session_owner(Some(popup_session_id))
            && conn
                .browser_context_by_id(browser_context_id)
                .and_then(|browser_context| {
                    loaded_page_for_target(browser_context, popup_target_id)
                })
                .is_some_and(|page| page.final_url().as_str() == NETWORK_ERROR_PAGE_URL)
    })
    .await;
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

// Chromium sources:
// content/browser/security/coop/cross_origin_opener_policy_status.cc
// content/browser/renderer_host/browsing_context_group_swap.cc
// third_party/blink/web_tests/external/wpt/html/cross-origin-opener-policy/
#[tokio::test(flavor = "multi_thread")]
async fn popup_coop_redirect_survives_fetch_response_override_and_severs_old_group_proxy() {
    target_8mb_stack("popup-coop-redirect-fetch-override", || async {
        run_popup_coop_redirect_fetch_response_override_regression().await;
    })
    .await;
}

async fn run_popup_coop_redirect_fetch_response_override_regression() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-coop",
                "TID-popup-coop-opener",
                "<main>COOP opener</main>",
            )
            .await;
            set_auto_attach_waiting_for_debugger(&mut ctx, 2_602_700).await;
            ctx.take_all();
            let opener_session_id = "SID-popup-coop-opener";
            ctx.conn
                .browser_context
                .as_mut()
                .expect("COOP opener browser context")
                .attach_active_session(opener_session_id);

            let requested_url = fixture.url("/coop-redirect-start");
            let coop_url = fixture.url("/coop-redirect-final");
            let messages = open_popup_from_runtime(
                &mut ctx,
                2_602_701,
                &format!(
                    "(() => {{ const popup = window.open({requested_url:?}, 'coop-protocol-target'); globalThis.__lmCoopPopup = popup; return popup !== null && popup.closed === false; }})()"
                ),
            )
            .await;
            assert_eq!(
                response(&messages, 2_602_701)["result"]["result"]["value"],
                true
            );
            let popup_target_id = event(&messages, "Target.targetCreated")["params"]
                ["targetInfo"]["targetId"]
                .as_str()
                .expect("COOP popup target id")
                .to_owned();
            let popup_session_id = event(&messages, "Target.attachedToTarget")["params"]
                ["sessionId"]
                .as_str()
                .expect("COOP popup session id")
                .to_owned();

            for (id, method) in [(2_602_702, "Page.enable"), (2_602_703, "Runtime.enable")] {
                ctx.process_async(json!({
                    "id": id,
                    "method": method,
                    "sessionId": popup_session_id,
                    "params": {}
                }))
                .await;
                ctx.expect_result(id, json!({}), Some(&popup_session_id));
            }
            ctx.take_all();
            Box::pin(enable_popup_document_response_stage(
                &mut ctx,
                &popup_session_id,
                2_602_790,
                "*coop-redirect-final*",
            ))
            .await;
            ctx.process_async(json!({
                "id": 2_602_704,
                "method": "Runtime.runIfWaitingForDebugger",
                "sessionId": popup_session_id
            }))
            .await;
            take_response_by_id(&mut ctx, 2_602_704);
            Box::pin(fulfill_popup_document_response_stage(
                &mut ctx,
                &popup_session_id,
                &coop_url,
                2_602_791,
                "PCFkb2N0eXBlIGh0bWw+PG1haW4gaWQ9J2Nvb3AtbWFya2VyJz5DT09QIHJlZGlyZWN0IEZldGNoIG92ZXJyaWRlIHBvcHVwPC9tYWluPg==",
            ))
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "COOP popup frame commit",
                |message| {
                    message["method"] == json!("Page.frameNavigated")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["frame"]["id"] == json!(popup_target_id)
                        && message["params"]["frame"]["url"] == json!(coop_url)
                },
            )
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "COOP replacement Runtime context",
                |message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                },
            )
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "COOP replacement load",
                |message| {
                    message["method"] == json!("Page.loadEventFired")
                        && message["sessionId"] == json!(popup_session_id)
                },
            )
            .await;
            ctx.wait_until_scheduler_state("COOP popup navigation completion", |conn| {
                !conn.has_pending_document_navigation_for_session_owner(Some(&popup_session_id))
                    && conn
                        .browser_context_by_id("BID-popup-coop")
                        .and_then(|browser_context| {
                            loaded_page_for_target(browser_context, &popup_target_id)
                        })
                        .is_some_and(|page| page.final_url().as_str() == coop_url)
            })
            .await;
            let commit_messages = ctx.take_all();
            assert!(
                commit_messages.iter().any(|message| {
                    message["method"] == json!("Runtime.executionContextsCleared")
                        && message["sessionId"] == json!(popup_session_id)
                }),
                "COOP agent replacement must clear the old session contexts: {commit_messages:?}"
            );
            assert!(
                !commit_messages.iter().any(|message| {
                    matches!(
                        message["method"].as_str(),
                        Some("Target.targetCreated" | "Target.targetDestroyed")
                    )
                }),
                "COOP group switch must not replace the protocol target: {commit_messages:?}"
            );

            ctx.process_async(json!({
                "id": 2_602_705,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ openerSevered: opener === null, nameCleared: name === '', closed, text: document.querySelector('#coop-marker').textContent })",
                    "returnByValue": true
                }
            }))
            .await;
            let new_realm_evaluation = take_response_by_id(&mut ctx, 2_602_705);
            assert_eq!(
                new_realm_evaluation["result"]["result"]["value"],
                json!({
                    "openerSevered": true,
                    "nameCleared": true,
                    "closed": false,
                    "text": "COOP redirect Fetch override popup"
                }),
                "unexpected COOP replacement realm evaluation: {new_realm_evaluation:?}"
            );

            ctx.process_async(json!({
                "id": 2_602_706,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": "({ oldProxyClosed: __lmCoopPopup.closed, openerClosed: closed })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 2_602_706)["result"]["result"]["value"],
                json!({ "oldProxyClosed": true, "openerClosed": false })
            );

            ctx.process_async(json!({
                "id": 2_602_707,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": r#"(() => {
  const stale = globalThis.__lmCoopPopup;
  globalThis.__lmStaleEndpointSelfMessages = 0;
  addEventListener("message", event => {
    if (event.data === "must-drop-stale-endpoint") {
      globalThis.__lmStaleEndpointSelfMessages++;
    }
  });
  let missingArgsTypeError;
  try {
    stale.postMessage();
    missingArgsTypeError = false;
  } catch (error) {
    missingArgsTypeError = error instanceof TypeError;
  }
  stale.postMessage("must-drop-stale-endpoint", "*");
  stale.location.href = "https://must-not-route.test/assign";
  stale.location.replace("https://must-not-route.test/replace");
  stale.close();
  stale.focus();
  return {
    closed: stale.closed,
    openerIsNull: stale.opener === null,
    length: stale.length,
    missingArgsTypeError,
    selfMessages: globalThis.__lmStaleEndpointSelfMessages
  };
})()"#,
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 2_602_707)["result"]["result"]["value"],
                json!({
                    "closed": true,
                    "openerIsNull": true,
                    "length": 0,
                    "missingArgsTypeError": true,
                    "selfMessages": 0
                }),
                "the disconnected endpoint must drop every routed operation"
            );

            ctx.process_async(json!({
                "id": 2_602_708,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ href: location.href, closed, openerSevered: opener === null, text: document.querySelector('#coop-marker').textContent })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 2_602_708)["result"]["result"]["value"],
                json!({
                    "href": coop_url,
                    "closed": false,
                    "openerSevered": true,
                    "text": "COOP redirect Fetch override popup"
                }),
                "stale operations must not mutate the replacement Page"
            );

            ctx.process_async(json!({
                "id": 2_602_709,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": "globalThis.__lmStaleEndpointSelfMessages",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 2_602_709)["result"]["result"]["value"],
                json!(0),
                "stale postMessage must not be rerouted to the opener"
            );

            ctx.process_async(json!({
                "id": 2_602_710,
                "method": "Target.getTargets"
            }))
            .await;
            let targets = take_response_by_id(&mut ctx, 2_602_710);
            let matching_targets = targets["result"]["targetInfos"]
                .as_array()
                .expect("targetInfos")
                .iter()
                .filter(|target| target["targetId"] == json!(popup_target_id))
                .collect::<Vec<_>>();
            assert_eq!(matching_targets.len(), 1);
            assert_eq!(matching_targets[0]["url"], json!(coop_url));
            assert_eq!(matching_targets[0]["attached"], json!(true));
        })
        .await;
}

// Chromium/WPT sources:
// content/browser/security/coop/cross_origin_opener_policy_status.cc::SanitizeResponse
// third_party/blink/web_tests/external/wpt/html/cross-origin-opener-policy/
// coop-csp-sandbox.https.html
#[tokio::test(flavor = "multi_thread")]
async fn popup_sandboxed_coop_redirect_is_blocked_before_follow_and_commits_one_error_document() {
    target_8mb_stack("popup-sandboxed-coop-redirect", || async {
        let fixture = SmokeFixtureServer::start().await;
        let mut ctx = TestContext::new_with_target_discovery(false);
        ctx.enable_background_navigation_scheduler_for_test();
        tokio::task::LocalSet::new()
            .run_until(async {
                load_bc_with_titled_page_async(
                    &mut ctx,
                    "BID-popup-coop-sandbox",
                    "TID-popup-coop-sandbox-opener",
                    "<main>COOP sandbox opener</main>",
                )
                .await;
                set_auto_attach_waiting_for_debugger(&mut ctx, 2_602_900).await;
                ctx.take_all();
                let opener_session_id = "SID-popup-coop-sandbox-opener";
                ctx.conn
                    .browser_context
                    .as_mut()
                    .expect("sandboxed COOP opener browser context")
                    .attach_active_session(opener_session_id);

                let blocked_url = fixture.url("/coop-sandbox-blocked-redirect");
                let messages = open_popup_from_runtime(
                    &mut ctx,
                    2_602_901,
                    &format!(
                        "(() => {{ const popup = window.open({blocked_url:?}, 'coop-sandbox-target'); globalThis.__lmBlockedCoopPopup = popup; return popup !== null && popup.closed === false; }})()"
                    ),
                )
                .await;
                assert_eq!(
                    response(&messages, 2_602_901)["result"]["result"]["value"],
                    true
                );
                let popup_target_id = event(&messages, "Target.targetCreated")["params"]
                    ["targetInfo"]["targetId"]
                    .as_str()
                    .expect("blocked popup target id")
                    .to_owned();
                let popup_session_id = event(&messages, "Target.attachedToTarget")["params"]
                    ["sessionId"]
                    .as_str()
                    .expect("blocked popup session id")
                    .to_owned();

                enable_popup_page_runtime_network(&mut ctx, &popup_session_id, 2_602_902).await;
                ctx.process_async(json!({
                    "id": 2_602_905,
                    "method": "Runtime.runIfWaitingForDebugger",
                    "sessionId": popup_session_id
                }))
                .await;
                take_response_by_id(&mut ctx, 2_602_905);
                wait_for_coop_sandbox_blocked_error_document(
                    &mut ctx,
                    "BID-popup-coop-sandbox",
                    &popup_target_id,
                    &popup_session_id,
                    "sandboxed COOP redirect error commit",
                )
                .await;
                let commit_messages = ctx.take_all();
                let loading_failed = commit_messages
                    .iter()
                    .find(|message| {
                        message["method"] == json!("Network.loadingFailed")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["errorText"]
                                == json!("net::ERR_BLOCKED_BY_RESPONSE")
                    })
                    .expect("blocked response Network.loadingFailed");
                let blocked_request_id = loading_failed["params"]["requestId"].clone();
                assert!(
                    commit_messages.iter().any(|message| {
                        message["method"] == json!("Network.responseReceived")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["requestId"] == blocked_request_id
                            && message["params"]["response"]["url"] == json!(blocked_url)
                            && message["params"]["response"]["status"] == json!(302)
                    }),
                    "the original blocked redirect must remain the Network response surface: {commit_messages:?}"
                );
                assert!(
                    !commit_messages.iter().any(|message| {
                        message["method"] == json!("Network.loadingFinished")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["requestId"] == blocked_request_id
                    }),
                    "the internal error Document body must not finish the blocked network request: {commit_messages:?}"
                );
                assert_eq!(
                    fixture.coop_blocked_redirect_target_requests(),
                    0,
                    "response sanitation must stop the redirect before the target request"
                );
                assert!(
                    !commit_messages.iter().any(|message| {
                        matches!(
                            message["method"].as_str(),
                            Some("Target.targetCreated" | "Target.targetDestroyed")
                        )
                    }),
                    "blocked response must preserve the exact target/session: {commit_messages:?}"
                );

                ctx.process_async(json!({
                    "id": 2_602_906,
                    "method": "Runtime.evaluate",
                    "sessionId": popup_session_id,
                    "params": {
                        "expression": "({ openerSevered: opener === null, href: location.href, blockedBodyAbsent: !document.querySelector('#must-not-commit'), scriptAbsent: globalThis.__blockedCoopBodyRan === undefined, closed })",
                        "returnByValue": true
                    }
                }))
                .await;
                assert_eq!(
                    take_response_by_id(&mut ctx, 2_602_906)["result"]["result"]["value"],
                    json!({
                        "openerSevered": true,
                        "href": NETWORK_ERROR_PAGE_URL,
                        "blockedBodyAbsent": true,
                        "scriptAbsent": true,
                        "closed": false
                    })
                );
                ctx.process_async(json!({
                    "id": 2_602_907,
                    "method": "Runtime.evaluate",
                    "sessionId": opener_session_id,
                    "params": {
                        "expression": "({ popupClosed: __lmBlockedCoopPopup.closed, openerClosed: closed })",
                        "returnByValue": true
                    }
                }))
                .await;
                assert_eq!(
                    take_response_by_id(&mut ctx, 2_602_907)["result"]["result"]["value"],
                    json!({ "popupClosed": true, "openerClosed": false })
                );
            })
            .await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_fetch_effective_sandboxed_coop_response_uses_the_same_blocked_terminal() {
    target_8mb_stack("popup-fetch-sandboxed-coop", || async {
        let fixture = SmokeFixtureServer::start().await;
        let mut ctx = TestContext::new_with_target_discovery(false);
        ctx.enable_background_navigation_scheduler_for_test();
        tokio::task::LocalSet::new()
            .run_until(async {
                load_bc_with_titled_page_async(
                    &mut ctx,
                    "BID-popup-fetch-coop-sandbox",
                    "TID-popup-fetch-coop-sandbox-opener",
                    "<main>Fetch COOP sandbox opener</main>",
                )
                .await;
                set_auto_attach_waiting_for_debugger(&mut ctx, 2_602_920).await;
                ctx.take_all();

                let requested_url = fixture.url("/plain?coop-fetch-sandbox");
                let messages = open_popup_from_runtime(
                    &mut ctx,
                    2_602_921,
                    &format!(
                        "(() => {{ const popup = window.open({requested_url:?}, '_blank'); globalThis.__lmFetchBlockedCoopPopup = popup; return popup !== null; }})()"
                    ),
                )
                .await;
                let popup_target_id = event(&messages, "Target.targetCreated")["params"]
                    ["targetInfo"]["targetId"]
                    .as_str()
                    .expect("Fetch-blocked popup target id")
                    .to_owned();
                let popup_session_id = event(&messages, "Target.attachedToTarget")["params"]
                    ["sessionId"]
                    .as_str()
                    .expect("Fetch-blocked popup session id")
                    .to_owned();
                enable_popup_page_runtime_network(&mut ctx, &popup_session_id, 2_602_922).await;
                enable_popup_document_response_stage(
                    &mut ctx,
                    &popup_session_id,
                    2_602_925,
                    "*plain*",
                )
                .await;
                ctx.process_async(json!({
                    "id": 2_602_926,
                    "method": "Runtime.runIfWaitingForDebugger",
                    "sessionId": popup_session_id
                }))
                .await;
                take_response_by_id(&mut ctx, 2_602_926);
                crate::testing::wait_until_scheduler_message(
                    &mut ctx,
                    "Fetch effective COOP sandbox response pause",
                    |message| {
                        message["method"] == json!("Fetch.requestPaused")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["request"]["url"] == json!(requested_url)
                            && message["params"]["responseStatusCode"] == json!(200)
                    },
                )
                .await;
                let paused_request_id = ctx
                    .sent
                    .iter()
                    .find(|message| {
                        message["method"] == json!("Fetch.requestPaused")
                            && message["sessionId"] == json!(popup_session_id)
                    })
                    .and_then(|message| message["params"]["requestId"].as_str())
                    .expect("Fetch effective response request id")
                    .to_owned();
                ctx.process_async(json!({
                    "id": 2_602_927,
                    "method": "Fetch.fulfillRequest",
                    "sessionId": popup_session_id,
                    "params": {
                        "requestId": paused_request_id,
                        "responseCode": 200,
                        "responseHeaders": [
                            { "name": "content-type", "value": "text/html; charset=utf-8" },
                            { "name": "cross-origin-opener-policy", "value": "same-origin" },
                            { "name": "content-security-policy", "value": "sandbox allow-popups allow-scripts allow-same-origin" }
                        ],
                        "body": "PCFkb2N0eXBlIGh0bWw+PG1haW4gaWQ9J211c3Qtbm90LWNvbW1pdCc+RmV0Y2ggYmxvY2tlZCBib2R5PC9tYWluPjxzY3JpcHQ+Z2xvYmFsVGhpcy5fX2Jsb2NrZWRDb29wQm9keVJhbiA9IHRydWU8L3NjcmlwdD4="
                    }
                }))
                .await;
                ctx.expect_result(2_602_927, json!({}), Some(&popup_session_id));
                wait_for_coop_sandbox_blocked_error_document(
                    &mut ctx,
                    "BID-popup-fetch-coop-sandbox",
                    &popup_target_id,
                    &popup_session_id,
                    "Fetch effective sandboxed COOP error commit",
                )
                .await;

                ctx.process_async(json!({
                    "id": 2_602_928,
                    "method": "Runtime.evaluate",
                    "sessionId": popup_session_id,
                    "params": {
                        "expression": "({ href: location.href, blockedBodyAbsent: !document.querySelector('#must-not-commit'), scriptAbsent: globalThis.__blockedCoopBodyRan === undefined, openerSevered: opener === null })",
                        "returnByValue": true
                    }
                }))
                .await;
                assert_eq!(
                    take_response_by_id(&mut ctx, 2_602_928)["result"]["result"]["value"],
                    json!({
                        "href": NETWORK_ERROR_PAGE_URL,
                        "blockedBodyAbsent": true,
                        "scriptAbsent": true,
                        "openerSevered": true
                    })
                );
            })
            .await;
    })
    .await;
}

// WPT negative control: response CSP sandbox belongs to the Document that
// received it; it must not be persisted as inherited popup frame policy for a
// later navigation whose own response carries COOP without CSP sandbox.
#[tokio::test(flavor = "multi_thread")]
async fn popup_response_csp_sandbox_does_not_block_later_unsandboxed_coop_navigation() {
    target_8mb_stack("popup-response-csp-then-coop", || async {
        let fixture = SmokeFixtureServer::start().await;
        let mut ctx = TestContext::new_with_target_discovery(false);
        ctx.enable_background_navigation_scheduler_for_test();
        tokio::task::LocalSet::new()
            .run_until(async {
                load_bc_with_titled_page_async(
                    &mut ctx,
                    "BID-popup-response-csp",
                    "TID-popup-response-csp-opener",
                    "<main>response CSP opener</main>",
                )
                .await;
                set_auto_attach_waiting_for_debugger(&mut ctx, 2_602_940).await;
                ctx.take_all();
                let opener_session_id = "SID-popup-response-csp-opener";
                ctx.conn
                    .browser_context
                    .as_mut()
                    .expect("response CSP opener browser context")
                    .attach_active_session(opener_session_id);

                let initial_url = fixture.url("/csp-sandbox-navigate-to-coop");
                let final_url = fixture.url("/coop-same-origin");
                let messages = open_popup_from_runtime(
                    &mut ctx,
                    2_602_941,
                    &format!(
                        "(() => {{ const popup = window.open({initial_url:?}, 'response-csp-target'); globalThis.__lmResponseCspPopup = popup; return popup !== null; }})()"
                    ),
                )
                .await;
                let popup_target_id = event(&messages, "Target.targetCreated")["params"]
                    ["targetInfo"]["targetId"]
                    .as_str()
                    .expect("response-CSP popup target id")
                    .to_owned();
                let popup_session_id = event(&messages, "Target.attachedToTarget")["params"]
                    ["sessionId"]
                    .as_str()
                    .expect("response-CSP popup session id")
                    .to_owned();
                enable_popup_page_runtime_network(&mut ctx, &popup_session_id, 2_602_942).await;
                ctx.process_async(json!({
                    "id": 2_602_945,
                    "method": "Runtime.runIfWaitingForDebugger",
                    "sessionId": popup_session_id
                }))
                .await;
                take_response_by_id(&mut ctx, 2_602_945);
                crate::testing::wait_until_scheduler_message(
                    &mut ctx,
                    "response CSP popup later COOP frame commit",
                    |message| {
                        message["method"] == json!("Page.frameNavigated")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["frame"]["id"] == json!(popup_target_id)
                            && message["params"]["frame"]["url"] == json!(final_url)
                    },
                )
                .await;
                assert!(
                    !ctx.sent.iter().any(|message| {
                        message["method"] == json!("Network.loadingFailed")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["errorText"]
                                == json!("net::ERR_BLOCKED_BY_RESPONSE")
                    }),
                    "the previous response's CSP sandbox must not poison the later COOP response: {:?}",
                    ctx.sent
                );
                let final_frame_position = ctx
                    .sent
                    .iter()
                    .rposition(|message| {
                        message["method"] == json!("Page.frameNavigated")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["frame"]["url"] == json!(final_url)
                    })
                    .expect("later COOP frame commit position");
                ctx.sent.drain(..=final_frame_position);
                crate::testing::wait_until_scheduler_message(
                    &mut ctx,
                    "response CSP popup later COOP realm",
                    |message| {
                        message["method"] == json!("Runtime.executionContextCreated")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["context"]["auxData"]["isDefault"]
                                == json!(true)
                    },
                )
                .await;
                crate::testing::wait_until_scheduler_message(
                    &mut ctx,
                    "response CSP popup later COOP load",
                    |message| {
                        message["method"] == json!("Page.loadEventFired")
                            && message["sessionId"] == json!(popup_session_id)
                    },
                )
                .await;
                ctx.wait_until_scheduler_state("response CSP popup later COOP completion", |conn| {
                    !conn.has_pending_document_navigation_for_session_owner(Some(&popup_session_id))
                        && conn
                            .browser_context_by_id("BID-popup-response-csp")
                            .and_then(|browser_context| {
                                loaded_page_for_target(browser_context, &popup_target_id)
                            })
                            .is_some_and(|page| page.final_url().as_str() == final_url)
                })
                .await;
                assert!(
                    !ctx.sent.iter().any(|message| {
                        message["method"] == json!("Network.loadingFailed")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["errorText"]
                                == json!("net::ERR_BLOCKED_BY_RESPONSE")
                    }),
                    "the previous response's CSP sandbox must not poison the later COOP response: {:?}",
                    ctx.sent
                );

                ctx.process_async(json!({
                    "id": 2_602_946,
                    "method": "Runtime.evaluate",
                    "sessionId": popup_session_id,
                    "params": {
                        "expression": "({ href: location.href, openerSevered: opener === null, nameCleared: name === '', marker: document.querySelector('#coop-marker').textContent })",
                        "returnByValue": true
                    }
                }))
                .await;
                assert_eq!(
                    take_response_by_id(&mut ctx, 2_602_946)["result"]["result"]["value"],
                    json!({
                        "href": final_url,
                        "openerSevered": true,
                        "nameCleared": true,
                        "marker": "COOP committed popup"
                    })
                );
                ctx.process_async(json!({
                    "id": 2_602_947,
                    "method": "Runtime.evaluate",
                    "sessionId": opener_session_id,
                    "params": {
                        "expression": "__lmResponseCspPopup.closed",
                        "returnByValue": true
                    }
                }))
                .await;
                assert_eq!(
                    take_response_by_id(&mut ctx, 2_602_947)["result"]["result"]["value"],
                    true
                );
            })
            .await;
    })
    .await;
}

// Chromium sources:
// third_party/blink/renderer/core/frame/local_dom_window.cc::close
// third_party/blink/renderer/core/frame/dom_window.cc::closed
// content/browser/web_contents/web_contents_impl.cc::ClosePage
#[tokio::test(flavor = "multi_thread")]
async fn popup_window_close_retires_target_and_parks_stable_window_proxy() {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("early-close destination listener");
    listener
        .set_nonblocking(true)
        .expect("early-close destination listener nonblocking mode");
    let destination_url = format!(
        "http://{}/must-not-start",
        listener.local_addr().expect("early-close listener address")
    );
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-window-close",
                "TID-popup-window-close-opener",
                "<main>opener</main>",
            )
            .await;
            set_auto_attach(&mut ctx, 2_602_430, true).await;
            ctx.take_all();
            let opener_session_id = "SID-popup-window-close-opener";
            ctx.conn
                .browser_context
                .as_mut()
                .expect("window.close opener browser context")
                .attach_active_session(opener_session_id);

            ctx.process_async(json!({
                "id": 2_602_431,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": format!(r#"(() => {{
  const popup = window.open({destination_url:?}, "_blank");
  globalThis.__lmClosedPopup = popup;
  globalThis.__lmClosedPopupAlias = popup;
  const initialDocument = popup.document;
  popup.close();
  const afterFirstClose = [
    popup.closed,
    popup === globalThis.__lmClosedPopupAlias,
    popup.window === popup,
    popup.opener === window,
    popup.document === initialDocument
  ];
  popup.close();
  return afterFirstClose;
}})()"#),
                    "returnByValue": true
                }
            }))
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "window.close popup targetDestroyed",
                |message| message["method"] == json!("Target.targetDestroyed"),
            )
            .await;

            let messages = ctx.take_all();
            assert_eq!(
                response(&messages, 2_602_431)["result"]["result"]["value"],
                json!([true, true, true, true, true]),
                "close() must synchronously expose Closing while the initial Document is still alive"
            );
            let popup_target_id = event(&messages, "Target.targetCreated")["params"]
                ["targetInfo"]["targetId"]
                .as_str()
                .expect("closed popup target id")
                .to_owned();
            let popup_session_id = event(&messages, "Target.attachedToTarget")["params"]
                ["sessionId"]
                .as_str()
                .expect("closed popup session id")
                .to_owned();
            let destroyed = messages
                .iter()
                .filter(|message| {
                    message["method"] == json!("Target.targetDestroyed")
                        && message["params"]["targetId"] == json!(popup_target_id)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                destroyed.len(),
                1,
                "duplicate close() calls must retire the target exactly once: {messages:?}"
            );
            let response_index = messages
                .iter()
                .position(|message| message["id"] == json!(2_602_431))
                .expect("window.close opener evaluation response");
            let created_index = messages
                .iter()
                .position(|message| message["method"] == json!("Target.targetCreated"))
                .expect("window.close popup targetCreated");
            let destroyed_index = messages
                .iter()
                .position(|message| {
                    message["method"] == json!("Target.targetDestroyed")
                        && message["params"]["targetId"] == json!(popup_target_id)
                })
                .expect("window.close popup targetDestroyed");
            assert!(
                response_index < destroyed_index && created_index < destroyed_index,
                "the opener command response and target creation must precede target retirement: {messages:?}"
            );
            assert!(
                ctx.conn
                    .browser_context_by_id("BID-popup-window-close")
                    .and_then(|browser_context| {
                        browser_context.background_target(&popup_target_id)
                    })
                    .is_none(),
                "window.close target must leave no background target residence"
            );
            assert!(
                ctx.conn
                    .target_page_residence_identity_for_session(Some(&popup_session_id))
                    .is_none(),
                "window.close target must leave no session-owned Page residence"
            );
            assert!(
                matches!(
                    listener.accept(),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
                ),
                "open(url); popup.close() must not start the destination fetch"
            );

            ctx.process_async(json!({
                "id": 2_602_432,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": r#"(() => {
  const popup = globalThis.__lmClosedPopup;
  let deniedDocument;
  try {
    void popup.document;
    deniedDocument = "allowed";
  } catch (error) {
    deniedDocument = error && error.name;
  }
  return {
    identity: popup === globalThis.__lmClosedPopupAlias,
    closed: popup.closed,
    openerIsWindow: popup.opener === window,
    length: popup.length,
    windowIdentity: popup.window === popup,
    deniedDocument
  };
})()"#,
                    "returnByValue": true
                }
            }))
            .await;
            let retained_proxy_response = take_response_by_id(&mut ctx, 2_602_432);
            assert_eq!(
                retained_proxy_response["result"]["result"]["value"],
                json!({
                    "identity": true,
                    "closed": true,
                    "openerIsWindow": true,
                    "length": 0,
                    "windowIdentity": true,
                    "deniedDocument": "SecurityError"
                }),
                "the opener must retain the exact stable WindowProxy on its host-free closed facade: {retained_proxy_response:?}"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn target_close_parks_the_same_stable_popup_window_proxy() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-target-close",
                "TID-popup-target-close-opener",
                "<main>opener</main>",
            )
            .await;
            set_auto_attach(&mut ctx, 2_602_433, true).await;
            ctx.take_all();

            let opened = open_popup_from_runtime(
                &mut ctx,
                2_602_434,
                "(() => { const popup = window.open('about:blank', '_blank'); globalThis.__lmTargetClosedPopup = popup; globalThis.__lmTargetClosedPopupAlias = popup; return popup.closed; })()",
            )
            .await;
            assert_eq!(
                response(&opened, 2_602_434)["result"]["result"]["value"],
                false
            );
            let popup_target_id = event(&opened, "Target.targetCreated")["params"]
                ["targetInfo"]["targetId"]
                .as_str()
                .expect("Target.closeTarget popup id")
                .to_owned();

            ctx.process_async(json!({
                "id": 2_602_435,
                "method": "Target.closeTarget",
                "params": { "targetId": popup_target_id }
            }))
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "Target.closeTarget popup targetDestroyed",
                |message| {
                    message["method"] == json!("Target.targetDestroyed")
                        && message["params"]["targetId"] == json!(popup_target_id)
                },
            )
            .await;
            let closed = ctx.take_all();
            assert_eq!(
                response(&closed, 2_602_435)["result"],
                json!({ "success": true })
            );
            assert_eq!(
                closed
                    .iter()
                    .filter(|message| {
                        message["method"] == json!("Target.targetDestroyed")
                            && message["params"]["targetId"] == json!(popup_target_id)
                    })
                    .count(),
                1,
                "Target.closeTarget must retire the popup exactly once: {closed:?}"
            );

            ctx.process_async(json!({
                "id": 2_602_436,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"(() => {
  const popup = globalThis.__lmTargetClosedPopup;
  let deniedDocument;
  try {
    void popup.document;
    deniedDocument = "allowed";
  } catch (error) {
    deniedDocument = error && error.name;
  }
  return [
    popup === globalThis.__lmTargetClosedPopupAlias,
    popup.closed,
    popup.opener === window,
    popup.length,
    popup.window === popup,
    deniedDocument
  ];
})()"#,
                    "returnByValue": true
                }
            }))
            .await;
            let retained_proxy_response = take_response_by_id(&mut ctx, 2_602_436);
            assert_eq!(
                retained_proxy_response["result"]["result"]["value"],
                json!([true, true, true, 0, true, "SecurityError"]),
                "Target.closeTarget and window.close must share final stable-WindowProxy teardown: {retained_proxy_response:?}"
            );
        })
        .await;
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

// Chromium source/runtime evidence:
// content/browser/renderer_host/navigation_request.cc::CommitErrorPage
// content/browser/renderer_host/navigation_request_browsertest.cc
// A failed popup destination replaces the initial empty Document while the
// auxiliary browsing context, stable Page, WindowProxy, and opener relation survive.
#[tokio::test(flavor = "multi_thread")]
async fn popup_transport_failure_commits_error_document_in_stable_auxiliary_page() {
    let (failing_addr, failing_server) = spawn_connection_drop_server().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-network-error",
                "TID-popup-network-error-opener",
                "<main>opener</main>",
            )
            .await;
            set_auto_attach_waiting_for_debugger(&mut ctx, 260_223).await;
            ctx.take_all();
            let opener_session_id = "SID-popup-network-error-opener";
            ctx.conn
                .browser_context
                .as_mut()
                .expect("network-error opener browser context")
                .attach_active_session(opener_session_id);

            let unreachable_url = format!("http://{failing_addr}/popup-error");
            let messages = open_popup_from_runtime(
                &mut ctx,
                260_224,
                &format!(
                    "(() => {{ const popup = window.open('{unreachable_url}', '_blank'); globalThis.__networkErrorPopup = popup; globalThis.__networkErrorPopupAlias = popup; popup.__openerImmediateMutation = 'old realm'; popup.document.body.dataset.openerMutation = 'old document'; return popup !== null; }})()"
                ),
            )
            .await;
            assert_eq!(
                response(&messages, 260_224)["result"]["result"]["value"],
                true
            );
            let popup = event(&messages, "Target.targetCreated");
            let popup_target_id = popup["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("popup target id")
                .to_owned();
            assert_eq!(popup["params"]["targetInfo"]["url"], unreachable_url);
            assert_eq!(
                popup["params"]["targetInfo"]["openerId"],
                "TID-popup-network-error-opener"
            );
            assert_eq!(popup["params"]["targetInfo"]["canAccessOpener"], true);
            let attached = event(&messages, "Target.attachedToTarget");
            let popup_session_id = attached["params"]["sessionId"]
                .as_str()
                .expect("popup session id")
                .to_owned();

            for (id, method, params) in [
                (260_225, "Page.enable", json!({})),
                (260_226, "Network.enable", json!({})),
                (260_227, "Runtime.enable", json!({})),
                (
                    260_228,
                    "Page.setLifecycleEventsEnabled",
                    json!({ "enabled": true }),
                ),
            ] {
                ctx.process_async(json!({
                    "id": id,
                    "method": method,
                    "sessionId": popup_session_id,
                    "params": params
                }))
                .await;
                ctx.expect_result(id, json!({}), Some(&popup_session_id));
            }

            ctx.process_async(json!({
                "id": 260_229,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ href: location.href, marker: __openerImmediateMutation, bodyMarker: document.body.dataset.openerMutation, historyLength: history.length })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_229)["result"]["result"]["value"],
                json!({
                    "href": "about:blank",
                    "marker": "old realm",
                    "bodyMarker": "old document",
                    "historyLength": 1
                })
            );

            let before_target_page = ctx
                .conn
                .target_page_residence_identity_for_session(Some(&popup_session_id))
                .expect("popup target Page residence");
            let before_renderer_page = ctx
                .conn
                .renderer_page_residence_identity_for_session_owner(Some(&popup_session_id))
                .expect("popup renderer Page residence");
            let before_renderer_attachment = ctx
                .conn
                .current_renderer_agent_attachment_id_for_session_owner(Some(&popup_session_id))
                .expect("popup renderer attachment");
            ctx.sent.clear();

            ctx.process_async(json!({
                "id": 260_230,
                "method": "Runtime.runIfWaitingForDebugger",
                "sessionId": popup_session_id
            }))
            .await;
            take_response_by_id(&mut ctx, 260_230);
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "popup network error Document load",
                |message| {
                    message["method"] == json!("Page.loadEventFired")
                        && message["sessionId"] == json!(popup_session_id)
                },
            )
            .await;
            ctx.wait_until_scheduler_state("popup network error commit", |conn| {
                !conn.has_pending_document_navigation_for_session_owner(Some(&popup_session_id))
                    && conn
                        .browser_context_by_id("BID-popup-network-error")
                        .and_then(|browser_context| {
                            loaded_page_for_target(browser_context, &popup_target_id)
                        })
                        .is_some_and(|page| page.final_url().as_str() == NETWORK_ERROR_PAGE_URL)
            })
            .await;

            let request_index = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Network.requestWillBeSent")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["request"]["url"] == json!(unreachable_url)
                })
                .unwrap_or_else(|| panic!("missing popup request: {:?}", ctx.sent));
            let request_id = ctx.sent[request_index]["params"]["requestId"]
                .as_str()
                .expect("popup request id")
                .to_owned();
            let failed_index = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Network.loadingFailed")
                        && message["params"]["requestId"] == json!(request_id)
                })
                .unwrap_or_else(|| panic!("missing popup loadingFailed: {:?}", ctx.sent));
            let frame_index = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Page.frameNavigated")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["frame"]["url"]
                            == json!(NETWORK_ERROR_PAGE_URL)
                })
                .unwrap_or_else(|| panic!("missing popup error frame: {:?}", ctx.sent));
            let finished_index = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Network.loadingFinished")
                        && message["params"]["requestId"] == json!(request_id)
                })
                .unwrap_or_else(|| panic!("missing popup loadingFinished: {:?}", ctx.sent));
            let dom_content_loaded_index = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Page.domContentEventFired")
                        && message["sessionId"] == json!(popup_session_id)
                })
                .expect("popup error Document DCL");
            assert!(
                request_index < failed_index
                    && failed_index < frame_index
                    && frame_index < finished_index
                    && finished_index < dom_content_loaded_index,
                "popup error-page event order should match Chromium: {:?}",
                ctx.sent
            );
            assert_eq!(
                ctx.sent[frame_index]["params"]["frame"]["unreachableUrl"],
                unreachable_url
            );
            assert_eq!(ctx.sent[failed_index]["params"]["canceled"], false);
            assert!(!ctx.sent.iter().any(|message| {
                message["method"] == json!("Network.responseReceived")
                    && message["params"]["requestId"] == json!(request_id)
            }));

            ctx.process_async(json!({
                "id": 260_231,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ href: location.href, origin: location.origin, oldMarkerType: typeof __openerImmediateMutation, oldBodyMarkerType: typeof document.body.dataset.openerMutation, title: document.title, readyState: document.readyState, historyLength: history.length, hasOpener: opener !== null })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_231)["result"]["result"]["value"],
                json!({
                    "href": NETWORK_ERROR_PAGE_URL,
                    "origin": "null",
                    "oldMarkerType": "undefined",
                    "oldBodyMarkerType": "undefined",
                    "title": "127.0.0.1",
                    "readyState": "complete",
                    "historyLength": 1,
                    "hasOpener": true
                })
            );

            ctx.process_async(json!({
                "id": 260_232,
                "method": "Page.getNavigationHistory",
                "sessionId": popup_session_id
            }))
            .await;
            let history = take_response_by_id(&mut ctx, 260_232);
            assert_eq!(history["result"]["currentIndex"], json!(0));
            assert_eq!(
                history["result"]["entries"]
                    .as_array()
                    .expect("popup error history entries")
                    .len(),
                1
            );
            assert_eq!(history["result"]["entries"][0]["url"], unreachable_url);

            ctx.process_async(json!({
                "id": 260_233,
                "method": "Target.getTargetInfo",
                "params": { "targetId": popup_target_id }
            }))
            .await;
            let target_info = take_response_by_id(&mut ctx, 260_233);
            assert_eq!(target_info["result"]["targetInfo"]["url"], unreachable_url);
            assert_eq!(
                target_info["result"]["targetInfo"]["openerId"],
                "TID-popup-network-error-opener"
            );
            assert_eq!(
                target_info["result"]["targetInfo"]["canAccessOpener"],
                true
            );

            assert_eq!(
                ctx.conn
                    .target_page_residence_identity_for_session(Some(&popup_session_id)),
                Some(before_target_page.clone()),
                "popup error navigation should keep its target Page residence"
            );
            assert_eq!(
                ctx.conn
                    .renderer_page_residence_identity_for_session_owner(Some(&popup_session_id)),
                Some(before_renderer_page),
                "popup error navigation should keep its renderer Page/WindowProxy residence"
            );
            assert_ne!(
                ctx.conn
                    .current_renderer_agent_attachment_id_for_session_owner(Some(
                        &popup_session_id,
                    )),
                Some(before_renderer_attachment),
                "popup error navigation should replace only the renderer Document/realm"
            );

            ctx.process_async(json!({
                "id": 260_234,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": r#"(() => {
  const popup = __networkErrorPopup;
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? "undefined" : String(value);
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };
  const dataDescriptor = descriptor => [
    typeof descriptor.value,
    descriptor.value.name,
    descriptor.value.length,
    descriptor.writable,
    descriptor.enumerable,
    descriptor.configurable
  ];
  const locationDescriptor = Object.getOwnPropertyDescriptor(popup, "location");
  const symbolDescriptor = Object.getOwnPropertyDescriptor(popup, Symbol.toStringTag);
  return {
    retainedWindowProxy: popup !== null && popup === __networkErrorPopupAlias,
    identity: [
      popup.window === popup,
      popup.self === popup,
      popup.frames === popup,
      popup.parent === popup,
      popup.top === popup,
      popup.opener === window,
      popup.location === popup.location
    ],
    scalar: [popup.closed, popup.length, popup.then],
    methods: [
      dataDescriptor(Object.getOwnPropertyDescriptor(popup, "postMessage")),
      dataDescriptor(Object.getOwnPropertyDescriptor(popup, "blur")),
      dataDescriptor(Object.getOwnPropertyDescriptor(popup, "close")),
      dataDescriptor(Object.getOwnPropertyDescriptor(popup, "focus"))
    ],
    stableMethods: [
      popup.postMessage === popup.postMessage,
      popup.blur === popup.blur,
      popup.close === popup.close,
      popup.focus === popup.focus
    ],
    locationDescriptor: [
      typeof locationDescriptor.get,
      typeof locationDescriptor.set,
      locationDescriptor.enumerable,
      locationDescriptor.configurable
    ],
    symbolDescriptor: [
      symbolDescriptor.value === undefined,
      symbolDescriptor.writable,
      symbolDescriptor.enumerable,
      symbolDescriptor.configurable
    ],
    names: Object.getOwnPropertyNames(popup).sort(),
    keys: Object.keys(popup),
    symbols: Object.getOwnPropertySymbols(popup).map(String),
    symbolValues: [
      popup[Symbol.toStringTag],
      popup[Symbol.hasInstance],
      popup[Symbol.isConcatSpreadable]
    ].map(value => value === undefined),
    prototype: Object.getPrototypeOf(popup) === null,
    tag: Object.prototype.toString.call(popup),
    locationSurface: [
      Object.getPrototypeOf(popup.location) === null,
      Object.prototype.toString.call(popup.location),
      typeof popup.location.replace
    ],
    denied: [
      probe(() => popup.document),
      probe(() => popup.name),
      probe(() => popup.globalThis),
      probe(() => popup.__not_exposed),
      probe(() => Object.getOwnPropertyDescriptor(popup, "document")),
      probe(() => Object.prototype.hasOwnProperty.call(popup, "document")),
      probe(() => popup.location.href),
      probe(() => popup.location.origin)
    ]
  };
})()"#,
                    "returnByValue": true
                }
            }))
            .await;
            let opener_probe = take_response_by_id(&mut ctx, 260_234);
            assert_eq!(
                opener_probe["result"]["result"]["value"],
                json!({
                    "retainedWindowProxy": true,
                    "identity": [true, true, true, true, true, true, true],
                    "scalar": [false, 0, null],
                    "methods": [
                        ["function", "postMessage", 1, false, false, true],
                        ["function", "blur", 0, false, false, true],
                        ["function", "close", 0, false, false, true],
                        ["function", "focus", 0, false, false, true]
                    ],
                    "stableMethods": [true, true, true, true],
                    "locationDescriptor": ["function", "function", false, true],
                    "symbolDescriptor": [true, false, false, true],
                    "names": [
                        "blur", "close", "closed", "focus", "frames", "length",
                        "location", "opener", "parent", "postMessage", "self", "then",
                        "top", "window"
                    ],
                    "keys": [],
                    "symbols": [
                        "Symbol(Symbol.toStringTag)",
                        "Symbol(Symbol.hasInstance)",
                        "Symbol(Symbol.isConcatSpreadable)"
                    ],
                    "symbolValues": [true, true, true],
                    "prototype": true,
                    "tag": "[object Object]",
                    "locationSurface": [true, "[object Object]", "function"],
                    "denied": [
                        "SecurityError:true", "SecurityError:true", "SecurityError:true",
                        "SecurityError:true", "SecurityError:true", "SecurityError:true",
                        "SecurityError:true", "SecurityError:true"
                    ]
                }),
                "opener-side stable popup WindowProxy probe failed: {opener_probe:?}"
            );

            ctx.process_async(json!({
                "id": 260_235,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": r#"(() => {
  globalThis.__relatedPopupMessage = null;
  addEventListener("message", event => {
    globalThis.__relatedPopupMessage = {
      data: event.data,
      origin: event.origin,
      sourceIsOpener: event.source === opener
    };
    console.log("__related-popup-message", JSON.stringify(__relatedPopupMessage));
  }, { once: true });
  return "ready";
})()"#,
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_235)["result"]["result"]["value"],
                "ready"
            );
            ctx.sent.clear();

            ctx.process_async(json!({
                "id": 260_236,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": "__networkErrorPopup.postMessage({ kind: 'related-page', value: 41 }, '*'); 'queued'",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_236)["result"]["result"]["value"],
                "queued"
            );
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "related popup Window.postMessage delivery",
                |message| {
                    message["method"] == json!("Runtime.consoleAPICalled")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["args"][0]["value"]
                            == json!("__related-popup-message")
                },
            )
            .await;

            ctx.process_async(json!({
                "id": 260_237,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "__relatedPopupMessage",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_237)["result"]["result"]["value"],
                json!({
                    "data": { "kind": "related-page", "value": 41 },
                    "origin": "null",
                    "sourceIsOpener": true
                })
            );

            let related_location_url =
                "data:text/html,<title>related-popup-location</title><main>location-routed</main>";
            ctx.sent.clear();
            ctx.process_async(json!({
                "id": 260_238,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": format!(
                        "__networkErrorPopup.location = {related_location_url:?}; 'navigating'"
                    ),
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_238)["result"]["result"]["value"],
                "navigating"
            );
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "cross-origin popup.location load",
                |message| {
                    message["method"] == json!("Page.loadEventFired")
                        && message["sessionId"] == json!(popup_session_id)
                },
            )
            .await;
            ctx.wait_until_scheduler_state("cross-origin popup.location commit", |conn| {
                !conn.has_pending_document_navigation_for_session_owner(Some(&popup_session_id))
                    && conn
                        .browser_context_by_id("BID-popup-network-error")
                        .and_then(|browser_context| {
                            loaded_page_for_target(browser_context, &popup_target_id)
                        })
                        .is_some_and(|page| page.final_url().as_str() == related_location_url)
            })
            .await;

            assert_eq!(
                ctx.conn
                    .target_page_residence_identity_for_session(Some(&popup_session_id)),
                Some(before_target_page),
                "cross-origin popup.location should preserve the target Page residence"
            );
            assert_eq!(
                ctx.conn
                    .renderer_page_residence_identity_for_session_owner(Some(&popup_session_id)),
                Some(before_renderer_page),
                "cross-origin popup.location should preserve the stable renderer Page/WindowProxy"
            );

            ctx.process_async(json!({
                "id": 260_239,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ title: document.title, text: document.querySelector('main').textContent, hasOpener: opener !== null })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_239)["result"]["result"]["value"],
                json!({
                    "title": "related-popup-location",
                    "text": "location-routed",
                    "hasOpener": true
                })
            );

            let related_replace_url =
                "data:text/html,<title>related-popup-replace</title><main>replace-routed</main>";
            ctx.sent.clear();
            ctx.process_async(json!({
                "id": 260_240,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": format!(
                        "__networkErrorPopup.location.replace({related_replace_url:?}); 'replacing'"
                    ),
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_240)["result"]["result"]["value"],
                "replacing"
            );
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "cross-origin popup.location.replace load",
                |message| {
                    message["method"] == json!("Page.loadEventFired")
                        && message["sessionId"] == json!(popup_session_id)
                },
            )
            .await;
            ctx.wait_until_scheduler_state("cross-origin popup.location.replace commit", |conn| {
                !conn.has_pending_document_navigation_for_session_owner(Some(&popup_session_id))
                    && conn
                        .browser_context_by_id("BID-popup-network-error")
                        .and_then(|browser_context| {
                            loaded_page_for_target(browser_context, &popup_target_id)
                        })
                        .is_some_and(|page| page.final_url().as_str() == related_replace_url)
            })
            .await;

            ctx.process_async(json!({
                "id": 260_241,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ title: document.title, text: document.querySelector('main').textContent, hasOpener: opener !== null })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_241)["result"]["result"]["value"],
                json!({
                    "title": "related-popup-replace",
                    "text": "replace-routed",
                    "hasOpener": true
                })
            );

            ctx.process_async(json!({
                "id": 260_242,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": "__networkErrorPopup === __networkErrorPopupAlias && typeof __networkErrorPopup.postMessage === 'function'",
                    "returnByValue": true
                }
            }))
            .await;
            let post_navigation_opener_probe = take_response_by_id(&mut ctx, 260_242);
            assert_eq!(
                post_navigation_opener_probe["result"]["result"]["value"],
                true,
                "opener should retain the same cross-origin WindowProxy after location navigation: {post_navigation_opener_probe:?}"
            );
        })
        .await;
    failing_server.abort();
}

// Chromium source:
// content/browser/security/coop/cross_origin_opener_policy_status.cc
// A COOP mismatch observed on a redirect remains authoritative when the next
// transport fails and Chromium commits its browser-owned error Document.
#[tokio::test(flavor = "multi_thread")]
async fn popup_coop_redirect_then_transport_error_still_severs_old_group_proxy() {
    let fixture = SmokeFixtureServer::start().await;
    let (failing_addr, failing_server) = spawn_connection_drop_server().await;
    let mut redirect_url =
        url::Url::parse(&fixture.url("/coop-redirect-to")).expect("valid redirect fixture URL");
    redirect_url
        .query_pairs_mut()
        .append_pair("url", &format!("http://{failing_addr}/after-coop-redirect"));
    let redirect_url = redirect_url.to_string();
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-coop-error",
                "TID-popup-coop-error-opener",
                "<main>COOP error opener</main>",
            )
            .await;
            set_auto_attach_waiting_for_debugger(&mut ctx, 2_602_800).await;
            ctx.take_all();
            let opener_session_id = "SID-popup-coop-error-opener";
            ctx.conn
                .browser_context
                .as_mut()
                .expect("COOP error opener browser context")
                .attach_active_session(opener_session_id);

            let messages = open_popup_from_runtime(
                &mut ctx,
                2_602_801,
                &format!(
                    "(() => {{ const popup = window.open({redirect_url:?}, '_blank'); globalThis.__lmCoopErrorPopup = popup; return popup !== null && popup.closed === false; }})()"
                ),
            )
            .await;
            assert_eq!(
                response(&messages, 2_602_801)["result"]["result"]["value"],
                true
            );
            let popup_target_id = event(&messages, "Target.targetCreated")["params"]
                ["targetInfo"]["targetId"]
                .as_str()
                .expect("COOP error popup target id")
                .to_owned();
            let popup_session_id = event(&messages, "Target.attachedToTarget")["params"]
                ["sessionId"]
                .as_str()
                .expect("COOP error popup session id")
                .to_owned();

            for (id, method) in [(2_602_802, "Page.enable"), (2_602_803, "Runtime.enable")] {
                ctx.process_async(json!({
                    "id": id,
                    "method": method,
                    "sessionId": popup_session_id,
                    "params": {}
                }))
                .await;
                ctx.expect_result(id, json!({}), Some(&popup_session_id));
            }
            ctx.take_all();
            ctx.process_async(json!({
                "id": 2_602_804,
                "method": "Runtime.runIfWaitingForDebugger",
                "sessionId": popup_session_id
            }))
            .await;
            take_response_by_id(&mut ctx, 2_602_804);
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "COOP redirect transport-error load",
                |message| {
                    message["method"] == json!("Page.loadEventFired")
                        && message["sessionId"] == json!(popup_session_id)
                },
            )
            .await;
            ctx.wait_until_scheduler_state("COOP redirect error commit", |conn| {
                !conn.has_pending_document_navigation_for_session_owner(Some(&popup_session_id))
                    && conn
                        .browser_context_by_id("BID-popup-coop-error")
                        .and_then(|browser_context| {
                            loaded_page_for_target(browser_context, &popup_target_id)
                        })
                        .is_some_and(|page| page.final_url().as_str() == NETWORK_ERROR_PAGE_URL)
            })
            .await;

            ctx.process_async(json!({
                "id": 2_602_805,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ openerSevered: opener === null, href: location.href, closed })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 2_602_805)["result"]["result"]["value"],
                json!({
                    "openerSevered": true,
                    "href": NETWORK_ERROR_PAGE_URL,
                    "closed": false
                })
            );
            ctx.process_async(json!({
                "id": 2_602_806,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id,
                "params": {
                    "expression": "({ popupClosed: __lmCoopErrorPopup.closed, openerClosed: closed })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 2_602_806)["result"]["result"]["value"],
                json!({ "popupClosed": true, "openerClosed": false })
            );
        })
        .await;
    failing_server.abort();
}

// Chromium/WPT sources:
// third_party/blink/web_tests/external/wpt/html/browsers/browsing-the-web/
// navigating-across-documents/initial-empty-document/window-open-204-fragment.html
// navigating-across-documents/initial-empty-document/
// window-open-204-pushState-replaceState.html
// third_party/blink/web_tests/http/tests/inspector-protocol/page/navigate-204.js
// third_party/blink/web_tests/http/tests/inspector-protocol/network/
// navigation-204-loading-failed.js
#[tokio::test(flavor = "multi_thread")]
async fn popup_no_commit_responses_preserve_initial_document_before_redirect_replacement() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.enable_background_navigation_scheduler_for_test();
    tokio::task::LocalSet::new()
        .run_until(async {
            load_bc_with_titled_page_async(
                &mut ctx,
                "BID-popup-terminal",
                "TID-popup-terminal-opener",
                "<main>opener</main>",
            )
            .await;
            set_auto_attach_waiting_for_debugger(&mut ctx, 260_230).await;
            ctx.take_all();

            let no_content_url = fixture.url("/no-content");
            let messages = open_popup_from_runtime(
                &mut ctx,
                260_231,
                &format!(
                    "(() => {{ const popup = window.open('{no_content_url}', '_blank'); popup.__openerImmediateMutation = 'kept'; popup.document.body.dataset.openerMutation = 'kept'; return popup !== null; }})()"
                ),
            )
            .await;
            assert_eq!(
                response(&messages, 260_231)["result"]["result"]["value"],
                true
            );
            let popup = event(&messages, "Target.targetCreated");
            let popup_target_id = popup["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("popup target id")
                .to_owned();
            let attached = event(&messages, "Target.attachedToTarget");
            let popup_session_id = attached["params"]["sessionId"]
                .as_str()
                .expect("popup session id")
                .to_owned();

            for (id, method, params) in [
                (260_232, "Page.enable", json!({})),
                (260_233, "Network.enable", json!({})),
                (
                    260_234,
                    "Page.setLifecycleEventsEnabled",
                    json!({ "enabled": true }),
                ),
            ] {
                ctx.process_async(json!({
                    "id": id,
                    "method": method,
                    "sessionId": popup_session_id,
                    "params": params
                }))
                .await;
                ctx.expect_result(id, json!({}), Some(&popup_session_id));
            }

            ctx.process_async(json!({
                "id": 260_235,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ href: location.href, globalMarker: __openerImmediateMutation, bodyMarker: document.body.dataset.openerMutation, historyLength: history.length })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_235)["result"]["result"]["value"],
                json!({
                    "href": "about:blank",
                    "globalMarker": "kept",
                    "bodyMarker": "kept",
                    "historyLength": 1
                })
            );
            ctx.sent.clear();

            ctx.process_async(json!({
                "id": 260_236,
                "method": "Runtime.runIfWaitingForDebugger",
                "sessionId": popup_session_id
            }))
            .await;
            take_response_by_id(&mut ctx, 260_236);
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "popup 204 navigation abort",
                |message| {
                    message["method"] == json!("Network.loadingFailed")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["errorText"] == json!("net::ERR_ABORTED")
                },
            )
            .await;
            ctx.wait_until_scheduler_state("popup 204 terminal completion", |conn| {
                !conn.has_pending_document_navigation_for_session_owner(Some(&popup_session_id))
            })
            .await;

            let response_204_index = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Network.responseReceived")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["response"]["url"] == json!(no_content_url)
                        && message["params"]["response"]["status"] == json!(204)
                })
                .unwrap_or_else(|| panic!("missing popup 204 response: {:?}", ctx.sent));
            let request_204 = ctx.sent[response_204_index]["params"]["requestId"]
                .as_str()
                .expect("204 request id")
                .to_owned();
            let failed_204_index = ctx
                .sent
                .iter()
                .position(|message| {
                    message["method"] == json!("Network.loadingFailed")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["requestId"] == json!(request_204)
                        && message["params"]["errorText"] == json!("net::ERR_ABORTED")
                        && message["params"]["canceled"] == json!(true)
                })
                .unwrap_or_else(|| panic!("missing popup 204 loadingFailed: {:?}", ctx.sent));
            assert!(response_204_index < failed_204_index);
            assert!(!ctx.sent.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_204)
            }));
            assert!(!ctx.sent.iter().any(|message| {
                (message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(no_content_url))
                    || (message["method"] == json!("Page.lifecycleEvent")
                        && matches!(
                            message["params"]["name"].as_str(),
                            Some("DOMContentLoaded" | "load")
                        ))
            }));

            ctx.process_async(json!({
                "id": 260_237,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "location.hash = 'after-204'; const fragmentHistoryLength = history.length; history.pushState({ source: 'initial-empty' }, '', '#after-push-state'); ({ href: location.href, globalMarker: __openerImmediateMutation, bodyMarker: document.body.dataset.openerMutation, fragmentHistoryLength, historyLength: history.length, historyState: history.state.source })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_237)["result"]["result"]["value"],
                json!({
                    "href": "about:blank#after-push-state",
                    "globalMarker": "kept",
                    "bodyMarker": "kept",
                    "fragmentHistoryLength": 1,
                    "historyLength": 1,
                    "historyState": "initial-empty"
                })
            );

            ctx.process_async(json!({
                "id": 260_243,
                "method": "Page.getNavigationHistory",
                "sessionId": popup_session_id
            }))
            .await;
            let fragment_history = take_response_by_id(&mut ctx, 260_243);
            assert_eq!(fragment_history["result"]["currentIndex"], json!(0));
            assert_eq!(
                fragment_history["result"]["entries"]
                    .as_array()
                    .expect("post-204 fragment history entries")
                    .len(),
                1
            );
            assert_eq!(
                fragment_history["result"]["entries"][0]["url"],
                "about:blank#after-push-state"
            );

            ctx.sent.clear();
            let reset_content_url = fixture.url("/reset-content");
            ctx.process_async(json!({
                "id": 260_238,
                "method": "Page.navigate",
                "sessionId": popup_session_id,
                "params": { "url": reset_content_url }
            }))
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "popup 205 Page.navigate result",
                |message| message["id"] == json!(260_238),
            )
            .await;
            let reset_result = take_response_by_id(&mut ctx, 260_238);
            assert_eq!(reset_result["result"]["frameId"], popup_target_id);
            assert!(reset_result["result"]["loaderId"].as_str().is_some());
            assert_eq!(
                reset_result["result"]["errorText"],
                json!("net::ERR_ABORTED")
            );
            assert_eq!(reset_result["result"]["isDownload"], json!(false));
            let response_205 = ctx
                .sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Network.responseReceived")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["response"]["url"] == json!(reset_content_url)
                        && message["params"]["response"]["status"] == json!(205)
                })
                .unwrap_or_else(|| panic!("missing popup 205 response: {:?}", ctx.sent));
            let request_205 = response_205["params"]["requestId"]
                .as_str()
                .expect("205 request id");
            assert!(ctx.sent.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(request_205)
                    && message["params"]["errorText"] == json!("net::ERR_ABORTED")
                    && message["params"]["canceled"] == json!(true)
            }));

            ctx.process_async(json!({
                "id": 260_239,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ href: location.href, marker: __openerImmediateMutation, historyLength: history.length })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_239)["result"]["result"]["value"],
                json!({
                    "href": "about:blank#after-push-state",
                    "marker": "kept",
                    "historyLength": 1
                })
            );

            ctx.sent.clear();
            let redirect_start_url = fixture.url("/redirect-start");
            let redirect_final_url = fixture.url("/redirect-final");
            ctx.process_async(json!({
                "id": 260_240,
                "method": "Page.navigate",
                "sessionId": popup_session_id,
                "params": { "url": redirect_start_url }
            }))
            .await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "popup redirect Page.navigate result",
                |message| message["id"] == json!(260_240),
            )
            .await;
            let redirect_result = take_response_by_id(&mut ctx, 260_240);
            let redirect_loader_id = redirect_result["result"]["loaderId"]
                .as_str()
                .expect("redirect loader id")
                .to_owned();
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "popup redirect load lifecycle",
                |message| {
                    message["method"] == json!("Page.lifecycleEvent")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["frameId"] == json!(popup_target_id)
                        && message["params"]["loaderId"] == json!(redirect_loader_id)
                        && message["params"]["name"] == json!("load")
                },
            )
            .await;
            ctx.wait_until_scheduler_state("popup redirect authoritative load", |conn| {
                conn.renderer_document_lifecycle_authoritative_state_for_session_owner(Some(
                    &popup_session_id,
                ))
                .is_some_and(|(binding, snapshot)| {
                    binding.loader_id == redirect_loader_id && snapshot.load.is_some()
                })
            })
            .await;

            let document_requests = ctx
                .sent
                .iter()
                .filter(|message| {
                    message["method"] == json!("Network.requestWillBeSent")
                        && message["sessionId"] == json!(popup_session_id)
                        && message["params"]["type"] == json!("Document")
                })
                .collect::<Vec<_>>();
            assert_eq!(
                document_requests
                    .iter()
                    .filter(|message| {
                        message["params"]["request"]["url"] == json!(redirect_start_url)
                    })
                    .count(),
                1,
                "redirect start must have one authoritative hop: {:?}",
                ctx.sent
            );
            assert_eq!(
                document_requests
                    .iter()
                    .filter(|message| {
                        message["params"]["request"]["url"] == json!(redirect_final_url)
                    })
                    .count(),
                1,
                "redirect final must have one authoritative hop: {:?}",
                ctx.sent
            );
            let start_request = document_requests
                .iter()
                .find(|message| {
                    message["params"]["request"]["url"] == json!(redirect_start_url)
                })
                .expect("redirect start request");
            let final_request = document_requests
                .iter()
                .find(|message| {
                    message["params"]["request"]["url"] == json!(redirect_final_url)
                })
                .expect("redirect final request");
            assert_eq!(
                start_request["params"]["requestId"],
                final_request["params"]["requestId"]
            );
            assert_eq!(
                final_request["params"]["redirectResponse"]["url"],
                redirect_start_url
            );
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|message| {
                        message["method"] == json!("Page.frameNavigated")
                            && message["sessionId"] == json!(popup_session_id)
                            && message["params"]["frame"]["loaderId"]
                                == json!(redirect_loader_id)
                            && message["params"]["frame"]["url"] == json!(redirect_final_url)
                    })
                    .count(),
                1
            );
            for lifecycle_name in ["DOMContentLoaded", "load"] {
                assert_eq!(
                    ctx.sent
                        .iter()
                        .filter(|message| {
                            message["method"] == json!("Page.lifecycleEvent")
                                && message["sessionId"] == json!(popup_session_id)
                                && message["params"]["loaderId"] == json!(redirect_loader_id)
                                && message["params"]["name"] == json!(lifecycle_name)
                        })
                        .count(),
                    1,
                    "{lifecycle_name} must be published once: {:?}",
                    ctx.sent
                );
            }

            ctx.process_async(json!({
                "id": 260_241,
                "method": "Runtime.evaluate",
                "sessionId": popup_session_id,
                "params": {
                    "expression": "({ href: location.href, text: document.querySelector('main').textContent, historyLength: history.length, readyState: document.readyState, oldMarkerType: typeof __openerImmediateMutation })",
                    "returnByValue": true
                }
            }))
            .await;
            assert_eq!(
                take_response_by_id(&mut ctx, 260_241)["result"]["result"]["value"],
                json!({
                    "href": redirect_final_url,
                    "text": "redirect final",
                    "historyLength": 1,
                    "readyState": "complete",
                    "oldMarkerType": "undefined"
                })
            );

            ctx.process_async(json!({
                "id": 260_242,
                "method": "Page.getNavigationHistory",
                "sessionId": popup_session_id
            }))
            .await;
            let history = take_response_by_id(&mut ctx, 260_242);
            assert_eq!(history["result"]["currentIndex"], json!(0));
            assert_eq!(
                history["result"]["entries"]
                    .as_array()
                    .expect("popup history entries")
                    .len(),
                1
            );
            assert_eq!(history["result"]["entries"][0]["url"], redirect_final_url);
        })
        .await;
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

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_popup_navigation_then_javascript_url_preserves_renderer_protocol_order() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    tokio::task::LocalSet::new()
        .run_until(async {
            enable_root_target_discovery_for_test(&mut ctx);
            let opener_url = fixture.url("/plain");
            let opener_target_id = create_target(&mut ctx, 2_600_240, None, &opener_url).await;
            let opener_session_id =
                attach_to_target(&mut ctx, 2_600_240, None, &opener_target_id).await;
            ctx.take_all();
            ctx.enable_background_navigation_scheduler_for_test();
            let ordinary_url = fixture.url("/history-b");
            let messages = open_popup_from_runtime(
                &mut ctx,
                2_600_241,
                &format!(
                    r#"(() => {{
  globalThis.__ordinaryThenJavascript = new Promise(resolve => {{
    globalThis.__resolveOrdinaryThenJavascript = resolve;
  }});
  const first = window.open('{ordinary_url}', 'phase5e-order');
  const second = window.open(
    'javascript:opener.__resolveOrdinaryThenJavascript("ran");void 0',
    'phase5e-order'
  );
  return [first !== null, first === second];
}})()"#
                ),
            )
            .await;
            assert_eq!(
                response(&messages, 2_600_241)["result"]["result"]["value"],
                json!([true, true]),
                "the second producer must reuse the synchronously selected stable WindowProxy: {messages:?}"
            );
            let popup = event(&messages, "Target.targetCreated");
            assert_eq!(popup["params"]["targetInfo"]["url"], ordinary_url);
            let popup_target_id = popup["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("ordered popup target id")
                .to_owned();
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == json!("Target.targetCreated"))
                    .count(),
                1,
                "ordinary and javascript producers must not create two targets"
            );

            ctx.process_async(json!({
                "id": 2_600_242,
                "method": "Runtime.evaluate",
                "sessionId": opener_session_id.clone(),
                "params": {
                    "expression": "globalThis.__ordinaryThenJavascript",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }))
            .await;
            crate::testing::wait_until_message(
                &mut ctx,
                Some(opener_session_id.as_str()),
                "ordinary-then-javascript opener completion",
                |message| message["id"] == json!(2_600_242),
            )
            .await;
            let completion = take_response_by_id(&mut ctx, 2_600_242);
            assert_eq!(
                completion["result"]["result"]["value"],
                json!("ran"),
                "later target task must resolve the opener promise: {completion:?}"
            );

            ctx.wait_until_scheduler_state("ordinary popup destination commit", |conn| {
                conn.browser_context
                    .as_ref()
                    .and_then(|browser_context| {
                        loaded_page_for_target(browser_context, &popup_target_id)
                    })
                    .is_some_and(|page| page.final_url().as_str() == ordinary_url.as_str())
            })
            .await;
        })
        .await;
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
        .promote_background_target_to_active_for_connection_async("TID-reset-opener")
        .await
        .expect("opener target promotion should succeed")
        .expect("foreground popup should have demoted its opener");

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
