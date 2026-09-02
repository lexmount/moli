use super::*;

async fn document_root_node_id(ctx: &mut TestContext, id: u64, session_id: Option<&str>) -> u64 {
    let mut command = json!({
        "id": id,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    });
    if let Some(session_id) = session_id {
        command["sessionId"] = json!(session_id);
    }
    ctx.process_async(command).await;
    take_response_by_id(ctx, id)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document root frontend node id")
}

async fn query_node_id(
    ctx: &mut TestContext,
    id: u64,
    session_id: Option<&str>,
    root_node_id: u64,
    selector: &str,
) -> u64 {
    let mut command = json!({
        "id": id,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_node_id, "selector": selector }
    });
    if let Some(session_id) = session_id {
        command["sessionId"] = json!(session_id);
    }
    ctx.process_async(command).await;
    take_response_by_id(ctx, id)["result"]["nodeId"]
        .as_u64()
        .unwrap_or_else(|| panic!("querySelector should find {selector}"))
}

async fn set_capture(ctx: &mut TestContext, id: u64, session_id: Option<&str>, enable: bool) {
    let mut command = json!({
        "id": id,
        "method": "DOM.setNodeStackTracesEnabled",
        "params": { "enable": enable }
    });
    if let Some(session_id) = session_id {
        command["sessionId"] = json!(session_id);
    }
    ctx.process_async(command).await;
    let response = take_response_by_id(ctx, id);
    assert_eq!(response["result"], json!({}));
    if let Some(session_id) = session_id {
        assert_eq!(response["sessionId"], json!(session_id));
    }
}

async fn creation_stack(
    ctx: &mut TestContext,
    id: u64,
    session_id: Option<&str>,
    node_id: u64,
) -> Value {
    let mut command = json!({
        "id": id,
        "method": "DOM.getNodeStackTraces",
        "params": { "nodeId": node_id }
    });
    if let Some(session_id) = session_id {
        command["sessionId"] = json!(session_id);
    }
    ctx.process_async(command).await;
    take_response_by_id(ctx, id)["result"].clone()
}

async fn runtime_node_id(
    ctx: &mut TestContext,
    evaluate_id: u64,
    request_id: u64,
    expression: &str,
) -> u64 {
    ctx.process_async(json!({
        "id": evaluate_id,
        "method": "Runtime.evaluate",
        "params": { "expression": expression }
    }))
    .await;
    let object_id = take_response_by_id(ctx, evaluate_id)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return a node object: {expression}"))
        .to_owned();
    ctx.process_async(json!({
        "id": request_id,
        "method": "DOM.requestNode",
        "params": { "objectId": object_id }
    }))
    .await;
    take_response_by_id(ctx, request_id)["result"]["nodeId"]
        .as_u64()
        .expect("DOM.requestNode frontend node id")
}

#[tokio::test(flavor = "multi_thread")]
async fn node_creation_stacks_capture_only_the_enabled_interval() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='before-enable'></div></body></html>",
    )
    .await;
    ctx.process_async(json!({ "id": 2, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(2, json!({}), None);
    let root_node_id = document_root_node_id(&mut ctx, 3, None).await;
    let old_node_id = query_node_id(&mut ctx, 4, None, root_node_id, "#before-enable").await;
    assert_eq!(
        creation_stack(&mut ctx, 5, None, old_node_id).await,
        json!({})
    );

    set_capture(&mut ctx, 6, None, true).await;
    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "function outer(){function inner(){const node=document.createElement('section');node.id='captured';document.body.append(node)}inner()}outer()\n//# sourceURL=dom-node-stack.js",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 7);
    let captured_node_id = query_node_id(&mut ctx, 8, None, root_node_id, "#captured").await;
    let captured = creation_stack(&mut ctx, 9, None, captured_node_id).await;
    let frames = captured["creation"]["callFrames"]
        .as_array()
        .unwrap_or_else(|| panic!("captured node should expose creation frames: {captured:?}"));
    assert!(frames.len() >= 3, "nested evaluation stack: {frames:?}");
    assert_eq!(frames[0]["functionName"], json!("inner"));
    assert_eq!(frames[1]["functionName"], json!("outer"));
    assert_eq!(frames[0]["url"], json!("dom-node-stack.js"));
    assert_eq!(frames[0]["lineNumber"], json!(0));
    assert!(frames[0]["columnNumber"].as_u64().is_some());
    assert!(
        frames[0]["scriptId"]
            .as_str()
            .is_some_and(|script_id| !script_id.is_empty())
    );

    set_capture(&mut ctx, 10, None, false).await;
    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "const node=document.createElement('aside');node.id='after-disable';document.body.append(node)\n//# sourceURL=dom-node-stack.js",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 11);
    let after_node_id = query_node_id(&mut ctx, 12, None, root_node_id, "#after-disable").await;
    assert_eq!(
        creation_stack(&mut ctx, 13, None, after_node_id).await,
        json!({})
    );
    assert_eq!(
        creation_stack(&mut ctx, 14, None, captured_node_id).await,
        captured,
        "disable must not erase creation stacks captured earlier"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn node_creation_stack_capture_is_session_local() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-stack-primary".to_owned());
    navigate_to_data_html_async(&mut ctx, 20, "<!doctype html><html><body></body></html>").await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1" }
    }))
    .await;
    let attached_session_id = take_response_by_id(&mut ctx, 21)["result"]["sessionId"]
        .as_str()
        .expect("attached session id")
        .to_owned();

    for (id, session_id) in [
        (22, "SID-stack-primary"),
        (23, attached_session_id.as_str()),
    ] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.enable"
        }))
        .await;
        let _ = take_response_by_id(&mut ctx, id);
    }
    let primary_root = document_root_node_id(&mut ctx, 24, Some("SID-stack-primary")).await;
    let attached_root = document_root_node_id(&mut ctx, 25, Some(&attached_session_id)).await;

    set_capture(&mut ctx, 26, Some("SID-stack-primary"), true).await;
    ctx.process_async(json!({
        "id": 27,
        "sessionId": "SID-stack-primary",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "const node=document.createElement('main');node.id='primary-only';document.body.append(node)\n//# sourceURL=session-stack.js",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 27);

    let primary_node = query_node_id(
        &mut ctx,
        28,
        Some("SID-stack-primary"),
        primary_root,
        "#primary-only",
    )
    .await;
    let attached_node = query_node_id(
        &mut ctx,
        29,
        Some(&attached_session_id),
        attached_root,
        "#primary-only",
    )
    .await;
    assert!(
        creation_stack(
            &mut ctx,
            30,
            Some("SID-stack-primary"),
            primary_node
        )
        .await["creation"]["callFrames"]
            .as_array()
            .is_some_and(|frames| !frames.is_empty())
    );
    assert_eq!(
        creation_stack(&mut ctx, 31, Some(&attached_session_id), attached_node).await,
        json!({}),
        "a peer session must not inherit another session's capture switch"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_dom_creation_apis_and_fragment_parsing_capture_stacks() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(&mut ctx, 40, "<!doctype html><html><body></body></html>").await;
    ctx.process_async(json!({ "id": 41, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(41, json!({}), None);
    let _ = document_root_node_id(&mut ctx, 42, None).await;
    set_capture(&mut ctx, 43, None, true).await;

    ctx.process_async(json!({
        "id": 44,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "function makeNodes(){const text=document.createTextNode('text');const comment=document.createComment('comment');const fragment=document.createDocumentFragment();const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');const pi=document.createProcessingInstruction('target','data');const container=document.createElement('div');container.innerHTML='<span id=parsed>parsed</span>';globalThis.__stackNodes=[text,comment,fragment,svg,pi,container.firstChild]}makeNodes()\n//# sourceURL=dom-node-factories.js",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 44);

    for index in 0..6_u64 {
        let node_id = runtime_node_id(
            &mut ctx,
            45 + index * 3,
            46 + index * 3,
            &format!("globalThis.__stackNodes[{index}]"),
        )
        .await;
        let stack = creation_stack(&mut ctx, 47 + index * 3, None, node_id).await;
        let frames = stack["creation"]["callFrames"]
            .as_array()
            .unwrap_or_else(|| panic!("factory node {index} should retain its creation stack"));
        assert_eq!(frames[0]["functionName"], json!("makeNodes"));
        assert_eq!(frames[0]["url"], json!("dom-node-factories.js"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_keeps_capture_enabled_and_retains_replacement_node_stacks() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        70,
        "<!doctype html><html><body><main>initial</main></body></html>",
    )
    .await;
    ctx.process_async(json!({ "id": 71, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(71, json!({}), None);
    let _ = document_root_node_id(&mut ctx, 72, None).await;
    set_capture(&mut ctx, 73, None, true).await;

    ctx.process_async(json!({
        "id": 74,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open();document.write('<!doctype html><html><body></body></html>');document.close();function createAfterOpen(){const node=document.createElement('article');node.id='after-open';document.body.append(node)}createAfterOpen()\n//# sourceURL=document-open-stack.js",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 74);
    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "document.open DOMContentLoaded binding refresh",
        |message| message["method"] == json!("DOM.documentUpdated"),
    )
    .await;

    let replacement_root = document_root_node_id(&mut ctx, 75, None).await;
    let replacement_node = query_node_id(&mut ctx, 76, None, replacement_root, "#after-open").await;
    let stack = creation_stack(&mut ctx, 77, None, replacement_node).await;
    let frames = stack["creation"]["callFrames"]
        .as_array()
        .unwrap_or_else(|| panic!("replacement node should retain its creation stack: {stack:?}"));
    assert_eq!(frames[0]["functionName"], json!("createAfterOpen"));
    assert_eq!(frames[0]["url"], json!("document-open-stack.js"));
}
