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

#[tokio::test(flavor = "multi_thread")]
async fn created_target_url_is_a_native_decision_before_debugger_release() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new();
    set_auto_attach_waiting_for_debugger(&mut ctx, 100_000).await;
    let url = fixture.url("/plain?created-native-decision");
    ctx.process_async(json!({"id": 100_001, "method": "Target.createTarget",
        "params": {"url": url}}))
        .await;
    let created = take_response_by_id(&mut ctx, 100_001);
    let target = created["result"]["targetId"].as_str().unwrap().to_owned();
    let attached = ctx.take_first_matching("created target attachment", |message| {
        message["method"] == "Target.attachedToTarget"
            && message["params"]["targetInfo"]["targetId"] == target
    });
    let session = attached["params"]["sessionId"].as_str().unwrap().to_owned();
    assert_eq!(attached["params"]["waitingForDebugger"], true);
    ctx.wait_until_scheduler_state("original created-target request decision", |conn| {
        conn.native_navigation_decision_for_target(&target)
            .is_some_and(|(_, paused)| {
                matches!(
                    paused.stage,
                    moli_core::browser::NavigationDecisionStage::Request { .. }
                )
            })
    })
    .await;
    let (_, paused) = ctx
        .conn
        .native_navigation_decision_for_target(&target)
        .expect("Browser must own the URL decision before debugger release");
    assert!(matches!(paused.stage,
        moli_core::browser::NavigationDecisionStage::Request { request, .. }
        if request.requested_url.as_str() == url));
    arm_popup_route(&mut ctx, 100_010, &target, &session, &url).await;
    fulfill_popup_document_and_evaluate(
        &mut ctx,
        100_020,
        &target,
        &session,
        &url,
        "created-native-decision",
    )
    .await;
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
            browser_context
                .target_document_url(popup_target_id)
                .map(|page| page.to_string())
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

    // Fulfillment resolves the exact Browser pause; the native driver then
    // commits independently. Observe that commit before inspecting its DOM.
    crate::testing::wait_until_scheduler_message(
        ctx,
        "fulfilled popup document commit",
        |message| {
            message["method"] == "Page.frameNavigated"
                && message["sessionId"] == popup_session_id
                && message["params"]["frame"]["id"] == popup_target_id
                && message["params"]["frame"]["url"] == popup_url
        },
    )
    .await;

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
    assert_eq!(
        evaluated["result"]["result"]["value"], expected_text,
        "{evaluated:?}"
    );
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| {
                message["method"] == "Page.frameNavigated"
                    && message["sessionId"] == popup_session_id
                    && message["params"]["frame"]["id"] == popup_target_id
                    && message["params"]["frame"]["url"] == popup_url
            })
            .count(),
        1,
        "one native commit must publish exactly one popup frame commit"
    );
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
    crate::testing::wait_until_navigation_document_load(
        &mut ctx,
        82_006,
        Some(&attached.session_id),
    )
    .await;
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
    crate::testing::wait_until_navigation_document_load(
        &mut ctx,
        83_006,
        Some(&attached.session_id),
    )
    .await;
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
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "original navigation reply",
        |message| {
            message["id"] == json!(84_006) && message["sessionId"] == json!(&attached.session_id)
        },
    )
    .await;
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
async fn rust_cdp_playwright_attached_session_network_event_contract() {
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
        .expect("attached session")
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
        "attached session should receive Network.requestWillBeSent: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_response_pause_precedes_body_eof_and_preserves_response_body() {
    assert_native_popup_response(NativePopupResponseResolution::ReadBody).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_response_body_stream_replays_after_eof() {
    assert_native_popup_response(NativePopupResponseResolution::ReadStream).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_response_fulfillment_does_not_wait_for_the_original_body() {
    assert_native_popup_response(NativePopupResponseResolution::Fulfill).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_response_failure_decides_even_while_a_body_reader_holds_the_transfer() {
    assert_native_popup_response(NativePopupResponseResolution::FailWhileBodyBorrowed).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_response_fetch_disable_resumes_the_browser_driver() {
    assert_native_popup_response(NativePopupResponseResolution::Disable).await;
}

#[derive(Clone, Copy)]
enum NativePopupResponseResolution {
    ReadBody,
    ReadStream,
    Fulfill,
    Disable,
    FailWhileBodyBorrowed,
}

async fn assert_native_popup_response(resolution: NativePopupResponseResolution) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/native-popup-stream",
        listener.local_addr().unwrap()
    );
    let (release_body, body_released) = tokio::sync::watch::channel(false);
    let (stop_server, mut stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        // The existing renderer Window proxy can fetch independently of the
        // Browser Document. A stalled first connection must not starve the
        // exact Browser navigation whose Fetch decision this test observes.
        let mut responses = tokio::task::JoinSet::new();
        loop {
            let (mut stream, _) = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            let mut body_released = body_released.clone();
            responses.spawn(async move {
                let mut request = [0; 2048];
                if stream.read(&mut request).await.unwrap_or(0) == 0 { return; }
                if stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.is_err() { return; }
                let _ = body_released.wait_for(|released| *released).await;
                let body = "<main id=content>native response body</main>";
                let _ = stream.write_all(format!("{:X}\r\n{body}\r\n0\r\n\r\n", body.len()).as_bytes()).await;
            });
        }
        while let Some(response) = responses.join_next().await {
            response.unwrap();
        }
    });
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    let opener = attached_smoke_session(&mut ctx, 93_000).await;
    set_auto_attach_waiting_for_debugger(&mut ctx, 93_010).await;
    ctx.take_all();
    let (target, session, _) =
        open_popup_from_session(&mut ctx, 93_011, &opener.session_id, &url).await;
    arm_popup_route(&mut ctx, 93_020, &target, &session, &url).await;
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native popup request pause",
        |message| {
            message["method"] == "Fetch.requestPaused"
                && message["sessionId"] == session
                && message["params"]["request"]["url"] == url
        },
    )
    .await;
    let request_id = paused_request_id(&mut ctx, "Document");
    ctx.process_async(json!({
        "id": 93_030, "method": "Fetch.continueRequest", "sessionId": session,
        "params": {"requestId": request_id, "interceptResponse": true}
    }))
    .await;
    ctx.expect_result(93_030, json!({}), Some(&session));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native popup response head pause before body release",
        |message| {
            message["method"] == "Fetch.requestPaused"
                && message["sessionId"] == session
                && message["params"]["responseStatusCode"] == 200
                && message["params"]["request"]["url"] == url
        },
    )
    .await;
    let paused = ctx.take_first_matching("native response decision", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["sessionId"] == session
            && message["params"]["responseStatusCode"] == 200
    });
    let response_id = paused["params"]["requestId"].as_str().unwrap();
    let (contents, decision) = ctx
        .conn
        .native_navigation_decision_for_target(&target)
        .unwrap();
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == "Network.responseReceivedExtraInfo"
                && message["sessionId"] == session
                && message["params"]["requestId"] == paused["params"]["networkId"]
        }),
        "real response extra-info precedes the response-stage Fetch pause"
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != "Page.frameNavigated"
                || message["sessionId"] != session
                || message["params"]["frame"]["url"] != url)
    );
    let expected = match resolution {
        NativePopupResponseResolution::FailWhileBodyBorrowed => {
            let owner = crate::conn::CommandOwnerScope::for_session(&session);
            let transfer = ctx
                .conn
                .take_pending_fetch_response_transfer_for_body_read_for_owner(&owner, response_id)
                .expect("exclusive body-read claim");
            assert!(transfer.has_pending_decision());
            ctx.process_async(
                json!({"id": 93_031, "method": "Fetch.failRequest", "sessionId": session,
                "params": {"requestId": response_id, "errorReason": "Aborted"}}),
            )
            .await;
            ctx.expect_result(93_031, json!({}), Some(&session));
            ctx.wait_until_scheduler_state("exact borrowed-response navigation canceled", |conn| {
                !conn.has_pending_document_navigation_for_owner(&owner)
            })
            .await;
            assert!(!ctx.conn.resolve_native_navigation_decision(
                contents,
                decision.permit,
                moli_core::browser::NavigationDecision::Continue
            ));
            assert!(
                !ctx.conn
                    .restore_pending_fetch_response_transfer_for_body_read_for_owner(
                        &owner,
                        response_id,
                        transfer
                    )
            );
            ctx.process_async(
                json!({"id": 93_033, "method": "Runtime.evaluate", "sessionId": session,
                "params": {"expression": "location.href", "returnByValue": true}}),
            )
            .await;
            let evaluated = take_response_by_id(&mut ctx, 93_033);
            assert_eq!(evaluated["result"]["result"]["value"], "about:blank");
            assert!(
                ctx.sent
                    .iter()
                    .all(|message| message["method"] != "Page.frameNavigated"
                        || message["sessionId"] != session
                        || message["params"]["frame"]["url"] != url)
            );
            release_body.send(true).unwrap();
            stop_server.send(()).unwrap();
            server.await.unwrap();
            return;
        }
        NativePopupResponseResolution::ReadBody | NativePopupResponseResolution::ReadStream => {
            let (read_command, body_key) = if matches!(
                resolution,
                NativePopupResponseResolution::ReadStream
            ) {
                ctx.process_async(json!({
                    "id": 93_040, "method": "Fetch.takeResponseBodyAsStream", "sessionId": session,
                    "params": {"requestId": response_id}
                }))
                .await;
                let opened = take_response_by_id(&mut ctx, 93_040);
                let stream = opened["result"]["stream"]
                    .as_str()
                    .expect("native body stream");
                ctx.process_async(json!({
                    "id": 93_041, "method": "Fetch.continueResponse", "sessionId": session,
                    "params": {"requestId": response_id}
                }))
                .await;
                ctx.expect_error(93_041, -32000, "ResponseBodyStreamActive");
                (
                    json!({"id": 93_031, "method": "IO.read", "sessionId": session,
                    "params": {"handle": stream}}),
                    "data",
                )
            } else {
                (
                    json!({"id": 93_031, "method": "Fetch.getResponseBody", "sessionId": session,
                    "params": {"requestId": response_id}}),
                    "body",
                )
            };
            release_body.send(true).unwrap();
            ctx.process_async(read_command).await;
            let body = take_response_by_id(&mut ctx, 93_031);
            let actual = body["result"][body_key].as_str().unwrap();
            let actual = if body["result"]["base64Encoded"] == true {
                String::from_utf8(BASE64_STANDARD.decode(actual).unwrap()).unwrap()
            } else {
                actual.to_owned()
            };
            assert_eq!(actual, "<main id=content>native response body</main>");
            if matches!(resolution, NativePopupResponseResolution::ReadStream) {
                assert_eq!(body["result"]["eof"], true);
            }
            ctx.process_async(json!({
                "id": 93_032, "method": "Fetch.continueResponse", "sessionId": session,
                "params": {"requestId": response_id}
            }))
            .await;
            "native response body"
        }
        NativePopupResponseResolution::Fulfill => {
            ctx.process_async(json!({
                "id": 93_032, "method": "Fetch.fulfillRequest", "sessionId": session,
                "params": {"requestId": response_id, "responseCode": 200,
                    "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
                    "body": BASE64_STANDARD.encode("<main id=content>native fulfilled body</main>")}
            }))
            .await;
            "native fulfilled body"
        }
        NativePopupResponseResolution::Disable => {
            ctx.process_async(
                json!({"id": 93_032, "method": "Fetch.disable", "sessionId": session}),
            )
            .await;
            release_body.send(true).unwrap();
            "native response body"
        }
    };
    ctx.expect_result(93_032, json!({}), Some(&session));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native response document commit",
        |message| {
            message["method"] == "Page.frameNavigated"
                && message["sessionId"] == session
                && message["params"]["frame"]["id"] == target
                && message["params"]["frame"]["url"] == url
        },
    )
    .await;
    ctx.process_async(json!({
        "id": 93_033, "method": "Runtime.evaluate", "sessionId": session,
        "params": {"expression": "document.getElementById('content').textContent", "returnByValue": true}
    })).await;
    let evaluated = take_response_by_id(&mut ctx, 93_033);
    assert_eq!(evaluated["result"]["result"]["value"], expected);
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("navigation network identity");
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native response Network completion",
        |message| {
            message["method"] == "Network.loadingFinished"
                && message["sessionId"] == session
                && message["params"]["requestId"] == network_id
        },
    )
    .await;
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| {
                message["method"] == "Network.responseReceived"
                    && message["sessionId"] == session
                    && message["params"]["requestId"] == network_id
                    && message["params"]["response"]["url"] == url
            })
            .count(),
        1
    );
    ctx.process_async(
        json!({"id": 93_034, "method": "Network.getResponseBody", "sessionId": session,
        "params": {"requestId": network_id}}),
    )
    .await;
    let body = take_response_by_id(&mut ctx, 93_034);
    let actual = body["result"]["body"]
        .as_str()
        .expect("native response capture");
    let actual = if body["result"]["base64Encoded"] == true {
        String::from_utf8(BASE64_STANDARD.decode(actual).unwrap()).unwrap()
    } else {
        actual.to_owned()
    };
    assert_eq!(actual, format!("<main id=content>{expected}</main>"));
    let resource = ctx
        .conn
        .current_main_document_resource_for_session_owner(Some(&session))
        .expect("committed native document resource");
    assert_eq!(resource.frame_id, target);
    assert_eq!(resource.url.as_str(), url);
    assert_eq!(
        resource
            .body
            .expect("native document resource body")
            .materialize_bytes()
            .unwrap(),
        actual.as_bytes()
    );
    assert!(
        ctx.conn
            .project_browser_navigation_responses(contents)
            .await
            .is_empty(),
        "reconciliation must not repeat response or completion events"
    );
    release_body.send(true).unwrap();
    stop_server.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_authentication_retries_the_exact_browser_navigation() {
    assert_native_popup_authentication("ProvideCredentials", "basic").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_download_transfers_the_response_and_finishes_network_once() {
    assert_native_popup_download(NativePopupDownloadResponse::Streaming).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_download_preserves_a_captured_fetch_response() {
    assert_native_popup_download(NativePopupDownloadResponse::Captured).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_download_accepts_a_fulfilled_attachment_response() {
    assert_native_popup_download(NativePopupDownloadResponse::Fulfilled).await;
}

enum NativePopupDownloadResponse {
    Streaming,
    Captured,
    Fulfilled,
}

async fn assert_native_popup_download(resolution: NativePopupDownloadResponse) {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(std::env::temp_dir().join(format!(
        "moli-native-popup-download-projection-{}-{}",
        std::process::id(),
        moli_core::browser::NavigationId::allocate().get()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let fixture = SmokeFixtureServer::start().await;
    let url = fixture.url("/download");
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    let opener = attached_smoke_session(&mut ctx, 95_000).await;
    set_auto_attach_waiting_for_debugger(&mut ctx, 95_010).await;
    ctx.process_async(json!({"id": 95_011, "method": "Browser.setDownloadBehavior", "params": {
        "behavior": "allowAndName", "downloadPath": directory.0.to_str().unwrap(), "eventsEnabled": true
    }})).await;
    ctx.expect_result(95_011, json!({}), None);
    ctx.take_all();
    let (target, session, _) =
        open_popup_from_session(&mut ctx, 95_012, &opener.session_id, &url).await;
    arm_popup_route(&mut ctx, 95_020, &target, &session, &url).await;
    crate::testing::wait_until_scheduler_message(&mut ctx, "download request pause", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["sessionId"] == session
            && message["params"]["request"]["url"] == url
    })
    .await;
    let request_id = paused_request_id(&mut ctx, "Document");
    ctx.process_async(
        json!({"id": 95_030, "method": "Fetch.continueRequest", "sessionId": session,
        "params": {"requestId": request_id, "interceptResponse": true}}),
    )
    .await;
    ctx.expect_result(95_030, json!({}), Some(&session));
    crate::testing::wait_until_scheduler_message(&mut ctx, "download response pause", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["sessionId"] == session
            && message["params"]["responseStatusCode"] == 200
    })
    .await;
    let paused = ctx.take_first_matching("download response permit", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["sessionId"] == session
            && message["params"]["responseStatusCode"] == 200
    });
    let response_id = paused["params"]["requestId"].as_str().unwrap();
    let network_id = paused["params"]["networkId"].as_str().unwrap();
    let (contents, _) = ctx
        .conn
        .native_navigation_decision_for_target(&target)
        .unwrap();
    if matches!(resolution, NativePopupDownloadResponse::Captured) {
        ctx.process_async(
            json!({"id": 95_031, "method": "Fetch.getResponseBody", "sessionId": session,
            "params": {"requestId": response_id}}),
        )
        .await;
        let body = take_response_by_id(&mut ctx, 95_031);
        let value = body["result"]["body"].as_str().unwrap();
        let bytes = if body["result"]["base64Encoded"] == true {
            BASE64_STANDARD.decode(value).unwrap()
        } else {
            value.as_bytes().to_vec()
        };
        assert_eq!(bytes, b"download contents");
    }
    let expected = if matches!(resolution, NativePopupDownloadResponse::Fulfilled) {
        ctx.process_async(json!({"id": 95_032, "method": "Fetch.fulfillRequest", "sessionId": session,
            "params": {"requestId": response_id, "responseCode": 200,
                "responseHeaders": [{"name": "Content-Disposition", "value": "attachment; filename=override.txt"}],
                "body": BASE64_STANDARD.encode("fulfilled download")}})).await;
        b"fulfilled download".as_slice()
    } else {
        ctx.process_async(
            json!({"id": 95_032, "method": "Fetch.continueResponse", "sessionId": session,
            "params": {"requestId": response_id}}),
        )
        .await;
        b"download contents".as_slice()
    };
    ctx.expect_result(95_032, json!({}), Some(&session));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native download admission",
        |message| {
            message["method"] == "Browser.downloadWillBegin"
                && message["params"]["url"] == url
                && message["params"]["frameId"] == target
        },
    )
    .await;
    let created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == "Browser.downloadWillBegin" && message["params"]["url"] == url
        })
        .unwrap()
        .clone();
    let guid = created["params"]["guid"].as_str().unwrap();
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native download completion",
        |message| {
            message["method"] == "Browser.downloadProgress"
                && message["params"]["guid"] == guid
                && message["params"]["state"] == "completed"
        },
    )
    .await;
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "download navigation failure",
        |message| {
            message["method"] == "Network.loadingFailed"
                && message["sessionId"] == session
                && message["params"]["requestId"] == network_id
        },
    )
    .await;
    assert_eq!(std::fs::read(directory.0.join(guid)).unwrap(), expected);
    let position = |method| {
        ctx.sent
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                (message["method"] == method
                    && message["sessionId"] == session
                    && message["params"]["requestId"] == network_id)
                    .then_some(index)
            })
            .collect::<Vec<_>>()
    };
    let responses = position("Network.responseReceived");
    let failures = position("Network.loadingFailed");
    assert_eq!(responses.len(), 1);
    assert_eq!(failures.len(), 1);
    assert!(responses[0] < failures[0]);
    assert_eq!(
        ctx.sent[failures[0]]["params"]["errorText"],
        "net::ERR_ABORTED"
    );
    assert!(position("Network.loadingFinished").is_empty());
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| message["method"] == "Page.frameStoppedLoading"
                && message["sessionId"] == session
                && message["params"]["frameId"] == target)
            .count(),
        1
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == "Page.frameNavigated"
                && message["sessionId"] == session
                && message["params"]["frame"]["url"] == url)
    );
    ctx.process_async(
        json!({"id": 95_033, "method": "Runtime.evaluate", "sessionId": session,
        "params": {"expression": "location.href", "returnByValue": true}}),
    )
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 95_033)["result"]["result"]["value"],
        "about:blank"
    );
    assert!(
        ctx.conn
            .project_browser_navigation(contents)
            .await
            .is_empty()
    );
    assert!(
        ctx.conn
            .project_browser_navigation_responses(contents)
            .await
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_authentication_cancel_preserves_the_challenge_response() {
    assert_native_popup_authentication("CancelAuth", "basic").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_authentication_digest_retries_the_exact_browser_navigation() {
    assert_native_popup_authentication("ProvideCredentials", "digest").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_authentication_default_cancels_the_exact_browser_navigation() {
    assert_native_popup_authentication("Default", "basic").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_popup_authentication_fetch_disable_cancels_the_exact_browser_navigation() {
    assert_native_popup_authentication("Disable", "basic").await;
}

async fn assert_native_popup_authentication(action: &str, scheme: &str) {
    let provide_credentials = action == "ProvideCredentials";
    let fixture = SmokeFixtureServer::start().await;
    let url = fixture.url(&format!("/api-auth?realm=native-popup&scheme={scheme}"));
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    let opener = attached_smoke_session(&mut ctx, 94_000).await;
    set_auto_attach_waiting_for_debugger(&mut ctx, 94_010).await;
    ctx.take_all();
    let (target, session, _) =
        open_popup_from_session(&mut ctx, 94_011, &opener.session_id, &url).await;
    for (offset, method, params) in [
        (0, "Page.enable", json!({})),
        (1, "Network.enable", json!({})),
        (
            2,
            "Fetch.enable",
            json!({"handleAuthRequests": true, "patterns": [{"urlPattern": "*", "resourceType": "Document", "requestStage": "Request"}]}),
        ),
        (3, "Runtime.runIfWaitingForDebugger", json!({})),
    ] {
        ctx.process_async(json!({"id": 94_020 + offset, "method": method, "sessionId": session, "params": params})).await;
        ctx.expect_result(94_020 + offset, json!({}), Some(&session));
    }
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native authentication request",
        |message| {
            message["method"] == "Fetch.requestPaused"
                && message["sessionId"] == session
                && message["params"]["request"]["url"] == url
        },
    )
    .await;
    let request_id = paused_request_id(&mut ctx, "Document");
    ctx.process_async(
        json!({"id": 94_030, "method": "Fetch.continueRequest", "sessionId": session,
        "params": {"requestId": request_id, "interceptResponse": true}}),
    )
    .await;
    ctx.expect_result(94_030, json!({}), Some(&session));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native authentication challenge",
        |message| {
            message["method"] == "Fetch.authRequired"
                && message["sessionId"] == session
                && message["params"]["request"]["url"] == url
        },
    )
    .await;
    let challenge = ctx.take_first_matching("native auth challenge", |message| {
        message["method"] == "Fetch.authRequired" && message["sessionId"] == session
    });
    assert_eq!(
        challenge["params"]["authChallenge"]["realm"],
        "native-popup"
    );
    assert_eq!(challenge["params"]["authChallenge"]["scheme"], scheme);
    let (contents, pause) = ctx
        .conn
        .native_navigation_decision_for_target(&target)
        .unwrap();
    assert!(matches!(
        pause.stage,
        moli_core::browser::NavigationDecisionStage::Auth { .. }
    ));
    assert!(
        ctx.conn
            .project_browser_navigation_decision(contents, None)
            .await
            .is_empty(),
        "snapshot reconciliation must not duplicate this authentication decision"
    );
    let network_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == "Network.requestWillBeSent"
                && message["sessionId"] == session
                && message["params"]["request"]["url"] == url
        })
        .expect("exact native network request")["params"]["requestId"]
        .clone();
    let command = if action == "Disable" {
        json!({"id": 94_031, "method": "Fetch.disable", "sessionId": session})
    } else {
        json!({"id": 94_031, "method": "Fetch.continueWithAuth", "sessionId": session,
        "params": {"requestId": challenge["params"]["requestId"], "authChallengeResponse": {
            "response": action, "username": "user", "password": "pass"
        }}})
    };
    ctx.process_async(command).await;
    ctx.expect_result(94_031, json!({}), Some(&session));
    if matches!(action, "Default" | "Disable") {
        crate::testing::wait_until_scheduler_message(
            &mut ctx,
            "native auth cancellation Network result",
            |message| {
                message["method"] == "Network.loadingFailed"
                    && message["sessionId"] == session
                    && message["params"]["requestId"] == network_id
            },
        )
        .await;
        assert!(
            ctx.conn
                .native_navigation_decision_for_target(&target)
                .is_none()
        );
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != "Page.frameNavigated"
                    || message["sessionId"] != session
                    || message["params"]["frame"]["url"] != url)
        );
        assert!(!ctx.conn.resolve_native_navigation_decision(
            contents,
            pause.permit,
            moli_core::browser::NavigationDecision::Continue
        ));
        return;
    }
    let expected_status = if provide_credentials { 200 } else { 401 };
    let expected_body = if provide_credentials {
        "authenticated fetch"
    } else {
        "auth required"
    };
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native post-auth response pause",
        |message| {
            message["method"] == "Fetch.requestPaused"
                && message["sessionId"] == session
                && message["params"]["responseStatusCode"] == expected_status
                && message["params"]["request"]["url"] == url
        },
    )
    .await;
    let response = ctx.take_first_matching("post-auth response", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["sessionId"] == session
            && message["params"]["responseStatusCode"] == expected_status
    });
    let (_, response_pause) = ctx
        .conn
        .native_navigation_decision_for_target(&target)
        .unwrap();
    assert_eq!(
        response_pause.permit.navigation(),
        pause.permit.navigation()
    );
    assert_ne!(response_pause.permit, pause.permit);
    assert!(!ctx.conn.resolve_native_navigation_decision(
        contents,
        pause.permit,
        moli_core::browser::NavigationDecision::Cancel
    ));
    ctx.process_async(
        json!({"id": 94_032, "method": "Fetch.continueResponse", "sessionId": session,
        "params": {"requestId": response["params"]["requestId"]}}),
    )
    .await;
    ctx.expect_result(94_032, json!({}), Some(&session));
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "native authenticated document commit",
        |message| {
            message["method"] == "Page.frameNavigated"
                && message["sessionId"] == session
                && message["params"]["frame"]["id"] == target
                && message["params"]["frame"]["url"] == url
        },
    )
    .await;
    ctx.process_async(
        json!({"id": 94_033, "method": "Runtime.evaluate", "sessionId": session,
        "params": {"expression": "document.body.textContent.trim()", "returnByValue": true}}),
    )
    .await;
    let evaluated = take_response_by_id(&mut ctx, 94_033);
    assert_eq!(evaluated["result"]["result"]["value"], expected_body);
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
    ctx.wait_until_scheduler_state(
        "native popup request admitted behind debugger barrier",
        |conn| {
            conn.native_navigation_decision_for_target(&popup_target_id)
                .is_some_and(|(_, paused)| {
                    matches!(
                        paused.stage,
                        moli_core::browser::NavigationDecisionStage::Request { .. }
                    )
                })
        },
    )
    .await;
    let action = crate::conn::TargetStartupOwnerAction::capture(
        &ctx.conn,
        &browser_context_id,
        &popup_target_id,
    )
    .expect("the paused popup should have an exact navigation owner action");
    let (contents, permit) = action.decision();

    assert!(
        ctx.conn
            .release_waiting_for_debugger_session(Some(&popup_session_id))
    );
    assert!(
        !ctx.conn
            .target_has_waiting_for_debugger_session(&popup_target_id)
    );

    let late_session_id = "SID-late-debugger".to_owned();
    let route = ctx
        .conn
        .prepare_auto_attached_page_session_binding(&popup_target_id, late_session_id.clone())
        .expect("popup target must remain addressable");
    let prepared = crate::conn::TargetAttachSessionCommit::auto_attached(
        late_session_id,
        Some(opener.session_id.clone()),
        route,
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

    let outcome = complete_target_startup_owner_action_async(&mut ctx.conn, action).await;
    assert!(outcome.into_parts().0.is_empty());
    let (still_paused_contents, still_paused) = ctx
        .conn
        .native_navigation_decision_for_target(&popup_target_id)
        .expect("late debugger barrier must preserve the unclaimed request decision");
    assert_eq!(still_paused_contents, contents);
    assert_eq!(
        still_paused.permit, permit,
        "queued work must not consume the request permit through a newly paused target"
    );
    assert!(matches!(
        still_paused.stage,
        moli_core::browser::NavigationDecisionStage::Request { .. }
    ));
    let page_url = ctx
        .conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|browser_context| browser_context.target_document_url(&popup_target_id))
        .map(|page| page.to_string())
        .expect("popup initial Page");
    assert_eq!(page_url, "about:blank");
}
