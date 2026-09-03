use super::*;
use crate::conn::CdpSessionRoute;
use moli_shared_worker::SharedWorkerInstanceId;

fn expect_session_error(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    code: i32,
    message: &str,
) {
    assert_eq!(
        take_response_by_id(ctx, id),
        json!({
            "id": id,
            "sessionId": session_id,
            "error": { "code": code, "message": message }
        })
    );
}

/// cdp.target: attachToTarget – no browser context
#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_no_bc() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 10, "method": "Target.attachToTarget",
                       "params": {"targetId": "X"}}))
        .await;
    ctx.expect_error(10, -31998, "BrowserContextNotLoaded");
}

/// cdp.target: attachToTarget – no target
#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_no_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 10, "method": "Target.attachToTarget",
                       "params": {"targetId": "TID-8"}}))
        .await;
    ctx.expect_error(10, -31998, "TargetNotLoaded");
}

/// cdp.target: attachToTarget – wrong target id
#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_wrong_id() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000B");
    ctx.process_async(json!({"id": 10, "method": "Target.attachToTarget",
                       "params": {"targetId": "TID-8"}}))
        .await;
    ctx.expect_error(10, -31998, "UnknownTargetId");
}

/// cdp.target: attachToTarget – success
#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_success() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000B");
    ctx.process_async(json!({"id": 11, "method": "Target.attachToTarget",
                       "params": {"targetId": "TID-000000000B"}}))
        .await;
    let sid = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_session_id_owned()
        .unwrap();

    // Puppeteer registers the CdpCDPSession from this event, then immediately
    // looks it up when the attach response resolves. Chromium therefore sends
    // attachedToTarget before the response, and the queue order is contractual.
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(attached["params"]["sessionId"], json!(sid));
    assert_eq!(attached["params"]["waitingForDebugger"], json!(false));
    assert_eq!(
        ctx.take_one(),
        json!({ "id": 11, "result": { "sessionId": sid } })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn puppeteer_custom_cdp_session_attach_does_not_report_extra_target_created() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 11000,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(11000, json!({}), None);

    ctx.process_async(json!({
        "id": 11001,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 11001)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    ctx.take_first_matching("initial page targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    assert!(ctx.sent.is_empty());

    ctx.process_async(json!({
        "id": 11002,
        "method": "Target.attachToTarget",
        "params": { "targetId": page_target_id.clone() }
    }))
    .await;

    let attached = ctx.take_one();
    assert_eq!(attached["method"], json!("Target.attachedToTarget"));
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        json!(page_target_id)
    );
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("custom page session id")
        .to_owned();
    assert_eq!(
        take_response_by_id(&mut ctx, 11002),
        json!({
            "id": 11002,
            "result": { "sessionId": session_id }
        })
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "creating a Puppeteer custom CDP session must not report another targetCreated: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_tab_returns_control_plane_session() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 11010,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 11010)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11011,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;

    let attach_response = take_response_by_id(&mut ctx, 11011);
    let tab_session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("tab session id")
        .to_owned();
    let attached = ctx.take_first_matching("tab attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
    });
    assert_eq!(attached["params"]["targetInfo"]["type"], json!("tab"));
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));
    assert!(matches!(
        ctx.conn.session_route(Some(&tab_session_id)),
        Some(CdpSessionRoute::TabTarget {
            tab_target_id: ref route_tab_target_id,
            ..
        }) if route_tab_target_id == &tab_target_id
    ));

    ctx.process_async(json!({
        "id": 11016,
        "sessionId": tab_session_id.clone(),
        "method": "Target.getTargetInfo"
    }))
    .await;
    let owner_info = take_response_by_id(&mut ctx, 11016);
    assert_eq!(
        owner_info["result"]["targetInfo"]["targetId"],
        json!(tab_target_id)
    );
    assert_eq!(owner_info["result"]["targetInfo"]["type"], json!("tab"));

    ctx.process_async(json!({
        "id": 11014,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;
    let second_attach_response = take_response_by_id(&mut ctx, 11014);
    let second_tab_session_id = second_attach_response["result"]["sessionId"]
        .as_str()
        .expect("second tab session id")
        .to_owned();
    assert_ne!(second_tab_session_id, tab_session_id);
    let second_attached = ctx.take_first_matching("second tab attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(second_tab_session_id)
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
    });
    assert_eq!(
        second_attached["params"]["targetInfo"]["type"],
        json!("tab")
    );
    assert!(matches!(
        ctx.conn.session_route(Some(&second_tab_session_id)),
        Some(CdpSessionRoute::TabTarget {
            tab_target_id: ref route_tab_target_id,
            ..
        }) if route_tab_target_id == &tab_target_id
    ));

    ctx.process_async(json!({
        "id": 11012,
        "sessionId": tab_session_id.clone(),
        "method": "Runtime.evaluate",
        "params": { "expression": "1 + 1" }
    }))
    .await;
    let runtime_response = take_response_by_id(&mut ctx, 11012);
    assert_eq!(runtime_response["error"]["code"], json!(-32601));
    assert_eq!(
        runtime_response["error"]["message"],
        json!("'Runtime.evaluate' wasn't found")
    );

    ctx.process_async(json!({
        "id": 11013,
        "method": "Target.getTargets",
        "params": { "filter": [{ "type": "tab" }] }
    }))
    .await;
    let targets_response = take_response_by_id(&mut ctx, 11013);
    assert_eq!(
        targets_response["result"]["targetInfos"][0]["attached"],
        json!(true)
    );

    ctx.process_async(json!({
        "id": 11014,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": tab_session_id.clone() }
    }))
    .await;
    ctx.expect_result(11014, json!({}), None);
    let detached = ctx.take_first_matching("tab detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!(tab_target_id)
    });
    assert_eq!(detached["params"]["sessionId"], json!(tab_session_id));
    assert_eq!(ctx.conn.session_route(Some(&tab_session_id)), None);

    ctx.process_async(json!({
        "id": 11015,
        "method": "Target.getTargets"
    }))
    .await;
    let page_targets = take_response_by_id(&mut ctx, 11015);
    assert_eq!(
        page_targets["result"]["targetInfos"][0]["targetId"],
        json!(page_target_id)
    );
    assert_eq!(
        page_targets["result"]["targetInfos"][0]["type"],
        json!("page")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tab_and_page_target_handlers_cannot_manage_browser_contexts() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 11020,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 11020)["result"]["targetId"]
        .as_str()
        .expect("created Page target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11021,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id }
    }))
    .await;
    let tab_session_id = take_response_by_id(&mut ctx, 11021)["result"]["sessionId"]
        .as_str()
        .expect("attached Tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11022,
        "sessionId": tab_session_id,
        "method": "Target.getBrowserContexts"
    }))
    .await;
    expect_session_error(&mut ctx, 11022, &tab_session_id, -32000, "Not allowed");

    ctx.process_async(json!({
        "id": 11025,
        "sessionId": tab_session_id,
        "method": "Target.autoAttachRelated",
        "params": {
            "targetId": page_target_id,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    expect_session_error(
        &mut ctx,
        11025,
        &tab_session_id,
        -32000,
        "Target.autoAttachRelated is only supported on the Browser target",
    );

    ctx.process_async(json!({
        "id": 11023,
        "method": "Target.attachToTarget",
        "params": { "targetId": page_target_id }
    }))
    .await;
    let page_session_id = take_response_by_id(&mut ctx, 11023)["result"]["sessionId"]
        .as_str()
        .expect("attached Page session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11024,
        "sessionId": page_session_id,
        "method": "Target.createBrowserContext"
    }))
    .await;
    expect_session_error(&mut ctx, 11024, &page_session_id, -32000, "Not allowed");
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_target_handler_is_auto_attach_only() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-worker-access");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(11030),
        "TID-worker-access",
        "https://example.test/worker.js",
        "worker-access",
        None,
    );
    ctx.process_async(json!({
        "id": 11031,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-worker-access", "flatten": true }
    }))
    .await;
    let worker_session_id = take_response_by_id(&mut ctx, 11031)["result"]["sessionId"]
        .as_str()
        .expect("attached worker session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11032,
        "sessionId": worker_session_id,
        "method": "Target.getTargets"
    }))
    .await;
    expect_session_error(&mut ctx, 11032, &worker_session_id, -32000, "Not allowed");

    ctx.process_async(json!({
        "id": 11033,
        "sessionId": worker_session_id,
        "method": "Target.getTargetInfo"
    }))
    .await;
    let own_info = take_response_by_id(&mut ctx, 11033);
    assert_eq!(own_info["sessionId"], json!(worker_session_id));
    assert_eq!(
        own_info["result"]["targetInfo"]["targetId"],
        json!("TID-worker-access")
    );

    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(11035),
        "TID-worker-child",
        "https://example.test/worker-child.js",
        "worker-child",
        Some("SID-worker-child"),
    );
    ctx.conn.mark_session_auto_attached_for_test(
        "SID-worker-child".to_owned(),
        Some(&worker_session_id),
    );
    assert!(
        ctx.conn
            .target_handler_may_close_target(Some(&worker_session_id), "TID-worker-child"),
        "an AutoAttachOnly handler may close its auto-attached child"
    );

    ctx.process_async(json!({
        "id": 11035,
        "sessionId": worker_session_id,
        "method": "Target.getTargetInfo",
        "params": { "targetId": "TID-worker-child" }
    }))
    .await;
    expect_session_error(&mut ctx, 11035, &worker_session_id, -32000, "Not allowed");

    ctx.process_async(json!({
        "id": 11036,
        "sessionId": worker_session_id,
        "method": "Target.getTargetInfo",
        "params": { "targetId": "browser" }
    }))
    .await;
    expect_session_error(&mut ctx, 11036, &worker_session_id, -32000, "Not allowed");
    assert!(ctx.sent.is_empty());

    ctx.process_async(json!({
        "id": 11037,
        "sessionId": worker_session_id,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-worker-child" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 11037),
        json!({
            "id": 11037,
            "sessionId": worker_session_id,
            "result": { "success": true }
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_target_auto_attach_requires_explicit_flatten_true() {
    let mut ctx = TestContext::new();
    ctx.conn
        .register_browser_session("SID-browser-flatten".to_owned());

    for (id, flatten) in [(11040, None), (11041, Some(false))] {
        let mut params = json!({
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        });
        if let Some(flatten) = flatten {
            params["flatten"] = json!(flatten);
        }
        ctx.process_async(json!({
            "id": id,
            "sessionId": "SID-browser-flatten",
            "method": "Target.setAutoAttach",
            "params": params
        }))
        .await;
        expect_session_error(
            &mut ctx,
            id,
            "SID-browser-flatten",
            -32602,
            "Only flatten protocol is supported with browser level auto-attach",
        );
    }

    ctx.process_async(json!({
        "id": 11042,
        "sessionId": "SID-browser-flatten",
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{ "type": "page", "exclude": true }]
        }
    }))
    .await;
    ctx.expect_result(11042, json!({}), Some("SID-browser-flatten"));
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_tab_session_detaches_auto_attached_child_page_session() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 11020,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 11020)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11021,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id.clone() }
    }))
    .await;
    let tab_session_id = take_response_by_id(&mut ctx, 11021)["result"]["sessionId"]
        .as_str()
        .expect("tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11022,
        "sessionId": tab_session_id.clone(),
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{}]
        }
    }))
    .await;
    ctx.expect_result(11022, json!({}), Some(&tab_session_id));
    let page_attached = ctx.take_first_matching("child page attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["sessionId"] == json!(tab_session_id)
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    let page_session_id = page_attached["params"]["sessionId"]
        .as_str()
        .expect("page session id")
        .to_owned();
    assert!(ctx.conn.session_route(Some(&page_session_id)).is_some());
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 11023,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": tab_session_id.clone() }
    }))
    .await;

    ctx.expect_result(11023, json!({}), None);
    let page_detached = ctx.take_first_matching("child page detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!(page_target_id)
            && message["params"]["sessionId"] == json!(page_session_id)
    });
    assert_eq!(page_detached["params"]["reason"], Value::Null);
    let tab_detached = ctx.take_first_matching("tab detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["targetId"] == json!(tab_target_id)
            && message["params"]["sessionId"] == json!(tab_session_id)
    });
    assert_eq!(tab_detached["params"]["reason"], Value::Null);
    assert_eq!(ctx.conn.session_route(Some(&page_session_id)), None);
    assert_eq!(ctx.conn.session_route(Some(&tab_session_id)), None);

    ctx.process_async(json!({
        "id": 11024,
        "method": "Target.getTargets",
        "params": { "filter": [{}] }
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 11024);
    let target_infos = targets["result"]["targetInfos"]
        .as_array()
        .expect("targetInfos");
    assert!(
        target_infos
            .iter()
            .any(|info| info["targetId"] == json!(page_target_id)
                && info["type"] == json!("page")
                && info["attached"] == json!(false)),
        "detaching the tab session must not destroy the page target: {targets:?}"
    );
    assert!(
        target_infos
            .iter()
            .any(|info| info["targetId"] == json!(tab_target_id)
                && info["type"] == json!("tab")
                && info["attached"] == json!(false)),
        "detaching the tab session must keep the tab target discoverable: {targets:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_ensures_pending_background_initial_document_before_attached_event() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-active",
        "<!doctype html><title>active</title>",
    )
    .await;
    {
        let bc = ctx.conn.browser_context.as_mut().expect("browser context");
        bc.stage_background_target(
            "TID-background-pending".to_owned(),
            None,
            "about:blank#direct-attach".to_owned(),
            None,
            None,
        );
    }
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .pending_document_page_build_count(),
        1,
        "staged background target should begin as pending initial document page build"
    );

    ctx.process_async(json!({
        "id": 12001,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-background-pending"}
    }))
    .await;

    let attach_response = take_response_by_id(&mut ctx, 12001);
    let session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("attached session id")
        .to_owned();
    let attached_event = ctx.take_one();
    assert_eq!(attached_event["method"], "Target.attachedToTarget");
    assert_eq!(
        attached_event["params"]["targetInfo"]["targetId"],
        "TID-background-pending"
    );
    assert_eq!(
        attached_event["params"]["sessionId"],
        json!(session_id.as_str())
    );
    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            bc.pending_document_page_build_count(),
            0,
            "attachToTarget must complete initial document before emitting attachedToTarget"
        );
        assert!(
            bc.background_target("TID-background-pending")
                .expect("background target")
                .has_loaded_page(),
            "attached background target should expose a current Page immediately"
        );
    }

    ctx.process_async(json!({
        "id": 12002,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {"expression": "document.URL"}
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 12002);
    assert_eq!(
        evaluation["result"]["result"]["value"],
        json!("about:blank#direct-attach")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_existing_session_creates_distinct_attached_session() {
    // Chromium TargetHandler::AttachToTarget calls Session::Attach on every
    // invocation, even when the host already has another attached session.
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000B");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-primary");
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000B",
        "SID-primary",
        moli_page_types::DevToolsSessionKey::Primary,
    );

    ctx.process_async(json!({"id": 12, "method": "Target.attachToTarget",
                       "params": {"targetId": "TID-000000000B"}}))
        .await;

    let response = take_response_by_id(&mut ctx, 12);
    let attached_session_id = response["result"]["sessionId"]
        .as_str()
        .expect("attached session id")
        .to_owned();
    assert_ne!(attached_session_id, "SID-primary");
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(
        attached["params"]["sessionId"],
        json!(attached_session_id.as_str())
    );
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_session_id(), Some("SID-primary"));
    assert_eq!(
        bc.attached_target_id_for_session(&attached_session_id),
        Some("TID-000000000B")
    );

    ctx.process_async(json!({
        "id": 120,
        "method": "Runtime.evaluate",
        "sessionId": "SID-primary",
        "params": {
            "expression": "6 * 7",
            "returnByValue": true
        }
    }))
    .await;
    let primary_evaluation = take_response_by_id(&mut ctx, 120);
    assert_eq!(primary_evaluation["result"]["result"]["value"], json!(42));

    ctx.process_async(json!({
        "id": 121,
        "method": "Runtime.evaluate",
        "sessionId": attached_session_id,
        "params": {
            "expression": "21 + 21",
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 121);
    assert_eq!(evaluation["result"]["result"]["value"], json!(42));
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_keeps_background_target_background() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    push_background_target(&mut ctx, "TID-000000000B", "about:blank", None);

    ctx.process_async(json!({"id": 13, "method": "Target.attachToTarget",
                       "params": {"targetId": "TID-000000000B"}}))
        .await;

    let sid = "SID-1";
    ctx.expect_result(13, json!({ "sessionId": sid }), None);
    ctx.expect_event("Target.attachedToTarget", None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.background_target_count(), 1);
    assert_eq!(
        bc.background_target_at(0).unwrap().target_id(),
        "TID-000000000B"
    );
    assert_eq!(
        bc.background_target_at(0).unwrap().session_id(),
        Some("SID-1")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_shared_worker_target_creates_shared_worker_session() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(7),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        None,
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-shared-worker"}
    }))
    .await;

    let attach_response = ctx.take_response_by_id(13);
    let session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("shared worker session id")
        .to_owned();
    assert!(matches!(
        ctx.conn.session_route(Some(&session_id)),
        Some(CdpSessionRoute::SharedWorkerTarget {
            browser_context_id,
            target_id,
        }) if browser_context_id == "BID-9" && target_id == "TID-shared-worker"
    ));

    let attached_event = ctx.take_one();
    assert_eq!(attached_event["method"], "Target.attachedToTarget");
    assert_eq!(attached_event["params"]["sessionId"], session_id);
    assert_eq!(
        attached_event["params"]["targetInfo"]["targetId"],
        "TID-shared-worker"
    );
    assert_eq!(
        attached_event["params"]["targetInfo"]["type"],
        "shared_worker"
    );
    assert_eq!(attached_event["params"]["targetInfo"]["attached"], true);

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.getTargetInfo",
        "params": {"targetId": "TID-shared-worker"}
    }))
    .await;
    let info = ctx.take_response_by_id(14);
    assert_eq!(info["result"]["targetInfo"]["attached"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_shared_worker_target_existing_session_creates_independent_session() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(8),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        Some("SID-existing-shared"),
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-shared-worker"}
    }))
    .await;

    let attach_response = ctx.take_response_by_id(13);
    let new_session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("new shared worker session id")
        .to_owned();
    assert_ne!(new_session_id, "SID-existing-shared");
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": new_session_id,
            "targetInfo": {
                "targetId": "TID-shared-worker",
                "type": "shared_worker",
                "title": "shared",
                "url": "https://example.test/shared-worker.js",
                "attached": true,
                "canAccessOpener": false,
                "browserContextId": "BID-9"
            }
        })),
    );
    let target = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .shared_worker_target("TID-shared-worker")
        .expect("shared worker target should remain registered");
    assert!(target.is_session("SID-existing-shared"));
    assert!(target.is_session(&new_session_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_shared_worker_target_is_noop_success() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-active");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(9),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        Some("SID-shared-worker"),
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-shared-worker"}
    }))
    .await;

    ctx.expect_result(14, json!({}), None);
    assert!(ctx.sent.is_empty());
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-active"));
    let target = bc
        .shared_worker_target("TID-shared-worker")
        .expect("shared worker target must remain registered");
    assert_eq!(target.session_id(), Some("SID-shared-worker"));
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_target_session_does_not_fall_back_to_active_page_commands() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-active");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(12),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        Some("SID-shared-worker"),
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.createTarget",
        "sessionId": "SID-shared-worker",
        "params": {"url": "about:blank"}
    }))
    .await;

    let response = ctx.take_response_by_id(14);
    assert_eq!(response["sessionId"], "SID-shared-worker");
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(response["error"]["message"], "Not allowed");
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-active"));
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_browser_target_emits_browser_attached_event() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");

    ctx.process_async(json!({"id": 13, "method": "Target.attachToBrowserTarget"}))
        .await;

    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.attachedToTarget");
    assert_eq!(event["params"]["sessionId"], "SID-1");
    assert_eq!(event["params"]["targetInfo"]["targetId"], "browser");
    assert_eq!(event["params"]["targetInfo"]["type"], "browser");
    assert_eq!(event["params"]["targetInfo"]["url"], "");
    assert_eq!(event["params"]["targetInfo"]["title"], "");

    ctx.expect_result(13, json!({ "sessionId": "SID-1" }), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert!(ctx.conn.is_browser_session_id(Some("SID-1")));
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_browser_target_does_not_reuse_page_session() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-7");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-7");

    ctx.process_async(json!({"id": 14, "method": "Target.attachToBrowserTarget"}))
        .await;

    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.attachedToTarget");
    assert_eq!(event["params"]["sessionId"], "SID-1");
    ctx.expect_result(14, json!({ "sessionId": "SID-1" }), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_session_id(), Some("SID-7"));
    assert!(ctx.conn.is_browser_session_id(Some("SID-1")));
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_browser_target_session_cascades_owned_target_sessions() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-page");

    ctx.process_async(json!({"id": 15, "method": "Target.attachToBrowserTarget"}))
        .await;
    let browser_attached = ctx.take_one();
    let browser_session_id = browser_attached["params"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    ctx.expect_result(15, json!({ "sessionId": browser_session_id.clone() }), None);

    ctx.process_async(json!({
        "id": 16,
        "method": "Target.attachToTarget",
        "sessionId": browser_session_id,
        "params": { "targetId": "TID-page", "flatten": true }
    }))
    .await;
    let attach_response = ctx.take_response_by_id(16);
    let page_session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("page session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);
    assert!(matches!(
        ctx.conn.session_route(Some(&page_session_id)),
        Some(CdpSessionRoute::PageTarget {
            session_key: moli_page_types::DevToolsSessionKey::Attached(_),
            ..
        })
    ));

    ctx.process_async(json!({
        "id": 17,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": browser_session_id }
    }))
    .await;

    ctx.expect_result(17, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-page",
            "sessionId": page_session_id,
        })),
    );
    assert_eq!(ctx.conn.session_route(Some(&browser_session_id)), None);
    assert_eq!(ctx.conn.session_route(Some(&page_session_id)), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_attached_page_session_cascades_owned_target_sessions() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-attached-cascade", "TID-page");

    ctx.process_async(json!({"id": 151, "method": "Target.attachToBrowserTarget"}))
        .await;
    let browser_session_id = ctx.take_one()["params"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    ctx.expect_result(
        151,
        json!({ "sessionId": browser_session_id.as_str() }),
        None,
    );

    ctx.process_async(json!({
        "id": 152,
        "method": "Target.attachToTarget",
        "sessionId": browser_session_id,
        "params": { "targetId": "TID-page", "flatten": true }
    }))
    .await;
    let page_session_id = ctx.take_response_by_id(152)["result"]["sessionId"]
        .as_str()
        .expect("attached page session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 153,
        "method": "Target.attachToTarget",
        "sessionId": page_session_id,
        "params": { "targetId": "TID-page", "flatten": true }
    }))
    .await;
    let child_session_id = ctx.take_response_by_id(153)["result"]["sessionId"]
        .as_str()
        .expect("child page session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);
    assert!(ctx.conn.session_route(Some(&page_session_id)).is_some());
    assert!(ctx.conn.session_route(Some(&child_session_id)).is_some());

    ctx.process_async(json!({
        "id": 154,
        "method": "Target.detachFromTarget",
        "sessionId": browser_session_id,
        "params": { "sessionId": page_session_id }
    }))
    .await;

    ctx.expect_result(154, json!({}), Some(&browser_session_id));
    assert_eq!(ctx.conn.session_route(Some(&page_session_id)), None);
    assert_eq!(
        ctx.conn.session_route(Some(&child_session_id)),
        None,
        "detaching a direct page frontend's attached session must release its child sessions"
    );
    let detached_child = ctx.take_first_matching("child detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["sessionId"] == json!(child_session_id)
    });
    assert_eq!(detached_child["sessionId"], json!(page_session_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn root_frontend_release_preserves_private_browser_owned_page_session() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-root-release", "TID-root-release");

    ctx.process_async(json!({"id": 18, "method": "Target.attachToBrowserTarget"}))
        .await;
    let private_browser_session_id = ctx.take_one()["params"]["sessionId"]
        .as_str()
        .expect("private browser session id")
        .to_owned();
    ctx.expect_result(
        18,
        json!({ "sessionId": private_browser_session_id.clone() }),
        None,
    );

    ctx.process_async(json!({
        "id": 19,
        "method": "Target.attachToTarget",
        "sessionId": private_browser_session_id,
        "params": { "targetId": "TID-root-release", "flatten": true }
    }))
    .await;
    let private_page_session_id = ctx.take_response_by_id(19)["result"]["sessionId"]
        .as_str()
        .expect("private page session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 20,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-root-release", "flatten": true }
    }))
    .await;
    let root_page_session_id = ctx.take_response_by_id(20)["result"]["sessionId"]
        .as_str()
        .expect("root page session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.conn
        .set_target_discovery_for_owner_from_devtools_filter(None, None);
    ctx.conn
        .set_target_discovery_for_owner_from_devtools_filter(
            Some(&private_browser_session_id),
            None,
        );
    ctx.conn.release_root_target_frontend_state_async().await;

    assert_eq!(ctx.conn.session_route(Some(&root_page_session_id)), None);
    assert!(
        ctx.conn
            .is_browser_session_id(Some(&private_browser_session_id))
    );
    assert!(matches!(
        ctx.conn.session_route(Some(&private_page_session_id)),
        Some(CdpSessionRoute::PageTarget {
            session_key: moli_page_types::DevToolsSessionKey::Attached(_),
            ..
        })
    ));
    assert!(ctx.conn.target_discovery_filter_for_owner(None).is_none());
    assert!(
        ctx.conn
            .target_discovery_filter_for_owner(Some(&private_browser_session_id))
            .is_some()
    );
    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(browser_context.active_session_id(), None);
    assert_eq!(
        browser_context.attached_target_id_for_session(&private_page_session_id),
        Some("TID-root-release")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_target_session_survives_browser_context_disposal() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({"id": 1, "method": "Target.createBrowserContext"}))
        .await;
    ctx.expect_result(1, json!({ "browserContextId": "BID-1" }), None);

    ctx.process_async(json!({"id": 2, "method": "Target.attachToBrowserTarget"}))
        .await;
    let browser_event = ctx.take_one();
    let browser_session_id = browser_event["params"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    ctx.expect_result(2, json!({ "sessionId": browser_session_id.clone() }), None);

    ctx.process_async(json!({
        "id": 3,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-1" }
    }))
    .await;
    ctx.expect_result(3, json!({}), None);

    ctx.process_async(json!({"id": 4, "method": "Target.createBrowserContext"}))
        .await;
    ctx.expect_result(4, json!({ "browserContextId": "BID-2" }), None);
    ctx.process_async(json!({
        "id": 5,
        "method": "Target.createTarget",
        "params": { "browserContextId": "BID-2", "url": "about:blank" }
    }))
    .await;
    let target_id = ctx.take_response_by_id(5)["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    ctx.expect_event("Target.targetCreated", None);

    ctx.process_async(json!({
        "id": 6,
        "method": "Target.attachToTarget",
        "sessionId": browser_session_id,
        "params": { "targetId": target_id, "flatten": true }
    }))
    .await;

    let attach_response = ctx.take_response_by_id(6);
    assert_eq!(attach_response["sessionId"], browser_session_id);
    assert!(
        attach_response["result"]["sessionId"].is_string(),
        "attach response should include an attached target session"
    );
    let attached_event = ctx.take_one();
    assert_eq!(attached_event["method"], "Target.attachedToTarget");
    assert_eq!(attached_event["sessionId"], browser_session_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_route_finds_committed_browser_page_and_worker_sessions() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-000000000A");
    ctx.conn.register_browser_session("SID-browser".to_owned());
    {
        let bc = ctx.conn.browser_context.as_mut().unwrap();
        bc.attach_active_session("SID-active");
        assert!(bc.assign_attached_session_to_target("TID-000000000A", "SID-attached".to_owned()));
    }
    register_page_session_route(
        &mut ctx,
        "BID-A",
        "TID-000000000A",
        "SID-active",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    register_page_session_route(
        &mut ctx,
        "BID-A",
        "TID-000000000A",
        "SID-attached",
        moli_page_types::DevToolsSessionKey::Attached("SID-attached".to_owned()),
    );
    push_background_target(
        &mut ctx,
        "TID-000000000B",
        "about:blank",
        Some("SID-background"),
    );
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(41),
        "TID-shared-active",
        "https://example.test/shared-worker.js",
        "shared-active",
        Some("SID-shared-active"),
    );

    let mut inactive = BrowserContext::new("BID-B".to_owned());
    inactive.set_active_target_id("TID-000000000C".to_owned());
    inactive.attach_active_session("SID-inactive");
    inactive.insert_page_target_host(crate::conn::PageTargetHost::new(
        "TID-000000000D".to_owned(),
        Some("SID-inactive-background".to_owned()),
        crate::conn::TargetIdentityState::about_blank(),
        crate::conn::TargetPageSlot::empty_for_test_fixture(),
    ));
    assert!(inactive.assign_attached_session_to_target(
        "TID-000000000D",
        "SID-inactive-attached-background".to_owned()
    ));
    let mut inactive_shared_worker = crate::conn::SharedWorkerTargetState::new(
        moli_core::RendererOwnerLocalHostId::new_for_testing(1),
        SharedWorkerInstanceId::from_u64(42),
        "TID-shared-inactive".to_owned(),
        None,
        "https://inactive.example.test/shared-worker.js".to_owned(),
        "shared-inactive".to_owned(),
    );
    inactive_shared_worker.attach_session("SID-shared-inactive".to_owned());
    inactive.insert_shared_worker_target(inactive_shared_worker);
    ctx.conn
        .push_inactive_browser_context_fixture_for_test(inactive);
    for (session_id, route) in [
        (
            "SID-inactive",
            CdpSessionRoute::PageTarget {
                browser_context_id: "BID-B".to_owned(),
                target_id: "TID-000000000C".to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            },
        ),
        (
            "SID-inactive-background",
            CdpSessionRoute::PageTarget {
                browser_context_id: "BID-B".to_owned(),
                target_id: "TID-000000000D".to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            },
        ),
        (
            "SID-inactive-attached-background",
            CdpSessionRoute::PageTarget {
                browser_context_id: "BID-B".to_owned(),
                target_id: "TID-000000000D".to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Attached(
                    "SID-inactive-attached-background".to_owned(),
                ),
            },
        ),
        (
            "SID-shared-inactive",
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id: "BID-B".to_owned(),
                target_id: "TID-shared-inactive".to_owned(),
            },
        ),
    ] {
        ctx.conn.register_session_route_for_test(session_id, route);
    }

    assert_eq!(
        ctx.conn.session_route(Some("SID-browser")),
        Some(CdpSessionRoute::Browser)
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-active")),
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: "BID-A".to_owned(),
            target_id: "TID-000000000A".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        })
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-attached")),
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: "BID-A".to_owned(),
            target_id: "TID-000000000A".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Attached("SID-attached".to_owned(),),
        })
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-background")),
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: "BID-A".to_owned(),
            target_id: "TID-000000000B".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        })
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-shared-active")),
        Some(CdpSessionRoute::SharedWorkerTarget {
            browser_context_id: "BID-A".to_owned(),
            target_id: "TID-shared-active".to_owned(),
        })
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-inactive")),
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: "BID-B".to_owned(),
            target_id: "TID-000000000C".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        })
    );
    assert_eq!(ctx.conn.session_route(Some("SID-missing")), None);
    assert_eq!(
        ctx.conn.session_route(Some("SID-inactive-background")),
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: "BID-B".to_owned(),
            target_id: "TID-000000000D".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        })
    );
    assert_eq!(
        ctx.conn
            .session_route(Some("SID-inactive-attached-background")),
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: "BID-B".to_owned(),
            target_id: "TID-000000000D".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Attached(
                "SID-inactive-attached-background".to_owned(),
            ),
        })
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-shared-inactive")),
        Some(CdpSessionRoute::SharedWorkerTarget {
            browser_context_id: "BID-B".to_owned(),
            target_id: "TID-shared-inactive".to_owned(),
        })
    );
    ctx.conn
        .browser_context_by_id_mut("BID-B")
        .expect("inactive browser context")
        .page_target_mut("TID-000000000D")
        .expect("stable page target")
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .console_output_session_state
        .console_enabled = true;
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "direct background route helpers must not activate inactive browser contexts"
    );
    assert!(
        ctx.conn
            .browser_context_by_id("BID-B")
            .expect("inactive browser context")
            .page_target("TID-000000000D")
            .expect("stable page target")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled
    );

    assert!(
        ctx.conn
            .activate_browser_context_for_session_async("SID-inactive")
            .await
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-B")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_from_browser_session_creates_distinct_attached_session() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000B");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-page");

    ctx.process_async(json!({"id": 15, "method": "Target.attachToBrowserTarget"}))
        .await;
    let browser_event = ctx.take_one();
    let browser_session_id = browser_event["params"]["sessionId"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    ctx.expect_result(15, json!({ "sessionId": browser_session_id.clone() }), None);

    ctx.process_async(json!({
        "id": 16,
        "method": "Target.attachToTarget",
        "sessionId": browser_session_id,
        "params": {"targetId": "TID-000000000B"}
    }))
    .await;

    let attach_response = ctx.take_response_by_id(16);
    assert_eq!(attach_response["sessionId"], browser_session_id);
    let target_session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("target session id");
    assert_ne!(target_session_id, "SID-page");
    assert_ne!(target_session_id, browser_session_id);

    let attached_event = ctx.take_one();
    assert_eq!(attached_event["method"], "Target.attachedToTarget");
    assert_eq!(attached_event["sessionId"], browser_session_id);
    assert_eq!(attached_event["params"]["sessionId"], target_session_id);
    assert_eq!(
        attached_event["params"]["targetInfo"]["targetId"],
        "TID-000000000B"
    );

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_session_id(), Some("SID-page"));
    assert_eq!(
        bc.attached_target_id_for_session(target_session_id),
        Some("TID-000000000B")
    );

    ctx.process_async(json!({
        "id": 17,
        "method": "Network.enable",
        "sessionId": target_session_id
    }))
    .await;
    ctx.expect_result(17, json!({}), Some(target_session_id));

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        bc.active_page_target()
            .runtime_slot
            .has_attached_network_events_for_session(target_session_id)
    );
}

/// cdp.target: issue#474 – attach to just-created target
#[tokio::test(flavor = "multi_thread")]
async fn attach_to_just_created_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                       "params": {"browserContextId": "BID-9"}}))
        .await;
    let tid = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_target_id_owned()
        .unwrap();
    ctx.expect_result(10, json!({ "targetId": tid }), None);

    ctx.process_async(json!({"id": 11, "method": "Target.attachToTarget",
                       "params": {"targetId": tid}}))
        .await;
    let sid = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_session_id_owned()
        .unwrap();
    ctx.expect_result(11, json!({ "sessionId": sid }), None);
}

/// cdp.target: detachFromTarget
#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    ctx.process_async(json!({"id": 10, "method": "Target.createTarget",
                       "params": {"browserContextId": "BID-9"}}))
        .await;
    let tid = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_target_id_owned()
        .unwrap();
    ctx.expect_result(10, json!({ "targetId": tid }), None);

    ctx.process_async(json!({"id": 11, "method": "Target.attachToTarget",
                       "params": {"targetId": tid}}))
        .await;
    let sid = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_session_id_owned()
        .unwrap();
    ctx.expect_result(11, json!({ "sessionId": sid }), None);

    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    bc.active_page_target_mut()
        .mutate_devtools_network_session_state(
            &moli_page_types::DevToolsSessionKey::Primary,
            |network| {
                network.network_enabled = true;
                network.cache_disabled = true;
                network.bypass_service_worker = true;
                network.extra_headers = vec![("X-Test".into(), "1".into())];
            },
        );
    bc.active_page_target_mut().css_enabled = true;
    bc.active_page_target_mut().input_intercept_drags_enabled = true;
    bc.active_page_target_mut().input_drag_intercepted = true;
    bc.active_page_target_mut().fetch_owner.configure(
        Some(sid.clone()),
        true,
        vec![crate::conn::FetchInterceptionPattern {
            url_pattern: "*".into(),
            resource_type_filter: None,
            request_stage: crate::conn::FetchRequestStage::Request,
        }],
    );

    ctx.process_async(json!({"id": 12, "method": "Target.detachFromTarget",
                       "params": {"targetId": tid}}))
        .await;
    ctx.expect_result(12, json!({}), None);
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert!(
        !bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_lifecycle_events
    );
    assert!(
        !bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled
    );
    assert!(
        !bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .inspector_enabled
    );
    assert!(
        !bc.active_page_target()
            .runtime_slot
            .primary_network_events_enabled()
    );
    assert!(!bc.active_page_target().effective_policy().cache_disabled());
    assert!(
        !bc.active_page_target()
            .effective_policy()
            .bypass_service_worker()
    );
    assert!(!bc.active_page_target().css_enabled);
    assert!(!bc.active_page_target().input_intercept_drags_enabled);
    assert!(!bc.active_page_target().input_drag_intercepted);
    assert!(!bc.active_page_target().fetch_owner.is_enabled());
    assert!(!bc.active_page_target().fetch_owner.handle_auth_requests());
    assert!(
        bc.active_page_target()
            .fetch_owner
            .config_snapshot()
            .patterns()
            .is_empty()
    );
    assert!(
        bc.active_page_target()
            .effective_policy()
            .extra_headers()
            .is_empty()
    );

    // Attach again after detach.
    ctx.process_async(json!({"id": 13, "method": "Target.attachToTarget",
                       "params": {"targetId": tid}}))
        .await;
    let sid2 = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_session_id_owned()
        .unwrap();
    ctx.expect_result(13, json!({ "sessionId": sid2 }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target_drops_only_selected_page_renderer_inspector_session() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-detach-inspector",
        "TID-detach-inspector",
        "<!doctype html><body>detach inspector</body>",
    )
    .await;
    {
        let bc = ctx.conn.browser_context.as_mut().unwrap();
        bc.attach_active_session("SID-detach-primary");
        assert!(bc.assign_attached_session_to_target(
            "TID-detach-inspector",
            "SID-detach-attached".to_owned()
        ));
    }
    register_page_session_route(
        &mut ctx,
        "BID-detach-inspector",
        "TID-detach-inspector",
        "SID-detach-primary",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    register_page_session_route(
        &mut ctx,
        "BID-detach-inspector",
        "TID-detach-inspector",
        "SID-detach-attached",
        moli_page_types::DevToolsSessionKey::Attached("SID-detach-attached".to_owned()),
    );
    ctx.sent.clear();

    let baseline_session_count =
        page_renderer_inspector_session_count(&mut ctx, None, "before Runtime.enable").await;
    ctx.process_async(json!({
        "id": 120_001,
        "sessionId": "SID-detach-primary",
        "method": "Runtime.enable"
    }))
    .await;
    ctx.expect_result(120_001, json!({}), Some("SID-detach-primary"));
    ctx.process_async(json!({
        "id": 120_002,
        "sessionId": "SID-detach-attached",
        "method": "Runtime.enable"
    }))
    .await;
    ctx.expect_result(120_002, json!({}), Some("SID-detach-attached"));
    ctx.sent.clear();

    assert_eq!(
        page_renderer_inspector_session_count(&mut ctx, None, "after both sessions enabled").await,
        baseline_session_count + 1,
        "primary Runtime.enable must reuse the target default Inspector session while attached Runtime.enable adds one session"
    );

    ctx.process_async(json!({
        "id": 120_004,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-detach-inspector",
            "sessionId": "SID-detach-attached"
        }
    }))
    .await;
    ctx.expect_result(120_004, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-detach-inspector",
            "sessionId": "SID-detach-attached"
        })),
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Target.detachedFromTarget")),
        "attached detach must publish exactly one target detach event"
    );
    assert_eq!(
        page_renderer_inspector_session_count(&mut ctx, None, "after attached detach").await,
        baseline_session_count,
        "attached detach should drop only that renderer V8 inspector session"
    );
    ctx.process_async(json!({
        "id": 120_005,
        "sessionId": "SID-detach-primary",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "6 * 7",
            "returnByValue": true
        }
    }))
    .await;
    let primary_evaluation = take_response_by_id(&mut ctx, 120_005);
    assert_eq!(primary_evaluation["result"]["result"]["value"], json!(42));

    {
        let bc = ctx.conn.browser_context.as_mut().unwrap();
        assert!(bc.assign_attached_session_to_target(
            "TID-detach-inspector",
            "SID-detach-attached-replacement".to_owned()
        ));
    }
    register_page_session_route(
        &mut ctx,
        "BID-detach-inspector",
        "TID-detach-inspector",
        "SID-detach-attached-replacement",
        moli_page_types::DevToolsSessionKey::Attached("SID-detach-attached-replacement".to_owned()),
    );
    ctx.process_async(json!({
        "id": 120_006,
        "sessionId": "SID-detach-attached-replacement",
        "method": "Runtime.enable"
    }))
    .await;
    ctx.expect_result(120_006, json!({}), Some("SID-detach-attached-replacement"));
    ctx.sent.clear();
    assert_eq!(
        page_renderer_inspector_session_count(&mut ctx, None, "after replacement attached enable",)
            .await,
        baseline_session_count + 1
    );
    ctx.process_async(json!({
        "id": 120_007,
        "sessionId": "SID-detach-attached-replacement",
        "method": "Network.enable"
    }))
    .await;
    ctx.expect_result(120_007, json!({}), Some("SID-detach-attached-replacement"));
    ctx.process_async(json!({
        "id": 120_008,
        "sessionId": "SID-detach-attached-replacement",
        "method": "Network.setCacheDisabled",
        "params": { "cacheDisabled": true }
    }))
    .await;
    ctx.expect_result(120_008, json!({}), Some("SID-detach-attached-replacement"));

    ctx.process_async(json!({
        "id": 120_009,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-detach-inspector",
            "sessionId": "SID-detach-primary"
        }
    }))
    .await;
    ctx.expect_result(120_009, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-detach-inspector",
            "sessionId": "SID-detach-primary"
        })),
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Target.detachedFromTarget")),
        "primary detach must publish exactly one target detach event"
    );
    assert_eq!(
        page_renderer_inspector_session_count(
            &mut ctx,
            Some("SID-detach-attached-replacement"),
            "after primary detach",
        )
        .await,
        baseline_session_count,
        "primary detach must release the default Inspector session while preserving the attached replacement session"
    );
    ctx.process_async(json!({
        "id": 120_010,
        "sessionId": "SID-detach-primary",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "6 * 7",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_error(120_010, -32001, "Unknown sessionId");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        bc.attached_target_id_for_session("SID-detach-attached-replacement"),
        Some("TID-detach-inspector"),
        "detaching primary must preserve the attached session binding"
    );
    assert!(
        bc.active_page_target().effective_policy().cache_disabled(),
        "detaching primary must preserve the attached Network handler state"
    );

    ctx.process_async(json!({
        "id": 120_011,
        "sessionId": "SID-detach-attached-replacement",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "21 + 21",
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 120_011);
    assert_eq!(evaluation["result"]["result"]["value"], json!(42));

    ctx.process_async(json!({
        "id": 120_012,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-detach-inspector",
            "sessionId": "SID-detach-attached-replacement"
        }
    }))
    .await;
    ctx.expect_result(120_012, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-detach-inspector",
            "sessionId": "SID-detach-attached-replacement"
        })),
    );
    ctx.process_async(json!({
        "id": 120_013,
        "sessionId": "SID-detach-attached-replacement",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "21 + 21",
            "returnByValue": true
        }
    }))
    .await;
    ctx.expect_error(120_013, -32001, "Unknown sessionId");

    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-detach-diagnostic");
    register_page_session_route(
        &mut ctx,
        "BID-detach-inspector",
        "TID-detach-inspector",
        "SID-detach-diagnostic",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    ctx.process_async(json!({
        "id": 120_014,
        "sessionId": "SID-detach-diagnostic",
        "method": "Runtime.enable"
    }))
    .await;
    ctx.expect_result(120_014, json!({}), Some("SID-detach-diagnostic"));
    assert_eq!(
        page_renderer_inspector_session_count(&mut ctx, None, "after fresh primary attach").await,
        baseline_session_count,
        "a fresh primary session must not observe a leaked attached renderer session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target_removes_only_selected_session_document_start_scripts() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-detach-preload",
        "TID-detach-preload",
        "<!doctype html><body>detach preload</body>",
    )
    .await;
    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.attach_active_session("SID-preload-primary");
        assert!(browser_context.assign_attached_session_to_target(
            "TID-detach-preload",
            "SID-preload-attached".to_owned(),
        ));
    }
    register_page_session_route(
        &mut ctx,
        "BID-detach-preload",
        "TID-detach-preload",
        "SID-preload-primary",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    register_page_session_route(
        &mut ctx,
        "BID-detach-preload",
        "TID-detach-preload",
        "SID-preload-attached",
        moli_page_types::DevToolsSessionKey::Attached("SID-preload-attached".to_owned()),
    );
    ctx.sent.clear();

    for (id, session_id, source) in [
        (
            120_101,
            "SID-preload-primary",
            "globalThis.__primaryPreload = 'primary';",
        ),
        (
            120_102,
            "SID-preload-attached",
            "globalThis.__auxPreload = 'attached';",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "params": { "source": source }
        }))
        .await;
        assert_eq!(
            take_response_by_id(&mut ctx, id)["result"]["identifier"],
            json!("1"),
            "each DevTools session must own an independent script identifier namespace"
        );
    }

    ctx.process_async(json!({
        "id": 120_107,
        "sessionId": "SID-preload-attached",
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "params": { "source": "globalThis.__auxSecondPreload = 'attached-2';" }
    }))
    .await;
    ctx.expect_result(
        120_107,
        json!({ "identifier": "2" }),
        Some("SID-preload-attached"),
    );

    ctx.process_async(json!({
        "id": 120_108,
        "sessionId": "SID-preload-attached",
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "params": { "identifier": "1" }
    }))
    .await;
    ctx.expect_result(120_108, json!({}), Some("SID-preload-attached"));
    ctx.process_async(json!({
        "id": 120_109,
        "sessionId": "SID-preload-attached",
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "params": { "identifier": "1" }
    }))
    .await;
    ctx.expect_error(120_109, -32000, "Script not found");
    {
        let owner_state = &ctx
            .conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .owner_state;
        assert_eq!(owner_state.document_start_scripts.len(), 2);
        assert!(
            owner_state
                .document_start_scripts
                .iter()
                .any(|(identifier, script)| {
                    identifier == "1"
                        && script.devtools_session
                            == Some(moli_page_types::DevToolsSessionKey::Primary)
                })
        );
        assert!(
            owner_state
                .document_start_scripts
                .iter()
                .any(|(identifier, script)| {
                    identifier == "2"
                        && script.devtools_session
                            == Some(moli_page_types::DevToolsSessionKey::Attached(
                                "SID-preload-attached".to_owned(),
                            ))
                })
        );
    }

    ctx.process_async(json!({
        "id": 120_103,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-detach-preload",
            "sessionId": "SID-preload-attached"
        }
    }))
    .await;
    ctx.expect_result(120_103, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-detach-preload",
            "sessionId": "SID-preload-attached"
        })),
    );

    {
        let owner_state = &ctx
            .conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .owner_state;
        assert_eq!(owner_state.document_start_scripts.len(), 1);
        assert_eq!(
            owner_state
                .document_start_scripts
                .iter()
                .filter_map(|(_, script)| script.devtools_session.clone())
                .collect::<Vec<_>>(),
            vec![moli_page_types::DevToolsSessionKey::Primary]
        );
    }

    ctx.process_async(json!({
        "id": 120_105,
        "sessionId": "SID-preload-primary",
        "method": "Page.navigate",
        "params": { "url": "data:text/html,<body>replacement</body>" }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 120_105);
    assert_eq!(navigation["result"]["frameId"], json!("TID-detach-preload"));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 120_106,
        "sessionId": "SID-preload-primary",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "JSON.stringify({ primary: globalThis.__primaryPreload ?? null, attached: globalThis.__auxPreload ?? null })",
            "returnByValue": true
        }
    }))
    .await;
    let replacement = take_response_by_id(&mut ctx, 120_106);
    assert_eq!(
        replacement["result"]["result"]["value"],
        json!(r#"{"primary":"primary","attached":null}"#),
        "only the surviving session script must replay into the replacement Document"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_fail_closes_page_before_retiring_unremovable_session_scripts() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-detach-cleanup-failure",
        "TID-detach-cleanup-failure",
        "<!doctype html><body>detach cleanup failure</body>",
    )
    .await;
    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.attach_active_session("SID-cleanup-primary");
        assert!(browser_context.assign_attached_session_to_target(
            "TID-detach-cleanup-failure",
            "SID-cleanup-attached".to_owned(),
        ));
    }
    register_page_session_route(
        &mut ctx,
        "BID-detach-cleanup-failure",
        "TID-detach-cleanup-failure",
        "SID-cleanup-primary",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    register_page_session_route(
        &mut ctx,
        "BID-detach-cleanup-failure",
        "TID-detach-cleanup-failure",
        "SID-cleanup-attached",
        moli_page_types::DevToolsSessionKey::Attached("SID-cleanup-attached".to_owned()),
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 120_110,
        "sessionId": "SID-cleanup-attached",
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "params": { "source": "globalThis.__mustNotOutliveSession = true;" }
    }))
    .await;
    ctx.expect_result(
        120_110,
        json!({ "identifier": "1" }),
        Some("SID-cleanup-attached"),
    );

    ctx.conn
        .browser_context
        .as_ref()
        .and_then(|browser_context| browser_context.active_page_target().loaded_page())
        .expect("fixture should retain a loaded Page")
        .crash_devtools_target_from_io();
    let cleanup_error = ctx
        .conn
        .remove_document_start_scripts_for_detached_session_async("SID-cleanup-attached")
        .await
        .expect_err("a closed renderer ingress must reject script cleanup");
    assert!(
        cleanup_error
            .to_string()
            .contains("document-start script cleanup"),
        "unexpected cleanup failure: {cleanup_error:#}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .owner_state
            .document_start_scripts
            .iter()
            .any(|(_, script)| {
                script.devtools_session
                    == Some(moli_page_types::DevToolsSessionKey::Attached(
                        "SID-cleanup-attached".to_owned(),
                    ))
            }),
        "failed renderer cleanup must retain protocol-side retry authority"
    );

    ctx.process_async(json!({
        "id": 120_111,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-detach-cleanup-failure",
            "sessionId": "SID-cleanup-attached"
        }
    }))
    .await;
    ctx.expect_result(120_111, json!({}), None);

    let target = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target();
    assert!(target.owner_state.target_crash_state.is_crashed());
    assert!(target.loaded_page().is_none());
    assert!(
        target
            .owner_state
            .document_start_scripts
            .iter()
            .all(|(_, script)| {
                script.devtools_session
                    != Some(moli_page_types::DevToolsSessionKey::Attached(
                        "SID-cleanup-attached".to_owned(),
                    ))
            }),
        "session records may retire after the failed renderer has been closed"
    );
    assert_eq!(
        ctx.conn.session_route(Some("SID-cleanup-attached")),
        None,
        "the binding should commit only after renderer ownership is gone"
    );
    assert!(
        ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Inspector.targetCrashed")),
        "cleanup fail-close should be observable to attached Inspector clients: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_fail_closes_page_when_fetch_disable_cannot_reach_renderer() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-detach-fetch-cleanup-failure",
        "TID-detach-fetch-cleanup-failure",
        "<!doctype html><body>detach fetch cleanup failure</body>",
    )
    .await;
    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.attach_active_session("SID-fetch-cleanup-primary");
        assert!(browser_context.assign_attached_session_to_target(
            "TID-detach-fetch-cleanup-failure",
            "SID-fetch-cleanup-attached".to_owned(),
        ));
    }
    register_page_session_route(
        &mut ctx,
        "BID-detach-fetch-cleanup-failure",
        "TID-detach-fetch-cleanup-failure",
        "SID-fetch-cleanup-primary",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    register_page_session_route(
        &mut ctx,
        "BID-detach-fetch-cleanup-failure",
        "TID-detach-fetch-cleanup-failure",
        "SID-fetch-cleanup-attached",
        moli_page_types::DevToolsSessionKey::Attached("SID-fetch-cleanup-attached".to_owned()),
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 120_112,
        "sessionId": "SID-fetch-cleanup-attached",
        "method": "Fetch.enable"
    }))
    .await;
    ctx.expect_result(120_112, json!({}), Some("SID-fetch-cleanup-attached"));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .fetch_owner
            .is_enabled(),
        "the attached session must own renderer Fetch interception before detach"
    );

    ctx.conn
        .browser_context
        .as_ref()
        .and_then(|browser_context| browser_context.active_page_target().loaded_page())
        .expect("fixture should retain a loaded Page")
        .crash_devtools_target_from_io();

    ctx.process_async(json!({
        "id": 120_113,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-detach-fetch-cleanup-failure",
            "sessionId": "SID-fetch-cleanup-attached"
        }
    }))
    .await;
    ctx.expect_result(120_113, json!({}), None);

    let target = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target();
    assert!(
        target.owner_state.target_crash_state.is_crashed(),
        "a failed renderer Fetch cleanup must fail the Page closed"
    );
    assert!(target.loaded_page().is_none());
    assert!(!target.fetch_owner.is_enabled());
    assert_eq!(
        ctx.conn.session_route(Some("SID-fetch-cleanup-attached")),
        None,
        "the binding should commit only after renderer ownership is gone"
    );
    assert!(
        ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Inspector.targetCrashed")),
        "Fetch cleanup fail-close should be observable to Inspector clients: {:?}",
        ctx.sent
    );
}

async fn page_renderer_inspector_session_count(
    ctx: &mut TestContext,
    inspector_session_id: Option<&str>,
    stage: &str,
) -> u64 {
    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|bc| bc.active_page_target_mut().runtime_slot.loaded_page_mut())
        .expect("active target should still have a loaded page");
    // This diagnostic bypasses CdpConnection's session-aware Page accessor, so
    // bind the exact surviving Inspector route instead of inheriting whichever
    // session the prior command happened to stamp on the Page facade.
    page.set_renderer_devtools_command_session_id(inspector_session_id.map(str::to_owned));
    let response = page
        .runtime_heap_usage_async()
        .await
        .unwrap_or_else(|error| {
            panic!("runtime heap usage diagnostics should be available {stage}: {error}")
        });
    u64::try_from(response.moli.runtime.inspector_session_count)
        .expect("inspector session count should fit u64")
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target_emits_detached_event() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000C");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-1");
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000C",
        "SID-1",
        moli_page_types::DevToolsSessionKey::Primary,
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-000000000C",
            "sessionId": "SID-1"
        }
    }))
    .await;

    ctx.expect_result(14, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000C",
            "sessionId": "SID-1",
        })),
    );
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert_eq!(bc.active_target_id(), Some("TID-000000000C"));
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_shared_worker_target_clears_session_and_emits_detached_event() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(9),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        Some("SID-shared-worker"),
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-shared-worker",
            "sessionId": "SID-shared-worker"
        }
    }))
    .await;

    ctx.expect_result(14, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-shared-worker",
            "sessionId": "SID-shared-worker",
        })),
    );
    assert_eq!(ctx.conn.session_route(Some("SID-shared-worker")), None);
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let target = bc
        .shared_worker_target("TID-shared-worker")
        .expect("shared worker target remains live after detach");
    assert_eq!(target.session_id(), None);
    assert_eq!(
        bc.target_info("TID-shared-worker").unwrap()["attached"],
        false
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_service_worker_target_disposes_session_runtime_state() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_service_worker_target(
        &mut ctx,
        91,
        "TID-service-worker",
        "https://example.test/service-worker.js",
        "https://example.test/",
        Some("SID-service-worker"),
    );
    {
        let target = ctx
            .conn
            .service_worker_target_for_session_mut(Some("SID-service-worker"))
            .expect("service worker target should be attached");
        target.register_pending_inspector_await(
            "SID-service-worker",
            14_001,
            Some("SID-service-worker"),
            None,
        );
        target.register_runtime_remote_object_ids_for_session(
            "SID-service-worker",
            ["service-worker-object".to_owned()],
        );
    }
    ctx.conn
        .set_service_worker_pause_on_start_owner(Some("SID-service-worker"), true);

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-service-worker",
            "sessionId": "SID-service-worker"
        }
    }))
    .await;

    ctx.expect_result(14, json!({}), None);
    let failed_await = ctx.take_one();
    assert_eq!(failed_await["id"], json!(14_001));
    assert_eq!(failed_await["sessionId"], json!("SID-service-worker"));
    assert_eq!(failed_await["error"]["message"], json!("Target detached"));
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-service-worker",
            "sessionId": "SID-service-worker",
        })),
    );

    assert_eq!(ctx.conn.session_route(Some("SID-service-worker")), None);
    assert!(!ctx.conn.has_pending_inspector_awaits());
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                &["service-worker-object".to_owned()],
            )
            .is_ok(),
        "service worker remote-object ownership must be retired with its session"
    );
    assert!(!ctx.conn.service_worker_pause_on_start_for_devtools());
    let target = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .service_worker_target("TID-service-worker")
        .expect("service worker target remains live after detach");
    assert!(!target.has_session());
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target_invalid_session_errors() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000C");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-1");

    ctx.process_async(json!({
        "id": 15,
        "method": "Target.detachFromTarget",
        "params": {
            "sessionId": "SID-2"
        }
    }))
    .await;

    ctx.expect_error(15, -31998, "InvalidSessionId");
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_session_id(), Some("SID-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target_aborts_paused_request_stage_navigation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>detach-target</body></html>",
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
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.install_browser_context_fixture_for_test(bc);
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "SID-1",
        moli_page_types::DevToolsSessionKey::Primary,
    );

    ctx.process_async(json!({
        "id": 40,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(40, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 41,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let network_id = paused["params"]["networkId"].clone();

    ctx.process_async(json!({
        "id": 42,
        "method": "Target.detachFromTarget",
        "params": { "targetId": "TID-000000000A", "sessionId": "SID-1" }
    }))
    .await;
    ctx.expect_result(42, json!({}), None);

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Target detached");

    let error = ctx.take_one();
    assert_eq!(error["id"], 41);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Target detached");

    let detached = ctx.take_one();
    assert_eq!(detached["method"], "Target.detachedFromTarget");
    assert_eq!(detached["params"]["targetId"], "TID-000000000A");
    assert_eq!(detached["params"]["sessionId"], "SID-1");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert!(
        !bc.active_page_target()
            .fetch_owner
            .has_pending_fetch_state_for_test()
    );
    assert!(!bc.active_page_target().fetch_owner.is_enabled());
    assert!(
        !bc.active_page_target()
            .runtime_slot
            .primary_network_events_enabled()
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_false_detaches_existing_target() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000D");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    bc.active_page_target_mut()
        .mutate_devtools_network_session_state(
            &moli_page_types::DevToolsSessionKey::Primary,
            |network| {
                network.network_enabled = true;
                network.cache_disabled = true;
                network.bypass_service_worker = true;
                network.extra_headers = vec![("X-Test".into(), "1".into())];
            },
        );
    bc.active_page_target_mut().css_enabled = true;
    bc.active_page_target_mut().fetch_owner.configure(
        Some("SID-1".to_owned()),
        true,
        vec![crate::conn::FetchInterceptionPattern {
            url_pattern: "*".into(),
            resource_type_filter: None,
            request_stage: crate::conn::FetchRequestStage::Request,
        }],
    );
    ctx.conn.commit_declared_session_fixtures_for_test();
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );
    ctx.conn
        .mark_session_auto_attached_for_test("SID-1".to_owned(), None);

    ctx.process_async(json!({
        "id": 16,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    let detached_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Target.detachedFromTarget"))
        .expect("detachedFromTarget event");
    let response_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(16))
        .expect("setAutoAttach response");
    assert!(
        detached_index < response_index,
        "Chromium completes setAutoAttach only after synchronous detach events: {:?}",
        ctx.sent
    );
    ctx.expect_result(16, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000D",
            "sessionId": "SID-1",
        })),
    );
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert_eq!(bc.active_target_id(), Some("TID-000000000D"));
    assert!(
        !bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_lifecycle_events
    );
    assert!(
        !bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled
    );
    assert!(
        !bc.active_page_target().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .inspector_enabled
    );
    assert!(
        !bc.active_page_target()
            .runtime_slot
            .primary_network_events_enabled()
    );
    assert!(!bc.active_page_target().effective_policy().cache_disabled());
    assert!(
        !bc.active_page_target()
            .effective_policy()
            .bypass_service_worker()
    );
    assert!(!bc.active_page_target().css_enabled);
    assert!(!bc.active_page_target().fetch_owner.is_enabled());
    assert!(!bc.active_page_target().fetch_owner.handle_auth_requests());
    assert!(
        bc.active_page_target()
            .fetch_owner
            .config_snapshot()
            .patterns()
            .is_empty()
    );
    assert!(
        bc.active_page_target()
            .effective_policy()
            .extra_headers()
            .is_empty()
    );
    assert!(!ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_false_detaches_existing_shared_worker_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(10),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        Some("SID-shared-worker"),
    );
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );
    ctx.conn
        .mark_session_auto_attached_for_test("SID-shared-worker".to_owned(), None);

    ctx.process_async(json!({
        "id": 16,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(16, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-shared-worker",
            "sessionId": "SID-shared-worker",
        })),
    );
    assert_eq!(ctx.conn.session_route(Some("SID-shared-worker")), None);
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        bc.shared_worker_target("TID-shared-worker")
            .and_then(|target| target.session_id()),
        None
    );
    assert!(!ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_false_cleans_shared_worker_runtime_state_before_detached_event() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(12),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        Some("SID-shared-worker"),
    );
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );
    ctx.conn
        .mark_session_auto_attached_for_test("SID-shared-worker".to_owned(), None);
    {
        let target = ctx
            .conn
            .shared_worker_target_for_session_mut(Some("SID-shared-worker"))
            .expect("shared worker target should be attached");
        target.register_pending_inspector_await(
            "SID-shared-worker",
            991,
            Some("SID-shared-worker"),
            Some("shared-cleanup-group"),
        );
        target.register_runtime_remote_object_ids_for_session(
            "SID-shared-worker",
            ["shared-ungrouped-object".to_owned()],
        );
        target.register_runtime_remote_object_ids_with_group(
            "SID-shared-worker",
            ["shared-grouped-object".to_owned()],
            "shared-cleanup-group",
        );
    }

    ctx.process_async(json!({
        "id": 17,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(17, json!({}), None);
    let failed_await = ctx.take_one();
    assert_eq!(failed_await["id"], json!(991));
    assert_eq!(failed_await["sessionId"], json!("SID-shared-worker"));
    assert_eq!(failed_await["error"]["message"], json!("Target detached"));
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-shared-worker",
            "sessionId": "SID-shared-worker",
        })),
    );

    assert_eq!(ctx.conn.session_route(Some("SID-shared-worker")), None);
    assert!(!ctx.conn.has_pending_inspector_awaits());
    assert!(
        ctx.conn
            .validate_runtime_remote_object_ids_for_session_owner(
                None,
                &[
                    "shared-ungrouped-object".to_owned(),
                    "shared-grouped-object".to_owned(),
                ],
            )
            .is_ok(),
        "auto-attach reset should clear shared worker target-local remote object ownership"
    );
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let target = bc
        .shared_worker_target("TID-shared-worker")
        .expect("shared worker target remains live after auto-attach reset");
    assert_eq!(target.session_id(), None);
    assert!(!ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_false_detaches_only_matching_browser_service_worker_owner() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_service_worker_target(
        &mut ctx,
        77,
        "TID-service-worker",
        "https://example.test/service-worker.js",
        "https://example.test/",
        None,
    );

    ctx.process_async(json!({
        "id": 18,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    ctx.expect_result(18, json!({}), None);
    let root_attached = ctx.take_one();
    assert_eq!(root_attached["method"], "Target.attachedToTarget");
    assert_eq!(
        root_attached["params"]["targetInfo"]["targetId"],
        "TID-service-worker"
    );
    let root_session_id = root_attached["params"]["sessionId"]
        .as_str()
        .expect("root auto-attached session")
        .to_owned();

    ctx.process_async(json!({
        "id": 19,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let browser_attached = ctx.take_first_matching("browser attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
    });
    let owner_session_id = browser_attached["params"]["sessionId"]
        .as_str()
        .expect("browser owner session")
        .to_owned();
    ctx.expect_result(19, json!({ "sessionId": owner_session_id.as_str() }), None);

    ctx.process_async(json!({
        "id": 20,
        "sessionId": owner_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    ctx.expect_result(20, json!({}), Some(&owner_session_id));
    let owner_auto_attached = ctx
        .take_first_matching("owner service worker attachedToTarget", |message| {
            message["method"] == json!("Target.attachedToTarget")
        });
    assert_eq!(
        owner_auto_attached["params"]["targetInfo"]["targetId"],
        "TID-service-worker"
    );
    let owner_auto_session_id = owner_auto_attached["params"]["sessionId"]
        .as_str()
        .expect("owner auto-attached session")
        .to_owned();
    assert_ne!(owner_auto_session_id, root_session_id);
    assert_ne!(owner_auto_session_id, owner_session_id);
    let owner_auto_attach_changed = ctx.take_first_matching(
        "root service worker targetInfoChanged after owner auto attach",
        |message| {
            message.get("sessionId").is_none()
                && message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!("TID-service-worker")
        },
    );
    assert_eq!(
        owner_auto_attach_changed["params"]["targetInfo"]["attached"],
        json!(true)
    );

    ctx.process_async(json!({
        "id": 21,
        "sessionId": owner_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    ctx.expect_result(21, json!({}), Some(&owner_session_id));
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-service-worker",
            "sessionId": owner_auto_session_id,
        })),
    );
    let root_changed = ctx.take_first_matching(
        "root service worker targetInfoChanged after owner detach",
        |message| {
            message.get("sessionId").is_none()
                && message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!("TID-service-worker")
        },
    );
    assert_eq!(
        root_changed["params"]["targetInfo"]["attached"],
        json!(true)
    );
    let owner_changed = ctx.take_first_matching(
        "owner service worker targetInfoChanged after owner detach",
        |message| {
            message["sessionId"] == json!(owner_session_id)
                && message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!("TID-service-worker")
        },
    );
    assert_eq!(
        owner_changed["params"]["targetInfo"]["attached"],
        json!(true)
    );
    assert!(ctx.sent.is_empty());
    assert!(
        ctx.conn.auto_attach_enabled(),
        "root autoAttach owner should remain active"
    );
    assert!(matches!(
        ctx.conn.session_route(Some(&root_session_id)),
        Some(CdpSessionRoute::ServiceWorkerTarget { .. })
    ));
    assert!(matches!(
        ctx.conn.session_route(Some(&owner_session_id)),
        Some(CdpSessionRoute::Browser)
    ));
    assert_eq!(ctx.conn.session_route(Some(&owner_auto_session_id)), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_attach_related_attaches_current_service_worker_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_service_worker_target(
        &mut ctx,
        77,
        "TID-service-worker",
        "https://example.test/service-worker.js",
        "https://example.test/",
        None,
    );

    ctx.process_async(json!({
        "id": 22,
        "method": "Target.autoAttachRelated",
        "params": {
            "targetId": "TID-service-worker",
            "waitForDebuggerOnStart": true
        }
    }))
    .await;

    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        "TID-service-worker"
    );
    assert_eq!(attached["params"]["targetInfo"]["type"], "service_worker");
    assert_eq!(attached["params"]["targetInfo"]["attached"], true);
    assert_eq!(attached["params"]["waitingForDebugger"], json!(false));
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("autoAttachRelated session id")
        .to_owned();
    assert_eq!(ctx.take_one(), json!({ "id": 22, "result": {} }));
    assert!(ctx.sent.is_empty());
    assert!(!ctx.conn.auto_attach_enabled());
    assert!(matches!(
        ctx.conn.session_route(Some(&session_id)),
        Some(CdpSessionRoute::ServiceWorkerTarget {
            target_id,
            ..
        }) if target_id == "TID-service-worker"
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_attach_related_filter_can_exclude_service_worker_targets() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_service_worker_target(
        &mut ctx,
        77,
        "TID-service-worker",
        "https://example.test/service-worker.js",
        "https://example.test/",
        None,
    );

    ctx.process_async(json!({
        "id": 23,
        "method": "Target.autoAttachRelated",
        "params": {
            "targetId": "TID-service-worker",
            "waitForDebuggerOnStart": false,
            "filter": [{ "type": "service_worker", "exclude": true }]
        }
    }))
    .await;

    ctx.expect_result(23, json!({}), None);
    assert!(ctx.sent.is_empty());
    let target = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.service_worker_target("TID-service-worker"))
        .expect("service worker target remains registered");
    assert!(!target.has_session());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_from_background_session_uses_direct_session_route() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-active");
    push_background_target(
        &mut ctx,
        "TID-background",
        "about:blank",
        Some("SID-background"),
    );

    ctx.process_async(json!({
        "id": 19,
        "method": "Target.setAutoAttach",
        "sessionId": "SID-background",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    ctx.expect_result(19, json!({}), Some("SID-background"));
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "Target.setAutoAttach must not be blocked by DirectSessionRouteRequired: {:?}",
        ctx.sent
    );
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_true_attaches_existing_unattached_target() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");

    ctx.process_async(json!({
        "id": 17,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.attachedToTarget");
    assert_eq!(event["params"]["targetInfo"]["targetId"], "TID-000000000E");
    assert_eq!(event["params"]["targetInfo"]["attached"], json!(true));
    assert_eq!(event["params"]["waitingForDebugger"], json!(false));
    assert_eq!(ctx.take_one(), json!({ "id": 17, "result": {} }));
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.has_active_session());
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_wait_for_debugger_does_not_mark_existing_page_as_waiting() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-page-waiting");

    ctx.process_async(json!({
        "id": 1700,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true
        }
    }))
    .await;

    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.attachedToTarget");
    assert_eq!(
        event["params"]["targetInfo"]["targetId"],
        "TID-page-waiting"
    );
    assert_eq!(event["params"]["waitingForDebugger"], json!(false));
    assert_eq!(ctx.take_one(), json!({ "id": 1700, "result": {} }));
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_true_attaches_existing_page_target_for_each_owner() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-page");

    ctx.process_async(json!({
        "id": 1710,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    let root_attached = ctx.take_one();
    assert_eq!(root_attached["method"], "Target.attachedToTarget");
    assert_eq!(
        root_attached["params"]["targetInfo"]["targetId"],
        "TID-page"
    );
    let root_session_id = root_attached["params"]["sessionId"]
        .as_str()
        .expect("root auto-attached page session")
        .to_owned();
    assert_eq!(ctx.take_one(), json!({ "id": 1710, "result": {} }));

    ctx.process_async(json!({
        "id": 1711,
        "method": "Target.attachToBrowserTarget"
    }))
    .await;
    let browser_attached = ctx.take_first_matching("browser attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
    });
    assert_eq!(
        browser_attached["params"]["targetInfo"]["targetId"],
        "browser"
    );
    let browser_session_id = browser_attached["params"]["sessionId"]
        .as_str()
        .expect("browser target session")
        .to_owned();
    ctx.expect_result(
        1711,
        json!({ "sessionId": browser_session_id.as_str() }),
        None,
    );

    ctx.process_async(json!({
        "id": 1712,
        "sessionId": browser_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    let owner_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["sessionId"] == json!(browser_session_id)
                && message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!("TID-page")
        })
        .expect("owner attachedToTarget event");
    let owner_response_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(1712))
        .expect("owner setAutoAttach response");
    assert!(
        owner_attached_index < owner_response_index,
        "existing target attachment must complete before setAutoAttach responds: {:?}",
        ctx.sent
    );
    let owner_attached = ctx.take_first_matching("owner attachedToTarget", |message| {
        message["sessionId"] == json!(browser_session_id)
            && message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!("TID-page")
    });
    assert_eq!(owner_attached["method"], "Target.attachedToTarget");
    assert_eq!(
        owner_attached["params"]["targetInfo"]["targetId"],
        "TID-page"
    );
    let owner_page_session_id = owner_attached["params"]["sessionId"]
        .as_str()
        .expect("owner auto-attached page session")
        .to_owned();
    ctx.take_first_matching("owner targetInfoChanged", |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!("TID-page")
    });
    assert_eq!(
        take_response_by_id(&mut ctx, 1712),
        json!({ "id": 1712, "sessionId": browser_session_id, "result": {} })
    );
    assert_ne!(owner_page_session_id, root_session_id);
    assert_ne!(owner_page_session_id, browser_session_id);
    assert!(matches!(
        ctx.conn.session_route(Some(&owner_page_session_id)),
        Some(CdpSessionRoute::PageTarget {
            target_id,
            session_key: moli_page_types::DevToolsSessionKey::Attached(_),
            ..
        }) if target_id == "TID-page"
    ));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_session_id(),
        Some(root_session_id.as_str())
    );

    ctx.process_async(json!({
        "id": 1713,
        "sessionId": browser_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;

    ctx.expect_result(1713, json!({}), Some(&browser_session_id));
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-page",
            "sessionId": owner_page_session_id,
        })),
    );
    assert_eq!(ctx.conn.session_route(Some(&owner_page_session_id)), None);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_session_id(),
        Some(root_session_id.as_str())
    );
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_true_attaches_existing_unattached_shared_worker_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(11),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        None,
    );

    ctx.process_async(json!({
        "id": 17,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(17, json!({}), None);
    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.attachedToTarget");
    assert_eq!(
        event["params"]["targetInfo"]["targetId"],
        "TID-shared-worker"
    );
    assert_eq!(event["params"]["targetInfo"]["type"], "shared_worker");
    assert_eq!(event["params"]["targetInfo"]["attached"], json!(true));
    let session_id = event["params"]["sessionId"]
        .as_str()
        .expect("shared worker session id");
    assert!(matches!(
        ctx.conn.session_route(Some(session_id)),
        Some(CdpSessionRoute::SharedWorkerTarget { .. })
    ));
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn non_browser_auto_attach_owners_do_not_replay_existing_shared_worker_targets() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-page");
    let page_route = ctx
        .conn
        .prepare_auto_attached_page_session_binding("TID-page", "SID-page".to_owned())
        .expect("page session binding");
    ctx.conn
        .register_session_route_for_test("SID-page", page_route);
    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-page"))
        .expect("page owner runtime slot")
        .set_page_attachment_id_for_test(1);
    let owner_page = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-page"))
        .expect("page owner residence");
    push_dedicated_worker_target(&mut ctx, 2401, "TID-dedicated-worker", owner_page);
    let dedicated_worker_route = ctx
        .conn
        .prepare_auto_attached_dedicated_worker_session_binding(
            "TID-dedicated-worker",
            "SID-dedicated-worker".to_owned(),
        )
        .expect("dedicated worker session binding");
    ctx.conn
        .register_session_route_for_test("SID-dedicated-worker", dedicated_worker_route);
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(2402),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        Some("SID-shared-worker"),
    );

    for (id, owner_session_id) in [
        (2403, "SID-page"),
        (2404, "SID-dedicated-worker"),
        (2405, "SID-shared-worker"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Target.setAutoAttach",
            "sessionId": owner_session_id,
            "params": {
                "autoAttach": true,
                "waitForDebuggerOnStart": true,
                "flatten": true,
                "filter": [{"type": "shared_worker"}],
            }
        }))
        .await;

        ctx.expect_result(id, json!({}), Some(owner_session_id));
        assert!(
            ctx.sent.is_empty(),
            "{owner_session_id} must not receive a browser-level shared-worker target: {:?}",
            ctx.sent
        );
    }

    let shared_worker_sessions = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|context| context.shared_worker_target("TID-shared-worker"))
        .expect("shared-worker target should remain registered")
        .session_ids();
    assert_eq!(
        shared_worker_sessions,
        vec!["SID-shared-worker"],
        "a shared-worker TargetHandler must not auto-attach its own target"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_session_auto_attach_replays_an_existing_shared_worker_once() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_shared_worker_target(
        &mut ctx,
        SharedWorkerInstanceId::from_u64(2410),
        "TID-shared-worker",
        "https://example.test/shared-worker.js",
        "shared",
        None,
    );
    ctx.conn.register_browser_session("SID-browser".to_owned());

    for id in [2411, 2412] {
        ctx.process_async(json!({
            "id": id,
            "method": "Target.setAutoAttach",
            "sessionId": "SID-browser",
            "params": {
                "autoAttach": true,
                "waitForDebuggerOnStart": false,
                "flatten": true,
                "filter": [{"type": "shared_worker"}],
            }
        }))
        .await;

        if id == 2411 {
            let attached = ctx.take_one();
            assert_eq!(attached["method"], "Target.attachedToTarget");
            assert_eq!(attached["sessionId"], "SID-browser");
            assert_eq!(
                attached["params"]["targetInfo"]["targetId"],
                "TID-shared-worker"
            );
            assert_eq!(attached["params"]["targetInfo"]["type"], "shared_worker");
        }
        ctx.expect_result(id, json!({}), Some("SID-browser"));
        assert!(ctx.sent.is_empty());
    }

    assert_eq!(
        ctx.conn
            .attached_sessions_for_target("TID-shared-worker")
            .len(),
        1,
        "re-enabling one browser TargetHandler must not duplicate its attachment"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_existing_target_auto_attach_requires_its_owner_page_session() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-owner-page");
    ctx.process_async(json!({
        "id": 2470,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-owner-page", "flatten": true}
    }))
    .await;
    let owner_attached = ctx.take_one();
    let owner_session_id = owner_attached["params"]["sessionId"]
        .as_str()
        .expect("owner Page session")
        .to_owned();
    ctx.expect_result(2470, json!({"sessionId": owner_session_id}), None);
    let owner_page = ctx
        .conn
        .target_page_residence_identity_for_session(Some(&owner_session_id))
        .expect("owner Page residence");
    push_dedicated_worker_target(&mut ctx, 2471, "TID-dedicated-worker", owner_page);

    let worker_only_filter = json!([
        {"type": "worker", "exclude": false},
        {"exclude": true}
    ]);
    ctx.process_async(json!({
        "id": 2472,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": worker_only_filter
        }
    }))
    .await;
    ctx.expect_result(2472, json!({}), None);
    assert!(
        ctx.sent.is_empty(),
        "browser/root auto-attach must not attach a Page-owned dedicated worker: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 2473,
        "method": "Target.setAutoAttach",
        "sessionId": owner_session_id,
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [
                {"type": "worker", "exclude": false},
                {"exclude": true}
            ]
        }
    }))
    .await;
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    assert_eq!(attached["sessionId"], owner_session_id);
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        "TID-dedicated-worker"
    );
    assert_eq!(attached["params"]["targetInfo"]["type"], "worker");
    ctx.expect_result(2473, json!({}), Some(&owner_session_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_filter_excludes_existing_page_target() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-page");

    ctx.process_async(json!({
        "id": 1701,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "filter": [
                { "type": "page", "exclude": true }
            ]
        }
    }))
    .await;

    ctx.expect_result(1701, json!({}), None);
    assert!(ctx.sent.is_empty());
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_tab_auto_attach_reconciles_a_newly_matching_existing_page() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 1706,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 1706)["result"]["targetId"]
        .as_str()
        .expect("created Page target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1707,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id }
    }))
    .await;
    let tab_session_id = take_response_by_id(&mut ctx, 1707)["result"]["sessionId"]
        .as_str()
        .expect("attached Tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1708,
        "sessionId": tab_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{ "type": "page", "exclude": true }]
        }
    }))
    .await;
    ctx.expect_result(1708, json!({}), Some(&tab_session_id));
    assert!(ctx.sent.is_empty());

    ctx.process_async(json!({
        "id": 1709,
        "sessionId": tab_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{ "type": "page" }]
        }
    }))
    .await;

    let attached = ctx.take_one();
    assert_eq!(attached["method"], json!("Target.attachedToTarget"));
    assert_eq!(attached["sessionId"], json!(tab_session_id));
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        json!(page_target_id)
    );
    assert_eq!(attached["params"]["targetInfo"]["type"], json!("page"));
    let info_changed = ctx.take_first_matching("attached Page targetInfoChanged", |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
    });
    assert_eq!(
        info_changed["params"]["targetInfo"]["attached"],
        json!(true)
    );
    ctx.expect_result(1709, json!({}), Some(&tab_session_id));
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_filter_page_exclude_catchall_auto_attaches_existing_tab() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.process_async(json!({
        "id": 17010,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 17010)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 17011,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }
    }))
    .await;

    ctx.expect_result(17011, json!({}), None);
    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.attachedToTarget");
    assert_eq!(event["params"]["targetInfo"]["targetId"], tab_target_id);
    assert_eq!(event["params"]["targetInfo"]["type"], "tab");
    assert_eq!(event["params"]["targetInfo"]["attached"], json!(true));
    let session_id = event["params"]["sessionId"]
        .as_str()
        .expect("tab session id");
    assert!(matches!(
        ctx.conn.session_route(Some(session_id)),
        Some(CdpSessionRoute::TabTarget {
            tab_target_id: route_tab_target_id,
            ..
        }) if route_tab_target_id == tab_target_id
    ));
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_active_session(),
        "tab auto-attach must not directly attach the page"
    );
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_filter_allows_only_existing_service_worker_target() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-page");
    push_service_worker_target(
        &mut ctx,
        17,
        "TID-service-worker",
        "https://example.test/service-worker.js",
        "https://example.test/",
        None,
    );

    ctx.process_async(json!({
        "id": 1702,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "filter": [{ "type": "service_worker" }]
        }
    }))
    .await;

    ctx.expect_result(1702, json!({}), None);
    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.attachedToTarget");
    assert_eq!(
        event["params"]["targetInfo"]["targetId"],
        "TID-service-worker"
    );
    assert_eq!(event["params"]["targetInfo"]["type"], "service_worker");
    assert_eq!(event["params"]["targetInfo"]["attached"], json!(true));
    assert!(ctx.sent.is_empty());
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        !bc.has_active_session(),
        "page target should not attach when the filter only allows service workers"
    );
    let session_id = event["params"]["sessionId"]
        .as_str()
        .expect("service worker session id");
    assert!(matches!(
        ctx.conn.session_route(Some(session_id)),
        Some(CdpSessionRoute::ServiceWorkerTarget { .. })
    ));
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn tab_auto_attach_does_not_capture_an_existing_service_worker() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 17100,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    let page_target_id = take_response_by_id(&mut ctx, 17100)["result"]["targetId"]
        .as_str()
        .expect("created Page target id")
        .to_owned();
    let tab_target_id = tab_id_for_page(&ctx, &page_target_id);
    push_service_worker_target(
        &mut ctx,
        17101,
        "TID-tab-unrelated-service-worker",
        "https://example.test/service-worker.js",
        "https://example.test/",
        None,
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 17102,
        "method": "Target.attachToTarget",
        "params": { "targetId": tab_target_id }
    }))
    .await;
    let tab_session_id = take_response_by_id(&mut ctx, 17102)["result"]["sessionId"]
        .as_str()
        .expect("attached Tab session id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 17103,
        "sessionId": tab_session_id,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [{ "type": "service_worker" }]
        }
    }))
    .await;

    ctx.expect_result(17103, json!({}), Some(&tab_session_id));
    assert!(ctx.sent.is_empty());
    assert!(
        !ctx.conn
            .attached_sessions_for_target("TID-tab-unrelated-service-worker")
            .iter()
            .any(|session_id| {
                ctx.conn
                    .auto_attached_sessions_for_owner(Some(&tab_session_id))
                    .contains(session_id)
            })
    );
    assert_eq!(ctx.conn.service_worker_pause_on_start_owner_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_filter_excludes_existing_service_worker_target() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-9");
    push_service_worker_target(
        &mut ctx,
        18,
        "TID-service-worker",
        "https://example.test/service-worker.js",
        "https://example.test/",
        None,
    );

    ctx.process_async(json!({
        "id": 1703,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "filter": [
                { "type": "service_worker", "exclude": true },
                { "type": "page" }
            ]
        }
    }))
    .await;

    ctx.expect_result(1703, json!({}), None);
    assert!(ctx.sent.is_empty());
    let target = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.service_worker_target("TID-service-worker"))
        .expect("service worker target remains registered");
    assert!(!target.has_session());
    assert!(ctx.conn.auto_attach_enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_disable_rejects_non_empty_filter() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 1704,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false,
            "filter": [{}]
        }
    }))
    .await;

    ctx.expect_error(
        1704,
        -32602,
        "Target filter should be empty when disabling auto-attach",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_rejects_filter_allowing_tab_and_page() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 1705,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "filter": [
                { "type": "tab" },
                { "type": "page" }
            ]
        }
    }))
    .await;

    ctx.expect_error(
        1705,
        -32602,
        "Filter should not simultaneously allow \"tab\" and \"page\", page targets are attached via tab targets",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_true_does_not_reattach_existing_session() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000F");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-keep");

    ctx.process_async(json!({
        "id": 18,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(18, json!({}), None);
    assert!(ctx.sent.is_empty());
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_session_id(), Some("SID-keep"));
    assert!(ctx.conn.auto_attach_enabled());
}
