use super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::*;
use crate::domains::page::LOADER_ID;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

async fn attached_smoke_session(ctx: &mut TestContext, base: u64) -> AttachedPageSession {
    create_attached_page_session_async(ctx, base, base + 1, base + 2, base + 3, base + 4).await
}

fn paused_request_id(ctx: &mut TestContext, resource_type: &str) -> String {
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!(resource_type)
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing {resource_type} requestPaused: {:?}", ctx.sent));
    paused["params"]["requestId"]
        .as_str()
        .expect("paused request id")
        .to_owned()
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

async fn open_popup_from_session(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    url: &str,
) -> (String, String, String) {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": format!("window.open('{url}', '_blank') !== null"),
            "returnByValue": true
        }
    }))
    .await;
    let evaluated = take_response_by_id(ctx, id);
    assert_eq!(evaluated["result"]["result"]["value"], true);
    let created = ctx.take_first_matching("popup Target.targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["openerId"].is_string()
    });
    let target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let browser_context_id = created["params"]["targetInfo"]["browserContextId"]
        .as_str()
        .expect("popup browser context id")
        .to_owned();
    let attached = ctx.take_first_matching("popup Target.attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    });
    let popup_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup session id")
        .to_owned();
    (target_id, popup_session_id, browser_context_id)
}

async fn arm_popup_route(
    ctx: &mut TestContext,
    base: u64,
    popup_target_id: &str,
    popup_session_id: &str,
    popup_url: &str,
) {
    assert!(
        ctx.conn
            .target_has_waiting_for_debugger_session(popup_target_id),
        "the auto-attached popup session must own the debugger gate",
    );
    let initial_url = ctx
        .conn
        .browser_contexts()
        .find_map(|browser_context| {
            loaded_page_for_target(browser_context, popup_target_id)
                .map(|page| page.final_url().to_string())
        })
        .expect("popup initial document");
    assert_eq!(
        initial_url, "about:blank",
        "the popup target URL must remain gated until debugger resume",
    );
    ctx.process_async(json!({
        "id": base,
        "method": "Page.enable",
        "sessionId": popup_session_id
    }))
    .await;
    ctx.expect_result(base, json!({}), Some(popup_session_id));

    ctx.process_async(json!({
        "id": base + 1,
        "method": "Network.enable",
        "sessionId": popup_session_id
    }))
    .await;
    ctx.expect_result(base + 1, json!({}), Some(popup_session_id));

    ctx.process_async(json!({
        "id": base + 2,
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
    ctx.expect_result(base + 2, json!({}), Some(popup_session_id));
    let fetch_snapshot = ctx
        .conn
        .target_fetch_subresource_interception_snapshot_for_owner(
            &crate::conn::CommandOwnerScope::for_session(popup_session_id),
        )
        .expect("popup target Fetch configuration");
    let matching_sessions = fetch_snapshot.matching_request_stage_pause_sessions(
        Some(popup_session_id),
        crate::devtools_runtime::DevToolsNetworkResourceType::Document,
        &url::Url::parse(popup_url).expect("popup URL"),
    );
    assert_eq!(
        matching_sessions
            .iter()
            .map(|session| session.session_id.as_deref())
            .collect::<Vec<_>>(),
        [Some(popup_session_id)],
        "Fetch.enable must commit the document pattern to the popup target before resume",
    );
    ctx.process_async(json!({
        "id": base + 3,
        "method": "Runtime.runIfWaitingForDebugger",
        "sessionId": popup_session_id
    }))
    .await;
    ctx.expect_result(base + 3, json!({}), Some(popup_session_id));
    assert!(
        !ctx.conn
            .target_has_waiting_for_debugger_session(popup_target_id),
        "runIfWaitingForDebugger must release the popup session's debugger barrier",
    );

    ctx.process_async(json!({
        "id": base + 4,
        "method": "Page.createIsolatedWorld",
        "sessionId": popup_session_id,
        "params": {
            "frameId": popup_target_id,
            "worldName": "__playwright_utility_world_page",
            "grantUniveralAccess": true
        }
    }))
    .await;
    let isolated = take_response_by_id(ctx, base + 4);
    assert!(
        isolated["result"]["executionContextId"].as_i64().is_some(),
        "popup utility world should be created before fulfilling initial document: {isolated:?}"
    );
}

async fn fulfill_popup_document_and_evaluate(
    ctx: &mut TestContext,
    base: u64,
    popup_target_id: &str,
    popup_session_id: &str,
    popup_url: &str,
    expected_text: &str,
) {
    let fetch_snapshot = ctx
        .conn
        .target_fetch_subresource_interception_snapshot_for_target(popup_target_id)
        .expect("popup target Fetch configuration after debugger resume");
    let matching_sessions = fetch_snapshot.matching_request_stage_pause_sessions(
        Some(popup_session_id),
        crate::devtools_runtime::DevToolsNetworkResourceType::Document,
        &url::Url::parse(popup_url).expect("popup URL"),
    );
    assert_eq!(
        matching_sessions
            .iter()
            .map(|session| session.session_id.as_deref())
            .collect::<Vec<_>>(),
        [Some(popup_session_id)],
        "popup activation and debugger resume must preserve target-owned Fetch configuration",
    );
    crate::testing::wait_until_scheduler_message(
        ctx,
        "debugger-resumed popup document request",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!(popup_session_id)
                && message["params"]["resourceType"] == json!("Document")
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!(popup_session_id)
                && message["params"]["resourceType"] == json!("Document")
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "missing popup document pause for session {popup_session_id}: {:?}",
                ctx.sent
            )
        });
    assert_eq!(paused["params"]["request"]["url"], popup_url);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(popup_session_id)
                && message["params"]["frameId"] == json!(popup_target_id)
                && message["params"]["request"]["url"] == json!(popup_url)
        }),
        "popup initial document Network event should stay on popup session: {:?}",
        ctx.sent
    );
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("paused request id")
        .to_owned();

    ctx.process_async(json!({
        "id": base,
        "method": "Fetch.fulfillRequest",
        "sessionId": popup_session_id,
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/html; charset=utf-8" }
            ],
            "body": BASE64_STANDARD.encode(format!("<!doctype html><main>{expected_text}</main>"))
        }
    }))
    .await;
    ctx.expect_result(base, json!({}), Some(popup_session_id));

    ctx.process_async(json!({
        "id": base + 1,
        "method": "Runtime.evaluate",
        "sessionId": popup_session_id,
        "params": {
            "expression": "document.querySelector('main').textContent",
            "returnByValue": true
        }
    }))
    .await;
    let evaluated = take_response_by_id(ctx, base + 1);
    assert_eq!(evaluated["result"]["result"]["value"], expected_text);
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_context_route_metadata_underlying_fetch_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 81_000).await;

    ctx.process_async(json!({
        "id": 81_005,
        "method": "Fetch.enable",
        "sessionId": attached.session_id,
        "params": {
            "patterns": [{ "urlPattern": "*plain*", "requestStage": "Request" }]
        }
    }))
    .await;
    ctx.expect_result(81_005, json!({}), Some(&attached.session_id));

    ctx.process_async(json!({
        "id": 81_006,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": { "url": fixture.url("/plain") }
    }))
    .await;

    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Fetch.requestPaused"))
        .cloned()
        .expect("requestPaused");
    assert_eq!(paused["sessionId"], attached.session_id);
    assert_eq!(paused["params"]["resourceType"], "Document");
    assert_eq!(paused["params"]["request"]["method"], "GET");
    assert!(
        paused["params"]["request"]["headers"]["User-Agent"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "{paused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_route_fulfill_underlying_navigation_response_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 82_000).await;

    ctx.process_async(json!({
        "id": 82_005,
        "method": "Fetch.enable",
        "sessionId": attached.session_id,
        "params": {
            "patterns": [{ "urlPattern": "*playwright-route-times*", "requestStage": "Request" }]
        }
    }))
    .await;
    ctx.expect_result(82_005, json!({}), Some(&attached.session_id));

    ctx.process_async(json!({
        "id": 82_006,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": { "url": fixture.url("/playwright-route-times") }
    }))
    .await;
    let request_id = paused_request_id(&mut ctx, "Document");
    ctx.process_async(json!({
        "id": 82_007,
        "method": "Fetch.fulfillRequest",
        "sessionId": attached.session_id,
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/html; charset=utf-8" },
                { "name": "foo", "value": "bar" }
            ],
            "body": BASE64_STANDARD.encode("<!doctype html><main>intercepted</main>")
        }
    }))
    .await;
    ctx.expect_result(82_007, json!({}), Some(&attached.session_id));
    let navigation = take_response_by_id(&mut ctx, 82_006);
    assert_eq!(
        navigation["result"],
        json!({ "frameId": attached.target_id, "loaderId": LOADER_ID })
    );

    ctx.process_async(json!({
        "id": 82_008,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": { "expression": "document.body.textContent.trim()", "returnByValue": true }
    }))
    .await;
    let text = take_response_by_id(&mut ctx, 82_008);
    assert_eq!(text["result"]["result"]["value"], "intercepted");
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_route_continue_underlying_document_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 83_000).await;

    ctx.process_async(json!({
        "id": 83_005,
        "method": "Fetch.enable",
        "sessionId": attached.session_id,
        "params": {
            "patterns": [{ "urlPattern": "*document-continue*", "requestStage": "Request" }]
        }
    }))
    .await;
    ctx.expect_result(83_005, json!({}), Some(&attached.session_id));

    ctx.process_async(json!({
        "id": 83_006,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": { "url": fixture.url("/document-continue") }
    }))
    .await;
    let request_id = paused_request_id(&mut ctx, "Document");
    ctx.process_async(json!({
        "id": 83_007,
        "method": "Fetch.continueRequest",
        "sessionId": attached.session_id,
        "params": {
            "requestId": request_id,
            "headers": [{ "name": "x-smoke-nav-route", "value": "continued" }]
        }
    }))
    .await;
    ctx.expect_result(83_007, json!({}), Some(&attached.session_id));
    let navigation = take_response_by_id(&mut ctx, 83_006);
    assert_eq!(navigation["result"]["frameId"], attached.target_id);

    ctx.process_async(json!({
        "id": 83_008,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": { "expression": "document.body.textContent.trim()", "returnByValue": true }
    }))
    .await;
    let text = take_response_by_id(&mut ctx, 83_008);
    assert_eq!(text["result"]["result"]["value"], "continued");
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_route_abort_underlying_navigation_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 84_000).await;

    ctx.process_async(json!({
        "id": 84_005,
        "method": "Fetch.enable",
        "sessionId": attached.session_id,
        "params": {
            "patterns": [{ "urlPattern": "*api-abort*", "requestStage": "Request" }]
        }
    }))
    .await;
    ctx.expect_result(84_005, json!({}), Some(&attached.session_id));

    ctx.process_async(json!({
        "id": 84_006,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": { "url": fixture.url("/api-abort") }
    }))
    .await;
    let request_id = paused_request_id(&mut ctx, "Document");
    ctx.process_async(json!({
        "id": 84_007,
        "method": "Fetch.failRequest",
        "sessionId": attached.session_id,
        "params": { "requestId": request_id, "errorReason": "BlockedByClient" }
    }))
    .await;
    ctx.expect_result(84_007, json!({}), Some(&attached.session_id));
    let navigation = take_response_by_id(&mut ctx, 84_006);
    assert_eq!(navigation["error"]["message"], "net::ERR_BLOCKED_BY_CLIENT");
    assert!(ctx.sent.iter().any(|message| {
        message["sessionId"] == json!(attached.session_id)
            && message["method"] == json!("Network.loadingFailed")
            && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_CLIENT")
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_cdp_session_runtime_error_and_detach_contracts() {
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 85_000).await;

    ctx.process_async(json!({
        "id": 85_005,
        "method": "Browser.getVersion",
        "sessionId": attached.session_id
    }))
    .await;
    let version = take_response_by_id(&mut ctx, 85_005);
    assert!(version["result"]["protocolVersion"].as_str().is_some());

    ctx.process_async(json!({
        "id": 85_006,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": { "expression": "1 + 2", "returnByValue": true }
    }))
    .await;
    let eval = take_response_by_id(&mut ctx, 85_006);
    assert_eq!(eval["result"]["result"]["value"], 3);

    ctx.process_async(json!({
        "id": 85_007,
        "method": "Runtime.doesNotExist",
        "sessionId": attached.session_id
    }))
    .await;
    let unknown = take_response_by_id(&mut ctx, 85_007);
    assert_eq!(unknown["error"]["code"], -32601);
    assert_eq!(unknown["error"]["message"], "UnknownMethod");

    ctx.process_async(json!({
        "id": 85_008,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": attached.session_id }
    }))
    .await;
    ctx.expect_result(85_008, json!({}), None);

    ctx.process_async(json!({
        "id": 85_009,
        "method": "Runtime.evaluate",
        "sessionId": attached.session_id,
        "params": { "expression": "3 + 1", "returnByValue": true }
    }))
    .await;
    let detached = take_response_by_id(&mut ctx, 85_009);
    assert_eq!(detached["error"]["code"], -32001);
    assert_eq!(detached["error"]["message"], "Unknown sessionId");
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_auxiliary_session_network_event_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let attached = attached_smoke_session(&mut ctx, 86_000).await;

    ctx.process_async(json!({
        "id": 86_005,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let browser_response = take_response_by_id(&mut ctx, 86_005);
    let browser_session_id = browser_response["result"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({ "sessionId": browser_session_id.clone() })),
    );

    ctx.process_async(json!({
        "id": 86_006,
        "method": "Target.attachToTarget",
        "sessionId": browser_session_id,
        "params": { "targetId": attached.target_id }
    }))
    .await;
    let aux_session_id = take_response_by_id(&mut ctx, 86_006)["result"]["sessionId"]
        .as_str()
        .expect("auxiliary session")
        .to_owned();
    assert_ne!(aux_session_id, attached.session_id);
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 86_007,
        "method": "Network.enable",
        "sessionId": aux_session_id
    }))
    .await;
    ctx.expect_result(86_007, json!({}), Some(&aux_session_id));

    ctx.process_async(json!({
        "id": 86_008,
        "method": "Page.navigate",
        "sessionId": attached.session_id,
        "params": { "url": fixture.url("/plain?playwright-cdp-event") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 86_008);
    assert!(
        ctx.sent.iter().any(|message| {
            message["sessionId"] == json!(aux_session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/plain?playwright-cdp-event"))
        }),
        "auxiliary session should receive Network.requestWillBeSent: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_multi_context_popup_route_and_evaluate_contract() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    let first = attached_smoke_session(&mut ctx, 87_000).await;
    let second = attached_smoke_session(&mut ctx, 87_100).await;
    assert_ne!(first.browser_context_id, second.browser_context_id);

    set_auto_attach_waiting_for_debugger(&mut ctx, 87_200).await;
    ctx.take_all();

    let first_popup_url = fixture.url("/plain?popup=first-context");
    let (first_popup_target_id, first_popup_session_id, first_popup_context_id) =
        open_popup_from_session(&mut ctx, 87_201, &first.session_id, &first_popup_url).await;
    assert_eq!(first_popup_context_id, first.browser_context_id);
    arm_popup_route(
        &mut ctx,
        87_210,
        &first_popup_target_id,
        &first_popup_session_id,
        &first_popup_url,
    )
    .await;
    fulfill_popup_document_and_evaluate(
        &mut ctx,
        87_220,
        &first_popup_target_id,
        &first_popup_session_id,
        &first_popup_url,
        "first-popup-routed",
    )
    .await;

    let second_popup_url = fixture.url("/plain?popup=second-context");
    let (second_popup_target_id, second_popup_session_id, second_popup_context_id) =
        open_popup_from_session(&mut ctx, 87_301, &second.session_id, &second_popup_url).await;
    assert_eq!(second_popup_context_id, second.browser_context_id);
    assert_ne!(first_popup_session_id, second_popup_session_id);
    arm_popup_route(
        &mut ctx,
        87_310,
        &second_popup_target_id,
        &second_popup_session_id,
        &second_popup_url,
    )
    .await;
    fulfill_popup_document_and_evaluate(
        &mut ctx,
        87_320,
        &second_popup_target_id,
        &second_popup_session_id,
        &second_popup_url,
        "second-popup-routed",
    )
    .await;

    assert!(!ctx.sent.iter().any(|message| {
        message["sessionId"] == json!(first_popup_session_id)
            && message["params"]["request"]["url"] == json!(second_popup_url)
    }));
    assert!(!ctx.sent.iter().any(|message| {
        message["sessionId"] == json!(second_popup_session_id)
            && message["params"]["request"]["url"] == json!(first_popup_url)
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_playwright_concurrent_popup_routes_keep_their_navigation_owners() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    let opener = attached_smoke_session(&mut ctx, 88_000).await;

    set_auto_attach_waiting_for_debugger(&mut ctx, 88_100).await;
    ctx.take_all();

    let first_popup_url = fixture.url("/plain?popup=concurrent-first");
    let (first_popup_target_id, first_popup_session_id, first_popup_context_id) =
        open_popup_from_session(&mut ctx, 88_101, &opener.session_id, &first_popup_url).await;
    assert_eq!(first_popup_context_id, opener.browser_context_id);
    arm_popup_route(
        &mut ctx,
        88_110,
        &first_popup_target_id,
        &first_popup_session_id,
        &first_popup_url,
    )
    .await;

    let second_popup_url = fixture.url("/plain?popup=concurrent-second");
    let (second_popup_target_id, second_popup_session_id, second_popup_context_id) =
        open_popup_from_session(&mut ctx, 88_201, &opener.session_id, &second_popup_url).await;
    assert_eq!(second_popup_context_id, opener.browser_context_id);
    arm_popup_route(
        &mut ctx,
        88_210,
        &second_popup_target_id,
        &second_popup_session_id,
        &second_popup_url,
    )
    .await;

    fulfill_popup_document_and_evaluate(
        &mut ctx,
        88_220,
        &second_popup_target_id,
        &second_popup_session_id,
        &second_popup_url,
        "second-popup-routed",
    )
    .await;
    fulfill_popup_document_and_evaluate(
        &mut ctx,
        88_230,
        &first_popup_target_id,
        &first_popup_session_id,
        &first_popup_url,
        "first-popup-routed",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_popup_waits_for_every_debugger_barrier_and_detach_releases_the_last() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    let opener = attached_smoke_session(&mut ctx, 89_000).await;

    set_auto_attach_waiting_for_debugger(&mut ctx, 89_100).await;
    ctx.process_async(json!({
        "id": 89_101,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let browser_attached = ctx.take_first_matching("browser target session", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["type"] == json!("browser")
    });
    let browser_session_id = browser_attached["params"]["sessionId"]
        .as_str()
        .expect("browser target session id")
        .to_owned();
    ctx.expect_result(89_101, json!({ "sessionId": browser_session_id }), None);
    ctx.process_async(json!({
        "id": 89_102,
        "sessionId": browser_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(89_102, json!({}), Some(&browser_session_id));
    ctx.take_all();

    let popup_url = fixture.url("/plain?popup=two-debugger-barriers");
    ctx.process_async(json!({
        "id": 89_103,
        "method": "Runtime.evaluate",
        "sessionId": opener.session_id,
        "params": {
            "expression": format!("window.open('{popup_url}', '_blank') !== null"),
            "returnByValue": true
        }
    }))
    .await;
    let evaluated = take_response_by_id(&mut ctx, 89_103);
    assert_eq!(evaluated["result"]["result"]["value"], true);
    let created = ctx.take_first_matching("two-owner popup target", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["url"] == json!(popup_url)
    });
    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let root_attached = ctx.take_first_matching("root popup attachment", |message| {
        message.get("sessionId").is_none()
            && message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(popup_target_id)
    });
    let browser_owned_attached =
        ctx.take_first_matching("browser-owned popup attachment", |message| {
            message["sessionId"] == json!(browser_session_id)
                && message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(popup_target_id)
        });
    assert_eq!(root_attached["params"]["waitingForDebugger"], true);
    assert_eq!(browser_owned_attached["params"]["waitingForDebugger"], true);
    let root_popup_session_id = root_attached["params"]["sessionId"]
        .as_str()
        .expect("root popup session id")
        .to_owned();
    let browser_popup_session_id = browser_owned_attached["params"]["sessionId"]
        .as_str()
        .expect("browser-owned popup session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 89_104,
        "method": "Fetch.enable",
        "sessionId": root_popup_session_id,
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "resourceType": "Document",
                "requestStage": "Request"
            }]
        }
    }))
    .await;
    ctx.expect_result(89_104, json!({}), Some(&root_popup_session_id));

    ctx.process_async(json!({
        "id": 89_105,
        "method": "Runtime.runIfWaitingForDebugger",
        "sessionId": root_popup_session_id
    }))
    .await;
    ctx.expect_result(89_105, json!({}), Some(&root_popup_session_id));
    assert!(
        ctx.conn
            .target_has_waiting_for_debugger_session(&popup_target_id),
        "the second inspector session must keep the target behind its debugger barrier"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "one of two waiting sessions must not release the popup navigation: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 89_106,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": browser_popup_session_id }
    }))
    .await;
    ctx.expect_result(89_106, json!({}), None);
    assert!(
        !ctx.conn
            .target_has_waiting_for_debugger_session(&popup_target_id),
        "detaching the final waiting session must release the target barrier"
    );

    fulfill_popup_document_and_evaluate(
        &mut ctx,
        89_110,
        &popup_target_id,
        &root_popup_session_id,
        &popup_url,
        "all-debugger-barriers-released",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_popup_navigation_rechecks_a_late_debugger_barrier() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    let opener = attached_smoke_session(&mut ctx, 90_000).await;
    set_auto_attach_waiting_for_debugger(&mut ctx, 90_100).await;

    let popup_url = fixture.url("/plain?popup=late-debugger-barrier");
    let (popup_target_id, popup_session_id, browser_context_id) =
        open_popup_from_session(&mut ctx, 90_101, &opener.session_id, &popup_url).await;
    let action = crate::conn::PopupTargetNavigationOwnerAction::capture(
        &ctx.conn,
        &browser_context_id,
        &popup_target_id,
        popup_url,
        crate::conn::PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume,
    )
    .expect("the paused popup should have an exact navigation owner action");

    assert!(
        ctx.conn
            .release_waiting_for_debugger_session(Some(&popup_session_id))
    );
    assert!(
        !ctx.conn
            .target_has_waiting_for_debugger_session(&popup_target_id)
    );

    let late_session_id = "SID-late-debugger".to_owned();
    assert!(
        ctx.conn
            .prepare_auto_attached_page_session_binding(&popup_target_id, late_session_id.clone(),)
    );
    let prepared = ctx.conn.prepare_auto_attach_session_commit(
        late_session_id,
        Some(opener.session_id.clone()),
        true,
    );
    let target_info = ctx
        .conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|browser_context| browser_context.devtools_target_info(&popup_target_id))
        .expect("popup target info");
    let _ = ctx
        .conn
        .commit_prepared_attach_event_plan(crate::conn::PreparedTargetAttach::new(
            &popup_target_id,
            target_info,
            [prepared],
        ));
    assert!(
        ctx.conn
            .target_has_waiting_for_debugger_session(&popup_target_id),
        "the late session must install a new target barrier before queued work runs",
    );

    let outcome = complete_popup_target_navigation_owner_action_async(&mut ctx.conn, action).await;
    assert!(outcome.into_parts().0.is_empty());
    assert!(
        !ctx.conn
            .has_pending_document_navigation_for_session_owner(Some(&popup_session_id)),
        "queued work must not start the target URL through a newly paused target",
    );
    let page_url = ctx
        .conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|browser_context| loaded_page_for_target(browser_context, &popup_target_id))
        .map(|page| page.final_url().as_str())
        .expect("popup initial Page");
    assert_eq!(page_url, "about:blank");
}
