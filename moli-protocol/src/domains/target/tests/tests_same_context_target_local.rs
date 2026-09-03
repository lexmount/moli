use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_activates_auto_attached_background_session_into_page_runtime() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );

    ctx.process_async(json!({
        "id": 103,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": "BID-9",
            "url": "about:blank#second",
            "background": true
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
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();
    ctx.expect_result(103, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(104, json!({}), None);

    ctx.process_async(json!({
            "id": 105,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<title>auto-activated</title><div id='ok'>auto attached target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 105);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 106,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let evaluation = take_response_by_id(&mut ctx, 106);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("auto-activated"));
    assert_eq!(payload["text"], json!("auto attached target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_target_allows_second_target_in_same_browser_context_for_frame_session_fanout() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");

    ctx.process_async(json!({
        "id": 10,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let target_created = ctx.take_one();
    assert_eq!(target_created["method"], "Target.targetCreated");
    assert_eq!(
        target_created["params"]["targetInfo"]["browserContextId"],
        "BID-9"
    );
    let second_target_id = target_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    assert_ne!(second_target_id, "TID-000000000A");
    ctx.expect_result(10, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 11,
        "method": "Target.getTargets"
    }))
    .await;
    let targets = take_response_by_id(&mut ctx, 11)["result"]["targetInfos"]
        .as_array()
        .expect("target infos array")
        .clone();
    assert!(
        targets
            .iter()
            .any(|target| target["targetId"] == json!("TID-000000000A")),
        "first target should still be present"
    );
    assert!(
        targets
            .iter()
            .any(|target| target["targetId"] == json!(second_target_id)),
        "second target should also be present"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_reports_background_target_in_same_browser_context() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.process_async(json!({
        "id": 10,
        "method": "Target.createTarget",
        "params": {
            "browserContextId": "BID-9",
            "url": "about:blank#second",
            "background": true
        }
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 10);

    ctx.process_async(json!({
        "id": 11,
        "method": "Target.getTargetInfo",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(
        11,
        json!({
            "targetInfo": {
                "targetId": second_target_id,
                "type": "page",
                "title": "",
                "url": "about:blank#second",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-9",
            }
        }),
        None,
    );
    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.background_target_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_target_diagnostics_report_distinct_page_vm_document_isolates() {
    let mut ctx = TestContext::new();
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );
    let browser_context = ctx
        .conn
        .new_browser_context("BID-popup-diagnostics".to_owned());
    ctx.conn.insert_browser_context(browser_context);
    {
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be active");
        browser_context.set_active_target_id("TID-popup-opener");
        browser_context.attach_active_session("SID-popup-opener");
    }

    let opener_page = ctx
        .conn
        .load_page_via_runtime_async(
            "data:text/html,<!doctype html><title>opener</title><body>opener</body>",
        )
        .await
        .expect("opener page should load");
    let opener_url = opener_page.final_url().as_str().to_owned();
    {
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should remain active");
        browser_context.set_target_url(opener_url);
        browser_context.replace_loaded_page(Some(opener_page));
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 104_215,
        "method": "Runtime.evaluate",
        "sessionId": "SID-popup-opener",
        "params": {
            "expression": "window.open('about:blank#diagnostics-popup', '_blank') !== null",
            "returnByValue": true
        }
    }))
    .await;
    let open_response = take_response_by_id(&mut ctx, 104_215);
    assert_eq!(
        open_response["result"]["result"]["value"],
        json!(true),
        "window.open should create a popup target: {open_response:?}"
    );
    let target_created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "popup targetCreated event should be emitted: {:?}",
                ctx.sent
            )
        });
    assert_eq!(
        target_created["params"]["targetInfo"]["openerId"],
        json!("TID-popup-opener")
    );
    let popup_target_id = target_created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id should be present")
        .to_owned();
    let attached = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(popup_target_id)
        })
        .cloned()
        .unwrap_or_else(|| panic!("popup target should be auto-attached: {:?}", ctx.sent));
    let popup_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup session id should be present")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 104_216,
        "method": "Page.navigate",
        "sessionId": popup_session_id,
        "params": {
            "url": "data:text/html,<!doctype html><title>popup</title><body>popup</body>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 104_216);
    assert_eq!(navigation["result"]["frameId"], json!(popup_target_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 104_217,
        "method": "HeapProfiler.moliDiagnostics"
    }))
    .await;
    let diagnostics = take_response_by_id(&mut ctx, 104_217);
    let isolate_scope = &diagnostics["result"]["isolateScope"];
    assert_eq!(isolate_scope["loadedDocumentPageCount"], json!(2));
    assert_eq!(
        isolate_scope["loadedDocumentRendererOwnerCount"],
        json!(2),
        "opener and loaded popup target must remain independently schedulable: {diagnostics:?}"
    );
    assert_eq!(
        isolate_scope["estimatedDocumentIsolateCount"],
        json!(2),
        "loaded popup PageVM must own a distinct document isolate: {diagnostics:?}"
    );
    assert_eq!(
        isolate_scope["estimatedLiveV8IsolateCount"],
        json!(2),
        "opener plus popup without workers should report two live page document isolates: {diagnostics:?}"
    );
    assert_eq!(
        isolate_scope["documentContextCount"],
        json!(2),
        "diagnostics should snapshot both opener and popup document contexts: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics["result"]["activeBrowserContext"]["backgroundLoadedPageCount"],
        json!(1),
        "the popup should remain a loaded background target"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_keeps_background_target_background_when_active_target_has_no_loaded_page()
{
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 10,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 10);

    ctx.process_async(json!({
        "id": 11,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let session_id = take_response_by_id(&mut ctx, 11)["result"]["sessionId"]
        .as_str()
        .expect("background target session id")
        .to_owned();
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": second_target_id,
                "type": "page",
                "title": "",
                "url": "about:blank#second",
                "attached": true,
                "canAccessOpener": false,
                "browserContextId": "BID-9",
            }
        })),
    );

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
    assert_eq!(
        bc.background_target(&second_target_id)
            .map(|target| target.target_url()),
        Some("about:blank#second")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn background_target_session_can_navigate_enable_and_evaluate_after_attach_without_activation()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 1010,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 1010);

    ctx.process_async(json!({
        "id": 1011,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let session_id = take_response_by_id(&mut ctx, 1011)["result"]["sessionId"]
        .as_str()
        .expect("background target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.active_target_id()),
        Some("TID-000000000A"),
        "attachToTarget must not activate the background target"
    );

    ctx.process_async(json!({
            "id": 1012,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
            "url": "data:text/html,<title>attached</title><div id='ok'>attached target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 1012);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.active_target_id()),
        Some("TID-000000000A"),
        "session-scoped navigation must keep the attached target background"
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1013,
        "method": "Runtime.enable",
        "sessionId": session_id,
    }))
    .await;
    take_response_by_id(&mut ctx, 1013);
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.active_target_id()),
        Some("TID-000000000A"),
        "session-scoped Runtime.enable must keep the attached target background"
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1014,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent, href: location.href })"
            }
        }))
        .await;
    let evaluation = take_response_by_id(&mut ctx, 1014);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("attached"));
    assert_eq!(payload["text"], json!("attached target"));
    assert!(
        payload["href"]
            .as_str()
            .expect("href should be a string")
            .contains("data:text/html"),
        "unexpected attached target href payload: {payload:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.active_target_id()),
        Some("TID-000000000A"),
        "session-scoped runtime evaluation must keep the attached target background"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_removes_background_target_without_disturbing_active_target() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.process_async(json!({
        "id": 10,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 10);

    ctx.process_async(json!({
        "id": 11,
        "method": "Target.closeTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(11, json!({ "success": true }), None);

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert!(bc.has_no_background_targets());
}

#[tokio::test(flavor = "multi_thread")]
async fn close_active_target_activates_background_target_to_active_slot() {
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
    ctx.process_async(json!({
        "id": 10,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 10);

    ctx.process_async(json!({
        "id": 11,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let background_session_id = take_response_by_id(&mut ctx, 11)["result"]["sessionId"]
        .as_str()
        .expect("background target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 12,
        "method": "Target.closeTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(12, json!({ "success": true }), None);

    let detached = ctx.take_first_matching("closed target detachedFromTarget", |message| {
        message["method"] == json!("Target.detachedFromTarget")
    });
    assert_eq!(detached["params"]["targetId"], json!(second_target_id));
    assert_eq!(
        detached["params"]["sessionId"],
        json!(background_session_id)
    );

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert!(bc.has_no_background_targets());
}

#[tokio::test(flavor = "multi_thread")]
async fn close_active_target_exposes_activated_target_screencast() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(&mut ctx, "BID-9", "TID-active", "<title>active</title>").await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 20,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#background"}
    }))
    .await;
    let background_target_id = take_created_target_id(&mut ctx, 20);

    ctx.process_async(json!({
        "id": 21,
        "method": "Target.attachToTarget",
        "params": {"targetId": background_target_id}
    }))
    .await;
    let background_session_id = take_response_by_id(&mut ctx, 21)["result"]["sessionId"]
        .as_str()
        .expect("background target session")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.startScreencast",
        "sessionId": background_session_id,
        "params": {}
    }))
    .await;
    let hidden = ctx.take_first_matching("background screencast starts hidden", |message| {
        message["method"] == json!("Page.screencastVisibilityChanged")
    });
    assert_eq!(hidden["sessionId"], json!(background_session_id));
    assert_eq!(hidden["params"]["visible"], json!(false));
    ctx.expect_result(22, json!({}), Some(&background_session_id));

    ctx.process_async(json!({
        "id": 23,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-active"}
    }))
    .await;
    ctx.expect_result(23, json!({ "success": true }), None);
    let visible = ctx.take_first_matching("activated screencast becomes visible", |message| {
        message["method"] == json!("Page.screencastVisibilityChanged")
            && message["sessionId"] == json!(background_session_id)
    });
    assert_eq!(visible["params"]["visible"], json!(true));

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some(background_target_id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_foreground_target_hides_deactivated_target_screencast() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(&mut ctx, "BID-9", "TID-active", "<title>active</title>").await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 24,
        "sessionId": "SID-active",
        "method": "Page.startScreencast",
        "params": {}
    }))
    .await;
    let initially_visible = ctx
        .take_first_matching("active screencast starts visible", |message| {
            message["method"] == json!("Page.screencastVisibilityChanged")
        });
    assert_eq!(initially_visible["params"]["visible"], json!(true));
    ctx.expect_result(24, json!({}), Some("SID-active"));

    ctx.process_async(json!({
        "id": 25,
        "method": "Target.createTarget",
        "params": { "browserContextId": "BID-9", "url": "about:blank#foreground" }
    }))
    .await;
    let hidden = ctx.take_first_matching("deactivated screencast becomes hidden", |message| {
        message["method"] == json!("Page.screencastVisibilityChanged")
            && message["sessionId"] == json!("SID-active")
    });
    assert_eq!(hidden["params"]["visible"], json!(false));
    let created_target_id = take_response_by_id(&mut ctx, 25)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some(created_target_id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn close_active_target_activated_background_session_can_navigate_and_evaluate() {
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
    ctx.process_async(json!({
        "id": 12,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 12);

    ctx.process_async(json!({
        "id": 13,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let activated_session_id = take_response_by_id(&mut ctx, 13)["result"]["sessionId"]
        .as_str()
        .expect("background target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 14,
        "method": "Target.closeTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(14, json!({ "success": true }), None);
    expect_inspector_detached(&mut ctx, &activated_session_id);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": second_target_id,
            "sessionId": activated_session_id,
        })),
    );
    ctx.expect_event(
        "Target.targetDestroyed",
        Some(&json!({
            "targetId": second_target_id,
        })),
    );

    ctx.process_async(json!({
        "id": 15,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>activated</title><div id='ok'>activated target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 15);
    assert_eq!(navigation["result"]["frameId"], json!("TID-000000000A"));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 16,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let evaluation = take_response_by_id(&mut ctx, 16);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("activated"));
    assert_eq!(payload["text"], json!("activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn close_active_target_restores_background_loaded_page_runtime_without_renavigation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 120,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 120);

    ctx.process_async(json!({
        "id": 121,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 121)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 122,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(122, json!({}), None);

    ctx.process_async(json!({
        "id": 123,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 123);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 124,
        "method": "Target.closeTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(124, json!({ "success": true }), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": second_target_id,
            "sessionId": second_session_id,
        })),
    );

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));

    ctx.process_async(json!({
            "id": 125,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let evaluation = take_response_by_id(&mut ctx, 125);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("first"));
    assert_eq!(payload["text"], json!("first target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn close_active_target_activates_auto_attached_background_session_into_page_runtime() {
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
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "SID-active",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );

    ctx.process_async(json!({
        "id": 17,
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
    let activated_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background target session id")
        .to_owned();
    ctx.expect_result(17, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 18,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    ctx.expect_result(18, json!({ "success": true }), None);
    expect_inspector_detached(&mut ctx, "SID-active");
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000A",
            "sessionId": "SID-active",
        })),
    );

    ctx.process_async(json!({
            "id": 19,
            "method": "Page.navigate",
            "sessionId": activated_session_id,
            "params": {
                "url": "data:text/html,<title>autoattach-activated</title><div id='ok'>auto attached activated target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 19);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 20,
            "method": "Runtime.evaluate",
            "sessionId": activated_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let evaluation = take_response_by_id(&mut ctx, 20);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("autoattach-activated"));
    assert_eq!(payload["text"], json!("auto attached activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn close_targets_chain_activates_multiple_auto_attached_background_sessions_into_runtime() {
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
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "SID-active",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );

    ctx.process_async(json!({
        "id": 1801,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created_second = ctx.take_one();
    assert_eq!(created_second["method"], "Target.targetCreated");
    let second_target_id = created_second["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached_second = ctx.take_one();
    assert_eq!(attached_second["method"], "Target.attachedToTarget");
    let second_session_id = attached_second["params"]["sessionId"]
        .as_str()
        .expect("second background target session id")
        .to_owned();
    ctx.expect_result(1801, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1802,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#third"}
    }))
    .await;
    let created_third = ctx.take_first_matching("third targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
    });
    let third_target_id = created_third["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("third target id")
        .to_owned();
    let attached_third = ctx.take_first_matching("third attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
    });
    let third_session_id = attached_third["params"]["sessionId"]
        .as_str()
        .expect("third background target session id")
        .to_owned();
    ctx.expect_result(1802, json!({ "targetId": third_target_id }), None);

    ctx.process_async(json!({
        "id": 1803,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    ctx.expect_result(1803, json!({ "success": true }), None);
    expect_inspector_detached(&mut ctx, "SID-active");
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000A",
            "sessionId": "SID-active",
        })),
    );

    ctx.process_async(json!({
            "id": 1804,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>second-activated</title><div id='ok'>second activated target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1804);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1805,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let second_evaluation = take_response_by_id(&mut ctx, 1805);
    let second_payload = second_evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let second_payload: serde_json::Value =
        serde_json::from_str(second_payload).expect("evaluation payload should be valid json");
    assert_eq!(second_payload["title"], json!("second-activated"));
    assert_eq!(second_payload["text"], json!("second activated target"));

    ctx.process_async(json!({
        "id": 1806,
        "method": "Target.closeTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(1806, json!({ "success": true }), None);
    expect_inspector_detached(&mut ctx, &second_session_id);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": second_target_id,
            "sessionId": second_session_id,
        })),
    );
    ctx.expect_event(
        "Target.targetDestroyed",
        Some(&json!({
            "targetId": second_target_id,
        })),
    );

    ctx.process_async(json!({
            "id": 1807,
            "method": "Page.navigate",
            "sessionId": third_session_id,
            "params": {
                "url": "data:text/html,<title>third-activated</title><div id='ok'>third activated target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let third_navigation = take_response_by_id(&mut ctx, 1807);
    assert_eq!(
        third_navigation["result"]["frameId"],
        json!(third_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1808,
            "method": "Runtime.evaluate",
            "sessionId": third_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let third_evaluation = take_response_by_id(&mut ctx, 1808);
    let third_payload = third_evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let third_payload: serde_json::Value =
        serde_json::from_str(third_payload).expect("evaluation payload should be valid json");
    assert_eq!(third_payload["title"], json!("third-activated"));
    assert_eq!(third_payload["text"], json!("third activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_chain_restores_multiple_auto_attached_loaded_page_runtimes_without_renavigation()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "SID-active",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    ctx.conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );

    ctx.process_async(json!({
        "id": 1822,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let created_second = ctx.take_one();
    assert_eq!(created_second["method"], "Target.targetCreated");
    let second_target_id = created_second["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached_second = ctx.take_one();
    assert_eq!(attached_second["method"], "Target.attachedToTarget");
    let second_session_id = attached_second["params"]["sessionId"]
        .as_str()
        .expect("second background target session id")
        .to_owned();
    ctx.expect_result(1822, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1823,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(1823, json!({}), None);

    ctx.process_async(json!({
            "id": 1824,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>second-activated</title><div id='ok'>second activated target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let activated_second_navigation = take_response_by_id(&mut ctx, 1824);
    assert_eq!(
        activated_second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1825,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#third"}
    }))
    .await;
    let created_third = ctx.take_one();
    assert_eq!(created_third["method"], "Target.targetCreated");
    let third_target_id = created_third["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("third target id")
        .to_owned();
    let attached_third = ctx.take_one();
    assert_eq!(attached_third["method"], "Target.attachedToTarget");
    let third_session_id = attached_third["params"]["sessionId"]
        .as_str()
        .expect("third background target session id")
        .to_owned();
    ctx.expect_result(1825, json!({ "targetId": third_target_id }), None);

    ctx.process_async(json!({
        "id": 1826,
        "method": "Target.activateTarget",
        "params": {"targetId": third_target_id}
    }))
    .await;
    ctx.expect_result(1826, json!({}), None);

    ctx.process_async(json!({
            "id": 1827,
            "method": "Page.navigate",
            "sessionId": third_session_id,
            "params": {
                "url": "data:text/html,<title>third-activated</title><div id='ok'>third activated target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let activated_third_navigation = take_response_by_id(&mut ctx, 1827);
    assert_eq!(
        activated_third_navigation["result"]["frameId"],
        json!(third_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1828,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    ctx.expect_result(1828, json!({}), None);

    ctx.process_async(json!({
            "id": 1829,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let first_eval = take_response_by_id(&mut ctx, 1829);
    let first_payload = first_eval["result"]["result"]["value"]
        .as_str()
        .expect("first evaluation payload should be a string");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first evaluation payload should be valid json");
    assert_eq!(first_payload["title"], json!("first"));
    assert_eq!(first_payload["text"], json!("first target"));

    ctx.process_async(json!({
        "id": 1830,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(1830, json!({}), None);

    ctx.process_async(json!({
            "id": 1831,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let second_eval = take_response_by_id(&mut ctx, 1831);
    let second_payload = second_eval["result"]["result"]["value"]
        .as_str()
        .expect("second evaluation payload should be a string");
    let second_payload: serde_json::Value = serde_json::from_str(second_payload)
        .expect("second evaluation payload should be valid json");
    assert_eq!(second_payload["title"], json!("second-activated"));
    assert_eq!(second_payload["text"], json!("second activated target"));

    ctx.process_async(json!({
        "id": 1832,
        "method": "Target.activateTarget",
        "params": {"targetId": third_target_id}
    }))
    .await;
    ctx.expect_result(1832, json!({}), None);

    ctx.process_async(json!({
            "id": 1833,
            "method": "Runtime.evaluate",
            "sessionId": third_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let third_eval_again = take_response_by_id(&mut ctx, 1833);
    let third_payload_again = third_eval_again["result"]["result"]["value"]
        .as_str()
        .expect("third evaluation payload should be a string");
    let third_payload_again: serde_json::Value = serde_json::from_str(third_payload_again)
        .expect("third evaluation payload should be valid json");
    assert_eq!(third_payload_again["title"], json!("third-activated"));
    assert_eq!(third_payload_again["text"], json!("third activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_chain_restores_multiple_set_auto_attach_background_loaded_page_runtimes_without_renavigation()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "SID-active",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    {
        let bc = ctx.conn.browser_context.as_mut().unwrap();
        bc.insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#second".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
        bc.insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-0000000010".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#third".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
    }

    ctx.process_async(json!({
        "id": 1835,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(1835, json!({}), None);
    let events = ctx.take_all();
    let second_session_id = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-000000000F")
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second target session id")
        .to_owned();
    let third_session_id = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-0000000010")
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("third target session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 1836,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000F"}
    }))
    .await;
    ctx.expect_result(1836, json!({}), None);

    ctx.process_async(json!({
            "id": 1837,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>second-sweep</title><div id='ok'>second sweep target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1837);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!("TID-000000000F")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1838,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-0000000010"}
    }))
    .await;
    ctx.expect_result(1838, json!({}), None);

    ctx.process_async(json!({
        "id": 1839,
        "method": "Page.navigate",
        "sessionId": third_session_id,
        "params": {
            "url": "data:text/html,<title>third-sweep</title><div id='ok'>third sweep target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let third_navigation = take_response_by_id(&mut ctx, 1839);
    assert_eq!(
        third_navigation["result"]["frameId"],
        json!("TID-0000000010")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1840,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    ctx.expect_result(1840, json!({}), None);

    ctx.process_async(json!({
            "id": 1841,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let first_eval = take_response_by_id(&mut ctx, 1841);
    let first_payload = first_eval["result"]["result"]["value"]
        .as_str()
        .expect("first evaluation payload should be a string");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first evaluation payload should be valid json");
    assert_eq!(first_payload["title"], json!("first"));
    assert_eq!(first_payload["text"], json!("first target"));

    ctx.process_async(json!({
        "id": 1842,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000F"}
    }))
    .await;
    ctx.expect_result(1842, json!({}), None);

    ctx.process_async(json!({
            "id": 1843,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let second_eval = take_response_by_id(&mut ctx, 1843);
    let second_payload = second_eval["result"]["result"]["value"]
        .as_str()
        .expect("second evaluation payload should be a string");
    let second_payload: serde_json::Value = serde_json::from_str(second_payload)
        .expect("second evaluation payload should be valid json");
    assert_eq!(second_payload["title"], json!("second-sweep"));
    assert_eq!(second_payload["text"], json!("second sweep target"));

    ctx.process_async(json!({
        "id": 1844,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-0000000010"}
    }))
    .await;
    ctx.expect_result(1844, json!({}), None);

    ctx.process_async(json!({
            "id": 1845,
            "method": "Runtime.evaluate",
            "sessionId": third_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let third_eval = take_response_by_id(&mut ctx, 1845);
    let third_payload = third_eval["result"]["result"]["value"]
        .as_str()
        .expect("third evaluation payload should be a string");
    let third_payload: serde_json::Value =
        serde_json::from_str(third_payload).expect("third evaluation payload should be valid json");
    assert_eq!(third_payload["title"], json!("third-sweep"));
    assert_eq!(third_payload["text"], json!("third sweep target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn close_target_restores_loaded_runtime_to_previous_set_auto_attach_target_without_renavigation()
 {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>first</title><div id='ok'>first target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "SID-active",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    {
        let bc = ctx.conn.browser_context.as_mut().unwrap();
        bc.insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#second".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
        bc.insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-0000000010".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#third".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
    }

    ctx.process_async(json!({
        "id": 1846,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;
    ctx.expect_result(1846, json!({}), None);
    let events = ctx.take_all();
    let second_session_id = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-000000000F")
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("second target session id")
        .to_owned();
    let third_session_id = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-0000000010")
        })
        .and_then(|event| event["params"]["sessionId"].as_str())
        .expect("third target session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 1847,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000F"}
    }))
    .await;
    ctx.expect_result(1847, json!({}), None);

    ctx.process_async(json!({
            "id": 1848,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>second-close</title><div id='ok'>second close target</div>"
            }
        }))
        .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1848);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!("TID-000000000F")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1849,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-0000000010"}
    }))
    .await;
    ctx.expect_result(1849, json!({}), None);

    ctx.process_async(json!({
        "id": 1850,
        "method": "Page.navigate",
        "sessionId": third_session_id,
        "params": {
            "url": "data:text/html,<title>third-close</title><div id='ok'>third close target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let third_navigation = take_response_by_id(&mut ctx, 1850);
    assert_eq!(
        third_navigation["result"]["frameId"],
        json!("TID-0000000010")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1851,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-0000000010"}
    }))
    .await;
    ctx.expect_result(1851, json!({ "success": true }), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-0000000010",
            "sessionId": third_session_id,
        })),
    );

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000F"));
    assert_eq!(bc.active_session_id(), Some(second_session_id.as_str()));

    ctx.process_async(json!({
            "id": 1852,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let second_eval = take_response_by_id(&mut ctx, 1852);
    let second_payload = second_eval["result"]["result"]["value"]
        .as_str()
        .expect("second evaluation payload should be a string");
    let second_payload: serde_json::Value = serde_json::from_str(second_payload)
        .expect("second evaluation payload should be valid json");
    assert_eq!(second_payload["title"], json!("second-close"));
    assert_eq!(second_payload["text"], json!("second close target"));

    ctx.process_async(json!({
        "id": 1853,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    ctx.expect_result(1853, json!({}), None);

    ctx.process_async(json!({
            "id": 1854,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        }))
        .await;
    let first_eval = take_response_by_id(&mut ctx, 1854);
    let first_payload = first_eval["result"]["result"]["value"]
        .as_str()
        .expect("first evaluation payload should be a string");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first evaluation payload should be valid json");
    assert_eq!(first_payload["title"], json!("first"));
    assert_eq!(first_payload["text"], json!("first target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target_clears_background_target_session() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "<title>loaded</title><div id='ok'>loaded target</div>",
    )
    .await;
    ctx.process_async(json!({
        "id": 10,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 10);

    ctx.process_async(json!({
        "id": 11,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let session_id = take_response_by_id(&mut ctx, 11)["result"]["sessionId"]
        .as_str()
        .expect("background target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 12,
        "method": "Target.detachFromTarget",
        "params": {"targetId": second_target_id, "sessionId": session_id}
    }))
    .await;
    ctx.expect_result(12, json!({}), None);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": second_target_id,
            "sessionId": session_id,
        })),
    );

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert!(
        bc.background_target(&second_target_id)
            .is_some_and(|target| target.session_id().is_none()),
        "detaching a background target session should clear the background session binding without activating it",
    );
    assert!(
        bc.background_target(&second_target_id).is_some(),
        "detaching the session should keep the background target itself addressable"
    );
    assert!(
        bc.loaded_page().is_some(),
        "detaching a background target session should keep the active loaded page active"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_command_on_background_target_session_routes_without_activating_loaded_active_target()
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
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            Some("SID-bg".into()),
            crate::conn::TargetIdentityState::new(
                "about:blank#second".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000A",
        "SID-active",
        moli_page_types::DevToolsSessionKey::Primary,
    );
    register_page_session_route(
        &mut ctx,
        "BID-9",
        "TID-000000000F",
        "SID-bg",
        moli_page_types::DevToolsSessionKey::Primary,
    );

    ctx.process_async(json!({
        "id": 12,
        "method": "Page.navigate",
        "sessionId": "SID-bg",
        "params": {
            "url": "data:text/html,<title>same-context-routed</title><div id='ok'>same-context routed target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 12);
    assert_eq!(navigation["result"]["frameId"], json!("TID-000000000F"));
    ctx.take_all();

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert_eq!(
        bc.background_target("TID-000000000F")
            .and_then(|target| target.session_id()),
        Some("SID-bg")
    );
    assert!(
        bc.background_target("TID-000000000F")
            .is_some_and(|target| target.has_loaded_page()),
        "background Page.navigate should load the background target without activating it"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "sessionId": "SID-bg",
        "params": {
            "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 13);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("same-context-routed"));
    assert_eq!(payload["text"], json!("same-context routed target"));
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
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.install_browser_context_fixture_for_test(bc);

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
    ctx.conn.install_browser_context_fixture_for_test(bc);
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
    ctx.conn.install_browser_context_fixture_for_test(bc);
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
    ctx.conn.install_browser_context_fixture_for_test(bc);
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
async fn playwright_over_cdp_context_target_attach_and_navigate_smoke() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 200,
        "method": "Target.createBrowserContext"
    }))
    .await;
    let browser_context_id = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context should exist")
        .id
        .clone();
    ctx.expect_result(200, json!({ "browserContextId": browser_context_id }), None);

    ctx.process_async(json!({
        "id": 201,
        "method": "Target.createTarget",
        "params": {
            "background": true,
            "browserContextId": browser_context_id,
            "url": "about:blank"
        }
    }))
    .await;
    let target_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_target_id_owned())
        .expect("target id should exist");
    ctx.expect_event("Target.targetCreated", None);
    ctx.expect_result(201, json!({ "targetId": target_id }), None);

    ctx.process_async(json!({
        "id": 202,
        "method": "Target.attachToTarget",
        "params": {
            "targetId": target_id,
            "flatten": true
        }
    }))
    .await;
    let session_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_session_id_owned())
        .expect("session id should exist");
    ctx.expect_result(202, json!({ "sessionId": session_id }), None);
    ctx.expect_event(
        "Target.attachedToTarget",
        Some(&json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "browserContextId": browser_context_id,
            }
        })),
    );

    ctx.process_async(json!({
        "id": 203,
        "method": "Runtime.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(203, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 204,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": session_id,
        "params": {
            "source": "globalThis.__lm_marker = 'seeded';"
        }
    }))
    .await;
    let preload = take_response_by_id(&mut ctx, 204);
    assert_eq!(preload["sessionId"], json!(session_id));
    assert!(preload["result"]["identifier"].as_str().is_some());

    ctx.process_async(json!({
        "id": 205,
        "method": "Emulation.setLocaleOverride",
        "sessionId": session_id,
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(205, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 206,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": session_id,
        "params": { "timezoneId": "UTC" }
    }))
    .await;
    ctx.expect_result(206, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 207,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": session_id,
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(207, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 208,
        "method": "Page.setInterceptFileChooserDialog",
        "sessionId": session_id,
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(208, json!({}), Some(&session_id));

    ctx.process_async(json!({
            "id": 209,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<body><script>document.body.dataset.marker = globalThis.__lm_marker;</script>ok</body>"
            }
        }))
        .await;
    let _ = take_response_by_id(&mut ctx, 209);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during navigation/setup: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 210,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ marker: document.body.dataset.marker, lang: navigator.language, locale: Intl.DateTimeFormat().resolvedOptions().locale, tz: Intl.DateTimeFormat().resolvedOptions().timeZone, dark: matchMedia('(prefers-color-scheme: dark)').matches })"
            }
        }))
        .await;
    let evaluation = take_response_by_id(&mut ctx, 210);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("stringified payload");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["marker"], "seeded");
    assert_eq!(payload["lang"], "en-US");
    assert_eq!(payload["locale"], "fr-FR");
    assert_eq!(payload["tz"], "UTC");
    assert_eq!(payload["dark"], true);

    let active = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(active.id, browser_context_id);
    assert_eq!(active.active_target_id(), Some(target_id.as_str()));
    assert_eq!(active.active_session_id(), Some(session_id.as_str()));
    assert_eq!(
        active
            .active_page_target()
            .effective_policy()
            .locale_override(),
        Some("fr-FR")
    );
    assert_eq!(
        active
            .active_page_target()
            .effective_policy()
            .timezone_override(),
        Some("UTC")
    );
}
