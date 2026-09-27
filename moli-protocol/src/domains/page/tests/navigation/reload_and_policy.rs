use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn reload_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 13, "method": "Page.reload"}))
        .await;
    ctx.expect_error(13, -31998, "BrowserContextNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_without_target_loaded_errors() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({"id": 14, "method": "Page.reload"}))
        .await;
    ctx.expect_error(14, -31998, "TargetNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_reloads_current_url_and_returns_empty_result() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn page(
        axum::extract::State(counter): axum::extract::State<Arc<AtomicUsize>>,
    ) -> impl axum::response::IntoResponse {
        let next = counter.fetch_add(1, Ordering::SeqCst) + 1;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{next}</body></html>"),
        )
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_counter = counter.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .with_state(server_counter),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 239,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let navigate = take_response_by_id(&mut ctx, 239);
    assert_eq!(
        navigate["result"],
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID })
    );

    let first_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        first_html.contains(">1<"),
        "expected first navigation html to contain counter 1, got {first_html}"
    );

    ctx.process_async(json!({
        "id": 241,
        "method": "Page.reload",
        "sessionId": "SID-1"
    }))
    .await;
    let reload = take_response_by_id(&mut ctx, 241);
    assert_eq!(reload["result"], json!({}));

    let second_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        second_html.contains(">2<"),
        "expected reloaded html to contain counter 2, got {second_html}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_csp_controls_response_policy_across_reload() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("content-security-policy", "script-src 'none'"),
            ],
            r#"<!doctype html>
<title>CSP Locked</title>
<script>
globalThis.__cspRan = true;
document.title = "CSP Bypassed";
</script>"#,
        )
    }

    async fn evaluate_csp_state(ctx: &mut TestContext, id: u64) -> serde_json::Value {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-CSP",
            "params": {
                "expression": "[globalThis.__cspRan === true, document.title]",
                "returnByValue": true
            }
        }))
        .await;
        take_response_by_id(ctx, id)["result"]["result"]["value"].clone()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/csp", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-CSP", "TID-CSP", "SID-CSP", "about:blank");
    ctx.process_async(json!({
        "id": 24_000,
        "method": "Page.enable",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_000);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_001,
        "method": "Page.navigate",
        "sessionId": "SID-CSP",
        "params": { "url": format!("http://{addr}/csp") }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_001);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_002).await,
        json!([false, "CSP Locked"])
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_003,
        "method": "Page.setBypassCSP",
        "sessionId": "SID-CSP",
        "params": { "enabled": true }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_003);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 24_004,
        "method": "Page.reload",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_004);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_005).await,
        json!([true, "CSP Bypassed"])
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_006,
        "method": "Page.setBypassCSP",
        "sessionId": "SID-CSP",
        "params": { "enabled": false }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_006);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 24_007,
        "method": "Page.reload",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_007);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_008).await,
        json!([false, "CSP Locked"])
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_response_csp_controls_parser_and_dynamic_inline_scripts() {
    async fn parent() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><iframe src="/child"></iframe>"#,
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("content-security-policy", "script-src 'nonce-allowed'"),
            ],
            r#"<!doctype html><body>
<script nonce="allowed">
globalThis.__allowedParserInlineRan = true;
globalThis.__inlineCspViolations = [];
addEventListener("securitypolicyviolation", event => {
  __inlineCspViolations.push(`${event.effectiveDirective}:${event.disposition}`);
});
</script>
<script>globalThis.__blockedParserInlineRan = true;</script>
<script nonce="allowed">
const script = document.createElement("script");
script.textContent = "globalThis.__blockedDynamicInlineRan = true";
document.body.append(script);
const allowedScript = document.createElement("script");
allowedScript.nonce = "allowed";
allowedScript.textContent = "globalThis.__allowedDynamicInlineRan = true";
document.body.append(allowedScript);
allowedScript.nonce = "changed-after-connection";
</script>
</body>"#,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(parent))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-CHILD-CSP",
        "TID-CHILD-CSP",
        "SID-CHILD-CSP",
        "about:blank",
    );
    ctx.process_async(json!({
        "id": 24_100,
        "method": "Page.enable",
        "sessionId": "SID-CHILD-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_100);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_101,
        "method": "Page.navigate",
        "sessionId": "SID-CHILD-CSP",
        "params": { "url": format!("http://{addr}/parent") }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_101);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CHILD-CSP").await;
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 24_102).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_103,
        "method": "Runtime.enable",
        "sessionId": "SID-CHILD-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_103);
    let child_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_104,
        "method": "Runtime.evaluate",
        "sessionId": "SID-CHILD-CSP",
        "params": {
            "contextId": child_context_id,
            "expression": "[globalThis.__blockedParserInlineRan === true, globalThis.__allowedParserInlineRan === true, globalThis.__blockedDynamicInlineRan === true, globalThis.__allowedDynamicInlineRan === true, globalThis.__inlineCspViolations]",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 24_104)["result"]["result"]["value"],
        json!([
            false,
            true,
            false,
            true,
            ["script-src-elem:enforce", "script-src-elem:enforce"]
        ])
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_targets_background_owner_without_activation() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn page(
        axum::extract::State(counter): axum::extract::State<Arc<AtomicUsize>>,
    ) -> impl axum::response::IntoResponse {
        let next = counter.fetch_add(1, Ordering::SeqCst) + 1;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{next}</body></html>"),
        )
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_counter = counter.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .with_state(server_counter),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let page_url = format!("http://{addr}/page");
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        page_url.clone(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<title>Active</title><main>active</main>".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
        .await;
    let initial_html = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|browser_context| {
            browser_context
                .background_target_mut("TID-background")
                .and_then(PageTargetHost::loaded_page_mut)
        })
        .expect("loaded background page")
        .serialize_html_async()
        .await
        .expect("background page should serialize HTML");
    assert!(
        initial_html.contains(">1<"),
        "expected first background load to contain counter 1"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 242,
        "method": "Page.reload",
        "sessionId": "SID-background"
    }))
    .await;
    let reload = take_response_by_id(&mut ctx, 242);
    assert_eq!(reload["sessionId"], json!("SID-background"));
    assert_eq!(reload["result"], json!({}));

    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Page.reload should not activate the target"
    );
    let background = browser_context
        .background_target_mut("TID-background")
        .expect("background target should remain background");
    let reloaded_html = background
        .loaded_page_mut()
        .expect("loaded background page after reload")
        .serialize_html_async()
        .await
        .expect("background page should serialize HTML");
    assert!(
        reloaded_html.contains(">2<"),
        "expected background reload html to contain counter 2, got {reloaded_html}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_after_crash_emits_target_reloaded_after_crash() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>reload-after-crash</body>",
    );
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();
    bc.active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 248,
        "method": "Page.reload",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 248);

    let events = ctx.take_all();
    assert!(events.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_crash_emits_target_reloaded_after_crash() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>before-crash</body>",
    );
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();
    bc.active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 249,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>after-crash</body>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 249);

    let events = ctx.take_all();
    assert!(events.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
    assert_eq!(
        loaded_page_html_for_test(&mut ctx).await,
        "<html><head></head><body>after-crash</body></html>"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_crash_without_inspector_enabled_clears_crash_without_event() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>before-crash</body>",
    );
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 250,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>after-crash</body>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 250);

    let events = ctx.take_all();
    assert!(
        !events
            .iter()
            .any(|message| message["method"] == json!("Inspector.targetReloadedAfterCrash"))
    );
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}
