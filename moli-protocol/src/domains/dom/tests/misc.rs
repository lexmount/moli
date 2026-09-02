use super::*;

#[test]
fn top_frame_id_is_only_injected_for_top_document_html_node() {
    let top_document_node_id = DocumentSnapshotNodeId::new(0);
    let top_html = html_snapshot(DocumentSnapshotNodeId::new(1), Some(top_document_node_id));
    let nested_html = html_snapshot(
        DocumentSnapshotNodeId::new(3),
        Some(DocumentSnapshotNodeId::new(2)),
    );

    let top_payload =
        node_snapshot_base_payload(&top_html, Some(top_document_node_id), Some("TID-1"))
            .expect("bound node should produce CDP payload");
    assert_eq!(top_payload.get("frameId"), Some(&json!("TID-1")));

    let nested_payload =
        node_snapshot_base_payload(&nested_html, Some(top_document_node_id), Some("TID-1"))
            .expect("bound node should produce CDP payload");
    assert!(nested_payload.get("frameId").is_none());

    let unknown_root_payload = node_snapshot_base_payload(&top_html, None, Some("TID-1"))
        .expect("bound node should produce CDP payload");
    assert!(unknown_root_payload.get("frameId").is_none());
}

#[test]
fn child_frame_id_is_read_from_iframe_snapshot_payload() {
    let mut iframe = html_snapshot(
        DocumentSnapshotNodeId::new(4),
        Some(DocumentSnapshotNodeId::new(1)),
    );
    iframe.node_name = "IFRAME".to_owned();
    iframe.local_name = "iframe".to_owned();
    iframe.frame_id = Some("TID-child".to_owned());

    let payload =
        node_snapshot_base_payload(&iframe, Some(DocumentSnapshotNodeId::new(0)), Some("TID-1"))
            .expect("bound node should produce CDP payload");
    assert_eq!(payload.get("frameId"), Some(&json!("TID-child")));
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_returns_empty_result_without_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 1, "method": "DOM.enable"}))
        .await;
    ctx.expect_result(1, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_agent_lifecycle_matches_chromium_errors_and_binding_reset() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        10,
        "<!doctype html><html><body><main id='target'></main></body></html>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({ "id": 11, "method": "DOM.disable" }))
        .await;
    ctx.expect_error(11, -32000, "DOM agent hasn't been enabled");

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.getFlattenedDocument",
        "params": { "depth": -1 }
    }))
    .await;
    ctx.expect_error(12, -32000, "DOM agent hasn't been enabled");

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let first_document = take_response_by_id(&mut ctx, 13);
    let first_root_id = first_document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("first root node id");

    ctx.process_async(json!({ "id": 14, "method": "DOM.disable" }))
        .await;
    ctx.expect_result(14, json!({}), None);
    ctx.process_async(json!({ "id": 15, "method": "DOM.disable" }))
        .await;
    ctx.expect_error(15, -32000, "DOM agent hasn't been enabled");

    ctx.process_async(json!({
        "id": 16,
        "method": "DOM.querySelector",
        "params": { "nodeId": first_root_id, "selector": "#target" }
    }))
    .await;
    ctx.expect_error(16, -32000, "Could not find node with given id");

    ctx.process_async(json!({ "id": 17, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(17, json!({}), None);
    ctx.process_async(json!({ "id": 18, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(18, json!({}), None);
    ctx.process_async(json!({
        "id": 19,
        "method": "DOM.getDocument",
        "params": { "depth": 0 }
    }))
    .await;
    let second_document = take_response_by_id(&mut ctx, 19);
    let second_root_id = second_document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("second root node id");
    assert!(second_root_id > first_root_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_enable_include_whitespace_controls_inspector_tree_projection() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>\n  <div id='first'></div>\n  <div id='second'></div>\n</body></html>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 2);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let children = body["children"].as_array().expect("body children");
    assert_eq!(body["childNodeCount"], json!(2));
    assert_eq!(
        children
            .iter()
            .map(|node| node["nodeName"].as_str().expect("node name"))
            .collect::<Vec<_>>(),
        ["DIV", "DIV"],
        "the default InspectorDOMAgent projection must omit whitespace-only text nodes"
    );

    ctx.process_async(json!({ "id": 3, "method": "DOM.disable" }))
        .await;
    ctx.expect_result(3, json!({}), None);
    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.enable",
        "params": { "includeWhitespace": "all" }
    }))
    .await;
    ctx.expect_result(4, json!({}), None);
    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.enable",
        "params": { "includeWhitespace": "none" }
    }))
    .await;
    ctx.expect_result(5, json!({}), None);
    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 6);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let children = body["children"].as_array().expect("body children");
    assert_eq!(body["childNodeCount"].as_u64(), Some(children.len() as u64));
    assert_eq!(
        children
            .iter()
            .filter(|node| node["nodeName"] == json!("DIV"))
            .count(),
        2
    );
    assert!(
        children
            .iter()
            .any(|node| node["nodeName"] == json!("#text")),
        "includeWhitespace=all must expose indentation text nodes"
    );
    assert!(
        children
            .iter()
            .filter(|node| node["nodeName"] == json!("#text"))
            .all(|node| node["nodeValue"]
                .as_str()
                .is_some_and(|value| value.trim().is_empty())),
        "the fixture's projected text nodes should be indentation only"
    );
    assert_eq!(
        children
            .iter()
            .filter(|node| node["nodeName"] == json!("DIV"))
            .map(|node| node["nodeName"].as_str().expect("node name"))
            .collect::<Vec<_>>(),
        ["DIV", "DIV"],
        "includeWhitespace is fixed by the first DOM.enable call until DOM.disable"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_document_reads_target_loaded_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-A".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><html><body><section id='owned'>background</section></body></html>",
        Some("SID-background"),
    )
    .await;

    ctx.process_async(json!({
        "id": 321,
        "sessionId": "SID-background",
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 321);
    assert_eq!(document["sessionId"], "SID-background");
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document root node id");

    ctx.process_async(json!({
        "id": 322,
        "sessionId": "SID-background",
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#owned" }
    }))
    .await;
    let query = take_response_by_id(&mut ctx, 322);
    assert_eq!(query["sessionId"], "SID-background");
    assert_ne!(query["result"]["nodeId"], json!(0));

    ctx.process_async(json!({
        "id": 323,
        "sessionId": "SID-background",
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": root_id, "depth": 1 }
    }))
    .await;
    ctx.expect_event("DOM.setChildNodes", None);
    ctx.expect_result(323, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 324,
        "sessionId": "SID-background",
        "method": "DOM.getFrameOwner",
        "params": { "frameId": "TID-background" }
    }))
    .await;
    let frame_owner = take_response_by_id(&mut ctx, 324);
    assert_eq!(frame_owner["sessionId"], "SID-background");
    assert_eq!(
        frame_owner["error"],
        json!({
            "code": -32000,
            "message": "Frame with the given id does not belong to the target."
        })
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some("TID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_document_reads_target_inactive_loaded_owner_without_activation() {
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
        "data:text/html,<!doctype html><html><body><section id='owned'>inactive</section></body></html>",
        Some("SID-inactive"),
    )
    .await;

    ctx.process_async(json!({
        "id": 331,
        "sessionId": "SID-inactive",
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 331);
    assert_eq!(document["sessionId"], "SID-inactive");
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document root node id");

    ctx.process_async(json!({
        "id": 332,
        "sessionId": "SID-inactive",
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": root_id, "selector": "section" }
    }))
    .await;
    let query = take_response_by_id(&mut ctx, 332);
    assert_eq!(query["sessionId"], "SID-inactive");
    assert_eq!(query["result"]["nodeIds"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn chromium_query_selector_observable_set_child_nodes_contract() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>
            <div class='testClass' id='firstDiv'></div>
            <div class='testClass' id='secondDiv'></div>
            <div class='testClass'></div>
            <div class='testClass'></div>
            <div class='testClass'></div>
            <div id='depth-1'>
                <div id='depth-2'>
                    <div id='targetDiv'></div>
                </div>
                <div id='targetUncle'>
                    <div id='targetCousin'></div>
                </div>
            </div>
        </body></html>",
    )
    .await;

    ctx.process_async(json!({ "id": 2, "method": "DOM.getDocument" }))
        .await;
    let document = take_response_by_id(&mut ctx, 2);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let body_node_id = body["nodeId"].as_u64().expect("body node id");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": body_node_id, "selector": "div" }
    }))
    .await;
    let first_div_messages = ctx.take_all();
    let first_div_response = first_div_messages.last().expect("querySelector response");
    assert_eq!(first_div_response["id"], json!(3));
    let first_div_node_id = first_div_response["result"]["nodeId"]
        .as_u64()
        .expect("querySelector result node id");
    assert_eq!(first_div_messages.len(), 2);
    let first_children = &first_div_messages[0];
    assert_eq!(first_children["method"], json!("DOM.setChildNodes"));
    assert_eq!(first_children["params"]["parentId"], json!(body_node_id));
    let first_div =
        node_array_tree_element_by_node_id(&first_children["params"]["nodes"], first_div_node_id)
            .expect("querySelector result should be present in setChildNodes payload");
    assert_eq!(node_attribute_value(first_div, "id"), Some("firstDiv"));
    let second_div_from_first_event =
        node_array_element_by_attribute(&first_children["params"]["nodes"], "id", "secondDiv")
            .expect("the first body expansion should expose all body children");
    let second_div_node_id = second_div_from_first_event["nodeId"]
        .as_u64()
        .expect("second div node id");

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": body_node_id, "selector": "div#secondDiv" }
    }))
    .await;
    let second_div_messages = ctx.take_all();
    assert_eq!(second_div_messages.len(), 1);
    let second_div_response = second_div_messages.last().expect("querySelector response");
    assert_eq!(second_div_response["id"], json!(4));
    assert_eq!(
        second_div_response["result"]["nodeId"],
        json!(second_div_node_id)
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": body_node_id, "selector": "div.testClass" }
    }))
    .await;
    let query_all_messages = ctx.take_all();
    assert_eq!(query_all_messages.len(), 1);
    let query_all_response = query_all_messages
        .last()
        .expect("querySelectorAll response");
    assert_eq!(
        query_all_response["result"]["nodeIds"]
            .as_array()
            .expect("nodeIds")
            .len(),
        5
    );

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.querySelector",
        "params": { "nodeId": body_node_id, "selector": "div#targetDiv" }
    }))
    .await;
    let deep_query_messages = ctx.take_all();
    assert_eq!(deep_query_messages.len(), 3);
    assert_eq!(
        deep_query_messages
            .last()
            .expect("deep querySelector response")["id"],
        json!(6)
    );
    let depth_1 =
        node_array_element_by_attribute(&first_children["params"]["nodes"], "id", "depth-1")
            .expect("body expansion should bind depth-1");
    let depth_1_id = depth_1["nodeId"].as_u64().expect("depth-1 node id");
    assert_eq!(
        deep_query_messages[0]["params"]["parentId"],
        json!(depth_1_id)
    );
    let depth_2 = node_array_element_by_attribute(
        &deep_query_messages[0]["params"]["nodes"],
        "id",
        "depth-2",
    )
    .expect("first path event should bind depth-2");
    let depth_2_id = depth_2["nodeId"].as_u64().expect("depth-2 node id");
    assert_eq!(
        deep_query_messages[1]["params"]["parentId"],
        json!(depth_2_id)
    );
    let target = node_array_element_by_attribute(
        &deep_query_messages[1]["params"]["nodes"],
        "id",
        "targetDiv",
    )
    .expect("second path event should expose the query result");
    assert_eq!(deep_query_messages[2]["result"]["nodeId"], target["nodeId"]);
    assert!(
        deep_query_messages[..deep_query_messages.len() - 1]
            .iter()
            .all(|message| {
                node_array_element_by_attribute(&message["params"]["nodes"], "id", "targetCousin")
                    .is_none()
            }),
        "querySelector must not emit DOM.setChildNodes for unrelated cousin parents"
    );

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.querySelector",
        "params": { "nodeId": body_node_id, "selector": "div#targetDiv" }
    }))
    .await;
    let repeated_query_messages = ctx.take_all();
    assert_eq!(repeated_query_messages.len(), 1);
    assert_eq!(repeated_query_messages[0]["id"], json!(7));
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_emits_set_child_nodes_event() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div><p id='a'>x</p></div></body></html>",
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
    let body_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": body_id, "depth": 1 }
    }))
    .await;
    ctx.expect_event(
        "DOM.setChildNodes",
        Some(&json!({
            "parentId": body_id,
            "nodes": [{
                "nodeName": "DIV",
                "nodeType": 1,
                "parentId": body_id,
            }]
        })),
    );
    ctx.expect_result(4, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_backend_ids_resolve_live_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='target'>live child</div></body></html>",
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
    let body_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": body_id, "depth": 1 }
    }))
    .await;
    let set_child_nodes = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("DOM.setChildNodes")
                && message["params"]["parentId"] == json!(body_id)
        })
        .unwrap_or_else(|| panic!("body child-node event should be present: {:?}", ctx.sent));
    let target = set_child_nodes["params"]["nodes"]
        .as_array()
        .and_then(|nodes| {
            nodes.iter().find(|node| {
                node["nodeName"] == json!("DIV") && node["attributes"] == json!(["id", "target"])
            })
        })
        .unwrap_or_else(|| panic!("target DIV should be in child-node payload: {set_child_nodes}"));
    let backend_node_id = target["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("target backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "DOM.requestChildNodes child backend id should be renderer-owned: {target}"
    );
    ctx.expect_result(4, json!({}), None);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.resolveNode",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 5);
    assert_eq!(resolved["result"]["object"]["subtype"], json!("node"));
    assert_eq!(
        resolved["result"]["object"]["className"],
        json!("HTMLDivElement")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_depth_two_includes_grandchildren() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div><p id='a'>x</p></div></body></html>",
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
    let body_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": body_id, "depth": 2 }
    }))
    .await;
    ctx.expect_event(
        "DOM.setChildNodes",
        Some(&json!({
            "parentId": body_id,
            "nodes": [{
                "nodeName": "DIV",
                "children": [{
                    "nodeName": "P"
                }]
            }]
        })),
    );
    ctx.expect_result(4, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn inspector_depth_boundary_includes_an_only_text_child_like_chromium() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><head><title>Example Domain</title></head><body><div id='single'>Only child</div><div id='multiple'>first<span>second</span></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": 3 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 2);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let head = child_element_by_node_name(html, "HEAD");
    let title = child_element_by_node_name(head, "TITLE");
    assert_eq!(title["childNodeCount"], json!(1));
    let title_children = title["children"].as_array().expect("title children");
    assert_eq!(title_children.len(), 1);
    assert_eq!(title_children[0]["nodeName"], json!("#text"));
    assert_eq!(title_children[0]["nodeValue"], json!("Example Domain"));
    let title_id = title["nodeId"].as_u64().expect("title frontend id");

    let body = child_element_by_node_name(html, "BODY");
    let single = node_array_element_by_attribute(&body["children"], "id", "single")
        .expect("single-text DIV in depth-three document");
    let single_children = single["children"].as_array().expect("single DIV children");
    assert_eq!(single_children.len(), 1);
    assert_eq!(single_children[0]["nodeName"], json!("#text"));
    assert_eq!(single_children[0]["nodeValue"], json!("Only child"));
    let multiple = node_array_element_by_attribute(&body["children"], "id", "multiple")
        .expect("multiple-child DIV in depth-three document");
    assert_eq!(multiple["childNodeCount"], json!(2));
    assert!(multiple.get("children").is_none());

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.querySelector('title').appendChild(document.createTextNode(' second'))",
            "returnByValue": true
        }
    }))
    .await;
    let mutation_messages = ctx.take_all();
    assert!(
        mutation_messages.iter().any(|message| {
            message["method"] == json!("DOM.childNodeInserted")
                && message["params"]["parentNodeId"] == json!(title_id)
                && message["params"]["node"]["nodeName"] == json!("#text")
                && message["params"]["node"]["nodeValue"] == json!(" second")
        }),
        "a forced single text child must mark its parent as children-requested: {mutation_messages:?}"
    );

    ctx.process_async(json!({"id": 4, "method": "DOM.getDocument"}))
        .await;
    let default_document = take_response_by_id(&mut ctx, 4);
    let html = child_element_by_node_name(&default_document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let body_id = body["nodeId"].as_u64().expect("body frontend id");
    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": body_id, "depth": 1 }
    }))
    .await;
    let set_child_nodes = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("DOM.setChildNodes")
                && message["params"]["parentId"] == json!(body_id)
        })
        .expect("body child-node publication");
    let single =
        node_array_element_by_attribute(&set_child_nodes["params"]["nodes"], "id", "single")
            .expect("single-text DIV in child-node publication");
    let single_children = single["children"].as_array().expect("single DIV children");
    assert_eq!(single_children.len(), 1);
    assert_eq!(single_children[0]["nodeName"], json!("#text"));
    assert_eq!(single_children[0]["nodeValue"], json!("Only child"));
    ctx.expect_result(5, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_rejects_zero_depth_and_invalid_node() {
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
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": 1, "depth": 0 }
    }))
    .await;
    ctx.expect_error(
        2,
        -32000,
        "Please provide a positive integer as a depth or -1 for entire subtree",
    );

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": 999, "depth": 1 }
    }))
    .await;
    ctx.expect_error(3, -32000, "Could not find node with given id");
}
