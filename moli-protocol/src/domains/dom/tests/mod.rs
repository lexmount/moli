use super::{
    backend_node_id_for_snapshot, frontend_node_id_for_snapshot, node_snapshot_base_payload,
    node_snapshot_to_cdp, node_snapshot_to_cdp_with_limit,
};
use crate::conn::{BrowserContext, CdpCommandTaskStep, PageTargetHost, PendingCdpCommandDispatch};
use crate::testing::TestContext;
use moli_core::page::{RENDERER_BACKEND_NODE_ID_START, is_renderer_backend_node_id};
use moli_page_types::{DocumentNodeSnapshot, DocumentSnapshotNodeId};
use serde_json::{Value, json};

fn load_bc(ctx: &mut TestContext, bc_id: &str) {
    let mut bc = BrowserContext::new(bc_id.into());
    bc.set_active_target_id("TID-1");
    ctx.conn.insert_browser_context(bc);
}

async fn create_about_blank_target_with_initial_document(ctx: &mut TestContext, id: u64) -> String {
    ctx.process_async(json!({
        "id": id,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    ctx.expect_event("Target.targetCreated", None);
    let create_response = take_response_by_id(ctx, id);
    let target_id = create_response["result"]["targetId"]
        .as_str()
        .unwrap_or_else(|| panic!("Target.createTarget should return target id: {create_response}"))
        .to_owned();
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .runtime_slot
            .has_loaded_page(),
        "Target.createTarget should install the initial about:blank page"
    );
    target_id
}

#[tokio::test]
async fn get_document_uses_fresh_initial_document_without_adapter() {
    let mut ctx = TestContext::new();
    let _target_id = create_about_blank_target_with_initial_document(&mut ctx, 30).await;

    let messages = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
        "id": 31,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
        }),
        "DOM.getDocument should observe already-loaded initial document",
    )
    .await;

    assert_eq!(
        messages.len(),
        1,
        "unexpected DOM.getDocument output: {messages:?}"
    );
    let response = &messages[0];
    assert_eq!(response["id"], json!(31));
    assert_eq!(response["result"]["root"]["nodeName"], json!("#document"));
    assert_eq!(
        response["result"]["root"]["documentURL"],
        json!("about:blank")
    );
    assert_tree_backend_node_ids_are_renderer_owned(&response["result"]["root"]);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .runtime_slot
            .has_loaded_page(),
        "Target.createTarget should install the initial about:blank page before DOM.getDocument"
    );
}

#[tokio::test]
async fn get_flattened_document_uses_fresh_initial_document_without_adapter() {
    let mut ctx = TestContext::new();
    let _target_id = create_about_blank_target_with_initial_document(&mut ctx, 32).await;

    ctx.process_async(json!({ "id": 320, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(320, json!({}), None);

    let messages = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
        "id": 33,
        "method": "DOM.getFlattenedDocument",
        "params": { "depth": -1 }
        }),
        "DOM.getFlattenedDocument should observe already-loaded initial document",
    )
    .await;

    assert_eq!(
        messages.len(),
        1,
        "unexpected DOM.getFlattenedDocument output: {messages:?}"
    );
    let response = &messages[0];
    assert_eq!(response["id"], json!(33));
    let nodes = response["result"]["nodes"]
        .as_array()
        .expect("flattened nodes");
    assert_eq!(
        nodes.first().and_then(|node| node["nodeName"].as_str()),
        Some("#document")
    );
    assert_eq!(
        nodes.first().and_then(|node| node["nodeType"].as_u64()),
        Some(9)
    );
    assert_eq!(
        nodes.first().and_then(|node| node["documentURL"].as_str()),
        Some("about:blank")
    );
    assert_node_array_backend_node_ids_are_renderer_owned(&response["result"]["nodes"]);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .runtime_slot
            .has_loaded_page(),
        "Target.createTarget should install the initial about:blank page before DOM.getFlattenedDocument"
    );
}

#[tokio::test]
async fn describe_node_uses_fresh_initial_document_without_adapter() {
    let mut ctx = TestContext::new();
    let _target_id = create_about_blank_target_with_initial_document(&mut ctx, 34).await;

    ctx.process_async(json!({
        "id": 34,
        "method": "DOM.getDocument",
        "params": { "depth": 0 }
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 34)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root frontend node id");
    let messages = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 35,
            "method": "DOM.describeNode",
            "params": {
                "nodeId": root_id,
                "depth": 1
            }
        }),
        "DOM.describeNode should observe already-loaded initial document",
    )
    .await;

    assert_eq!(
        messages.len(),
        1,
        "unexpected DOM.describeNode output: {messages:?}"
    );
    let response = &messages[0];
    assert_eq!(response["id"], json!(35));
    assert_eq!(response["result"]["node"]["nodeName"], json!("#document"));
    assert_eq!(
        response["result"]["node"]["documentURL"],
        json!("about:blank")
    );
    assert_tree_backend_node_ids_are_renderer_owned(&response["result"]["node"]);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .runtime_slot
            .has_loaded_page(),
        "Target.createTarget should install the initial about:blank page before DOM.describeNode"
    );
}

fn take_response_by_id(ctx: &mut TestContext, id: u64) -> Value {
    ctx.sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .map(|position| ctx.sent.remove(position))
        .expect("expected response with matching id")
}

fn take_query_selector_node_id(ctx: &mut TestContext, id: u64) -> u64 {
    let response = take_response_by_id(ctx, id);
    let path_events = ctx.take_all();
    assert!(
        path_events
            .iter()
            .all(|message| message["method"] == json!("DOM.setChildNodes")),
        "unexpected querySelector side effects: {path_events:?}"
    );
    response["result"]["nodeId"]
        .as_u64()
        .expect("querySelector node id")
}

fn axis_aligned_geometry_quad(x: f64, y: f64, width: f64, height: f64) -> Value {
    json!([x, y, x + width, y, x + width, y + height, x, y + height])
}

fn axis_aligned_box_model(x: f64, y: f64, width: i32, height: i32) -> Value {
    let quad = axis_aligned_geometry_quad(x, y, f64::from(width), f64::from(height));
    json!({
        "model": {
            "content": quad,
            "padding": quad,
            "border": quad,
            "margin": quad,
            "width": width,
            "height": height
        }
    })
}

fn assert_non_empty_geometry_quads(value: &Value) {
    let quads = value
        .as_array()
        .expect("DOM.getContentQuads should return a quads array");
    assert!(!quads.is_empty(), "rendered content should have a quad");
    for quad in quads {
        let coordinates = quad.as_array().expect("quad coordinates");
        assert_eq!(coordinates.len(), 8);
        assert!(
            coordinates
                .iter()
                .all(|coordinate| coordinate.as_f64().is_some_and(f64::is_finite))
        );
    }
}

async fn complete_pending_command_task_for_test(
    ctx: &mut TestContext,
    pending: PendingCdpCommandDispatch,
) -> Vec<Value> {
    ctx.complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
        .await
        .0
}

async fn complete_command_dispatch_without_legacy_fallback_for_test(
    ctx: &mut TestContext,
    command: Value,
    _expectation: &str,
) -> Vec<Value> {
    let raw = command.to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    ctx.complete_command_task_step_for_test(step).await.0
}

fn child_element_by_node_name<'a>(node: &'a Value, node_name: &str) -> &'a Value {
    node["children"]
        .as_array()
        .and_then(|children| {
            children.iter().find(|child| {
                child["nodeName"] == json!(node_name) && child["nodeType"] == json!(1)
            })
        })
        .unwrap_or_else(|| panic!("expected child node named {node_name}"))
}

fn node_array_element_by_node_name<'a>(nodes: &'a Value, node_name: &str) -> &'a Value {
    nodes
        .as_array()
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node["nodeName"] == json!(node_name) && node["nodeType"] == json!(1))
        })
        .unwrap_or_else(|| panic!("expected node named {node_name}"))
}

fn node_tree_element_by_node_id(node: &Value, node_id: u64) -> Option<&Value> {
    if node["nodeId"].as_u64() == Some(node_id) {
        return Some(node);
    }
    for child in node["children"].as_array().into_iter().flatten() {
        if let Some(found) = node_tree_element_by_node_id(child, node_id) {
            return Some(found);
        }
    }
    None
}

fn node_array_tree_element_by_node_id(nodes: &Value, node_id: u64) -> Option<&Value> {
    nodes
        .as_array()?
        .iter()
        .find_map(|node| node_tree_element_by_node_id(node, node_id))
}

fn assert_tree_backend_node_ids_are_renderer_owned(node: &Value) {
    let backend_node_id = node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("node should carry u32 backendNodeId: {node}"));
    assert!(
        is_renderer_backend_node_id(backend_node_id),
        "node should use renderer backend id namespace: {node}"
    );
    for child in node["children"].as_array().into_iter().flatten() {
        assert_tree_backend_node_ids_are_renderer_owned(child);
    }
    for shadow_root in node["shadowRoots"].as_array().into_iter().flatten() {
        assert_tree_backend_node_ids_are_renderer_owned(shadow_root);
    }
    for pseudo_element in node["pseudoElements"].as_array().into_iter().flatten() {
        assert_tree_backend_node_ids_are_renderer_owned(pseudo_element);
    }
}

fn assert_node_array_backend_node_ids_are_renderer_owned(nodes: &Value) {
    for node in nodes.as_array().into_iter().flatten() {
        assert_tree_backend_node_ids_are_renderer_owned(node);
    }
}

fn node_array_element_by_attribute<'a>(
    nodes: &'a Value,
    name: &str,
    value: &str,
) -> Option<&'a Value> {
    nodes
        .as_array()?
        .iter()
        .find(|node| node_attribute_value(node, name) == Some(value))
}

fn node_tree_element_by_attribute<'a>(
    node: &'a Value,
    name: &str,
    value: &str,
) -> Option<&'a Value> {
    if node_attribute_value(node, name) == Some(value) {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| node_tree_element_by_attribute(child, name, value))
}

fn html_snapshot(
    node_id: DocumentSnapshotNodeId,
    parent_id: Option<DocumentSnapshotNodeId>,
) -> DocumentNodeSnapshot {
    let frontend_node_id = 10_000 + node_id.encoded();
    let parent_frontend_node_id = parent_id.map(|parent_id| 10_000 + parent_id.encoded());
    DocumentNodeSnapshot {
        node_id,
        parent_id,
        inspector_identity: None,
        inspector_parent_identity: None,
        frontend_node_id: Some(frontend_node_id),
        parent_frontend_node_id,
        backend_node_id: Some(RENDERER_BACKEND_NODE_ID_START + node_id.encoded()),
        frame_id: None,
        node_type: 1,
        node_name: "HTML".to_owned(),
        local_name: "html".to_owned(),
        node_value: String::new(),
        child_count: 0,
        document_url: "about:blank".to_owned(),
        base_url: "about:blank".to_owned(),
        namespace_uri: Some("http://www.w3.org/1999/xhtml".to_owned()),
        attributes: Vec::new(),
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: true,
        has_geometry: false,
        shadow_root_type: None,
        shadow_roots: Vec::new(),
        pseudo_type: None,
        pseudo_elements: Vec::new(),
        associated: None,
        children: Vec::new(),
    }
}

#[test]
fn cdp_node_conversion_requires_renderer_node_id_bindings() {
    let snapshot = html_snapshot(DocumentSnapshotNodeId::new(7), None);
    let frontend_node_id = snapshot.frontend_node_id.expect("test frontend id");
    let backend_node_id = snapshot.backend_node_id.expect("test backend id");

    let node = node_snapshot_to_cdp(&snapshot, Some(snapshot.node_id), Some("FRAME-1"))
        .expect("bound snapshot should produce CDP node");

    assert_eq!(
        frontend_node_id_for_snapshot(&snapshot),
        Some(frontend_node_id)
    );
    assert_eq!(
        backend_node_id_for_snapshot(&snapshot),
        Some(backend_node_id)
    );
    assert_eq!(node["nodeId"], json!(frontend_node_id));
    assert_eq!(node["backendNodeId"], json!(backend_node_id));
}

#[test]
fn cdp_node_conversion_rejects_unbound_snapshot_node() {
    let mut snapshot = html_snapshot(DocumentSnapshotNodeId::new(7), None);
    snapshot.frontend_node_id = None;
    snapshot.backend_node_id = None;

    let node = node_snapshot_to_cdp(&snapshot, Some(snapshot.node_id), Some("FRAME-1"));

    assert_eq!(frontend_node_id_for_snapshot(&snapshot), None);
    assert_eq!(backend_node_id_for_snapshot(&snapshot), None);
    assert!(
        node.is_none(),
        "CDP DOM.Node has required nodeId/backendNodeId fields, so unbound snapshots must not forge one"
    );
}

#[test]
fn cdp_node_conversion_respects_structural_limit_without_recursing_unbounded() {
    let mut root = html_snapshot(DocumentSnapshotNodeId::new(1), None);
    let mut child = html_snapshot(DocumentSnapshotNodeId::new(2), Some(root.node_id));
    child.children.push(html_snapshot(
        DocumentSnapshotNodeId::new(3),
        Some(child.node_id),
    ));
    child.child_count = child.children.len();
    root.children.push(child);
    root.child_count = root.children.len();

    let shallow = node_snapshot_to_cdp_with_limit(&root, Some(root.node_id), Some("FRAME-1"), 0);
    let shallow = shallow.expect("bound root should produce CDP node");
    assert!(shallow.get("children").is_none());

    let one_child_level =
        node_snapshot_to_cdp_with_limit(&root, Some(root.node_id), Some("FRAME-1"), 1)
            .expect("bound root should produce CDP node");
    let children = one_child_level["children"]
        .as_array()
        .expect("one child level should be serialized");
    assert_eq!(children.len(), 1);
    assert_eq!(
        children[0]["nodeId"],
        json!(
            root.children[0]
                .frontend_node_id
                .expect("child frontend id")
        )
    );
    assert!(children[0].get("children").is_none());
}

fn patchright_dom_position_for_backend_node_id(
    node: &Value,
    backend_node_id: u64,
) -> Option<String> {
    fn walk(node: &Value, backend_node_id: u64, current_index: &str) -> Option<String> {
        if node["backendNodeId"].as_u64() == Some(backend_node_id) {
            return Some(current_index.to_owned());
        }

        if let Some(children) = node["children"].as_array() {
            for (index, child) in children.iter().enumerate() {
                let child_index = format!("{current_index}.{index}");
                if let Some(position) = walk(child, backend_node_id, &child_index) {
                    return Some(position);
                }
            }
        }

        if let Some(shadow_roots) = node["shadowRoots"].as_array() {
            for shadow_root in shadow_roots {
                if shadow_root["shadowRootType"] == json!("closed")
                    && let Some(position) = walk(shadow_root, backend_node_id, current_index)
                {
                    return Some(position);
                }

                if let Some(children) = shadow_root["children"].as_array() {
                    for (index, child) in children.iter().enumerate() {
                        let child_index = format!("{current_index}.{index}");
                        if let Some(position) = walk(child, backend_node_id, &child_index) {
                            return Some(position);
                        }
                    }
                }
            }
        }

        None
    }

    walk(node, backend_node_id, "")
}

fn patchright_position_sort_key(position: &str) -> Vec<i64> {
    position
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.parse::<i64>().unwrap_or(-1))
        .collect()
}

fn patchright_collect_closed_shadow_root_backend_ids(node: &Value) -> Vec<u64> {
    fn walk(node: &Value, results: &mut Vec<u64>) {
        if let Some(shadow_roots) = node["shadowRoots"].as_array() {
            for shadow_root in shadow_roots {
                if shadow_root["shadowRootType"] == json!("closed")
                    && let Some(backend_node_id) = shadow_root["backendNodeId"].as_u64()
                {
                    results.push(backend_node_id);
                }
                walk(shadow_root, results);
            }
        }

        if node["nodeName"] != json!("IFRAME")
            && let Some(children) = node["children"].as_array()
        {
            for child in children {
                walk(child, results);
            }
        }
    }

    let mut results = Vec::new();
    walk(node, &mut results);
    results
}

fn patchright_element_id_attr(node: &Value) -> String {
    node["attributes"]
        .as_array()
        .and_then(|attributes| {
            attributes
                .chunks(2)
                .find(|chunk| chunk.first() == Some(&json!("id")) && chunk.get(1).is_some())
                .and_then(|chunk| chunk.get(1))
        })
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn node_attribute_value<'a>(node: &'a Value, name: &str) -> Option<&'a str> {
    node["attributes"]
        .as_array()?
        .chunks(2)
        .find(|chunk| chunk.first() == Some(&json!(name)) && chunk.get(1).is_some())
        .and_then(|chunk| chunk.get(1))
        .and_then(Value::as_str)
}

fn flat_node_by_attribute<'a>(nodes: &'a [Value], name: &str, value: &str) -> &'a Value {
    nodes
        .iter()
        .find(|node| node_attribute_value(node, name) == Some(value))
        .unwrap_or_else(|| panic!("expected flattened node with {name}={value}"))
}

fn interleaved_attributes_to_map(attributes: &Value) -> std::collections::HashMap<String, String> {
    attributes
        .as_array()
        .expect("attributes array")
        .chunks(2)
        .filter_map(|chunk| {
            Some((
                chunk.first()?.as_str()?.to_owned(),
                chunk.get(1)?.as_str()?.to_owned(),
            ))
        })
        .collect()
}

pub(super) async fn navigate_to_data_html_async(ctx: &mut TestContext, id: u64, html: &str) {
    navigate_to_url_and_wait_for_load_async(ctx, id, format!("data:text/html,{html}")).await;
}

async fn navigate_to_url_and_wait_for_load_async(
    ctx: &mut TestContext,
    id: u64,
    url: impl Into<String>,
) {
    ctx.process_async(json!({
        "id": id,
        "method": "Page.navigate",
        "params": {
            "url": url.into()
        }
    }))
    .await;
    let response = take_response_by_id(ctx, id);
    assert_eq!(response["result"]["frameId"], json!("TID-1"));
    let loader_id = response["result"]["loaderId"]
        .as_str()
        .expect("cross-document data navigation loader id")
        .to_owned();
    crate::testing::wait_until_renderer_document_load(ctx, None, "TID-1", &loader_id).await;
    let _ = ctx.take_all();
}

async fn enable_runtime_and_take_execution_context_id_async(ctx: &mut TestContext, id: u64) -> i64 {
    ctx.process_async(json!({"id": id, "method": "Runtime.enable"}))
        .await;
    let response = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .map(|position| ctx.sent.remove(position))
        .expect("expected Runtime.enable response");
    assert_eq!(response["result"], json!({}));
    ctx.sent
        .iter()
        .find(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("Runtime.enable must emit executionContextCreated")
}

async fn child_frame_id_for_single_iframe_async(ctx: &mut TestContext, id: u64) -> String {
    ctx.process_async(json!({"id": id, "method": "Page.getFrameTree"}))
        .await;
    take_response_by_id(ctx, id)["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned()
}

fn child_default_context_id_from_events(ctx: &TestContext, child_frame_id: &str) -> i64 {
    ctx.sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id")
}

mod async_dispatch;
mod frames;
mod geometry;
mod misc;
mod node_ops;
mod search;
mod stack_traces;
