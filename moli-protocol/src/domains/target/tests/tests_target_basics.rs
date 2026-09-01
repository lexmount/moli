use super::*;
use crate::conn::PageTargetHost;

#[tokio::test(flavor = "multi_thread")]
async fn get_browser_contexts_returns_active_and_inactive_ids() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.conn
        .inactive_browser_contexts
        .push(BrowserContext::new("BID-B".into()));

    ctx.process_async(json!({"id": 5, "method": "Target.getBrowserContexts"}))
        .await;
    ctx.expect_result(5, json!({ "browserContextIds": ["BID-A", "BID-B"] }), None);
}

/// cdp.target: createBrowserContext – additional contexts are kept inactive
#[tokio::test(flavor = "multi_thread")]
async fn create_browser_context_adds_inactive_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 4, "method": "Target.createBrowserContext"}))
        .await;
    let active_id = ctx.conn.browser_context.as_ref().unwrap().id.clone();
    ctx.expect_result(4, json!({ "browserContextId": active_id }), None);

    ctx.process_async(json!({"id": 5, "method": "Target.createBrowserContext"}))
        .await;
    let second = ctx.take_one();
    let second_id = second["result"]["browserContextId"]
        .as_str()
        .expect("second browser context id")
        .to_owned();
    assert_ne!(second_id, active_id);
    assert_eq!(ctx.conn.browser_context.as_ref().unwrap().id, active_id);
    assert_eq!(ctx.conn.inactive_browser_contexts.len(), 1);
    assert_eq!(ctx.conn.inactive_browser_contexts[0].id, second_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn close_active_target_fails_only_active_owner_pending_awaits() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-await-owner".into());
    bc.set_active_target_id("TID-active-await".to_owned());
    bc.attach_active_session("SID-active-await".to_owned());
    assert!(
        bc.assign_auxiliary_session_to_target(
            "TID-active-await",
            "SID-active-aux-await".to_owned(),
        )
    );
    bc.insert_page_target_host(PageTargetHost::with_url(
        "TID-bg-await".to_owned(),
        Some("SID-bg-await".to_owned()),
        "about:blank#bg-await".to_owned(),
    ));
    ctx.conn.browser_context = Some(bc);
    ctx.conn
        .register_pending_inspector_await(1041201, Some("SID-active-await"));
    ctx.conn
        .register_pending_inspector_await(1041203, Some("SID-active-aux-await"));
    ctx.conn
        .register_pending_inspector_await(1041202, Some("SID-bg-await"));

    ctx.process_async(json!({
        "id": 1041200,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-active-await" }
    }))
    .await;

    let close = take_response_by_id(&mut ctx, 1041200);
    assert_eq!(close["result"]["success"], json!(true));
    let active_failed = take_response_by_id(&mut ctx, 1041201);
    assert_eq!(active_failed["sessionId"], json!("SID-active-await"));
    assert_eq!(active_failed["error"]["message"], json!("Target closed"));
    let auxiliary_failed = take_response_by_id(&mut ctx, 1041203);
    assert_eq!(auxiliary_failed["sessionId"], json!("SID-active-aux-await"));
    assert_eq!(auxiliary_failed["error"]["message"], json!("Target closed"));
    assert!(
        ctx.take_all().into_iter().all(|message| {
            message["id"] != json!(1041201)
                && message["id"] != json!(1041202)
                && message["id"] != json!(1041203)
        }),
        "target-local awaits must settle exactly once without touching the background owner"
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some("SID-bg-await")),
        "background owner await should remain pending after active target close"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_browser_context_records_proxy_server_override() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 41,
        "method": "Target.createBrowserContext",
        "params": {
            "proxyServer": "http://proxy.test:8080",
            "proxyBypassList": "<-loopback>"
        }
    }))
    .await;
    let (active_id, active_proxy, active_no_proxy) = {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        (
            active.id.clone(),
            active.default_http_proxy_override.clone(),
            active.default_http_no_proxy_override.clone(),
        )
    };
    ctx.expect_result(41, json!({ "browserContextId": active_id }), None);
    assert_eq!(active_proxy.as_deref(), Some("http://proxy.test:8080"));
    assert_eq!(
        active_no_proxy.as_deref(),
        Some(""),
        "`<-loopback>` must explicitly disable bypass instead of inheriting process NO_PROXY"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_browser_context_preserves_non_loopback_proxy_bypass_entries() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 42,
        "method": "Target.createBrowserContext",
        "params": {
            "proxyServer": "http://proxy.test:8080",
            "proxyBypassList": "example.com, <-loopback>, .internal"
        }
    }))
    .await;
    let (active_id, active_no_proxy) = {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        (
            active.id.clone(),
            active.default_http_no_proxy_override.clone(),
        )
    };
    ctx.expect_result(42, json!({ "browserContextId": active_id }), None);
    assert_eq!(active_no_proxy.as_deref(), Some("example.com,.internal"));
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_selects_inactive_browser_context() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-A");
    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B");
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 6,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-B"}
    }))
    .await;

    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id after attach");
    ctx.expect_result(6, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": "TID-B",
                "browserContextId": "BID-B",
            }
        })),
    );
    assert_eq!(ctx.conn.browser_context.as_ref().unwrap().id, "BID-B");
    assert!(
        ctx.conn
            .inactive_browser_contexts
            .iter()
            .any(|bc| bc.id == "BID-A")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_creates_auxiliary_session_and_keeps_target_context_active() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-A");
    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B");
    inactive.attach_active_session("SID-B");
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 61,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-B"}
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 61);
    let attached_session_id = response["result"]["sessionId"]
        .as_str()
        .expect("attached session id")
        .to_owned();
    assert_ne!(attached_session_id, "SID-B");
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(
        attached["params"]["sessionId"],
        json!(attached_session_id.as_str())
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-B"),
        "attachToTarget should keep the selected target context active while creating another session"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.auxiliary_target_id_for_session(&attached_session_id)),
        Some("TID-B")
    );
    assert!(
        ctx.conn
            .inactive_browser_contexts
            .iter()
            .any(|bc| bc.id == "BID-A")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_aborts_paused_request_stage_navigation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>dispose-bc</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-9".into());
    bc.set_active_target_id("TID-000000000A");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 20,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(20, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let network_id = paused["params"]["networkId"].clone();

    ctx.process_async(json!({
        "id": 22,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-9" }
    }))
    .await;
    ctx.expect_result(22, json!({}), None);

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Browser context disposed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 21);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Browser context disposed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.detached");

    let target = ctx.take_one();
    assert_eq!(target["method"], "Target.detachedFromTarget");
    assert_eq!(target["params"]["targetId"], "TID-000000000A");

    assert!(ctx.conn.browser_context.is_none());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_browser_context_aborts_paused_runtime_fetch_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn data() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "payload")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/data", any(data)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let data_url = format!("http://{addr}/data");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-9".into());
    bc.set_active_target_id("TID-000000000A");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("Fetch fixture target")
        .enable_primary_network_events();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(23, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 24,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(24, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 25,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
  fetch("{}").catch(() => {{}});
  return "scheduled";
}})()"#, data_url)
        }
    }))
    .await;
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(25))
        .expect("runtime evaluate response");
    ctx.sent.remove(pos);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(data_url)
        })
        .cloned()
        .expect("subresource fetch requestPaused event");
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 26,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-9" }
    }))
    .await;
    ctx.expect_result(26, json!({}), None);

    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        }),
        "Chromium does not synthesize a subresource loadingFailed event while disposing its BrowserContext"
    );
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.detached") && message["sessionId"] == json!("SID-1")
    }));

    assert!(ctx.conn.browser_context.is_none());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_for_inactive_browser_context_keeps_previously_active_context() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.conn
        .inactive_browser_contexts
        .push(BrowserContext::new("BID-B".into()));

    ctx.process_async(json!({
        "id": 1010,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-B", "url": "about:blank"}
    }))
    .await;

    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.targetCreated");
    assert_eq!(event["params"]["targetInfo"]["browserContextId"], "BID-B");
    let target_id = event["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    ctx.expect_result(1010, json!({ "targetId": target_id }), None);
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "creating a target in another browser context must not leave that context selected as the default active context"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive browser context should still be present");
    assert!(inactive.has_active_target());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_with_auto_attach_attaches_second_target_in_same_browser_context() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                           "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank"}}))
        .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        second_target_id
    );
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10, json!({ "targetId": second_target_id }), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(
        bc.background_target(&second_target_id)
            .and_then(|target| target.session_id()),
        Some(session_id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_command_on_auto_attached_background_target_session_routes_without_promoting_loaded_active_target()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>loaded</title><div id='ok'>loaded target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 101,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(101, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 102,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<title>routed-loaded-active</title><div id='ok'>routed from loaded active target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 102);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert_eq!(
        bc.background_target(&second_target_id)
            .and_then(|target| target.session_id()),
        Some(session_id.as_str())
    );
    assert!(
        bc.background_target(&second_target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "background Page.navigate should load the parked target without promoting it"
    );

    ctx.process_async(json!({
        "id": 103,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 103);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("routed-loaded-active"));
    assert_eq!(payload["text"], json!("routed from loaded active target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn page_bring_to_front_promotes_background_session_explicitly() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 1021,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(1021, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1022,
        "method": "Page.bringToFront",
        "sessionId": session_id,
    }))
    .await;
    ctx.expect_result(1022, json!({}), Some(session_id.as_str()));

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
    assert_eq!(bc.active_session_id(), Some(session_id.as_str()));
    assert_eq!(
        bc.background_target("TID-000000000A")
            .and_then(|target| target.session_id()),
        Some("SID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_bring_to_front_on_inactive_context_restores_previous_context() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-active-a");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active-a");

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-active-b".to_owned());
    inactive.attach_active_session("SID-active-b".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 1023,
        "method": "Page.bringToFront",
        "sessionId": "SID-active-b",
    }))
    .await;
    ctx.expect_result(1023, json!({}), Some("SID-active-b"));

    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "direct Page.bringToFront should preserve dispatcher-style context restore"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive context should remain parked");
    assert_eq!(inactive.active_target_id(), Some("TID-active-b"));
    assert_eq!(inactive.active_session_id(), Some("SID-active-b"));
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_on_auto_attached_background_target_session_routes_without_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 1031,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(1031, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
            "id": 1032,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<title>autoattach-session-routed</title><div id='ok'>autoattach session routed target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 1032);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert_eq!(
        bc.background_target(&second_target_id)
            .and_then(|target| target.session_id()),
        Some(session_id.as_str())
    );
    assert!(
        bc.background_target(&second_target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "background Page.navigate should load the parked target without promoting it"
    );

    ctx.process_async(json!({
            "id": 1033,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let evaluation = take_response_by_id(&mut ctx, 1033);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("autoattach-session-routed"));
    assert_eq!(payload["text"], json!("autoattach session routed target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn page_stop_loading_aborts_background_pending_fetch_without_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<!doctype html><title>active</title>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 1034,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(1034, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1035,
        "method": "Network.enable",
        "sessionId": session_id,
    }))
    .await;
    ctx.expect_result(1035, json!({}), Some(session_id.as_str()));

    ctx.process_async(json!({
        "id": 1036,
        "method": "Fetch.enable",
        "sessionId": session_id,
        "params": {
            "patterns": [{"urlPattern": "*", "requestStage": "Request"}]
        }
    }))
    .await;
    ctx.expect_result(1036, json!({}), Some(session_id.as_str()));

    ctx.process_async(json!({
        "id": 1037,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {"url": "http://example.test/background-stop-loading"}
    }))
    .await;
    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["sessionId"], json!(session_id));
    assert_eq!(paused["params"]["resourceType"], json!("Document"));
    let network_id = paused["params"]["networkId"].clone();
    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
        assert!(
            !bc.background_target(&second_target_id)
                .expect("background target must exist")
                .fetch_owner
                .pending_state()
                .is_empty()
        );
    }

    ctx.process_async(json!({
        "id": 1038,
        "method": "Page.stopLoading",
        "sessionId": session_id,
    }))
    .await;
    ctx.expect_result(1038, json!({}), Some(session_id.as_str()));

    let navigation = take_response_by_id(&mut ctx, 1037);
    assert_eq!(navigation["sessionId"], json!(session_id));
    assert_eq!(navigation["error"]["message"], json!("Navigation stopped"));
    let failed = ctx.take_one();
    assert_eq!(failed["method"], json!("Network.loadingFailed"));
    assert_eq!(failed["sessionId"], json!(session_id));
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], json!("Navigation stopped"));
    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
        let background = bc
            .background_target(&second_target_id)
            .expect("background target should remain parked");
        assert_eq!(background.session_id(), Some(session_id.as_str()));
        assert!(
            bc.background_target(&second_target_id)
                .expect("background target must exist")
                .fetch_owner
                .pending_state()
                .is_empty()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_add_binding_and_preload_then_remove_on_auto_attached_background_target_session_prevent_first_navigation_replay_after_empty_slot_promotion()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 10360,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10360, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10361,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": { "name": "temporaryPreDocumentBinding" }
    }))
    .await;
    ctx.expect_result(10361, json!({}), Some(&session_id));

    ctx.process_async(json!({
            "id": 10362,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": session_id,
            "params": {
                "source": "globalThis.__lm_removed_pre_document_preload = 'ready'; if (typeof globalThis.temporaryPreDocumentBinding === 'function') globalThis.temporaryPreDocumentBinding('unexpected');"
            }
        }))
        .await;
    let add_script = take_response_by_id(&mut ctx, 10362);
    let identifier = add_script["result"]["identifier"]
        .as_str()
        .expect("preload identifier")
        .to_owned();

    ctx.process_async(json!({
        "id": 10363,
        "method": "Runtime.removeBinding",
        "sessionId": session_id,
        "params": { "name": "temporaryPreDocumentBinding" }
    }))
    .await;
    ctx.expect_result(10363, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 10364,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": { "identifier": identifier }
    }))
    .await;
    let remove_script = take_response_by_id(&mut ctx, 10364);
    assert_eq!(remove_script["result"], json!({}));

    ctx.process_async(json!({
        "id": 10365,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<body><div id='ok'>removed pre-document state</div></body>"
        }
    }))
    .await;
    let navigation = take_response_by_id(&mut ctx, 10365);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Runtime.bindingCalled")
                && message["method"] != json!("Runtime.executionContextCreated")
                && message["method"] != json!("Runtime.executionContextsCleared")
        }),
        "removed pre-document binding/preload should not replay during first navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10366,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "typeof globalThis.temporaryPreDocumentBinding"
        }
    }))
    .await;
    let kind = take_response_by_id(&mut ctx, 10366);
    assert_eq!(kind["result"]["result"]["value"], json!("undefined"));

    ctx.process_async(json!({
        "id": 10367,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "globalThis.__lm_removed_pre_document_preload ?? 'absent'"
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 10367);
    assert_eq!(preload["result"]["result"]["value"], json!("absent"));

    ctx.process_async(json!({
        "id": 10368,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": "document.getElementById('ok').textContent"
        }
    }))
    .await;
    let text = take_response_by_id(&mut ctx, 10368);
    assert_eq!(
        text["result"]["result"]["value"],
        json!("removed pre-document state")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_remove_binding_on_background_target_session_routes_without_promotion_when_active_target_has_no_loaded_page()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 1036,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(1036, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1037,
        "method": "Runtime.addBinding",
        "sessionId": session_id,
        "params": { "name": "patchedBinding" }
    }))
    .await;
    ctx.expect_result(1037, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 1038,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(1038, json!({}), None);

    ctx.process_async(json!({
        "id": 1039,
        "method": "Runtime.removeBinding",
        "sessionId": session_id,
        "params": { "name": "patchedBinding" }
    }))
    .await;
    ctx.expect_result(1039, json!({}), Some(&session_id));

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert_eq!(
        bc.background_target(&second_target_id)
            .and_then(|target| target.session_id()),
        Some(session_id.as_str())
    );
    assert!(
        ctx.conn
            .target_devtools_session_state_for_session(Some(&session_id))
            .is_none_or(|state| state
                .runtime_bindings
                .iter()
                .all(|binding| binding.name != "patchedBinding")),
        "binding definition should be removed from the background DevTools session without promotion"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_replay_only_their_own_pre_document_binding_and_preload_after_switching()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 10390,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": { "name": "targetABinding" }
    }))
    .await;
    ctx.expect_result(10390, json!({}), Some("SID-active"));

    ctx.process_async(json!({
            "id": 10391,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": "SID-active",
            "params": {
                "source": "globalThis.__lm_target_marker = 'A'; if (typeof globalThis.targetABinding === 'function') globalThis.targetABinding('payload-A');"
            }
        }))
        .await;
    let add_a = take_response_by_id(&mut ctx, 10391);
    assert!(add_a["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10392,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10392, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10393,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10393, json!({}), None);

    ctx.process_async(json!({
        "id": 10394,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": { "name": "targetBBinding" }
    }))
    .await;
    ctx.expect_result(10394, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 10395,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.__lm_target_marker = 'B'; if (typeof globalThis.targetBBinding === 'function') globalThis.targetBBinding('payload-B');"
            }
        }))
        .await;
    let add_b = take_response_by_id(&mut ctx, 10395);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10396,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10396, json!({}), None);

    ctx.process_async(json!({
        "id": 10397,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>target-a</title><div id='ok'>A page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let first_navigation = take_response_by_id(&mut ctx, 10397);
    assert_eq!(
        first_navigation["result"]["frameId"],
        json!("TID-000000000A")
    );
    let binding_called_a = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
        })
        .cloned()
        .expect("target A binding should replay on target A navigation");
    assert_eq!(binding_called_a["params"]["payload"], json!("payload-A"));
    assert_eq!(binding_called_a["sessionId"], json!("SID-active"));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
        }),
        "target B binding should not leak into target A navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 10398,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ marker: globalThis.__lm_target_marker, hasA: typeof globalThis.targetABinding, hasB: typeof globalThis.targetBBinding, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let eval_a = take_response_by_id(&mut ctx, 10398);
    let payload_a = eval_a["result"]["result"]["value"]
        .as_str()
        .expect("target A payload should be string");
    let payload_a: serde_json::Value =
        serde_json::from_str(payload_a).expect("target A payload should be valid json");
    assert_eq!(payload_a["marker"], json!("A"));
    assert_eq!(payload_a["hasA"], json!("function"));
    assert_eq!(payload_a["hasB"], json!("undefined"));
    assert_eq!(payload_a["text"], json!("A page"));

    ctx.process_async(json!({
        "id": 10399,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10399, json!({}), None);

    ctx.process_async(json!({
        "id": 10400,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b</title><div id='ok'>B page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 10400);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    let binding_called_b = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBBinding")
        })
        .cloned()
        .expect("target B binding should replay on target B navigation");
    assert_eq!(binding_called_b["params"]["payload"], json!("payload-B"));
    assert_eq!(binding_called_b["sessionId"], json!(second_session_id));
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetABinding")
        }),
        "target A binding should not leak into target B navigation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 10401,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ marker: globalThis.__lm_target_marker, hasA: typeof globalThis.targetABinding, hasB: typeof globalThis.targetBBinding, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let eval_b = take_response_by_id(&mut ctx, 10401);
    let payload_b = eval_b["result"]["result"]["value"]
        .as_str()
        .expect("target B payload should be string");
    let payload_b: serde_json::Value =
        serde_json::from_str(payload_b).expect("target B payload should be valid json");
    assert_eq!(payload_b["marker"], json!("B"));
    assert_eq!(payload_b["hasA"], json!("undefined"));
    assert_eq!(payload_b["hasB"], json!("function"));
    assert_eq!(payload_b["text"], json!("B page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_materialize_only_their_own_utility_pre_document_binding_and_preload_after_switching()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 10402,
        "method": "Runtime.addBinding",
        "sessionId": "SID-active",
        "params": {
            "name": "targetAUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10402, json!({}), Some("SID-active"));

    ctx.process_async(json!({
            "id": 10403,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": "SID-active",
            "params": {
                "source": "globalThis.__lm_target_utility_marker = 'A'; if (typeof globalThis.targetAUtilityBinding === 'function') globalThis.targetAUtilityBinding('payload-A-utility');",
                "worldName": "utility"
            }
        }))
        .await;
    let add_a = take_response_by_id(&mut ctx, 10403);
    assert!(add_a["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10404,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(10404, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 10405,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10405, json!({}), None);

    ctx.process_async(json!({
        "id": 10406,
        "method": "Runtime.addBinding",
        "sessionId": second_session_id,
        "params": {
            "name": "targetBUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    ctx.expect_result(10406, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
            "id": 10407,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": second_session_id,
            "params": {
                "source": "globalThis.__lm_target_utility_marker = 'B'; if (typeof globalThis.targetBUtilityBinding === 'function') globalThis.targetBUtilityBinding('payload-B-utility');",
                "worldName": "utility"
            }
        }))
        .await;
    let add_b = take_response_by_id(&mut ctx, 10407);
    assert!(add_b["result"]["identifier"].is_string());

    ctx.process_async(json!({
        "id": 10408,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(10408, json!({}), None);

    ctx.process_async(json!({
        "id": 10409,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>target-a-utility</title><div id='ok'>A utility page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let first_navigation = take_response_by_id(&mut ctx, 10409);
    assert_eq!(
        first_navigation["result"]["frameId"],
        json!("TID-000000000A")
    );
    let binding_called_a_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        }),
        "target B utility binding should not leak into target A navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10410,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-active",
        "params": {
            "frameId": "TID-000000000A",
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_a = take_response_by_id(&mut ctx, 10410)["result"]["executionContextId"]
        .as_i64()
        .expect("target A utility context id");
    let binding_called_a = binding_called_a_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetAUtilityBinding")
                })
                .cloned()
        })
        .expect("target A utility binding should replay when target A utility world materializes");
    assert_eq!(
        binding_called_a["params"]["payload"],
        json!("payload-A-utility")
    );
    assert_eq!(
        binding_called_a["params"]["executionContextId"],
        json!(utility_context_a)
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        }),
        "target B utility binding should not leak into target A utility world: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 10411,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "contextId": utility_context_a,
                "expression": "JSON.stringify({ marker: globalThis.__lm_target_utility_marker, hasA: typeof globalThis.targetAUtilityBinding, hasB: typeof globalThis.targetBUtilityBinding, title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let eval_a = take_response_by_id(&mut ctx, 10411);
    let payload_a = eval_a["result"]["result"]["value"]
        .as_str()
        .expect("target A utility payload should be string");
    let payload_a: serde_json::Value =
        serde_json::from_str(payload_a).expect("target A utility payload should be valid json");
    assert_eq!(payload_a["marker"], json!("A"));
    assert_eq!(payload_a["hasA"], json!("function"));
    assert_eq!(payload_a["hasB"], json!("undefined"));
    assert_eq!(payload_a["title"], json!("target-a-utility"));
    assert_eq!(payload_a["text"], json!("A utility page"));

    ctx.process_async(json!({
        "id": 10412,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(10412, json!({}), None);

    ctx.process_async(json!({
        "id": 10413,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-utility</title><div id='ok'>B utility page</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 10413);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    let binding_called_b_during_navigation = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetBUtilityBinding")
        })
        .cloned();
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak into target B navigation/materialization: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 10414,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility"
        }
    }))
    .await;
    let utility_context_b = take_response_by_id(&mut ctx, 10414)["result"]["executionContextId"]
        .as_i64()
        .expect("target B utility context id");
    let binding_called_b = binding_called_b_during_navigation
        .or_else(|| {
            ctx.sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("targetBUtilityBinding")
                })
                .cloned()
        })
        .expect("target B utility binding should replay when target B utility world materializes");
    assert_eq!(
        binding_called_b["params"]["payload"],
        json!("payload-B-utility")
    );
    assert_eq!(
        binding_called_b["params"]["executionContextId"],
        json!(utility_context_b)
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("targetAUtilityBinding")
        }),
        "target A utility binding should not leak into target B utility world: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
            "id": 10415,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "contextId": utility_context_b,
                "expression": "JSON.stringify({ marker: globalThis.__lm_target_utility_marker, hasA: typeof globalThis.targetAUtilityBinding, hasB: typeof globalThis.targetBUtilityBinding, title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let eval_b = take_response_by_id(&mut ctx, 10415);
    let payload_b = eval_b["result"]["result"]["value"]
        .as_str()
        .expect("target B utility payload should be string");
    let payload_b: serde_json::Value =
        serde_json::from_str(payload_b).expect("target B utility payload should be valid json");
    assert_eq!(payload_b["marker"], json!("B"));
    assert_eq!(payload_b["hasA"], json!("undefined"));
    assert_eq!(payload_b["hasB"], json!("function"));
    assert_eq!(payload_b["title"], json!("target-b-utility"));
    assert_eq!(payload_b["text"], json!("B utility page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_targets_do_not_replay_bare_isolated_worlds_after_switching() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    let first_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>first</body>")
        .await
        .expect("first target page should initialize");
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        let _ = bc
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(first_page));
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
    }

    ctx.process_async(json!({
        "id": 104150,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-active",
        "params": {
            "frameId": "TID-000000000A",
            "worldName": "utility-a"
        }
    }))
    .await;
    let create_a = take_response_by_id(&mut ctx, 104150);
    assert!(create_a["result"]["executionContextId"].as_i64().is_some());
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104151,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    ctx.expect_result(104151, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041511,
        "method": "Target.attachToTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1041511)["result"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 104152,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104152, json!({}), None);

    ctx.process_async(json!({
        "id": 104153,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second-target</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 104153);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();
    ctx.process_async(json!({
        "id": 10415041,
        "method": "Runtime.evaluate",
        "sessionId": second_session_id,
        "params": { "expression": "document.title" }
    }))
    .await;
    let second_promote = take_response_by_id(&mut ctx, 10415041);
    assert_eq!(
        second_promote["result"]["result"]["value"],
        json!("second-target")
    );
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 104154,
        "method": "Page.createIsolatedWorld",
        "sessionId": second_session_id,
        "params": {
            "frameId": second_target_id,
            "worldName": "utility-b"
        }
    }))
    .await;
    let create_b = take_response_by_id(&mut ctx, 104154);
    assert!(create_b["result"]["executionContextId"].as_i64().is_some());
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104155,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(104155, json!({}), None);
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    let target_a_replay_url =
        "data:text/html,<title>target-a-replay</title><div id='ok'>target a replay</div>";
    let target_a_commit = ctx
        .conn
        .prepare_loaded_navigation_commit_for_owner(&crate::conn::CommandOwnerScope::for_session(
            "SID-active",
        ))
        .expect("target A commit state should be available before navigation");
    assert!(
        target_a_commit.runtime_frontend_enabled,
        "target A commit state should keep Runtime enabled"
    );
    assert_eq!(
        target_a_commit
            .renderer_runtime_inspector_session_id
            .as_deref(),
        None,
        "target A primary session should use the target default renderer inspector session"
    );

    ctx.process_async(json!({
        "id": 104156,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": target_a_replay_url
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let first_replay = take_response_by_id(&mut ctx, 104156);
    assert_eq!(first_replay["result"]["frameId"], json!("TID-000000000A"));
    let first_worlds = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .filter_map(|message| message["params"]["context"]["name"].as_str())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(
        first_worlds
            .iter()
            .all(|name| name != "utility-a" && name != "utility-b"),
        "target A navigation must not recreate either document-scoped world: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104157,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104157, json!({}), None);
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 104158,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>target-b-replay</title><div id='ok'>target b replay</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_replay = take_response_by_id(&mut ctx, 104158);
    assert_eq!(second_replay["result"]["frameId"], json!(second_target_id));
    let second_worlds = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .filter_map(|message| message["params"]["context"]["name"].as_str())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(
        second_worlds
            .iter()
            .all(|name| name != "utility-a" && name != "utility-b"),
        "target B navigation must not recreate either document-scoped world: {:?}",
        ctx.sent
    );
}
