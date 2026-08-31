use super::*;
use moli_core::LayoutPolicy;

async fn renderer_backend_node_id_for_live_expression(
    ctx: &mut TestContext,
    evaluate_id: u64,
    describe_id: u64,
    expression: &str,
) -> u32 {
    ctx.process_async(json!({
        "id": evaluate_id,
        "method": "Runtime.evaluate",
        "params": { "expression": expression }
    }))
    .await;
    let object_id = take_response_by_id(ctx, evaluate_id)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return objectId for {expression}"))
        .to_owned();

    ctx.process_async(json!({
        "id": describe_id,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let described = take_response_by_id(ctx, describe_id);
    let backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("describeNode should return u32 backendNodeId: {described}"));
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "live describeNode should use renderer backend id namespace: {described}"
    );
    backend_node_id
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_resolve_geometry_and_mutation_target_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let background_url = url::Url::parse("https://background.test/owned").unwrap();
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        background_url.as_str().to_owned(),
    );

    let mut bc = BrowserContext::new("BID-A".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_buffered_navigation_fixture_for_session_owner(
        background_url,
        "<!doctype html><html><body><section id='owned' data-route='background' style='position:absolute;left:11px;top:13px;width:17px;height:19px'>background</section></body></html>".to_owned(),
        Some("SID-background"),
    )
    .await;
    ctx.wait_for_scheduler_message("background fixture load", |message| {
        message["sessionId"] == json!("SID-background")
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 340,
        "sessionId": "SID-background",
        "method": "Runtime.enable"
    }))
    .await;
    ctx.expect_result(340, json!({}), Some("SID-background"));
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 341,
        "sessionId": "SID-background",
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 341);
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document root node id");
    let root_backend_node_id = document["result"]["root"]["backendNodeId"]
        .as_u64()
        .expect("document root backend node id");

    ctx.process_async(json!({
        "id": 342,
        "sessionId": "SID-background",
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#owned" }
    }))
    .await;
    let owned_node_id = take_query_selector_node_id(&mut ctx, 342);

    ctx.process_async(json!({
        "id": 343,
        "sessionId": "SID-background",
        "method": "DOM.getAttributes",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    let attributes = take_response_by_id(&mut ctx, 343);
    assert_eq!(attributes["sessionId"], "SID-background");
    assert_eq!(
        attributes["result"]["attributes"],
        json!([
            "id",
            "owned",
            "data-route",
            "background",
            "style",
            "position:absolute;left:11px;top:13px;width:17px;height:19px"
        ])
    );

    ctx.process_async(json!({
        "id": 344,
        "sessionId": "SID-background",
        "method": "DOM.describeNode",
        "params": { "nodeId": owned_node_id, "depth": 1 }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 344);
    assert_eq!(described["sessionId"], "SID-background");
    assert_eq!(described["result"]["node"]["nodeName"], json!("SECTION"));
    let owned_backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("owned backend node id");

    ctx.process_async(json!({
        "id": 345,
        "sessionId": "SID-background",
        "method": "DOM.pushNodesByBackendIdsToFrontend",
        "params": { "backendNodeIds": [owned_backend_node_id, 999999, root_backend_node_id] }
    }))
    .await;
    let pushed = take_response_by_id(&mut ctx, 345);
    assert_eq!(pushed["sessionId"], "SID-background");
    let pushed_node_ids = pushed["result"]["nodeIds"]
        .as_array()
        .expect("pushed frontend node ids");
    assert_eq!(pushed_node_ids.len(), 3);
    assert_eq!(pushed_node_ids[1], json!(0));
    assert!(pushed_node_ids[0].as_u64().is_some_and(|id| id > 0));
    assert!(pushed_node_ids[2].as_u64().is_some_and(|id| id > 0));

    ctx.process_async(json!({
        "id": 346,
        "sessionId": "SID-background",
        "method": "DOM.resolveNode",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 346);
    assert_eq!(resolved["sessionId"], "SID-background");
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .expect("resolveNode object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 347,
        "sessionId": "SID-background",
        "method": "DOM.requestNode",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(
        347,
        json!({ "nodeId": owned_node_id }),
        Some("SID-background"),
    );

    ctx.process_async(json!({
        "id": 348,
        "sessionId": "SID-background",
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    let outer_html = take_response_by_id(&mut ctx, 348);
    assert_eq!(outer_html["sessionId"], "SID-background");
    assert!(
        outer_html["result"]["outerHTML"]
            .as_str()
            .is_some_and(|html| html.contains("data-route=\"background\""))
    );

    ctx.process_async(json!({
        "id": 349,
        "sessionId": "SID-background",
        "method": "DOM.getBoxModel",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    ctx.expect_result(
        349,
        axis_aligned_box_model(11.0, 13.0, 17, 19),
        Some("SID-background"),
    );

    ctx.process_async(json!({
        "id": 350,
        "sessionId": "SID-background",
        "method": "DOM.getContentQuads",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    ctx.expect_result(
        350,
        json!({ "quads": [axis_aligned_geometry_quad(11.0, 13.0, 17.0, 19.0)] }),
        Some("SID-background"),
    );

    ctx.process_async(json!({
        "id": 351,
        "sessionId": "SID-background",
        "method": "DOM.getNodeForLocation",
        "params": { "x": 12, "y": 14 }
    }))
    .await;
    let node_for_location = take_response_by_id(&mut ctx, 351);
    assert_eq!(node_for_location["sessionId"], "SID-background");
    assert_eq!(
        node_for_location["result"]["backendNodeId"],
        json!(owned_backend_node_id)
    );

    ctx.process_async(json!({
        "id": 352,
        "sessionId": "SID-background",
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    ctx.expect_result(352, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 353,
        "sessionId": "SID-background",
        "method": "DOM.removeNode",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    let remove_messages = ctx.take_all();
    let mutation_position = remove_messages
        .iter()
        .position(|message| {
            message["method"] == json!("DOM.childNodeRemoved")
                && message["params"]["nodeId"] == json!(owned_node_id)
                && message["sessionId"] == json!("SID-background")
        })
        .expect("background removeNode should publish its DOM mutation");
    let response_position = remove_messages
        .iter()
        .position(|message| message["id"] == json!(353))
        .expect("background removeNode response");
    assert!(mutation_position < response_position);
    assert_eq!(remove_messages[response_position]["result"], json!({}));
    assert_eq!(
        remove_messages[response_position]["sessionId"],
        json!("SID-background")
    );

    ctx.process_async(json!({
        "id": 354,
        "sessionId": "SID-background",
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#owned" }
    }))
    .await;
    assert_eq!(take_query_selector_node_id(&mut ctx, 354), 0);

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some("TID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_resolve_geometry_targets_inactive_loaded_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut active = BrowserContext::new("BID-active".to_owned());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active".to_owned());
    ctx.conn.browser_context = Some(active);

    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.set_target_url("about:blank".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><html><body><article id='owned' style='position:absolute;left:7px;top:9px;width:13px;height:15px'>inactive</article></body></html>",
        Some("SID-inactive"),
    )
    .await;
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        Some("SID-inactive"),
        "TID-inactive",
        crate::domains::page::LOADER_ID,
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 361,
        "sessionId": "SID-inactive",
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 361)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document root node id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 362,
        "sessionId": "SID-inactive",
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#owned" }
    }))
    .await;
    let owned_node_id = take_query_selector_node_id(&mut ctx, 362);

    ctx.process_async(json!({
        "id": 363,
        "sessionId": "SID-inactive",
        "method": "DOM.getBoxModel",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    ctx.expect_result(
        363,
        axis_aligned_box_model(7.0, 9.0, 13, 15),
        Some("SID-inactive"),
    );

    ctx.process_async(json!({
        "id": 364,
        "sessionId": "SID-inactive",
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": owned_node_id }
    }))
    .await;
    let outer_html = take_response_by_id(&mut ctx, 364);
    assert_eq!(outer_html["sessionId"], "SID-inactive");
    assert!(
        outer_html["result"]["outerHTML"]
            .as_str()
            .is_some_and(|html| html.contains("inactive"))
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_box_model() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p style='position:absolute;left:10px;top:10px;width:5px;height:5px;margin:0'>box</p></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 3, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 3)["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(root_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "p" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 4)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(node_id > 0);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getBoxModel",
        "params": { "nodeId": node_id }
    }))
    .await;
    ctx.expect_result(5, axis_aligned_box_model(10.0, 10.0, 5, 5), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn top_frame_geometry_non_element_waits_for_renderer_completion() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>plain text</body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 2)["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "body" }
    }))
    .await;
    let body_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(body_node_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.body.firstChild",
            "returnByValue": false
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .expect("text node object id")
        .to_owned();

    let raw = json!({
        "id": 5,
        "method": "DOM.getBoxModel",
        "params": { "objectId": object_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("BiDi top-frame text node geometry should start a renderer command");
    assert_eq!(pending.kind_name(), "DOM");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(5))
        .unwrap_or_else(|| {
            panic!("pending BiDi top-frame DOM.getBoxModel should respond: {messages:?}")
        });
    assert_eq!(
        response["error"],
        json!({ "code": -32000, "message": "Node is not an element" })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn geometry_and_remove_node_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p id='target' style='position:absolute;left:10px;top:10px;width:5px;height:5px;margin:0'>box</p></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let node_id = take_query_selector_node_id(&mut ctx, 3);
    assert!(node_id > 0);

    let box_raw = json!({
        "id": 4,
        "method": "DOM.getBoxModel",
        "params": { "nodeId": node_id }
    })
    .to_string();
    let box_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&box_raw)
        .expect("DOM.getBoxModel with nodeId should start as a pending command");
    let box_messages = complete_pending_command_task_for_test(&mut ctx, box_pending).await;
    let box_response = box_messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .expect("pending DOM.getBoxModel should produce a response");
    assert_eq!(
        box_response["result"]["model"]["content"],
        axis_aligned_geometry_quad(10.0, 10.0, 5.0, 5.0)
    );

    ctx.process_async(json!({
        "id": 31,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id }
    }))
    .await;
    let backend_node_id = take_response_by_id(&mut ctx, 31)["result"]["node"]["backendNodeId"]
        .as_u64()
        .unwrap_or(0);

    let quads_raw = json!({
        "id": 5,
        "method": "DOM.getContentQuads",
        "params": { "backendNodeId": backend_node_id }
    })
    .to_string();
    let quads_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&quads_raw)
        .expect("DOM.getContentQuads with backendNodeId should start as a pending command");
    let quads_messages = complete_pending_command_task_for_test(&mut ctx, quads_pending).await;
    let quads_response = quads_messages
        .iter()
        .find(|message| message["id"] == json!(5))
        .expect("pending DOM.getContentQuads should produce a response");
    assert_eq!(
        quads_response["result"]["quads"],
        json!([axis_aligned_geometry_quad(10.0, 10.0, 5.0, 5.0)])
    );

    let _default_context_id = enable_runtime_and_take_execution_context_id_async(&mut ctx, 6).await;
    let _ = ctx.take_all();
    let resolve_raw = json!({
        "id": 7,
        "method": "DOM.resolveNode",
        "params": { "nodeId": node_id }
    })
    .to_string();
    let resolve_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&resolve_raw)
        .expect("DOM.resolveNode without executionContextId should start as a pending command");
    let resolve_messages = complete_pending_command_task_for_test(&mut ctx, resolve_pending).await;
    let resolve_response = resolve_messages
        .iter()
        .find(|message| message["id"] == json!(7))
        .expect("pending DOM.resolveNode should produce a response");
    assert_eq!(
        resolve_response["result"]["object"]["type"],
        json!("object")
    );
    assert_eq!(
        resolve_response["result"]["object"]["subtype"],
        json!("node")
    );
    assert!(
        resolve_response["result"]["object"]["objectId"]
            .as_str()
            .is_some()
    );
    let resolved_object_id = resolve_response["result"]["object"]["objectId"]
        .as_str()
        .expect("pending DOM.resolveNode should return object id")
        .to_owned();

    let request_raw = json!({
        "id": 10,
        "method": "DOM.requestNode",
        "params": { "objectId": resolved_object_id }
    })
    .to_string();
    let request_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&request_raw)
        .expect("DOM.requestNode with objectId should start as a pending command");
    let request_messages = complete_pending_command_task_for_test(&mut ctx, request_pending).await;
    let request_response = request_messages
        .iter()
        .find(|message| message["id"] == json!(10))
        .expect("pending DOM.requestNode should produce a response");
    assert_eq!(request_response["result"]["nodeId"], json!(node_id));

    let outer_raw = json!({
        "id": 11,
        "method": "DOM.getOuterHTML",
        "params": { "objectId": resolved_object_id }
    })
    .to_string();
    let outer_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&outer_raw)
        .expect("DOM.getOuterHTML with objectId should start as a pending command");
    let outer_messages = complete_pending_command_task_for_test(&mut ctx, outer_pending).await;
    let outer_response = outer_messages
        .iter()
        .find(|message| message["id"] == json!(11))
        .expect("pending DOM.getOuterHTML should produce a response");
    assert_eq!(
        outer_response["result"]["outerHTML"],
        json!(
            "<p id=\"target\" style=\"position:absolute;left:10px;top:10px;width:5px;height:5px;margin:0\">box</p>"
        )
    );

    let box_raw = json!({
        "id": 12,
        "method": "DOM.getBoxModel",
        "params": { "objectId": resolved_object_id }
    })
    .to_string();
    let box_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&box_raw)
        .expect("DOM.getBoxModel with objectId should start as a pending command");
    let box_messages = complete_pending_command_task_for_test(&mut ctx, box_pending).await;
    let box_response = box_messages
        .iter()
        .find(|message| message["id"] == json!(12))
        .expect("pending DOM.getBoxModel should produce a response");
    assert_eq!(
        box_response["result"]["model"]["content"],
        axis_aligned_geometry_quad(10.0, 10.0, 5.0, 5.0)
    );

    let quads_raw = json!({
        "id": 13,
        "method": "DOM.getContentQuads",
        "params": { "objectId": resolved_object_id }
    })
    .to_string();
    let quads_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&quads_raw)
        .expect("DOM.getContentQuads with objectId should start as a pending command");
    let quads_messages = complete_pending_command_task_for_test(&mut ctx, quads_pending).await;
    let quads_response = quads_messages
        .iter()
        .find(|message| message["id"] == json!(13))
        .expect("pending DOM.getContentQuads should produce a response");
    assert_eq!(
        quads_response["result"]["quads"],
        json!([axis_aligned_geometry_quad(10.0, 10.0, 5.0, 5.0)])
    );

    let scroll_raw = json!({
        "id": 14,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": resolved_object_id }
    })
    .to_string();
    let scroll_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&scroll_raw)
        .expect("DOM.scrollIntoViewIfNeeded with objectId should start as a pending command");
    let scroll_messages = complete_pending_command_task_for_test(&mut ctx, scroll_pending).await;
    let scroll_response = scroll_messages
        .iter()
        .find(|message| message["id"] == json!(14))
        .expect("pending DOM.scrollIntoViewIfNeeded should produce a response");
    assert_eq!(scroll_response["result"], json!({}));

    ctx.process_async(json!({
        "id": 8,
        "method": "DOM.removeNode",
        "params": { "nodeId": node_id }
    }))
    .await;
    let remove_messages = ctx.take_all();
    let remove_response = remove_messages
        .iter()
        .find(|message| message["id"] == json!(8))
        .expect("pending DOM.removeNode should produce a response");
    assert_eq!(remove_response["result"], json!({}));
    let mutation_position = remove_messages
        .iter()
        .position(|message| {
            message["method"] == json!("DOM.childNodeRemoved")
                && message["params"]["nodeId"] == json!(node_id)
        })
        .expect("pending DOM.removeNode should publish its DOM mutation");
    let response_position = remove_messages
        .iter()
        .position(|message| message["id"] == json!(8))
        .expect("pending DOM.removeNode response position");
    assert!(
        mutation_position < response_position,
        "remove mutation must precede its command response: {remove_messages:?}"
    );
    ctx.process_async(json!({
        "id": 9,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    assert_eq!(take_query_selector_node_id(&mut ctx, 9), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_box_model_supports_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p style='position:absolute;left:12px;top:8px;width:7px;height:6px;margin:0'>box</p></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "p" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(node_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id }
    }))
    .await;
    let backend_node_id = take_response_by_id(&mut ctx, 4)["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("described paragraph backend node id");

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getBoxModel",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    ctx.expect_result(5, axis_aligned_box_model(12.0, 8.0, 7, 6), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_box_model_supports_renderer_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box' style='position:absolute;left:3px;top:4px;width:9px;height:11px'>box</div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let backend_node_id = renderer_backend_node_id_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#box')",
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getBoxModel",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    ctx.expect_result(13, axis_aligned_box_model(3.0, 4.0, 9, 11), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_backend_node_id_geometry_is_scoped_to_document_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='old'>old</div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let backend_node_id = renderer_backend_node_id_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#old')",
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open(); document.write(\"<!doctype html><html><body><div id='fresh'>fresh</div></body></html>\"); document.close(); document.querySelector('#fresh')"
        }
    }))
    .await;
    let fresh = take_response_by_id(&mut ctx, 13);
    assert!(
        fresh["result"]["result"]["objectId"].as_str().is_some(),
        "replacement element should produce a fresh object: {fresh}"
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "DOM.getBoxModel",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    ctx.expect_error(14, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_box_model_supports_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box' style='position:absolute;left:3px;top:4px;width:9px;height:11px'></div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('box')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!object_id.is_empty());

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.getBoxModel",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(12, axis_aligned_box_model(3.0, 4.0, 9, 11), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn object_id_geometry_reads_live_document_after_document_open_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>old text node</body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let old_document = take_response_by_id(&mut ctx, 11);
    assert_eq!(
        old_document["result"]["root"]["nodeName"],
        json!("#document")
    );

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                document.open();
                document.write("<!doctype html><html><body><div id='fresh-geometry' style='position:absolute;left:3px;top:4px;width:9px;height:11px'>fresh</div></body></html>");
                document.close();
                return document.querySelector('#fresh-geometry');
            })()"#
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 12)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("replacement Runtime.evaluate should return objectId"))
        .to_owned();

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getBoxModel",
        "params": { "objectId": object_id.clone() }
    }))
    .await;
    ctx.expect_result(13, axis_aligned_box_model(3.0, 4.0, 9, 11), None);

    ctx.process_async(json!({
        "id": 14,
        "method": "DOM.getContentQuads",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(
        14,
        json!({ "quads": [axis_aligned_geometry_quad(3.0, 4.0, 9.0, 11.0)] }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn object_id_box_model_rejects_text_node_but_content_quads_accepts_it() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>hello</body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.body.firstChild" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 11)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return text node objectId"))
        .to_owned();

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.getBoxModel",
        "params": { "objectId": object_id.clone() }
    }))
    .await;
    ctx.expect_error(12, -32000, "Node is not an element");

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getContentQuads",
        "params": { "objectId": object_id }
    }))
    .await;
    let quads = take_response_by_id(&mut ctx, 13);
    assert_non_empty_geometry_quads(&quads["result"]["quads"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_box_model_rejects_non_element_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>hello</body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "body" }
    }))
    .await;
    let body_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(body_node_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": body_node_id, "depth": 1 }
    }))
    .await;
    let text_node_id = take_response_by_id(&mut ctx, 4)["result"]["node"]["children"][0]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(text_node_id > 0);

    let raw = json!({
        "id": 5,
        "method": "DOM.getBoxModel",
        "params": { "nodeId": text_node_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("non-element DOM.getBoxModel should start a renderer geometry command");
    assert_eq!(pending.kind_name(), "DOM");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(5))
        .unwrap_or_else(|| panic!("pending DOM.getBoxModel should respond: {messages:?}"));
    assert_eq!(
        response["error"],
        json!({ "code": -32000, "message": "Node is not an element" })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_returns_single_quad_for_element() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p style='position:absolute;left:10px;top:10px;width:5px;height:5px;margin:0'>box</p></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "p" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getContentQuads",
        "params": { "nodeId": node_id }
    }))
    .await;
    ctx.expect_result(
        4,
        json!({ "quads": [axis_aligned_geometry_quad(10.0, 10.0, 5.0, 5.0)] }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_supports_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box' style='position:absolute;left:3px;top:4px;width:9px;height:11px'></div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('box')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.getContentQuads",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(
        12,
        json!({ "quads": [axis_aligned_geometry_quad(3.0, 4.0, 9.0, 11.0)] }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_supports_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box' style='position:absolute;left:6px;top:7px;width:8px;height:9px'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#box" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id }
    }))
    .await;
    let backend_node_id = take_response_by_id(&mut ctx, 4)["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("described DIV backend node id");

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getContentQuads",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    let quads = take_response_by_id(&mut ctx, 5);
    assert_non_empty_geometry_quads(&quads["result"]["quads"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_supports_renderer_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box' style='position:absolute;left:6px;top:7px;width:8px;height:9px'>box</div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let backend_node_id = renderer_backend_node_id_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#box')",
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getContentQuads",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    ctx.expect_result(
        13,
        json!({ "quads": [axis_aligned_geometry_quad(6.0, 7.0, 8.0, 9.0)] }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_without_page_errors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.getContentQuads",
        "params": { "nodeId": 1 }
    }))
    .await;
    ctx.expect_error(1, -32000, "NoDocumentLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_invalid_node_errors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getContentQuads",
        "params": { "nodeId": 999 }
    }))
    .await;
    ctx.expect_error(2, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_accepts_connected_rendered_text_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>hello</body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "body" }
    }))
    .await;
    let body_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": body_node_id, "depth": 1 }
    }))
    .await;
    let text_node_id = take_response_by_id(&mut ctx, 4)["result"]["node"]["children"][0]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getContentQuads",
        "params": { "nodeId": text_node_id }
    }))
    .await;
    let quads = take_response_by_id(&mut ctx, 5);
    assert_non_empty_geometry_quads(&quads["result"]["quads"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_invalid_params_error() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.getContentQuads"
    }))
    .await;
    ctx.expect_error(1, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_node_for_location_uses_real_layout_hit_testing() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='behind' style='position:absolute;left:10px;top:10px;width:60px;height:60px'></div><button id='front' style='position:absolute;left:20px;top:20px;width:20px;height:20px'>hit</button></body></html>",
    )
    .await;
    let expected_backend_node_id = renderer_backend_node_id_for_live_expression(
        &mut ctx,
        2,
        3,
        "document.getElementById('front')",
    )
    .await;

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getNodeForLocation",
        "params": { "x": 25, "y": 25 }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 4);
    assert_eq!(
        response["result"]["backendNodeId"],
        json!(expected_backend_node_id)
    );
    assert!(
        response["result"]["nodeId"]
            .as_u64()
            .is_some_and(|id| id > 0)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_node_for_location_mock_policy_uses_compatibility_geometry() {
    let mut ctx = TestContext::new_with_layout_policy(LayoutPolicy::Mock);
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='behind' style='position:absolute;left:10px;top:10px;width:60px;height:60px'></div><button id='front' style='position:absolute;left:20px;top:20px;width:20px;height:20px'>hit</button></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getNodeForLocation",
        "params": { "x": 10, "y": 25 }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 4);
    assert!(
        response["result"]["backendNodeId"]
            .as_u64()
            .is_some_and(|id| id > 0)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_node_for_location_negative_coordinates_report_no_hit() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box' style='position:absolute;left:10px;top:10px;width:10px;height:10px'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getNodeForLocation",
        "params": {
            "x": -50,
            "y": -50,
            "includeUserAgentShadowDOM": true,
            "ignorePointerEventsNone": true
        }
    }))
    .await;

    ctx.expect_error(2, -32000, "No node found at given location");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_accepts_element_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    let rows = "<div>row</div>".repeat(60);
    let url = format!(
        "data:text/html,<!doctype html><html><body>{rows}<div id='box'></div></body></html>"
    );

    navigate_to_url_and_wait_for_load_async(&mut ctx, 1, url).await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#box" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "nodeId": node_id }
    }))
    .await;
    ctx.expect_result(4, json!({}), None);

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": { "expression": "window.scrollY" }
    }))
    .await;
    let scroll = take_response_by_id(&mut ctx, 5)["result"]["result"]["value"]
        .as_f64()
        .unwrap_or_default();
    assert!(scroll > 0.0, "DOM command should update observable scrollY");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_accepts_document_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "nodeId": root_id }
    }))
    .await;
    ctx.expect_result(3, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_accepts_connected_rendered_text_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>hello</body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "body" }
    }))
    .await;
    let body_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": body_node_id, "depth": 1 }
    }))
    .await;
    let text_node_id = take_response_by_id(&mut ctx, 4)["result"]["node"]["children"][0]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "nodeId": text_node_id }
    }))
    .await;
    ctx.expect_result(5, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_without_page_errors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "nodeId": 1 }
    }))
    .await;
    ctx.expect_error(1, -32000, "NoDocumentLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_invalid_node_errors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "nodeId": 999 }
    }))
    .await;
    ctx.expect_error(2, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_supports_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    let rows = "<div>row</div>".repeat(60);
    let url = format!(
        "data:text/html,<!doctype html><html><body>{rows}<div id='box'></div></body></html>"
    );

    navigate_to_url_and_wait_for_load_async(&mut ctx, 1, url).await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('box')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(12, json!({}), None);

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": { "expression": "window.scrollY" }
    }))
    .await;
    let scroll = take_response_by_id(&mut ctx, 13)["result"]["result"]["value"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        scroll > 0.0,
        "objectId path should update observable scrollY"
    );
}

// Ported from Chromium's inspector-protocol/dom/dom-scrollIntoViewIfNeeded.js.
#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_uses_first_rendered_child_of_display_contents() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    let rows = "<div>row</div>".repeat(60);
    let url = format!(
        "data:text/html,<!doctype html><body>{rows}<button id=contents style='display:contents'>target</button></body>"
    );
    navigate_to_url_and_wait_for_load_async(&mut ctx, 30, url).await;

    ctx.process_async(json!({
        "id": 31,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#contents')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 31)["result"]["result"]["objectId"]
        .as_str()
        .expect("display:contents object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 32,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(32, json!({}), None);

    ctx.process_async(json!({
        "id": 33,
        "method": "Runtime.evaluate",
        "params": { "expression": "window.scrollY" }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 33)["result"]["result"]["value"]
            .as_f64()
            .is_some_and(|scroll_y| scroll_y > 0.0),
        "the display:contents element should use its rendered descendant geometry"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_honors_stylesheet_display_and_visibility() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    let rows = "<div>row</div>".repeat(60);
    let url = format!(
        "data:text/html,<!doctype html><style>.contents{{display:contents}}.none{{display:none}}.invisible{{visibility:hidden}}</style><body>{rows}<button id=contents class=contents>contents text</button><div id=none class=none><span>suppressed child</span></div><button id=invisible class=invisible>invisible box</button></body>"
    );
    navigate_to_url_and_wait_for_load_async(&mut ctx, 60, url).await;

    ctx.process_async(json!({
        "id": 67,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "['#contents', '#none', '#invisible'].map(selector => getComputedStyle(document.querySelector(selector)).display + ':' + getComputedStyle(document.querySelector(selector)).visibility).join('|')",
            "returnByValue": true
        }
    }))
    .await;
    let styles = take_response_by_id(&mut ctx, 67);
    assert_eq!(
        styles["result"]["result"]["value"],
        json!("contents:visible|none:visible|inline-block:hidden")
    );

    for (evaluate_id, command_id, selector, should_succeed) in [
        (61, 62, "#contents", true),
        (63, 64, "#none", false),
        (65, 66, "#invisible", true),
    ] {
        ctx.process_async(json!({
            "id": evaluate_id,
            "method": "Runtime.evaluate",
            "params": { "expression": format!("document.querySelector('{selector}')") }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, evaluate_id)["result"]["result"]["objectId"]
            .as_str()
            .expect("element object id")
            .to_owned();
        ctx.process_async(json!({
            "id": command_id,
            "method": "DOM.scrollIntoViewIfNeeded",
            "params": { "objectId": object_id }
        }))
        .await;
        if should_succeed {
            ctx.expect_result(command_id, json!({}), None);
        } else {
            ctx.expect_error(command_id, -32000, "Node does not have a layout object");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_rejects_hidden_input_like_chromium() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        34,
        "<!doctype html><body><input id=hidden-input type=hidden></body>",
    )
    .await;
    ctx.process_async(json!({
        "id": 35,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#hidden-input')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 35)["result"]["result"]["objectId"]
        .as_str()
        .expect("hidden input object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 36,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_error(36, -32000, "Node does not have a layout object");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_distinguishes_detached_node_from_missing_geometry() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        37,
        "<!doctype html><body><div id=target></div></body>",
    )
    .await;
    ctx.process_async(json!({
        "id": 38,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.detachedScrollTarget = document.querySelector('#target'); detachedScrollTarget.remove(); detachedScrollTarget"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 38)["result"]["result"]["objectId"]
        .as_str()
        .expect("detached object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 39,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_error(39, -32000, "Node is detached from document");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_observes_live_style_change_before_scrolling() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        40,
        "<!doctype html><body><div id=spacer style='height:24px'></div><div id=target></div></body>",
    )
    .await;
    ctx.process_async(json!({
        "id": 41,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.querySelector('#spacer').style.height = '2000px'; document.querySelector('#target')"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 41)["result"]["result"]["objectId"]
        .as_str()
        .expect("style-updated target object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 42,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(42, json!({}), None);
    ctx.process_async(json!({
        "id": 43,
        "method": "Runtime.evaluate",
        "params": { "expression": "window.scrollY" }
    }))
    .await;
    assert!(
        take_response_by_id(&mut ctx, 43)["result"]["result"]["value"]
            .as_f64()
            .is_some_and(|scroll_y| scroll_y > 0.0),
        "scroll geometry must observe the style mutation made immediately before the command"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_clamps_relative_rect_to_document_scroll_range() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    let rows = "<div>row</div>".repeat(60);
    let url = format!("data:text/html,<!doctype html><body>{rows}<div id=target></div></body>");
    navigate_to_url_and_wait_for_load_async(&mut ctx, 44, url).await;
    ctx.process_async(json!({
        "id": 45,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#target')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 45)["result"]["result"]["objectId"]
        .as_str()
        .expect("relative rect target object id")
        .to_owned();

    let mut observed = Vec::new();
    for (command_id, y) in [(46, 0), (49, 20)] {
        ctx.process_async(json!({
            "id": command_id,
            "method": "DOM.scrollIntoViewIfNeeded",
            "params": {
                "objectId": object_id.as_str(),
                "rect": { "x": 0, "y": y, "width": 0, "height": 0 }
            }
        }))
        .await;
        ctx.expect_result(command_id, json!({}), None);
        ctx.process_async(json!({
            "id": command_id + 1,
            "method": "Runtime.evaluate",
            "params": { "expression": "window.scrollY" }
        }))
        .await;
        observed.push(
            take_response_by_id(&mut ctx, command_id + 1)["result"]["result"]["value"]
                .as_f64()
                .expect("scrollY"),
        );
        ctx.process_async(json!({
            "id": command_id + 2,
            "method": "Runtime.evaluate",
            "params": { "expression": "window.scrollTo(0, 0)" }
        }))
        .await;
        take_response_by_id(&mut ctx, command_id + 2);
    }
    // Chromium 147 clamps both points to the document's maximum scroll offset:
    // a relative point beyond a zero-height element at the end of the document
    // does not extend the scrollable overflow area.
    assert_eq!(observed[1], observed[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_supports_document_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'></div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 11)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return document objectId"))
        .to_owned();

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(12, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_object_id_reads_live_document_after_document_open_replacement()
{
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>old text node</body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let old_document = take_response_by_id(&mut ctx, 11);
    assert_eq!(
        old_document["result"]["root"]["nodeName"],
        json!("#document")
    );

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                document.open();
                document.write("<!doctype html><html><body><div id='fresh-scroll'>fresh</div></body></html>");
                document.close();
                return document.querySelector('#fresh-scroll');
            })()"#
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 12)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("replacement Runtime.evaluate should return objectId"))
        .to_owned();

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(13, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_object_id_accepts_connected_rendered_text_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>hello</body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.body.firstChild" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 11)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return text node objectId"))
        .to_owned();

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(12, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_supports_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#box" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id }
    }))
    .await;
    let backend_node_id = take_response_by_id(&mut ctx, 4)["result"]["node"]["backendNodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": {
            "backendNodeId": backend_node_id,
            "rect": { "x": 1, "y": 2, "width": 3, "height": 4 }
        }
    }))
    .await;
    ctx.expect_result(5, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_supports_renderer_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>box</div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let backend_node_id = renderer_backend_node_id_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#box')",
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": {
            "backendNodeId": backend_node_id,
            "rect": { "x": 1, "y": 2, "width": 3, "height": 4 }
        }
    }))
    .await;
    ctx.expect_result(13, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_if_needed_invalid_params_error() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.scrollIntoViewIfNeeded"
    }))
    .await;
    ctx.expect_error(1, -32602, "InvalidParams");
}
