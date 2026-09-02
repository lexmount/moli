use super::*;

async fn install_network_session_page(ctx: &mut TestContext, url: &str) {
    let mut browser_context = BrowserContext::new("BID-navigation".into());
    browser_context.set_active_target_id("TID-navigation");
    browser_context.attach_active_session("SID-navigation");
    ctx.conn.browser_context = Some(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async(url)
        .await
        .expect("the target should have a committed document");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);
}

/// Network.enable without a browser context fails.
#[tokio::test(flavor = "multi_thread")]
async fn enable_no_bc_error() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 1, "method": "Network.enable"}))
        .await;
    ctx.expect_error(1, -31998, "BrowserContextNotLoaded");
}
/// Network.enable with a browser context succeeds.
#[tokio::test(flavor = "multi_thread")]
async fn enable_with_bc_succeeds() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));
    ctx.process_async(json!({"id": 1, "method": "Network.enable"}))
        .await;
    ctx.expect_result(1, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .runtime_slot
            .primary_network_events_enabled()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_configuration_commands_succeed_while_the_target_is_changing_documents() {
    let mut ctx = TestContext::new();
    install_network_session_page(&mut ctx, "data:text/html,committed").await;
    ctx.conn
        .start_document_navigation_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-navigation"),
            "LID-pending".to_owned(),
        )
        .expect("the replacement navigation should start");

    ctx.process_async(json!({
        "id": 2,
        "method": "Network.enable",
        "sessionId": "SID-navigation",
    }))
    .await;

    ctx.expect_result(2, json!({}), Some("SID-navigation"));

    for (id, method, params) in [
        (
            3,
            "Network.setCacheDisabled",
            json!({ "cacheDisabled": true }),
        ),
        (
            4,
            "Network.setBypassServiceWorker",
            json!({ "bypass": true }),
        ),
        (
            5,
            "Network.setExtraHTTPHeaders",
            json!({ "headers": { "X-During-Navigation": "current" } }),
        ),
        (
            6,
            "Network.setBlockedURLs",
            json!({ "urls": ["*://blocked.example/*"] }),
        ),
        (
            7,
            "Network.emulateNetworkConditions",
            json!({
                "offline": true,
                "latency": 0,
                "downloadThroughput": -1,
                "uploadThroughput": -1,
            }),
        ),
        (
            8,
            "Network.setUserAgentOverride",
            json!({ "userAgent": "Moli/During-Navigation" }),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": "SID-navigation",
            "params": params,
        }))
        .await;
        ctx.expect_result(id, json!({}), Some("SID-navigation"));
    }

    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .runtime_slot
            .primary_network_events_enabled()
    );
    let configuration = ctx
        .conn
        .prepared_document_commit_configuration_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-navigation"),
            &url::Url::parse("data:text/html,committed").unwrap(),
        )
        .expect("commit configuration should resolve the target resource runtime");
    assert!(configuration.cache_disabled);
    assert!(configuration.bypass_service_worker);
    assert!(configuration.network_offline);
    assert_eq!(
        configuration.extra_http_headers,
        [("X-During-Navigation".to_owned(), "current".to_owned())]
    );
    assert_eq!(
        configuration.blocked_url_patterns,
        ["*://blocked.example/*".to_owned()]
    );
    assert_eq!(
        moli_core::network::ResourceRequestClient::from_browser_resource_runtime(
            configuration.browser_resource_runtime,
        )
        .user_agent(),
        "Moli/During-Navigation",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_configuration_completion_does_not_restore_a_replaced_document() {
    let mut ctx = TestContext::new();
    install_network_session_page(&mut ctx, "data:text/html,<body id=outgoing>outgoing</body>")
        .await;

    let raw = json!({
        "id": 20,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-navigation",
        "params": { "headers": { "X-Replacement-Race": "configured" } },
    })
    .to_string();
    let crate::conn::CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw)
    else {
        panic!("the loaded Page should receive the Network configuration command");
    };
    let completed = pending.wait().await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.navigate",
        "sessionId": "SID-navigation",
        "params": {
            "url": "data:text/html,<body id=replacement>replacement</body>"
        },
    }))
    .await;
    let navigate = ctx.take_response_by_id(21);
    assert!(navigate["result"]["loaderId"].is_string());

    let crate::conn::CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("the settled Network command should complete in one protocol phase");
    };
    assert!(outcome.into_parts().0.iter().any(|message| {
        message["id"] == json!(20)
            && message["sessionId"] == json!("SID-navigation")
            && message["result"] == json!({})
    }));

    let html = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .loaded_page_mut()
        .expect("the replacement Page should remain installed")
        .serialize_html_async()
        .await
        .expect("the replacement Page should remain usable");
    assert!(html.contains("id=\"replacement\""));
    assert!(!html.contains("id=\"outgoing\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn commit_configuration_resolves_the_exact_target_network_runtime() {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-runtime".into());
    browser_context.set_active_target_id("TID-a");
    browser_context.attach_active_session("SID-a");
    browser_context.insert_page_target_host(PageTargetHost::with_url(
        "TID-b".to_owned(),
        Some("SID-b".to_owned()),
        "about:blank".to_owned(),
    ));
    ctx.conn.browser_context = Some(browser_context);

    for (id, session_id, user_agent) in [
        (30, "SID-a", "Moli/Target-A"),
        (31, "SID-b", "Moli/Target-B"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Network.setUserAgentOverride",
            "sessionId": session_id,
            "params": { "userAgent": user_agent },
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }

    for (session_id, expected_user_agent) in [
        ("SID-b", "Moli/Target-B"),
        ("SID-a", "Moli/Target-A"),
        ("SID-b", "Moli/Target-B"),
    ] {
        let configuration = ctx
            .conn
            .prepared_document_commit_configuration_for_owner(
                &crate::conn::CommandOwnerScope::for_session(session_id),
                &url::Url::parse("about:blank").unwrap(),
            )
            .expect("the target-specific resource runtime should resolve");
        let request_client =
            moli_core::network::ResourceRequestClient::from_browser_resource_runtime(
                configuration.browser_resource_runtime,
            );
        assert_eq!(request_client.user_agent(), expected_user_agent);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_network_enable_does_not_enable_primary_session() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 10_101,
        "method": "Network.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(10_101, json!({}), Some("SID-aux"));

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        !bc.active_page_target()
            .runtime_slot
            .primary_network_events_enabled(),
        "auxiliary Network.enable must not enable the primary session"
    );
    assert!(bc.has_network_event_listeners());
    assert_eq!(
        bc.network_event_session_ids(Some("SID-primary")),
        vec![Some("SID-aux".to_owned())]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_network_policy_aggregates_enabled_sessions_like_chromium_handlers() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    ctx.conn.browser_context = Some(bc);

    for (id, session_id) in [(20_001, "SID-primary"), (20_002, "SID-aux")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Network.enable",
            "sessionId": session_id,
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }

    for (id, session_id, method, params) in [
        (
            20_003,
            "SID-primary",
            "Network.setCacheDisabled",
            json!({"cacheDisabled": true}),
        ),
        (
            20_004,
            "SID-primary",
            "Network.setBypassServiceWorker",
            json!({"bypass": true}),
        ),
        (
            20_005,
            "SID-primary",
            "Network.setExtraHTTPHeaders",
            json!({"headers": {"X-Primary": "primary", "X-Shared": "primary"}}),
        ),
        (
            20_011,
            "SID-primary",
            "Network.setBlockedURLs",
            json!({"urls": ["*primary-only*", "*shared-pattern*"]}),
        ),
        (
            20_006,
            "SID-aux",
            "Network.setCacheDisabled",
            json!({"cacheDisabled": false}),
        ),
        (
            20_007,
            "SID-aux",
            "Network.setBypassServiceWorker",
            json!({"bypass": false}),
        ),
        (
            20_008,
            "SID-aux",
            "Network.setExtraHTTPHeaders",
            json!({"headers": {"X-Aux": "aux", "x-shared": "aux"}}),
        ),
        (
            20_012,
            "SID-aux",
            "Network.setBlockedURLs",
            json!({"urls": ["*aux-only*", "*shared-pattern*"]}),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": session_id,
            "params": params,
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }

    let policy = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target()
        .effective_policy();
    assert!(policy.cache_disabled());
    assert!(policy.bypass_service_worker());
    assert_eq!(
        policy.extra_headers(),
        [
            ("X-Primary".to_owned(), "primary".to_owned()),
            ("x-shared".to_owned(), "aux".to_owned()),
            ("X-Aux".to_owned(), "aux".to_owned()),
        ]
    );
    assert_eq!(
        policy.blocked_url_patterns(),
        [
            "*primary-only*".to_owned(),
            "*shared-pattern*".to_owned(),
            "*aux-only*".to_owned(),
        ],
        "enabled Network handlers contribute the union of blocked URL patterns"
    );

    ctx.process_async(json!({
        "id": 20_009,
        "method": "Network.disable",
        "sessionId": "SID-primary",
    }))
    .await;
    ctx.expect_result(20_009, json!({}), Some("SID-primary"));

    let policy = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target()
        .effective_policy();
    assert!(!policy.cache_disabled());
    assert!(!policy.bypass_service_worker());
    assert_eq!(
        policy.extra_headers(),
        [
            ("X-Aux".to_owned(), "aux".to_owned()),
            ("x-shared".to_owned(), "aux".to_owned()),
        ]
    );
    assert_eq!(
        policy.blocked_url_patterns(),
        ["*aux-only*".to_owned(), "*shared-pattern*".to_owned()],
        "Network.disable removes only the disabled handler's blocked URL contribution"
    );

    ctx.process_async(json!({
        "id": 20_010,
        "method": "Network.enable",
        "sessionId": "SID-primary",
    }))
    .await;
    ctx.expect_result(20_010, json!({}), Some("SID-primary"));

    let policy = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target()
        .effective_policy();
    assert!(!policy.cache_disabled());
    assert!(!policy.bypass_service_worker());
    assert_eq!(
        policy.extra_headers(),
        [
            ("X-Aux".to_owned(), "aux".to_owned()),
            ("x-shared".to_owned(), "aux".to_owned()),
        ],
        "Network.disable must clear the disabled session's policy state"
    );
    assert_eq!(
        policy.blocked_url_patterns(),
        ["*aux-only*".to_owned(), "*shared-pattern*".to_owned()],
        "re-enabling a cleared handler must not restore its old blocked URLs"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn enable_after_page_load_does_not_replay_historical_subresource_events() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><script src="/before-enable.js"></script>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
            "globalThis.__before_network_enable = true;",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/before-enable.js", get(script)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let script_url = format!("http://{addr}/before-enable.js");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    let page = ctx
        .conn
        .load_page_via_runtime_async(&page_url)
        .await
        .expect("page should load before Network is enabled");
    assert!(
        page.subresource_network_records()
            .iter()
            .any(|record| record.url().as_str() == script_url)
    );
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10_111,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(10_111, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 10_112,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__before_network_enable" }
    }))
    .await;
    ctx.expect_result(
        10_112,
        json!({ "result": { "type": "boolean", "value": true }}),
        Some("SID-1"),
    );

    let messages = ctx.take_all();
    assert!(
        !messages.iter().any(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(script_url)
        }),
        "Network.enable must not replay subresource events recorded before the first listener"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_enable_after_pending_subresource_does_not_replay_history_to_new_session() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><script src="/pending-before-aux.js"></script>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
            "globalThis.__pending_before_aux_network_enable = true;",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/pending-before-aux.js", get(script)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let script_url = format!("http://{addr}/pending-before-aux.js");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-primary");
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".into()));
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 10_121,
        "method": "Network.enable",
        "sessionId": "SID-primary"
    }))
    .await;
    ctx.expect_result(10_121, json!({}), Some("SID-primary"));

    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-primary"))
        .await;
    assert!(
        ctx.conn
            .runtime_session_owner_slot(Some("SID-primary"))
            .unwrap()
            .loaded_page()
            .unwrap()
            .subresource_network_records()
            .iter()
            .any(|record| record.url().as_str() == script_url)
    );
    wait_until_messages(
        &mut ctx,
        "SID-primary",
        "primary subresource delivery before auxiliary Network.enable",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.requestWillBeSent")
                    && message["sessionId"] == json!("SID-primary")
                    && message["params"]["request"]["url"] == json!(script_url)
            })
        },
    )
    .await;
    let primary_messages = ctx.take_all();
    assert_eq!(
        primary_messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.requestWillBeSent")
                    && message["sessionId"] == json!("SID-primary")
                    && message["params"]["request"]["url"] == json!(script_url)
            })
            .count(),
        1,
        "the existing primary listener should receive the concrete subresource record once"
    );

    ctx.process_async(json!({
        "id": 10_122,
        "method": "Network.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(10_122, json!({}), Some("SID-aux"));

    ctx.process_async(json!({
        "id": 10_123,
        "method": "Runtime.evaluate",
        "sessionId": "SID-aux",
        "params": { "expression": "globalThis.__pending_before_aux_network_enable" }
    }))
    .await;
    ctx.expect_result(
        10_123,
        json!({ "result": { "type": "boolean", "value": true }}),
        Some("SID-aux"),
    );

    let messages = ctx.take_all();
    assert!(
        !messages.iter().any(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!("SID-aux")
                && message["params"]["request"]["url"] == json!(script_url)
        }),
        "newly enabled auxiliary listener must not receive subresource events from before its Network.enable"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn websocket_runtime_activity_broadcasts_to_auxiliary_network_session() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/socket", get(websocket_echo_handler)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let socket_url = format!("ws://{addr}/socket");
    let socket_literal = serde_json::to_string(&socket_url).unwrap();
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    bc.attach_active_session("SID-primary".to_owned());
    bc.set_active_target_id("TID-1".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.enable_auxiliary_network_events("SID-aux");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-primary"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_101,
        "method": "Runtime.evaluate",
        "sessionId": "SID-primary",
        "params": {
            "expression": format!(r#"(() => {{
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => socket.send('aux event'));
                socket.addEventListener('message', () => socket.close(1000, 'done'));
                return 'scheduled';
            }})()"#)
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_101);

    wait_until_messages(
        &mut ctx,
        "SID-aux",
        "auxiliary session websocket CDP frame events",
        |messages| {
            messages.iter().any(|message| {
                message["sessionId"] == json!("SID-aux")
                    && message["method"] == json!("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(9)
            })
        },
    )
    .await;

    let primary_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-primary")
                && message["method"] == json!("Network.webSocketCreated")
        })
        .expect("primary webSocketCreated event");
    assert_eq!(primary_created["params"]["url"], socket_url);
    let request_id = primary_created["params"]["requestId"]
        .as_str()
        .expect("primary websocket request id")
        .to_owned();

    let auxiliary_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-aux")
                && message["method"] == json!("Network.webSocketCreated")
        })
        .expect("auxiliary webSocketCreated event");
    assert_eq!(auxiliary_created["params"]["url"], socket_url);
    assert_eq!(
        auxiliary_created["params"]["requestId"],
        json!(request_id),
        "primary and auxiliary sessions must observe the same WebSocket requestId"
    );
    assert!(ctx.sent.iter().any(|message| {
        message["sessionId"] == json!("SID-primary")
            && message["method"] == json!("Network.webSocketFrameReceived")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["payloadLength"] == json!(9)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["sessionId"] == json!("SID-aux")
            && message["method"] == json!("Network.webSocketFrameReceived")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["payloadLength"] == json!(9)
    }));

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_network_enable_after_websocket_activity_does_not_replay_history() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/socket", get(websocket_echo_handler)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let socket_url = format!("ws://{addr}/socket");
    let socket_literal = serde_json::to_string(&socket_url).unwrap();
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    bc.attach_active_session("SID-primary".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-primary"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_111,
        "method": "Runtime.evaluate",
        "sessionId": "SID-primary",
        "params": {
            "expression": format!(r#"(() => {{
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => socket.send('primary history'));
                socket.addEventListener('message', () => socket.close(1000, 'done'));
                return 'scheduled';
            }})()"#)
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_111);

    wait_until_messages(
        &mut ctx,
        "SID-primary",
        "primary websocket history before auxiliary Network.enable",
        |messages| {
            let Some(request_id) = messages.iter().find_map(|message| {
                (message["sessionId"] == json!("SID-primary")
                    && message["method"] == json!("Network.webSocketCreated"))
                .then(|| message["params"]["requestId"].clone())
            }) else {
                return false;
            };
            messages.iter().any(|message| {
                message["sessionId"] == json!("SID-primary")
                    && message["method"] == json!("Network.webSocketFrameReceived")
                    && message["params"]["requestId"] == request_id
                    && message["params"]["response"]["payloadLength"] == json!(15)
            }) && messages.iter().any(|message| {
                message["sessionId"] == json!("SID-primary")
                    && message["method"] == json!("Network.webSocketClosed")
                    && message["params"]["requestId"] == request_id
            })
        },
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_112,
        "method": "Network.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(7_112, json!({}), Some("SID-aux"));
    ctx.process_async(json!({
        "id": 7_113,
        "method": "Runtime.evaluate",
        "sessionId": "SID-aux",
        "params": { "expression": "42" }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_113);

    assert!(
        ctx.sent.iter().all(|message| {
            message["sessionId"] != json!("SID-aux")
                || !message["method"]
                    .as_str()
                    .is_some_and(|method| method.starts_with("Network.webSocket"))
        }),
        "late auxiliary Network.enable must not replay old WebSocket events: {:?}",
        ctx.sent
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn fetch_runtime_activity_broadcasts_to_auxiliary_network_session() {
    async fn data() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "aux body")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/data", get(data)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    bc.attach_active_session("SID-primary".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.enable_auxiliary_network_events("SID-aux");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-primary"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_201,
        "method": "Runtime.evaluate",
        "sessionId": "SID-primary",
        "params": {
            "expression": "fetch('/data').then(response => response.text()).then(text => { document.body.dataset.auxFetch = text; }); 'scheduled';"
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_201);

    wait_until_messages(
        &mut ctx,
        "SID-primary",
        "auxiliary session fetch CDP events",
        |messages| {
            let Some(request_id) = messages.iter().find_map(|message| {
                if message["sessionId"] == json!("SID-aux")
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Fetch")
                {
                    message["params"]["requestId"].as_str()
                } else {
                    None
                }
            }) else {
                return false;
            };
            messages.iter().any(|message| {
                message["sessionId"] == json!("SID-aux")
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
        },
    )
    .await;

    let messages = ctx.take_all();
    let aux_request = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-aux")
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("auxiliary fetch request event");
    assert_eq!(aux_request["params"]["documentURL"], page_url);
    assert_eq!(
        aux_request["params"]["request"]["url"],
        format!("http://{addr}/data")
    );
    let request_id = aux_request["params"]["requestId"]
        .as_str()
        .expect("auxiliary fetch request id")
        .to_owned();
    assert!(messages.iter().any(|message| {
        message["sessionId"] == json!("SID-primary")
            && message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 7_202,
        "method": "Network.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        7_202,
        json!({
            "body": "aux body",
            "base64Encoded": false
        }),
        Some("SID-aux"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn background_fetch_runtime_activity_broadcasts_to_auxiliary_network_session() {
    async fn data() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "background aux body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/data", get(data)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let target = PageTargetHost::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    );

    let mut bc = BrowserContext::new("BID-background".into());
    bc.insert_page_target_host(target);
    assert!(
        bc.assign_auxiliary_session_to_target("TID-background", "SID-aux-background".to_owned())
    );
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
        .await;

    ctx.process_async(json!({
        "id": 7_221,
        "method": "Network.enable",
        "sessionId": "SID-aux-background"
    }))
    .await;
    ctx.expect_result(7_221, json!({}), Some("SID-aux-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_222,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "fetch('/data').then(response => response.text()).then(text => { document.body.dataset.backgroundAuxFetch = text; }); 'scheduled';"
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_222);

    wait_until_messages(
        &mut ctx,
        "SID-background",
        "background auxiliary session fetch CDP events",
        |messages| {
            let Some(request_id) = messages.iter().find_map(|message| {
                if message["sessionId"] == json!("SID-aux-background")
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Fetch")
                {
                    message["params"]["requestId"].as_str()
                } else {
                    None
                }
            }) else {
                return false;
            };
            messages.iter().any(|message| {
                message["sessionId"] == json!("SID-aux-background")
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
        },
    )
    .await;

    let messages = ctx.take_all();
    let aux_request = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-aux-background")
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("background auxiliary fetch request event");
    assert_eq!(aux_request["params"]["documentURL"], page_url);
    assert_eq!(
        aux_request["params"]["request"]["url"],
        format!("http://{addr}/data")
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_broadcasts_to_auxiliary_network_session() {
    async fn next_page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>next document</main></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/next", get(next_page)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let next_url = format!("http://{addr}/next");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    bc.attach_active_session("SID-primary".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.enable_auxiliary_network_events("SID-aux");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-primary"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_260,
        "method": "Page.navigate",
        "sessionId": "SID-primary",
        "params": { "url": next_url }
    }))
    .await;
    ctx.expect_result(
        7_260,
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID }),
        Some("SID-primary"),
    );

    wait_until_messages(
        &mut ctx,
        Some("SID-primary"),
        "auxiliary session document navigation CDP events",
        |messages| {
            messages.iter().any(|message| {
                message["sessionId"] == json!("SID-aux")
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(LOADER_ID)
            })
        },
    )
    .await;

    let messages = ctx.take_all();
    let aux_request = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-aux")
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Document")
                && message["params"]["request"]["url"] == json!(next_url)
        })
        .expect("auxiliary document request event");
    assert_eq!(aux_request["params"]["requestId"], json!(LOADER_ID));
    assert_eq!(aux_request["params"]["loaderId"], json!(LOADER_ID));
    assert!(messages.iter().any(|message| {
        message["sessionId"] == json!("SID-aux")
            && message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(LOADER_ID)
            && message["params"]["type"] == json!("Document")
            && message["params"]["response"]["url"] == json!(next_url)
    }));
    assert!(messages.iter().any(|message| {
        message["sessionId"] == json!("SID-primary")
            && message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(LOADER_ID)
            && message["params"]["type"] == json!("Document")
    }));

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_reads_background_auxiliary_target_slot() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "https://background.example/".to_owned(),
    ));
    assert!(
        bc.assign_auxiliary_session_to_target("TID-background", "SID-aux-background".to_owned())
    );
    ctx.conn.browser_context = Some(bc);

    assert!(
        ctx.conn
            .enable_network_listener_for_session_owner(Some("SID-aux-background"))
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-aux-background"))
        .expect("background auxiliary runtime slot")
        .record_captured_response_body(
            "REQ-background".to_owned(),
            "background body".to_owned(),
            [Some("SID-aux-background".to_owned())],
        );

    ctx.process_async(json!({
        "id": 7_282,
        "method": "Network.getResponseBody",
        "sessionId": "SID-aux-background",
        "params": { "requestId": "REQ-background" }
    }))
    .await;
    ctx.expect_result(
        7_282,
        json!({ "body": "background body", "base64Encoded": false }),
        Some("SID-aux-background"),
    );

    ctx.process_async(json!({
        "id": 7_283,
        "method": "Network.getResponseBody",
        "sessionId": "SID-active",
        "params": { "requestId": "REQ-background" }
    }))
    .await;
    ctx.expect_error(7_283, -32000, "No resource with given identifier found");
}
#[tokio::test(flavor = "multi_thread")]
async fn network_disable_removes_session_response_body_visibility() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.enable_auxiliary_network_events("SID-aux");
    bc.record_captured_response_body(
        "REQ-shared".to_owned(),
        "shared body".to_owned(),
        [Some("SID-primary".to_owned()), Some("SID-aux".to_owned())],
    );
    bc.record_captured_response_body(
        "REQ-aux-only".to_owned(),
        "aux-only body".to_owned(),
        [Some("SID-aux".to_owned())],
    );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_290,
        "method": "Network.disable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(7_290, json!({}), Some("SID-aux"));

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        bc.has_captured_response_body_for_test("REQ-shared"),
        "shared body remains visible to primary after auxiliary Network.disable"
    );
    assert!(
        !bc.has_captured_response_body_for_test("REQ-aux-only"),
        "auxiliary-only body is dropped when that session disables Network"
    );

    ctx.process_async(json!({
        "id": 7_291,
        "method": "Network.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(7_291, json!({}), Some("SID-aux"));

    ctx.process_async(json!({
        "id": 7_292,
        "method": "Network.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": "REQ-shared" }
    }))
    .await;
    ctx.expect_error(7_292, -32000, "No resource with given identifier found");

    ctx.process_async(json!({
        "id": 7_293,
        "method": "Network.getResponseBody",
        "sessionId": "SID-primary",
        "params": { "requestId": "REQ-shared" }
    }))
    .await;
    ctx.expect_result(
        7_293,
        json!({ "body": "shared body", "base64Encoded": false }),
        Some("SID-primary"),
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn disable_clears_enabled_flag_and_captured_bodies() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    bc.record_captured_response_body("REQ-1".to_owned(), "body".to_owned(), [None]);
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({"id": 2, "method": "Network.disable"}))
        .await;
    ctx.expect_result(2, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        !bc.active_page_target()
            .runtime_slot
            .primary_network_events_enabled()
    );
    assert!(bc.captured_response_bodies_empty_for_test());
}
#[tokio::test(flavor = "multi_thread")]
async fn primary_network_disable_preserves_auxiliary_network_session() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.enable_auxiliary_network_events("SID-aux");
    bc.record_captured_response_body(
        "REQ-1".to_owned(),
        "body".to_owned(),
        [Some("SID-primary".to_owned()), Some("SID-aux".to_owned())],
    );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 10_201,
        "method": "Network.disable",
        "sessionId": "SID-primary"
    }))
    .await;
    ctx.expect_result(10_201, json!({}), Some("SID-primary"));

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        !bc.active_page_target()
            .runtime_slot
            .primary_network_events_enabled()
    );
    assert!(bc.has_network_event_listeners());
    assert!(
        bc.active_page_target()
            .runtime_slot
            .has_auxiliary_network_events_for_session("SID-aux")
    );
    assert!(
        bc.has_captured_response_body_for_test("REQ-1"),
        "shared body cache remains observable while an auxiliary Network session is enabled"
    );
    assert_eq!(
        bc.network_event_session_ids(Some("SID-primary")),
        vec![Some("SID-aux".to_owned())]
    );

    ctx.process_async(json!({
        "id": 10_202,
        "method": "Network.disable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(10_202, json!({}), Some("SID-aux"));

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_network_event_listeners());
    assert!(bc.captured_response_bodies_empty_for_test());
}
#[tokio::test(flavor = "multi_thread")]
async fn parser_external_script_navigation_broadcasts_network_events_to_auxiliary_session() {
    const SCRIPT_BODY: &str =
        r#"globalThis.__lm_aux_parser_script_loaded = "aux parser script body";"#;

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script src="/script.js"></script>
</body></html>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
            SCRIPT_BODY,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/script.js", get(script)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let script_url = format!("http://{addr}/script.js");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-primary");
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".into()));
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 70_050,
        "method": "Network.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(70_050, json!({}), Some("SID-aux"));

    ctx.process_async(json!({
        "id": 70_051,
        "method": "Page.navigate",
        "sessionId": "SID-primary",
        "params": { "url": page_url }
    }))
    .await;

    wait_until_messages(
        &mut ctx,
        Some("SID-aux"),
        "auxiliary parser script network events",
        |messages| {
            let Some(request_id) = messages.iter().find_map(|message| {
                if message["sessionId"] == json!("SID-aux")
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Script")
                    && message["params"]["request"]["url"] == json!(script_url)
                {
                    message["params"]["requestId"].as_str()
                } else {
                    None
                }
            }) else {
                return false;
            };
            messages.iter().any(|message| {
                message["sessionId"] == json!("SID-aux")
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
        },
    )
    .await;

    let messages = ctx.take_all();
    let script_request = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-aux")
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Script")
                && message["params"]["request"]["url"] == json!(script_url)
        })
        .expect("auxiliary session should receive parser script request event");
    let script_request_id = script_request["params"]["requestId"]
        .as_str()
        .expect("auxiliary parser script request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 70_052,
        "method": "Network.getResponseBody",
        "sessionId": "SID-primary",
        "params": { "requestId": script_request_id }
    }))
    .await;
    ctx.expect_error(70_052, -32000, "No resource with given identifier found");

    ctx.process_async(json!({
        "id": 70_053,
        "method": "Network.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": script_request_id }
    }))
    .await;
    ctx.expect_result(
        70_053,
        json!({
            "body": SCRIPT_BODY,
            "base64Encoded": false
        }),
        Some("SID-aux"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn network_disable_suppresses_navigation_network_events() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>ok</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 2,
        "method": "Network.disable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(2, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    let messages = ctx.take_all();
    assert!(messages.iter().any(|message| message["id"] == json!(3)));
    assert!(!messages.iter().any(|message| {
        message["method"] == json!("Network.requestWillBeSent")
            || message["method"] == json!("Network.responseReceived")
            || message["method"] == json!("Network.loadingFinished")
            || message["method"] == json!("Network.loadingFailed")
    }));

    server.abort();
}
