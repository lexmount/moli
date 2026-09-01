use super::*;

fn take_created_target_id(ctx: &mut TestContext, command_id: u64, description: &str) -> String {
    let response = take_response_by_id(ctx, command_id);
    let target_id = response["result"]["targetId"]
        .as_str()
        .unwrap_or_else(|| panic!("{description} target id"))
        .to_owned();
    assert_eq!(
        response["result"],
        json!({ "targetId": target_id.as_str() }),
        "{description} response should carry only its target id"
    );
    ctx.take_first_matching(description, |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"].as_str() == Some(target_id.as_str())
    });
    target_id
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_promotes_background_target_when_active_target_has_no_loaded_page() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 1801,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 1801, "second targetCreated");

    ctx.process_async(json!({
        "id": 1802,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(1802, json!({}), None);

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
    assert_eq!(bc.target_url(), "about:blank#second");
    assert_eq!(
        bc.background_target("TID-000000000A")
            .and_then(|target| target.session_id()),
        Some("SID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_handoffs_loaded_page_runtime_and_restores_it_when_switching_back() {
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
        "id": 1803,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 1803, "second targetCreated");

    ctx.process_async(json!({
        "id": 1804,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1804)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1805,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(1805, json!({}), None);

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
    assert_eq!(bc.active_session_id(), Some(second_session_id.as_str()));
    assert!(
        bc.background_target("TID-000000000A")
            .and_then(|target| target.loaded_page())
            .is_some(),
        "the previously active target should keep its loaded page runtime while backgrounded"
    );

    ctx.process_async(json!({
        "id": 1806,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>activated</title><div id='ok'>activated target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 1806);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1807,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    ctx.expect_result(1807, json!({}), None);

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some("TID-000000000A"));
    assert_eq!(bc.active_session_id(), Some("SID-active"));
    assert!(
        bc.background_target(&second_target_id)
            .and_then(|target| target.loaded_page())
            .is_some(),
        "the demoted second target should now keep its loaded page runtime while backgrounded"
    );

    ctx.process_async(json!({
            "id": 1808,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 1808);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("loaded"));
    assert_eq!(payload["text"], json!("loaded target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_to_target_chain_restores_multiple_loaded_page_runtimes_without_renavigation() {
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
        "id": 18081,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 18081, "second targetCreated");

    ctx.process_async(json!({
        "id": 18082,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 18082)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 18083,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 18083);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 18084,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#third"}
    }))
    .await;
    let third_target_id = take_created_target_id(&mut ctx, 18084, "third targetCreated");

    ctx.process_async(json!({
        "id": 18085,
        "method": "Target.attachToTarget",
        "params": {"targetId": third_target_id}
    }))
    .await;
    let third_session_id = take_response_by_id(&mut ctx, 18085)["result"]["sessionId"]
        .as_str()
        .expect("third target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 18086,
        "method": "Page.navigate",
        "sessionId": third_session_id,
        "params": {
            "url": "data:text/html,<title>third</title><div id='ok'>third target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let third_navigation = take_response_by_id(&mut ctx, 18086);
    assert_eq!(
        third_navigation["result"]["frameId"],
        json!(third_target_id)
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 18087,
        "method": "Target.attachToTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    let first_session_id = take_response_by_id(&mut ctx, 18087)["result"]["sessionId"]
        .as_str()
        .expect("first target session id")
        .to_owned();
    assert_ne!(first_session_id, "SID-active");
    let first_attached = ctx.take_first_matching("first target reattached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(first_session_id)
    });
    assert_eq!(
        first_attached["params"]["targetInfo"]["targetId"],
        "TID-000000000A"
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 18088,
            "method": "Runtime.evaluate",
            "sessionId": first_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let first_evaluation = take_response_by_id(&mut ctx, 18088);
    let first_payload = first_evaluation["result"]["result"]["value"]
        .as_str()
        .expect("first evaluation payload should be a string");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first evaluation payload should be valid json");
    assert_eq!(first_payload["title"], json!("first"));
    assert_eq!(first_payload["text"], json!("first target"));

    ctx.process_async(json!({
        "id": 18089,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let second_reattach_session_id = take_response_by_id(&mut ctx, 18089)["result"]["sessionId"]
        .as_str()
        .expect("second target reattach session id")
        .to_owned();
    assert_ne!(second_reattach_session_id, second_session_id);
    ctx.take_first_matching("second target reattached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(second_reattach_session_id)
    });
    ctx.take_all();

    ctx.process_async(json!({
            "id": 18090,
            "method": "Runtime.evaluate",
            "sessionId": second_reattach_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let second_evaluation = take_response_by_id(&mut ctx, 18090);
    let second_payload = second_evaluation["result"]["result"]["value"]
        .as_str()
        .expect("second evaluation payload should be a string");
    let second_payload: serde_json::Value = serde_json::from_str(second_payload)
        .expect("second evaluation payload should be valid json");
    assert_eq!(second_payload["title"], json!("second"));
    assert_eq!(second_payload["text"], json!("second target"));

    ctx.process_async(json!({
        "id": 18091,
        "method": "Target.attachToTarget",
        "params": {"targetId": third_target_id}
    }))
    .await;
    let third_reattach_session_id = take_response_by_id(&mut ctx, 18091)["result"]["sessionId"]
        .as_str()
        .expect("third target reattach session id")
        .to_owned();
    assert_ne!(third_reattach_session_id, third_session_id);
    ctx.take_first_matching("third target reattached", |message| {
        message["method"] == json!("Target.attachedToTarget")
            && message["params"]["sessionId"] == json!(third_reattach_session_id)
    });
    ctx.take_all();

    ctx.process_async(json!({
            "id": 18092,
            "method": "Runtime.evaluate",
            "sessionId": third_reattach_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let third_evaluation = take_response_by_id(&mut ctx, 18092);
    let third_payload = third_evaluation["result"]["result"]["value"]
        .as_str()
        .expect("third evaluation payload should be a string");
    let third_payload: serde_json::Value =
        serde_json::from_str(third_payload).expect("third evaluation payload should be valid json");
    assert_eq!(third_payload["title"], json!("third"));
    assert_eq!(third_payload["text"], json!("third target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_chain_restores_multiple_loaded_page_runtimes_without_renavigation() {
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
        "id": 1810,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 1810, "second targetCreated");

    ctx.process_async(json!({
        "id": 1811,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let second_session_id = take_response_by_id(&mut ctx, 1811)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1812,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 1812);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!(second_target_id)
    );
    ctx.take_all();
    ctx.process_async(json!({
        "id": 18121,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(18121, json!({}), None);

    ctx.process_async(json!({
        "id": 1813,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#third"}
    }))
    .await;
    let third_target_id = take_created_target_id(&mut ctx, 1813, "third targetCreated");

    ctx.process_async(json!({
        "id": 1814,
        "method": "Target.attachToTarget",
        "params": {"targetId": third_target_id}
    }))
    .await;
    let third_session_id = take_response_by_id(&mut ctx, 1814)["result"]["sessionId"]
        .as_str()
        .expect("third target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1815,
        "method": "Page.navigate",
        "sessionId": third_session_id,
        "params": {
            "url": "data:text/html,<title>third</title><div id='ok'>third target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let third_navigation = take_response_by_id(&mut ctx, 1815);
    assert_eq!(
        third_navigation["result"]["frameId"],
        json!(third_target_id)
    );
    ctx.take_all();
    ctx.process_async(json!({
        "id": 18151,
        "method": "Target.activateTarget",
        "params": {"targetId": third_target_id}
    }))
    .await;
    ctx.expect_result(18151, json!({}), None);

    let bc = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context");
    assert_eq!(bc.active_target_id(), Some(third_target_id.as_str()));
    assert!(
        bc.background_target("TID-000000000A")
            .and_then(|target| target.loaded_page())
            .is_some(),
        "first target runtime should stay parked in the background",
    );
    assert!(
        bc.background_target(&second_target_id)
            .and_then(|target| target.loaded_page())
            .is_some(),
        "second target runtime should stay parked in the background",
    );

    ctx.process_async(json!({
        "id": 1816,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000A"}
    }))
    .await;
    ctx.expect_result(1816, json!({}), None);

    ctx.process_async(json!({
            "id": 1817,
            "method": "Runtime.evaluate",
            "sessionId": "SID-active",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let first_eval = take_response_by_id(&mut ctx, 1817);
    let first_payload = first_eval["result"]["result"]["value"]
        .as_str()
        .expect("first evaluation payload should be a string");
    let first_payload: serde_json::Value =
        serde_json::from_str(first_payload).expect("first evaluation payload should be valid json");
    assert_eq!(first_payload["title"], json!("first"));
    assert_eq!(first_payload["text"], json!("first target"));

    ctx.process_async(json!({
        "id": 1818,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(1818, json!({}), None);

    ctx.process_async(json!({
            "id": 1819,
            "method": "Runtime.evaluate",
            "sessionId": second_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let second_eval = take_response_by_id(&mut ctx, 1819);
    let second_payload = second_eval["result"]["result"]["value"]
        .as_str()
        .expect("second evaluation payload should be a string");
    let second_payload: serde_json::Value = serde_json::from_str(second_payload)
        .expect("second evaluation payload should be valid json");
    assert_eq!(second_payload["title"], json!("second"));
    assert_eq!(second_payload["text"], json!("second target"));

    ctx.process_async(json!({
        "id": 1820,
        "method": "Target.activateTarget",
        "params": {"targetId": third_target_id}
    }))
    .await;
    ctx.expect_result(1820, json!({}), None);

    ctx.process_async(json!({
            "id": 1821,
            "method": "Runtime.evaluate",
            "sessionId": third_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let third_eval = take_response_by_id(&mut ctx, 1821);
    let third_payload = third_eval["result"]["result"]["value"]
        .as_str()
        .expect("third evaluation payload should be a string");
    let third_payload: serde_json::Value =
        serde_json::from_str(third_payload).expect("third evaluation payload should be valid json");
    assert_eq!(third_payload["title"], json!("third"));
    assert_eq!(third_payload["text"], json!("third target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_then_attach_can_navigate_on_promoted_target_without_loaded_page() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000A");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");
    ctx.process_async(json!({
        "id": 1805,
        "method": "Target.createTarget",
        "params": {"browserContextId": "BID-9", "url": "about:blank#second"}
    }))
    .await;
    let second_target_id = take_created_target_id(&mut ctx, 1805, "second targetCreated");

    ctx.process_async(json!({
        "id": 1806,
        "method": "Target.activateTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    ctx.expect_result(1806, json!({}), None);

    ctx.process_async(json!({
        "id": 1807,
        "method": "Target.attachToTarget",
        "params": {"targetId": second_target_id}
    }))
    .await;
    let session_id = take_response_by_id(&mut ctx, 1807)["result"]["sessionId"]
        .as_str()
        .expect("promoted target session id")
        .to_owned();
    ctx.expect_event("Target.attachedToTarget", None);

    ctx.process_async(json!({
        "id": 1808,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": {
            "url": "data:text/html,<title>activated</title><div id='ok'>activated target</div>"
        }
    }))
    .await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 1808);
    assert_eq!(navigation["result"]["frameId"], json!(second_target_id));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 1809,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 1809);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("activated"));
    assert_eq!(payload["text"], json!("activated target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_target_info_for_inactive_target_keeps_previously_active_context() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-A");
    let mut inactive = BrowserContext::new_with_page_for_test("BID-B", "TID-B");
    inactive.set_active_target_id("TID-B");
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 111,
        "method": "Target.getTargetInfo",
        "params": {"targetId": "TID-B"}
    }))
    .await;
    ctx.expect_result(
        111,
        json!({
            "targetInfo": {
                "targetId": "TID-B",
                "type": "page",
                "title": "",
                "url": "about:blank",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-B",
            }
        }),
        None,
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "querying another target must not leave its browser context selected as the default active context"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_message_to_target_restores_previously_active_context() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-A");
    let mut inactive = BrowserContext::new_with_page_for_test("BID-B", "TID-B");
    inactive.attach_active_session("SID-B");
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 1501,
        "method": "Target.sendMessageToTarget",
        "params": {
            "message": "{\"id\":1,\"method\":\"Target.getTargetInfo\"}",
            "sessionId": "SID-B"
        }
    }))
    .await;

    ctx.expect_result(1501, json!({}), None);
    let event = ctx.take_one();
    assert_eq!(event["method"], "Target.receivedMessageFromTarget");
    assert_eq!(event["params"]["sessionId"], "SID-B");
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "sendMessageToTarget for another context must restore the original active context"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_from_target_error_restores_previously_active_context() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-A");
    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B");
    inactive.attach_active_session("SID-B");
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 1502,
        "method": "Target.detachFromTarget",
        "params": {
            "sessionId": "SID-B",
            "targetId": "TID-WRONG"
        }
    }))
    .await;

    ctx.expect_error(1502, -31998, "UnknownTargetId");
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "failing detachFromTarget for another context must restore the original active context"
    );
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
    let attached =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 34, 35, 36).await;
    let target_id = attached.target_id;
    let session_id = attached.session_id;

    ctx.process_async(json!({
        "id": 39,
        "method": "Network.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(39, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 40,
        "method": "Fetch.enable",
        "sessionId": session_id
    }))
    .await;
    ctx.expect_result(40, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 41,
        "method": "Page.navigate",
        "sessionId": session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let network_id = paused["params"]["networkId"].clone();

    ctx.process_async(json!({
        "id": 42,
        "method": "Target.detachFromTarget",
        "params": { "targetId": target_id, "sessionId": session_id }
    }))
    .await;
    ctx.expect_result(42, json!({}), None);

    let failed = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == network_id
        })
        .cloned()
        .expect("network loadingFailed event");
    assert_eq!(failed["sessionId"], json!(session_id));
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Target detached");

    let error = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(41))
        .cloned()
        .expect("navigation error response");
    assert_eq!(error["id"], 41);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Target detached");

    let detached = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["targetId"] == json!(target_id)
        })
        .cloned()
        .expect("target detached event");
    assert_eq!(detached["method"], "Target.detachedFromTarget");
    assert_eq!(detached["params"]["targetId"], json!(target_id));
    assert_eq!(detached["params"]["sessionId"], json!(session_id));

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert_eq!(bc.active_target_id(), Some(target_id.as_str()));
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
async fn same_context_targets_keep_paused_fetch_state_target_local_after_switching() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>paused-fetch</body></html>",
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
    let first_attached =
        create_attached_page_session_without_runtime_enable_async(&mut ctx, 104190, 104191, 104192)
            .await;
    let browser_context_id = first_attached.browser_context_id.clone();
    let first_target_id = first_attached.target_id.clone();
    let first_session_id = first_attached.session_id.clone();

    ctx.process_async(json!({
        "id": 104197,
        "method": "Fetch.enable",
        "sessionId": first_session_id,
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
    ctx.expect_result(104197, json!({}), Some(&first_session_id));

    ctx.process_async(json!({
        "id": 104198,
        "method": "Page.navigate",
        "sessionId": first_session_id,
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    let second_attached = attach_page_session_without_runtime_enable_in_existing_context_async(
        &mut ctx,
        &browser_context_id,
        104199,
        104200,
    )
    .await;
    let second_target_id = second_attached.target_id.clone();

    ctx.process_async(json!({
        "id": 104201,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(104201, json!({}), None);

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some(second_target_id.as_str()));
        assert!(
            !bc.active_page_target()
                .fetch_owner
                .has_pending_fetch_state_for_test(),
            "promoted target should not see another target's pending fetch ids",
        );
        let parked = bc
            .nonempty_background_fetch_state_for_test(&first_target_id)
            .expect("first target pending fetch state should be parked");
        assert!(
            parked.has_pending_fetch_request_id_for_test(&request_id),
            "first target pending fetch id should move with parked target state",
        );
    }

    ctx.process_async(json!({
        "id": 104202,
        "method": "Target.activateTarget",
        "params": { "targetId": first_target_id }
    }))
    .await;
    ctx.expect_result(104202, json!({}), None);

    ctx.process_async(json!({
        "id": 104203,
        "method": "Fetch.continueRequest",
        "sessionId": first_session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(104203, json!({}), Some(&first_session_id));

    let navigation = take_response_by_id(&mut ctx, 104198);
    assert_eq!(navigation["result"]["frameId"], json!(first_target_id));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn production_default_target_auto_attach_exposes_initial_about_blank_page() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.conn.enable_default_target_on_auto_attach();

    ctx.process_async(json!({
        "id": 1700,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;

    ctx.expect_result(1700, json!({}), None);
    let events = ctx.take_all();
    let attached = events
        .iter()
        .find(|event| event["method"] == json!("Target.attachedToTarget"))
        .expect("auto-attach should report the production default page target");
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        json!(ctx.conn.default_target_id())
    );
    assert_eq!(
        attached["params"]["targetInfo"]["url"],
        json!("about:blank")
    );
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));
    assert_eq!(
        attached["params"]["targetInfo"]["browserContextId"],
        json!(ctx.conn.default_browser_context_id())
    );

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some(ctx.conn.default_target_id()));
    assert!(bc.has_active_session());
    let loaded_page = bc
        .loaded_page()
        .expect("default auto-attached target should install initial about:blank page");
    assert_eq!(loaded_page.final_url().as_str(), "about:blank");
}

#[tokio::test(flavor = "multi_thread")]
async fn production_default_target_is_the_initial_browser_page_target() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    ctx.conn.install_default_browser_target();
    ctx.conn.enable_default_target_on_auto_attach();

    ctx.process_async(json!({
        "id": 1690,
        "method": "Target.getTargets"
    }))
    .await;
    let get_targets = ctx.take_response_by_id(1690);
    let initial_targets = get_targets["result"]["targetInfos"]
        .as_array()
        .expect("Target.getTargets targetInfos");
    assert_eq!(initial_targets.len(), 1);
    assert_eq!(
        initial_targets[0]["targetId"],
        json!(ctx.conn.default_target_id())
    );
    assert_eq!(initial_targets[0]["url"], json!("about:blank"));
    assert_eq!(initial_targets[0]["attached"], json!(false));

    ctx.process_async(json!({
        "id": 1691,
        "method": "Target.setDiscoverTargets",
        "params": {
            "discover": true,
            "filter": [{ "type": "page" }]
        }
    }))
    .await;
    ctx.expect_result(1691, json!({}), None);
    let created = ctx.take_first_matching("default targetCreated", |message| {
        message["method"] == json!("Target.targetCreated")
    });
    assert_eq!(
        created["params"]["targetInfo"]["targetId"],
        json!(ctx.conn.default_target_id())
    );
    assert!(ctx.sent.is_empty());

    ctx.process_async(json!({
        "id": 1692,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(1692, json!({}), None);
    let attached = ctx.take_first_matching("default attachedToTarget", |message| {
        message["method"] == json!("Target.attachedToTarget")
    });
    assert_eq!(
        attached["params"]["targetInfo"]["targetId"],
        json!(ctx.conn.default_target_id())
    );
    assert_eq!(attached["params"]["targetInfo"]["attached"], json!(true));

    ctx.process_async(json!({
        "id": 1693,
        "method": "Target.getTargets"
    }))
    .await;
    let get_targets = ctx.take_response_by_id(1693);
    let targets_after_auto_attach = get_targets["result"]["targetInfos"]
        .as_array()
        .expect("Target.getTargets targetInfos after auto-attach");
    assert_eq!(targets_after_auto_attach.len(), 1);
    assert_eq!(
        targets_after_auto_attach[0]["targetId"],
        json!(ctx.conn.default_target_id())
    );
    assert_eq!(targets_after_auto_attach[0]["attached"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_true_attaches_existing_background_targets() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#bg".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));

    ctx.process_async(json!({
        "id": 1701,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(1701, json!({}), None);
    let events = ctx.take_all();
    let attached_target_ids = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .map(|event| {
            event["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("attached target id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        attached_target_ids,
        vec!["TID-000000000E".to_owned(), "TID-000000000F".to_owned()]
    );
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000F"));
    assert!(bc.has_active_session());
    assert!(
        bc.background_target("TID-000000000E")
            .and_then(|target| target.session_id())
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_promotes_existing_background_target_when_active_target_has_no_loaded_page()
{
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#bg".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));

    ctx.process_async(json!({
        "id": 17015,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(17015, json!({}), None);
    let events = ctx.take_all();
    let attached = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-000000000F")
        })
        .expect("background attached event for TID-000000000F");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000F"));
    assert_eq!(bc.active_session_id(), Some(session_id.as_str()));
    assert_eq!(
        bc.background_target("TID-000000000E")
            .and_then(|target| target.session_id()),
        Some("SID-1"),
        "the old active target should be demoted into the background with its newly attached session",
    );

    ctx.process_async(json!({
            "id": 17016,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<title>autoattach-sweep-promoted</title><div id='ok'>setAutoAttach sweep promoted target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 17016);
    assert_eq!(navigation["result"]["frameId"], json!("TID-000000000F"));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 17017,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 17017);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("autoattach-sweep-promoted"));
    assert_eq!(
        payload["text"],
        json!("setAutoAttach sweep promoted target")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_sweep_chain_promotes_multiple_existing_background_targets_into_runtime() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
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

    ctx.process_async(json!({
        "id": 17018,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(17018, json!({}), None);
    let events = ctx.take_all();
    let second_attached = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-000000000F")
        })
        .expect("background attached event for TID-000000000F");
    let second_session_id = second_attached["params"]["sessionId"]
        .as_str()
        .expect("second background session id")
        .to_owned();
    let third_attached = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-0000000010")
        })
        .expect("background attached event for TID-0000000010");
    let third_session_id = third_attached["params"]["sessionId"]
        .as_str()
        .expect("third background session id")
        .to_owned();

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000F"));
    assert_eq!(bc.active_session_id(), Some(second_session_id.as_str()));

    ctx.process_async(json!({
            "id": 17019,
            "method": "Page.navigate",
            "sessionId": second_session_id,
            "params": {
                "url": "data:text/html,<title>sweep-second-promoted</title><div id='ok'>second sweep promoted target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let second_navigation = take_response_by_id(&mut ctx, 17019);
    assert_eq!(
        second_navigation["result"]["frameId"],
        json!("TID-000000000F")
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 17020,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000F"}
    }))
    .await;
    ctx.expect_result(17020, json!({ "success": true }), None);
    expect_inspector_detached(&mut ctx, &second_session_id);
    ctx.expect_event(
        "Target.detachedFromTarget",
        Some(&json!({
            "targetId": "TID-000000000F",
            "sessionId": second_session_id,
        })),
    );

    ctx.process_async(json!({
            "id": 17021,
            "method": "Page.navigate",
            "sessionId": third_session_id,
            "params": {
                "url": "data:text/html,<title>sweep-third-promoted</title><div id='ok'>third sweep promoted target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let third_navigation = take_response_by_id(&mut ctx, 17021);
    assert_eq!(
        third_navigation["result"]["frameId"],
        json!("TID-0000000010")
    );
    ctx.take_all();

    ctx.process_async(json!({
            "id": 17022,
            "method": "Runtime.evaluate",
            "sessionId": third_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 17022);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("sweep-third-promoted"));
    assert_eq!(payload["text"], json!("third sweep promoted target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_prefers_existing_background_target_with_parked_loaded_runtime() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
    {
        let bc = ctx.conn.browser_context.as_mut().unwrap();
        bc.insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#metadata-only".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
        bc.stage_active_target_demoting_current(
            "TID-0000000010".into(),
            None,
            "about:blank#parked".into(),
            Some("about:blank".into()),
        );
    }
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>parked</title><div id='ok'>parked runtime</div>",
        None,
    )
    .await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .promote_background_target_to_active_slot_async("TID-000000000E")
            .await
            .expect("restoring the original active target should succeed"),
        "the original active target should remain parked during fixture setup"
    );

    ctx.process_async(json!({
        "id": 17023,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(17023, json!({}), None);
    let events = ctx.take_all();
    let metadata_attached = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-000000000F")
        })
        .expect("background attached event for metadata-only target");
    let metadata_session_id = metadata_attached["params"]["sessionId"]
        .as_str()
        .expect("metadata-only background session id")
        .to_owned();
    let parked_attached = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-0000000010")
        })
        .expect("background attached event for parked target");
    let parked_session_id = parked_attached["params"]["sessionId"]
        .as_str()
        .expect("parked background session id")
        .to_owned();

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-0000000010"));
    assert_eq!(bc.active_session_id(), Some(parked_session_id.as_str()));
    assert_eq!(
        bc.background_target("TID-000000000F")
            .and_then(|target| target.session_id()),
        Some(metadata_session_id.as_str())
    );
    assert_eq!(
        bc.background_target("TID-000000000E")
            .and_then(|target| target.session_id()),
        Some("SID-1")
    );

    ctx.process_async(json!({
            "id": 17024,
            "method": "Runtime.evaluate",
            "sessionId": parked_session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 17024);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("parked"));
    assert_eq!(payload["text"], json!("parked runtime"));
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_promotes_set_auto_attach_background_session_into_page_runtime() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            None,
            crate::conn::TargetIdentityState::new(
                "about:blank#bg".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));

    ctx.process_async(json!({
        "id": 17011,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(17011, json!({}), None);
    let events = ctx.take_all();
    let attached = events
        .iter()
        .find(|event| {
            event["method"] == json!("Target.attachedToTarget")
                && event["params"]["targetInfo"]["targetId"] == json!("TID-000000000F")
        })
        .expect("background attached event for TID-000000000F");
    let session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("background session id")
        .to_owned();

    ctx.process_async(json!({
        "id": 17012,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000F"}
    }))
    .await;
    ctx.expect_result(17012, json!({}), None);

    ctx.process_async(json!({
            "id": 17013,
            "method": "Page.navigate",
            "sessionId": session_id,
            "params": {
                "url": "data:text/html,<title>autoattach-activated</title><div id='ok'>setAutoAttach promoted target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 17013);
    assert_eq!(navigation["result"]["frameId"], json!("TID-000000000F"));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 17014,
            "method": "Runtime.evaluate",
            "sessionId": session_id,
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 17014);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("autoattach-activated"));
    assert_eq!(payload["text"], json!("setAutoAttach promoted target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_target_chain_switches_between_multiple_attached_background_targets_without_loaded_page()
 {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.attach_active_session("SID-active");
    bc.insert_page_target_host(crate::conn::PageTargetHost::new(
        "TID-000000000F".into(),
        Some("SID-second".into()),
        crate::conn::TargetIdentityState::new(
            "about:blank#second".into(),
            crate::conn::URL_BASE.into(),
            "Secure".into(),
        ),
        crate::conn::TargetPageSlot::empty_for_test_fixture(),
    ));
    bc.insert_page_target_host(crate::conn::PageTargetHost::new(
        "TID-0000000010".into(),
        Some("SID-third".into()),
        crate::conn::TargetIdentityState::new(
            "about:blank#third".into(),
            crate::conn::URL_BASE.into(),
            "Secure".into(),
        ),
        crate::conn::TargetPageSlot::empty_for_test_fixture(),
    ));

    ctx.process_async(json!({
        "id": 17030,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-000000000F"}
    }))
    .await;
    ctx.expect_result(17030, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-000000000F"));
    assert_eq!(bc.active_session_id(), Some("SID-second"));
    assert_eq!(
        bc.background_target("TID-000000000E")
            .and_then(|target| target.session_id()),
        Some("SID-active"),
    );

    ctx.process_async(json!({
        "id": 17031,
        "method": "Target.activateTarget",
        "params": {"targetId": "TID-0000000010"}
    }))
    .await;
    ctx.expect_result(17031, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(bc.active_target_id(), Some("TID-0000000010"));
    assert_eq!(bc.active_session_id(), Some("SID-third"));
    assert_eq!(
        bc.background_target("TID-000000000F")
            .and_then(|target| target.session_id()),
        Some("SID-second"),
    );

    ctx.process_async(json!({
            "id": 17032,
            "method": "Page.navigate",
            "sessionId": "SID-third",
            "params": {
                "url": "data:text/html,<title>activate-chain-promoted</title><div id='ok'>activate chain promoted target</div>"
            }
        })).await;
    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 17032);
    assert_eq!(navigation["result"]["frameId"], json!("TID-0000000010"));
    ctx.take_all();

    ctx.process_async(json!({
            "id": 17033,
            "method": "Runtime.evaluate",
            "sessionId": "SID-third",
            "params": {
                "expression": "JSON.stringify({ title: document.title, text: document.getElementById('ok').textContent })"
            }
        })).await;
    let evaluation = take_response_by_id(&mut ctx, 17033);
    let payload = evaluation["result"]["result"]["value"]
        .as_str()
        .expect("evaluation payload should be a string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("evaluation payload should be valid json");
    assert_eq!(payload["title"], json!("activate-chain-promoted"));
    assert_eq!(payload["text"], json!("activate chain promoted target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_false_detaches_existing_background_targets() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-9", "TID-000000000E");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-1");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-000000000F".into(),
            Some("SID-bg".into()),
            crate::conn::TargetIdentityState::new(
                "about:blank#bg".into(),
                crate::conn::URL_BASE.into(),
                "Secure".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
    ctx.conn.auto_attach = true;

    ctx.process_async(json!({
        "id": 1702,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": false,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(1702, json!({}), None);
    let events = ctx.take_all();
    let detached_target_ids = events
        .iter()
        .filter(|event| event["method"] == json!("Target.detachedFromTarget"))
        .map(|event| {
            event["params"]["targetId"]
                .as_str()
                .expect("detached target id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        detached_target_ids,
        vec!["TID-000000000E".to_owned(), "TID-000000000F".to_owned()]
    );
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_session());
    assert_eq!(
        bc.background_target("TID-000000000F")
            .and_then(|target| target.session_id()),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_auto_attach_restores_previously_active_context_after_sweeping_contexts() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-A", "TID-A");
    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B");
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 181,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .await;

    ctx.expect_result(181, json!({}), None);
    let events = ctx.take_all();
    let attached_target_ids = events
        .iter()
        .filter(|event| event["method"] == json!("Target.attachedToTarget"))
        .map(|event| {
            event["params"]["targetInfo"]["targetId"]
                .as_str()
                .expect("attached target id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        attached_target_ids,
        vec!["TID-A".to_owned(), "TID-B".to_owned()]
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "setAutoAttach should restore the previously active browser context after sweeping all contexts"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("BID-B should remain inactive after the sweep");
    assert!(inactive.has_active_session());
}
