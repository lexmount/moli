use super::*;

/// cdp.target: closeTarget – no browser context
#[tokio::test(flavor = "multi_thread")]
async fn close_target_no_bc() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 10, "method": "Target.closeTarget",
                       "params": {"targetId": "X"}}))
        .await;
    ctx.expect_error(10, -31998, "BrowserContextNotLoaded");
}

/// cdp.target: closeTarget – no target
#[tokio::test(flavor = "multi_thread")]
async fn close_target_no_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 10, "method": "Target.closeTarget",
                       "params": {"targetId": "TID-8"}}))
        .await;
    ctx.expect_error(10, -31998, "TargetNotLoaded");
}

/// cdp.target: closeTarget – wrong target id
#[tokio::test(flavor = "multi_thread")]
async fn close_target_wrong_id() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.process_async(json!({"id": 10, "method": "Target.closeTarget",
                       "params": {"targetId": "TID-8"}}))
        .await;
    ctx.expect_error(10, -31998, "UnknownTargetId");
}

/// cdp.target: closeTarget – success
#[tokio::test(flavor = "multi_thread")]
async fn close_target_success() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.active_page_state_mut()
        .mutate_devtools_network_session_state(false, None, |network| {
            network.network_enabled = true;
            network.cache_disabled = true;
            network.bypass_service_worker = true;
            network.extra_headers = vec![("X-Test".into(), "1".into())];
        });
    bc.active_page_state_mut().css_enabled = true;
    bc.active_page_state_mut()
        .active_target
        .fetch_owner
        .configure(
            None,
            true,
            vec![crate::conn::FetchInterceptionPattern {
                url_pattern: "*".into(),
                resource_type_filter: None,
                request_stage: crate::conn::FetchRequestStage::Response,
            }],
        );
    bc.set_target_security_origin("https://old.example".into());
    bc.set_target_secure_context_type("InsecureScheme".into());
    bc.set_next_network_request_sequence_for_test(41);
    bc.set_subresource_network_emitted_record_count_for_test(12);
    bc.set_next_io_stream_sequence_for_test(7);
    bc.active_page_state_mut()
        .active_target
        .runtime_slot
        .set_next_subresource_fetch_request_id_for_test(5);
    bc.active_page_state_mut()
        .active_target
        .owner_state
        .target_crash_state
        .mark_crashed();
    bc.record_captured_response_body("REQ-old".into(), "body".into(), [None]);
    bc.insert_io_stream("STREAM-old".into(), b"body".to_vec(), 0);
    ctx.process_async(json!({"id": 11, "method": "Target.closeTarget",
                       "params": {"targetId": "TID-000000000A"}}))
        .await;
    ctx.expect_result(11, json!({ "success": true }), None);
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_target());
    assert!(!bc.has_active_session());
    assert!(!bc.has_loaded_page());
    assert!(bc.page_target("TID-000000000A").is_none());
    assert_eq!(bc.background_target_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_tab_id_closes_page_pair_and_detaches_tab_session() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 12010,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 12010)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 12011,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;
    let tab_session_id = take_response_by_id(&mut ctx, 12011)["result"]["sessionId"]
        .as_str()
        .expect("tab session id")
        .to_owned();
    assert_eq!(
        ctx.conn.attached_sessions_for_target(&tab_target_id),
        vec![tab_session_id.clone()]
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 12012,
        "method": "Target.closeTarget",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;

    ctx.expect_result(12012, json!({ "success": true }), None);
    let detached = ctx.take_first_matching("tab detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!(tab_target_id)
    });
    assert_eq!(detached["params"]["sessionId"], json!(tab_session_id));
    assert_eq!(ctx.conn.session_route(Some(&tab_session_id)), None);
    assert!(
        ctx.conn
            .attached_sessions_for_target(&tab_target_id)
            .is_empty()
    );
    assert_eq!(
        ctx.conn
            .primary_page_target_id_for_tab_target_id(&tab_target_id),
        None
    );

    ctx.process_async(json!({
        "id": 12013,
        "method": "Target.getTargets",
        "params": { "filter": [{}] }
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 12013);
    assert!(
        targets["result"]["targetInfos"]
            .as_array()
            .expect("targetInfos")
            .is_empty(),
        "closing tab target should close page target too: {targets:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn puppeteer_target_events_create_navigate_close_page_sequence() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 12100,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(12100, json!({}), None);
    assert!(ctx.sent.is_empty());

    ctx.process_async(json!({
        "id": 12101,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 12101)["result"]["targetId"]
        .as_str()
        .expect("page target id")
        .to_owned();
    let created = ctx.take_first_matching("Puppeteer page targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    assert_eq!(created["params"]["targetInfo"]["type"], json!("page"));
    assert_eq!(created["params"]["targetInfo"]["url"], json!("about:blank"));

    ctx.process_async(json!({
        "id": 12102,
        "method": "Target.attachToTarget",
        "params": { "targetId": page_target_id.clone() }
    }))
    .await;
    let page_session_id = take_response_by_id(&mut ctx, 12102)["result"]["sessionId"]
        .as_str()
        .expect("page session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);
    assert_eq!(
        ctx.conn.attached_sessions_for_target(&page_target_id),
        vec![page_session_id.clone()]
    );
    let attached_changed =
        ctx.take_first_matching("Puppeteer page attached targetInfoChanged", |message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
        });
    assert_eq!(
        attached_changed["params"]["targetInfo"]["attached"],
        json!(true)
    );
    assert!(ctx.sent.is_empty());

    let navigated_url = "data:text/html,<title>Puppeteer Target Events</title><main>changed</main>";
    ctx.process_async(json!({
        "id": 12103,
        "method": "Page.navigate",
        "sessionId": page_session_id.clone(),
        "params": { "url": navigated_url }
    }))
    .await;
    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "Puppeteer parsed title targetInfoChanged",
        |message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
                && message["params"]["targetInfo"]["title"] == json!("Puppeteer Target Events")
        },
    )
    .await;
    let changed = ctx.take_first_matching("Puppeteer page targetInfoChanged", |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
            && message["params"]["targetInfo"]["title"] == json!("Puppeteer Target Events")
    });
    assert_eq!(changed["params"]["targetInfo"]["type"], json!("page"));
    assert_eq!(
        changed["params"]["targetInfo"]["title"],
        json!("Puppeteer Target Events")
    );
    assert_eq!(changed["params"]["targetInfo"]["url"], json!(navigated_url));
    take_response_by_id(&mut ctx, 12103);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 12104,
        "method": "Target.closeTarget",
        "params": { "targetId": page_target_id.clone() }
    }))
    .await;

    assert_eq!(
        take_response_by_id(&mut ctx, 12104)["result"],
        json!({ "success": true })
    );
    ctx.take_first_matching("Puppeteer page targetDestroyed", |message| {
        message["method"] == json!("Target.targetDestroyed")
            && message["params"]["targetId"] == json!(page_target_id)
    });
    assert_eq!(ctx.conn.session_route(Some(&page_session_id)), None);
    assert!(
        ctx.conn
            .attached_sessions_for_target(&page_target_id)
            .is_empty()
    );

    ctx.process_async(json!({
        "id": 12105,
        "method": "Target.getTargets"
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 12105);
    assert!(
        targets["result"]["targetInfos"]
            .as_array()
            .expect("targetInfos")
            .is_empty(),
        "closed Puppeteer page target should not remain discoverable: {targets:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_emits_detached_events() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    let session_id = "SID-000000000A".to_owned();
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.attach_active_session(session_id.clone());
    assert!(bc.assign_auxiliary_session_to_target("TID-000000000A", "SID-aux".into()));
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;

    ctx.process_async(json!({"id": 12, "method": "Target.closeTarget",
                       "params": {"targetId": "TID-000000000A"}}))
        .await;

    ctx.expect_result(12, json!({ "success": true }), None);
    let primary_inspector = ctx.take_one();
    assert_eq!(primary_inspector["method"], "Inspector.detached");
    assert_eq!(primary_inspector["sessionId"], session_id);
    assert_eq!(
        primary_inspector["params"]["reason"],
        "Render process gone."
    );
    let auxiliary_inspector = ctx.take_one();
    assert_eq!(auxiliary_inspector["method"], "Inspector.detached");
    assert_eq!(auxiliary_inspector["sessionId"], "SID-aux");
    assert_eq!(
        auxiliary_inspector["params"]["reason"],
        "Render process gone."
    );
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000A",
            "sessionId": session_id
        })),
    );
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000A",
            "sessionId": "SID-aux"
        })),
    );
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.auxiliary_target_id_for_session("SID-aux").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_without_inspector_enabled_emits_inspector_detached_event() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-9".into());
    bc.set_active_target_id("TID-000000000A");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 29,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(29, json!({ "success": true }), None);

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.detached");
    assert_eq!(inspector["sessionId"], "SID-1");
    assert_eq!(inspector["params"]["reason"], "Render process gone.");

    let detached = ctx.take_one();
    assert_eq!(detached["method"], "Target.detachedFromTarget");
    assert_eq!(detached["params"]["targetId"], "TID-000000000A");
    assert_eq!(detached["params"]["sessionId"], "SID-1");
    assert!(ctx.take_all().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_invalidates_runtime_context_and_object_without_active_page_fallback() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-runtime-close".into());
    bc.set_active_target_id("TID-active");
    bc.attach_active_session("SID-active");
    let background_target = crate::conn::PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );
    bc.insert_page_target_host(background_target);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><body><script>globalThis.__lm_closed_target_marker = 'active-clean';</script>active</body></html>",
        Some("SID-active"),
    )
    .await;
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><body><script>globalThis.__lm_closed_target_marker = 'background-clean';</script>background</body></html>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(40, json!({}), Some("SID-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 41,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-background",
        "params": {
            "frameId": "TID-background",
            "worldName": "close-target-utility"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 41)["result"]["executionContextId"]
        .as_i64()
        .expect("isolated execution context id");

    ctx.process_async(json!({
        "id": 42,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "contextId": isolated_context_id,
            "expression": "globalThis.__lm_closed_target_marker = 'background-isolated'; ({ owner: 'background' })"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 42)["result"]["result"]["objectId"]
        .as_str()
        .expect("background isolated-world object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 43,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-background" }
    }))
    .await;
    ctx.expect_result(43, json!({ "success": true }), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-background",
            "sessionId": "SID-background"
        })),
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 44,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "contextId": isolated_context_id,
            "expression": "globalThis.__lm_closed_target_marker = 'active-mutated-by-stale-context'; 'wrong';",
            "returnByValue": true
        }
    }))
    .await;
    let stale_context_response = take_response_by_id(&mut ctx, 44);
    assert_eq!(stale_context_response["error"]["code"], json!(-32000));
    assert_eq!(
        stale_context_response["error"]["message"],
        json!("Cannot find context with specified id")
    );

    ctx.process_async(json!({
        "id": 45,
        "method": "Runtime.callFunctionOn",
        "sessionId": "SID-active",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { globalThis.__lm_closed_target_marker = 'active-mutated-by-stale-object'; return this.owner; }",
            "returnByValue": true
        }
    }))
    .await;
    let stale_object_response = take_response_by_id(&mut ctx, 45);
    assert_eq!(stale_object_response["error"]["code"], json!(-32000));
    assert_eq!(
        stale_object_response["error"]["message"],
        json!("Cannot find context with specified id")
    );

    ctx.process_async(json!({
        "id": 46,
        "method": "Runtime.evaluate",
        "sessionId": "SID-active",
        "params": {
            "expression": "globalThis.__lm_closed_target_marker",
            "returnByValue": true
        }
    }))
    .await;
    let active_probe = take_response_by_id(&mut ctx, 46);
    assert_eq!(
        active_probe["result"]["result"]["value"],
        json!("active-clean"),
        "closed target context/object ids must not fall back into the active page"
    );

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-active"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert!(bc.background_target("TID-background").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn close_background_target_emits_detached_events_and_clears_attached_sessions() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    push_background_target(
        &mut ctx,
        "TID-000000000B",
        "about:blank#background",
        Some("SID-bg"),
    );
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    assert!(bc.assign_auxiliary_session_to_target("TID-000000000B", "SID-aux".into()));
    bc.mutate_parked_page_session_state("TID-000000000B", |state| {
        state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary] =
            crate::conn::DevToolsSessionState {
                runtime_session_state: crate::conn::TargetRuntimeSessionState {
                    inspector_enabled: true,
                    ..Default::default()
                },
                ..Default::default()
            };
    });

    ctx.process_async(json!({
        "id": 121,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000B" }
    }))
    .await;

    ctx.expect_result(121, json!({ "success": true }), None);
    ctx.expect_event(
        "Inspector.detached",
        Some(&json!({ "reason": "Render process gone." })),
    );
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000B",
            "sessionId": "SID-bg"
        })),
    );
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000B",
            "sessionId": "SID-aux"
        })),
    );

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.background_target("TID-000000000B").is_none());
    assert!(bc.auxiliary_target_id_for_session("SID-aux").is_none());
    assert!(bc.parked_page_session_state("TID-000000000B").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_aborts_paused_request_stage_navigation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>target-close</body></html>",
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
    bc.active_page_state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_page_state_mut()
        .active_target
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
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(22, json!({ "success": true }), None);

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Target closed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 21);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Target closed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.detached");

    let target = ctx.take_one();
    assert_eq!(target["method"], "Target.detachedFromTarget");
    assert_eq!(target["params"]["targetId"], "TID-000000000A");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_target());
    assert!(!bc.has_active_session());
    assert!(!bc.has_loaded_page());
    assert!(bc.page_target("TID-000000000A").is_none());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_aborts_paused_runtime_fetch_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "ok")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
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
        "sessionId": "SID-1"
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
            "expression": r#"(() => {
  globalThis.__lm_target_close_fetch = "pending";
  fetch('/api')
.then(response => response.text())
.then(text => { globalThis.__lm_target_close_fetch = text; })
.catch(() => { globalThis.__lm_target_close_fetch = "failed"; });
  return "scheduled";
})()"#
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
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .cloned()
        .expect("subresource fetch requestPaused event");
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 26,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(26, json!({ "success": true }), None);

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["errorText"], "Target closed");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_target());
    assert!(!bc.has_active_session());
    assert!(!bc.has_loaded_page());
    assert!(bc.page_target("TID-000000000A").is_none());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_aborts_paused_response_stage_runtime_xhr_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-target-close", "ok"),
            ],
            "xhr-ok",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", any(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
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
        "id": 27,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(27, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 28,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(28, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 29,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/xhr');
  xhr.onload = () => {};
  xhr.onerror = () => {};
  xhr.send('payload');
  return "scheduled";
})()"#
        }
    }))
    .await;
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(29))
        .expect("runtime evaluate response");
    ctx.sent.remove(pos);

    crate::testing::wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "subresource xhr response-stage requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
                && message["params"]["responseStatusCode"] == json!(200)
        },
    )
    .await;

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(xhr_url)
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .cloned()
        .expect("subresource xhr response-stage requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 30,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(30, json!({ "success": true }), None);

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Target closed");

    ctx.process_async(json!({
        "id": 31,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(31, -32001, "Unknown sessionId");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_target());
    assert!(!bc.has_active_session());
    assert!(!bc.has_loaded_page());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_aborts_paused_runtime_xhr_auth_subresource() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ready</body></html>",
        )
    }

    async fn protected(headers: HeaderMap) -> impl IntoResponse {
        let expected = "Basic dXNlcjpwYXNz";
        match headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "xhr secret",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"xhr-area\"")],
                "auth required",
            )
                .into_response(),
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/protected", any(protected)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let protected_url = format!("http://{addr}/protected");
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
        "id": 32,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(32, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 33,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(33, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 34,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/protected');
  xhr.onerror = () => {};
  xhr.send();
  return "scheduled";
})()"#
        }
    }))
    .await;
    let pos = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(34))
        .expect("runtime evaluate response");
    ctx.sent.remove(pos);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(protected_url)
        })
        .cloned()
        .expect("subresource xhr request-stage requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 35,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(35, json!({}), Some("SID-1"));
    crate::testing::wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "subresource xhr authRequired event before target close",
        |message| {
            message["method"] == json!("Fetch.authRequired")
                && message["params"]["requestId"].as_str() == Some(request_id.as_str())
        },
    )
    .await;

    let auth_required = ctx.take_first_matching(
        "subresource xhr authRequired event before target close",
        |message| {
            message["method"] == json!("Fetch.authRequired")
                && message["params"]["requestId"].as_str() == Some(request_id.as_str())
        },
    );
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], request_id);
    assert!(auth_required["params"].get("networkId").is_none());
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 36,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-000000000A" }
    }))
    .await;
    ctx.expect_result(36, json!({ "success": true }), None);

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["errorText"], "Target closed");

    ctx.process_async(json!({
        "id": 37,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "user",
                "password": "pass"
            }
        }
    }))
    .await;
    ctx.expect_error(37, -32001, "Unknown sessionId");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_target());
    assert!(!bc.has_active_session());
    assert!(!bc.has_loaded_page());

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 15, "method": "Target.activateTarget",
                       "params": {"targetId": "TID-1"}}))
        .await;
    ctx.expect_error(15, -31998, "BrowserContextNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_requires_loaded_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 16, "method": "Target.activateTarget",
                       "params": {"targetId": "TID-1"}}))
        .await;
    ctx.expect_error(16, -31998, "TargetNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_rejects_unknown_target_id() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000B");
    ctx.process_async(json!({"id": 17, "method": "Target.activateTarget",
                       "params": {"targetId": "TID-8"}}))
        .await;
    ctx.expect_error(17, -31998, "UnknownTargetId");
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_returns_empty_result_for_known_target() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000B");
    ctx.process_async(json!({"id": 18, "method": "Target.activateTarget",
                       "params": {"targetId": "TID-000000000B"}}))
        .await;
    ctx.expect_result(18, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_promotes_background_target_into_active_slot() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    push_background_target(&mut ctx, "TID-000000000B", "about:blank", None);

    ctx.process_async(json!({"id": 19, "method": "Target.activateTarget",
                       "params": {"targetId": "TID-000000000B"}}))
        .await;
    ctx.expect_result(19, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000B"));
    assert_eq!(bc.background_target_count(), 1);
    assert_eq!(
        bc.background_target_at(0).unwrap().target_id(),
        "TID-000000000A"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_targets_includes_background_targets() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    push_background_target(&mut ctx, "TID-000000000B", "about:blank", Some("SID-2"));

    ctx.process_async(json!({"id": 20, "method": "Target.getTargets"}))
        .await;
    let result = ctx.take_one();
    let infos = result["result"]["targetInfos"]
        .as_array()
        .expect("target info array");
    assert_eq!(infos.len(), 2);
    assert!(
        infos
            .iter()
            .any(|info| { info["targetId"] == "TID-000000000A" && info["attached"] == false })
    );
    assert!(
        infos
            .iter()
            .any(|info| { info["targetId"] == "TID-000000000B" && info["attached"] == true })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_targets_reports_pending_initial_document_page_target_info() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-targets-pending",
        "TID-loaded-for-targets",
        "<main>loaded</main>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .stage_background_target(
            "TID-pending-targets".to_owned(),
            None,
            "about:blank#pending-targets".to_owned(),
            None,
            None,
        );

    ctx.process_async(json!({"id": 20120, "method": "Target.getTargets"}))
        .await;

    let result = ctx.take_response_by_id(20120);
    let target_infos = result["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos array");
    let pending = target_infos
        .iter()
        .find(|info| info["targetId"] == json!("TID-pending-targets"))
        .expect("pending initial document target should be reported");
    assert_eq!(pending["type"], json!("page"));
    assert_eq!(pending["url"], json!("about:blank#pending-targets"));
    assert_eq!(pending["title"], json!(""));
    assert_eq!(pending["attached"], json!(false));
}

/// cdp.target: getTargetInfo – no params returns browser info
#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_no_params() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 9, "method": "Target.getTargetInfo"}))
        .await;
    ctx.expect_result(
        9,
        json!({
            "targetInfo": {
                "targetId": "browser",
                "type": "browser",
                "title": "",
                "url": "",
                "attached": true,
                "canAccessOpener": false,
            }
        }),
        None,
    );
}

/// cdp.target: getTargetInfo – unknown target id
#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_wrong_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 10, "method": "Target.getTargetInfo",
                       "params": {"targetId": "X"}}))
        .await;
    ctx.expect_error(10, -31998, "TargetNotLoaded");
}

/// cdp.target: getTargetInfo – known target
#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_known_target() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000C");
    ctx.process_async(json!({"id": 11, "method": "Target.getTargetInfo",
                       "params": {"targetId": "TID-000000000C"}}))
        .await;
    ctx.expect_result(
        11,
        json!({
            "targetInfo": {
                "targetId": "TID-000000000C",
                "type": "page",
                "title": "",
                "url": "about:blank",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-9",
            }
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_reports_pending_initial_document_page_target_info() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-target-info-pending",
        "TID-loaded-for-info",
        "<main>loaded</main>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .stage_background_target(
            "TID-pending-info".to_owned(),
            None,
            "about:blank#pending-info".to_owned(),
            None,
            None,
        );

    ctx.process_async(json!({"id": 20121, "method": "Target.getTargetInfo",
                       "params": {"targetId": "TID-pending-info"}}))
        .await;

    ctx.expect_result(
        20121,
        json!({
            "targetInfo": {
                "targetId": "TID-pending-info",
                "type": "page",
                "title": "",
                "url": "about:blank#pending-info",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-target-info-pending",
            }
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_targets_reports_loaded_document_title() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000C",
        "<html><head><title>Hello CDP</title></head><body></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 12, "method": "Target.getTargets"}))
        .await;
    ctx.expect_result(
        12,
        json!({
            "targetInfos": [{
                "targetId": "TID-000000000C",
                "type": "page",
                "title": "Hello CDP",
                "url": "data:text/html,<html><head><title>Hello CDP</title></head><body></body></html>",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-9",
            }]
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_discover_targets_reports_pending_initial_document_page_target_created() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-discover-pending",
        "TID-loaded-for-discover",
        "<main>loaded</main>",
    )
    .await;
    ctx.conn.set_root_target_discovery_enabled(false);
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .stage_background_target(
            "TID-pending-discover".to_owned(),
            None,
            "about:blank#pending-discover".to_owned(),
            None,
            None,
        );

    ctx.process_async(json!({
        "id": 20122,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;

    ctx.expect_result(20122, json!({}), None);
    assert!(
        ctx.conn.root_target_discovery_enabled(),
        "successful discovery enable should remain enabled"
    );
    let pending_created = ctx.take_first_matching("pending targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!("TID-pending-discover")
    });
    assert_eq!(
        pending_created["params"]["targetInfo"]["url"],
        json!("about:blank#pending-discover")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_reports_loaded_document_title() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000C",
        "<html><head><title>Hello TargetInfo</title></head><body></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 13, "method": "Target.getTargetInfo",
                       "params": {"targetId": "TID-000000000C"}}))
        .await;
    ctx.expect_result(
        13,
        json!({
            "targetInfo": {
                "targetId": "TID-000000000C",
                "type": "page",
                "title": "Hello TargetInfo",
                "url": "data:text/html,<html><head><title>Hello TargetInfo</title></head><body></body></html>",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-9",
            }
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_to_target_wraps_nested_result_in_received_message_event() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-9");

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.sendMessageToTarget",
        "params": {
            "message": serde_json::to_string(&json!({
                "id": 77,
                "method": "Target.getBrowserContexts"
            }))
            .expect("nested message json"),
            "sessionId": "SID-9"
        }
    }))
    .await;

    ctx.expect_result(14, json!({}), None);
    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.receivedMessageFromTarget");
    assert_eq!(event["params"]["sessionId"], "SID-9");
    let nested: serde_json::Value =
        serde_json::from_str(event["params"]["message"].as_str().expect("nested message"))
            .expect("nested payload should be valid json");
    assert_eq!(nested["id"], 77);
    assert_eq!(nested["result"]["browserContextIds"], json!(["BID-9"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn async_send_message_to_target_wraps_nested_page_navigation() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-9");

    ctx.process_async(json!({
        "id": 1401,
        "method": "Target.sendMessageToTarget",
        "params": {
            "message": serde_json::to_string(&json!({
                "id": 7701,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<html><body>async target nav</body></html>"
                }
            }))
            .expect("nested message json"),
            "sessionId": "SID-9"
        }
    }))
    .await;

    ctx.expect_result(1401, json!({}), None);
    let nested_result = ctx
        .sent
        .iter()
        .filter(|event| event["method"] == json!("Target.receivedMessageFromTarget"))
        .find_map(|event| {
            let message = event["params"]["message"].as_str()?;
            let nested = serde_json::from_str::<serde_json::Value>(message).ok()?;
            (nested["id"] == json!(7701) && nested.get("result").is_some()).then_some(nested)
        })
        .expect("nested Page.navigate result should be wrapped in a Target event");
    assert_eq!(nested_result["result"]["frameId"], "TID-000000000A");
    assert!(
        nested_result["result"]["loaderId"]
            .as_str()
            .is_some_and(|loader_id| !loader_id.is_empty()),
        "nested navigation result should include a loader id"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_to_target_preserves_nested_scheduler_events() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-9");

    let raw = json!({
        "id": 1402,
        "method": "Target.sendMessageToTarget",
        "params": {
            "message": serde_json::to_string(&json!({
                "id": 7702,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<html><body>nested scheduler event</body></html>"
                }
            }))
            .expect("nested message json"),
            "sessionId": "SID-9"
        }
    })
    .to_string();

    let outcome = ctx.conn.process_message_with_turn_outcome_async(&raw).await;
    let (messages, scheduler_events) = ctx.route_completed_command_outcome_for_test(outcome).await;

    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(1402) && message["result"] == json!({})),
        "outer Target.sendMessageToTarget response should be present: {messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            if message["method"] != json!("Target.receivedMessageFromTarget") {
                return false;
            }
            let Some(nested_raw) = message["params"]["message"].as_str() else {
                return false;
            };
            let Ok(nested) = serde_json::from_str::<serde_json::Value>(nested_raw) else {
                return false;
            };
            nested["id"] == json!(7702) && nested.get("result").is_some()
        }),
        "nested Page.navigate result should be wrapped in a Target event: {messages:?}"
    );
    assert!(
        scheduler_events.iter().any(|event| matches!(
            event,
            crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                if work.kind()
                    == crate::domains::activity::ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
        )),
        "nested Page.navigate deferred scheduler event should be returned to the outer turn: {scheduler_events:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_to_target_without_session_errors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");

    ctx.process_async(json!({
        "id": 15,
        "method": "Target.sendMessageToTarget",
        "params": {
            "message": "{\"id\":1,\"method\":\"Target.getTargetInfo\"}",
            "sessionId": "SID-9"
        }
    }))
    .await;

    ctx.expect_error(15, -31998, "InvalidSessionId");
}
