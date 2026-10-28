use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_runtime_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-RUNTIME-DISABLE",
        "TID-000000000PRD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949360,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949360, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949370,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-RUNTIME-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949370, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949380,
        "method": "Runtime.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949380, json!({}), Some(&second_session_id));
    take_staged_about_blank_runtime_context(&mut ctx, &second_session_id, &second_target_id);

    ctx.process_async(json!({
        "id": 1041949381,
        "method": "Runtime.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949381, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
    }

    ctx.process_async(json!({
        "id": 1041949390,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949390);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!("SID-active") && is_runtime_context_event(message)
        }),
        "active target should not emit runtime context events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949393,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PRD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949393);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949394,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949394);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!(second_session_id)
                && matches!(
                    message["method"].as_str(),
                    Some("Runtime.executionContextsCleared")
                        | Some("Runtime.executionContextCreated")
                )
        }),
        "disabled runtime should not emit execution context events on first activated navigation: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_inspector_enable_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-INSPECTOR",
        "TID-000000000PI",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 1041949395,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949395, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949396,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-INSPECTOR", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949396, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949397,
        "method": "Inspector.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949397, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            active
                .active_page_target()
                .owner_state
                .target_crash_state
                .is_crashed()
        );
        assert!(
            !active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
    }
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Inspector.targetCrashed")
                && message["sessionId"] == json!(second_session_id)
        }),
        "background inspector enable should not replay active target crash state"
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949398,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PI"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949398);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949390,
        "method": "Page.crash",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949390, json!({}), Some(&second_session_id));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Inspector.targetCrashed")
            && message["sessionId"] == json!(second_session_id)
    }));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949399,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949399);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Inspector.targetReloadedAfterCrash")
                && message["sessionId"] == json!(second_session_id)
        }),
        "activated target should emit crash-reload event when staged inspector is enabled"
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
async fn same_context_background_session_can_disable_its_own_inspector_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-INSPECTOR-DISABLE",
        "TID-000000000PID",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949400,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949400, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949401,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-INSPECTOR-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949401, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949402,
        "method": "Inspector.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949402, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949403,
        "method": "Inspector.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949403, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "inspector disable should collapse staged background state back to default"
        );
    }

    ctx.process_async(json!({
        "id": 1041949404,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PID"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949404);
    ctx.take_all();

    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 1041949405,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949405);
    assert!(
        !ctx.sent.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Inspector.targetReloadedAfterCrash") | Some("Inspector.targetCrashed")
            ) && message["sessionId"] == json!(second_session_id)
        }),
        "disabled inspector should not emit crash-related events on first activated navigation"
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed(),
        "navigation should still clear crash state even when inspector is disabled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_css_enable_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-CSS",
        "TID-000000000PC",
        "<title>active</title><style>body{color:red}</style><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949406,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949406, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949407,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-CSS", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949407, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949408,
        "method": "CSS.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949408, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_target().css_enabled);
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(staged.css_enabled);
    }

    ctx.process_async(json!({
        "id": 1041949409,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PC"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949409);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("activated browser context");
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        assert!(active.active_page_target().css_enabled);
    }

    ctx.process_async(json!({
            "id": 1041949410,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>page-b</title><style>body{color:blue}</style><div id='ok'>page b</div>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949410);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .css_enabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_css_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-CSS-DISABLE",
        "TID-000000000PCD",
        "<title>active</title><style>body{color:red}</style><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949411,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949411, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949412,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-CSS-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949412, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949413,
        "method": "CSS.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949413, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 1041949414,
        "method": "CSS.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(1041949414, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_target().css_enabled);
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "css disable should collapse staged background state back to default"
        );
    }

    ctx.process_async(json!({
        "id": 1041949415,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PCD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949415);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("activated browser context");
        assert_eq!(active.active_target_id(), Some(second_target_id.as_str()));
        assert!(!active.active_page_target().css_enabled);
    }

    ctx.process_async(json!({
            "id": 1041949416,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>page-b</title><style>body{color:blue}</style><div id='ok'>page b</div>"
            }
        })).await;
    let _ = take_response_by_id(&mut ctx, 1041949416);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .css_enabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_fetch_enable_before_activation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-stage</body></html>",
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
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH",
        "TID-000000000PF",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194940,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194940, json!({}), None);

    ctx.process_async(json!({
        "id": 104194941,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-FETCH", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194941, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194942,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": {
            "patterns": [
                {
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(104194942, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_target().fetch_owner.is_enabled());
        assert!(
            active
                .active_page_target()
                .fetch_owner
                .config_snapshot()
                .patterns()
                .is_empty()
        );
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(staged.fetch_owner.config_snapshot().is_enabled());
        assert_eq!(staged.fetch_owner.config_snapshot().patterns().len(), 1);
        assert_eq!(
            staged.fetch_owner.config_snapshot().patterns()[0].url_pattern,
            "*"
        );
        assert_eq!(
            staged.fetch_owner.config_snapshot().patterns()[0].resource_type_filter,
            Some(crate::conn::FetchResourceTypeFilter::Document)
        );
        assert_eq!(
            staged.fetch_owner.config_snapshot().patterns()[0].request_stage,
            crate::conn::FetchRequestStage::Request
        );
    }

    ctx.navigate_with_page_events(json!({
        "id": 104194943,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194943);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "active target should not be intercepted by background-staged fetch config: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194944,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PF"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194944);
    ctx.take_all();

    ctx.navigate_with_page_events(json!({
        "id": 104194945,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], json!(second_session_id));
    assert_eq!(paused["params"]["frameId"], json!(second_target_id));

    ctx.process_async(json!({
        "id": 104194946,
        "method": "Fetch.continueRequest",
        "sessionId": second_session_id,
        "params": {
            "requestId": paused["params"]["requestId"]
        }
    }))
    .await;
    ctx.expect_result(104194946, json!({}), Some(&second_session_id));

    let _ = take_response_by_id(&mut ctx, 104194945);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_fetch_continue_request_keeps_target_background() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-continue-background</body></html>",
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
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH-CONTINUE",
        "TID-000000000PFC",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194962,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194962, json!({}), None);

    ctx.process_async(json!({
        "id": 104194963,
        "method": "Target.createTarget",
        "params": {
            "background": true,
            "browserContextId": "BID-9-PRE-FETCH-CONTINUE",
            "url": "about:blank#second"
        }
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194963, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194964,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": {
            "patterns": [
                {
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(104194964, json!({}), Some(&second_session_id));

    ctx.navigate_with_page_events(json!({
        "id": 104194965,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], json!(second_session_id));
    assert_eq!(paused["params"]["frameId"], json!(second_target_id));
    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PFC"));
        assert!(
            active
                .background_target(&second_target_id)
                .expect("background target must exist")
                .fetch_owner
                .pending_state()
                .has_pending_fetch_navigation(),
            "background fetch pause should stay background before continueRequest"
        );
    }

    ctx.process_async(json!({
        "id": 104194966,
        "method": "Fetch.continueRequest",
        "sessionId": second_session_id,
        "params": {
            "requestId": paused["params"]["requestId"]
        }
    }))
    .await;
    ctx.expect_result(104194966, json!({}), Some(&second_session_id));

    let navigation = take_response_by_id(&mut ctx, 104194965);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert_eq!(active.active_target_id(), Some("TID-000000000PFC"));
        assert!(
            active
                .background_target(&second_target_id)
                .is_some_and(|target| target.has_loaded_page()),
            "continued background navigation should commit to background owner"
        );
    }

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_fetch_auth_handling_before_activation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-auth-stage</body></html>",
        )
    }

    async fn protected(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let expected = "Basic dXNlcjpwYXNz";
        match headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == expected => (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body>secret</body></html>",
            )
                .into_response(),
            _ => (
                axum::http::StatusCode::UNAUTHORIZED,
                [(
                    axum::http::header::WWW_AUTHENTICATE.as_str(),
                    "Basic realm=\"stage-area\"",
                )],
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
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .route("/protected", axum::routing::any(protected)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH-AUTH",
        "TID-000000000PFA",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194954,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194954, json!({}), None);

    ctx.process_async(json!({
        "id": 104194955,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-FETCH-AUTH", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194955, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194956,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(104194956, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_target().fetch_owner.is_enabled());
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(staged.fetch_owner.config_snapshot().is_enabled());
        assert!(staged.fetch_owner.config_snapshot().handle_auth_requests());
    }

    ctx.navigate_with_page_events(json!({
        "id": 104194957,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194957);
    assert!(
        !ctx.sent.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Fetch.requestPaused") | Some("Fetch.authRequired")
            )
        }),
        "active target should not see auth interception from background-staged fetch config: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194958,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PFA"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194958);
    ctx.take_all();

    ctx.navigate_with_page_events(json!({
        "id": 104194959,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/protected") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], json!(second_session_id));
    assert_eq!(paused["params"]["frameId"], json!(second_target_id));
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("paused request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 104194960,
        "method": "Fetch.continueRequest",
        "sessionId": second_session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(104194960, json!({}), Some(&second_session_id));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["sessionId"], json!(second_session_id));
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert_eq!(auth_required["params"]["authChallenge"]["scheme"], "basic");
    assert_eq!(
        auth_required["params"]["authChallenge"]["realm"],
        "stage-area"
    );

    ctx.process_async(json!({
        "id": 104194961,
        "method": "Fetch.continueWithAuth",
        "sessionId": second_session_id,
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
    ctx.expect_result(104194961, json!({}), Some(&second_session_id));

    let navigation = take_response_by_id(&mut ctx, 104194959);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated auth navigation: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_fetch_before_activation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>fetch-disable-stage</body></html>",
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
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-FETCH-DISABLE",
        "TID-000000000PFD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194947,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194947, json!({}), None);

    ctx.process_async(json!({
        "id": 104194948,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-FETCH-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194948, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194949,
        "method": "Fetch.enable",
        "sessionId": second_session_id,
        "params": {
            "patterns": [
                {
                    "urlPattern": "*",
                    "resourceType": "Document",
                    "requestStage": "Request"
                }
            ]
        }
    }))
    .await;
    ctx.expect_result(104194949, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194950,
        "method": "Fetch.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194950, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(!active.active_page_target().fetch_owner.is_enabled());
        assert!(
            active
                .active_page_target()
                .fetch_owner
                .config_snapshot()
                .patterns()
                .is_empty()
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "fetch disable should collapse staged background state back to default"
        );
    }

    ctx.process_async(json!({
        "id": 104194951,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194951);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "active target should not be intercepted after background-staged fetch disable: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194952,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PFD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194952);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194953,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194953);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Fetch.requestPaused")),
        "disabled fetch should not pause first activated navigation: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_network_enable_before_activation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET",
        "TID-000000000PN",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194950,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194950, json!({}), None);

    ctx.process_async(json!({
        "id": 104194951,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194951, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194952,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194952, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_target()
                .runtime_slot
                .primary_network_events_enabled()
        );
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(staged.runtime_slot.primary_network_events_enabled());
    }

    ctx.navigate_with_page_events(json!({
        "id": 104194953,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194953);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Network.requestWillBeSent")),
        "active target should not emit network events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194954,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PN"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194954);
    ctx.take_all();

    ctx.navigate_with_page_events(json!({
        "id": 104194955,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194955);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    wait_for_session_main_document_loading_finished(
        &mut ctx,
        &second_session_id,
        &page_url,
        "activated session main-document network completion",
    )
    .await;
    let emitted = ctx.take_all();
    let request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .cloned()
        .expect("activated target should emit requestWillBeSent");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    assert_eq!(request["params"]["frameId"], json!(second_target_id));

    let response = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["requestId"] == json!(request_id)
        })
        .cloned()
        .expect("activated target should emit responseReceived");
    assert_eq!(response["params"]["response"]["url"], json!(page_url));
    assert_eq!(response["params"]["response"]["status"], json!(200));

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_network_before_activation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-disable-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-DISABLE",
        "TID-000000000PND",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194958,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194958, json!({}), None);

    ctx.process_async(json!({
        "id": 104194959,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-DISABLE", "url": "about:blank#second"}
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
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194959, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194960,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194960, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194961,
        "method": "Network.disable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194961, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_target()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(
            active
                .background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "network disable should collapse staged background state back to default"
        );
        assert!(
            active
                .background_target(&second_target_id)
                .expect("background target")
                .runtime_slot
                .network_artifacts_are_default_for_test(),
            "network disable should clear staged background network artifacts"
        );
    }

    ctx.process_async(json!({
        "id": 104194962,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url.clone() }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194962);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Network.requestWillBeSent")
                    | Some("Network.responseReceived")
                    | Some("Network.loadingFinished")
            )
        }),
        "active target should not emit network events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194963,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PND"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194963);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194964,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194964);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!(second_session_id)
                && matches!(
                    message["method"].as_str(),
                    Some("Network.requestWillBeSent")
                        | Some("Network.responseReceived")
                        | Some("Network.loadingFinished")
                )
        }),
        "disabled network should not emit first-navigation network events after activation: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_cache_and_service_worker_policy_before_activation()
 {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-policy-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-POLICY",
        "TID-000000000NP",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194956,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194956, json!({}), None);

    ctx.process_async(json!({
        "id": 104194957,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-POLICY", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_first_matching(
        "Target.targetCreated for staged network-policy target",
        |message| message["method"] == json!("Target.targetCreated"),
    );
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_first_matching(
        "Target.attachedToTarget for staged network-policy target",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(second_target_id)
        },
    );
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194957, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194958,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194958, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194959,
        "method": "Network.setCacheDisabled",
        "sessionId": second_session_id,
        "params": { "cacheDisabled": true }
    }))
    .await;
    ctx.expect_result(104194959, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194960,
        "method": "Network.setBypassServiceWorker",
        "sessionId": second_session_id,
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_result(104194960, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_target()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(
            !active
                .active_page_target()
                .effective_policy()
                .cache_disabled()
        );
        assert!(
            !active
                .active_page_target()
                .effective_policy()
                .bypass_service_worker()
        );
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(staged.runtime_slot.primary_network_events_enabled());
        assert!(staged.effective_policy().cache_disabled());
        assert!(staged.effective_policy().bypass_service_worker());
    }

    ctx.navigate_with_page_events(json!({
        "id": 104194961,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194961);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Network.requestWillBeSent")),
        "active target should not emit network events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194962,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000NP"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194962);
    ctx.take_all();

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("activated browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            bc.active_page_target()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(bc.active_page_target().effective_policy().cache_disabled());
        assert!(
            bc.active_page_target()
                .effective_policy()
                .bypass_service_worker()
        );
    }

    ctx.navigate_with_page_events(json!({
        "id": 104194963,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194963);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    wait_for_session_main_document_loading_finished(
        &mut ctx,
        &second_session_id,
        &page_url,
        "activated session main-document completion with staged network policy",
    )
    .await;
    let emitted = ctx.take_all();
    let request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .cloned()
        .expect("activated target should emit requestWillBeSent");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));
    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_disable_its_own_cache_and_service_worker_policy_before_activation()
 {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network-policy-disable-stage</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-NET-POLICY-DISABLE",
        "TID-000000000NPD",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194965,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194965, json!({}), None);

    ctx.process_async(json!({
        "id": 104194966,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-NET-POLICY-DISABLE", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_first_matching(
        "Target.targetCreated for staged disabled network-policy target",
        |message| message["method"] == json!("Target.targetCreated"),
    );
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_first_matching(
        "Target.attachedToTarget for staged disabled network-policy target",
        |message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(second_target_id)
        },
    );
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194966, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194967,
        "method": "Network.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194967, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194968,
        "method": "Network.setCacheDisabled",
        "sessionId": second_session_id,
        "params": { "cacheDisabled": true }
    }))
    .await;
    ctx.expect_result(104194968, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194969,
        "method": "Network.setBypassServiceWorker",
        "sessionId": second_session_id,
        "params": { "bypass": true }
    }))
    .await;
    ctx.expect_result(104194969, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194970,
        "method": "Network.setCacheDisabled",
        "sessionId": second_session_id,
        "params": { "cacheDisabled": false }
    }))
    .await;
    ctx.expect_result(104194970, json!({}), Some(&second_session_id));

    ctx.process_async(json!({
        "id": 104194971,
        "method": "Network.setBypassServiceWorker",
        "sessionId": second_session_id,
        "params": { "bypass": false }
    }))
    .await;
    ctx.expect_result(104194971, json!({}), Some(&second_session_id));

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active
                .active_page_target()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(
            !active
                .active_page_target()
                .effective_policy()
                .cache_disabled()
        );
        assert!(
            !active
                .active_page_target()
                .effective_policy()
                .bypass_service_worker()
        );
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(staged.runtime_slot.primary_network_events_enabled());
        assert!(!staged.effective_policy().cache_disabled());
        assert!(!staged.effective_policy().bypass_service_worker());
    }

    ctx.navigate_with_page_events(json!({
        "id": 104194972,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": { "url": page_url.clone() }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194972);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Network.requestWillBeSent")),
        "active target should not emit network events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 104194973,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000NPD"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194973);
    ctx.take_all();

    {
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("activated browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            bc.active_page_target()
                .runtime_slot
                .primary_network_events_enabled()
        );
        assert!(!bc.active_page_target().effective_policy().cache_disabled());
        assert!(
            !bc.active_page_target()
                .effective_policy()
                .bypass_service_worker()
        );
    }

    ctx.navigate_with_page_events(json!({
        "id": 104194974,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": { "url": page_url }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104194974);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    wait_for_session_main_document_loading_finished(
        &mut ctx,
        &second_session_id,
        &page_url,
        "activated session main-document completion after staged policy reset",
    )
    .await;
    let emitted = ctx.take_all();
    let request = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["sessionId"] == json!(second_session_id)
                && message["params"]["request"]["url"] == json!(page_url)
        })
        .cloned()
        .expect("activated target should still emit requestWillBeSent");
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));
    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["sessionId"] == json!(second_session_id)
            && message["params"]["requestId"] == json!(request_id)
    }));

    server.abort();
}
